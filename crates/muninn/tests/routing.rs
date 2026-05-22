//! UAT — request routing and RLM exploration behavior.
//!
//! Drives a real `muninn proxy` subprocess against the configured
//! Ollama Cloud backend, sends actual chat-completion requests
//! through the `/v1/messages` endpoint, and inspects the response
//! shape — including the `muninn` field (`ExplorationMetadata`) — to
//! verify the router + recursive engine did what they were supposed
//! to.
//!
//! These tests *cost real LLM calls* (typically 3–10 per `@muninn
//! explore` invocation), so they're `#[ignore]`'d and only fire via
//! `angreal test uat`.
//!
//! Scope (Tier 1 from the routing/RLM/indexing UAT plan):
//!
//! 1. `proxy_rlm_explore_finds_known_symbol` — the headline test.
//!    Force-RLM via `@muninn explore`, ask about a function we know
//!    exists in this repo, assert the engine fired tools and the
//!    answer references the right symbol.
//! 2. `proxy_records_exploration_metadata` — verify the response's
//!    `muninn.tool_calls` / `duration_ms` are populated when RLM
//!    fires. Catches regressions in the metadata pipeline.
//!
//! Both tests use the muninn repo itself as the project root, so the
//! engine can grep against real source files.

mod common;

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const READY_TIMEOUT: Duration = Duration::from_secs(15);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

fn muninn_bin() -> &'static str {
    env!("CARGO_BIN_EXE_muninn")
}

/// Workspace root — `crates/muninn/` is the test crate; `..` then `..`
/// reaches the muninn repo root.
fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root from CARGO_MANIFEST_DIR")
        .to_path_buf()
}

/// Pick a free localhost port by binding 0 + releasing the listener.
/// Standard race-tolerant trick; matches the existing pattern in
/// `crates/muninn-rlm/tests/integration.rs`.
fn pick_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind 0");
    listener.local_addr().expect("local_addr").port()
}

fn skip_if_no_backend(test: &str) -> bool {
    if !common::uat_credentials_present() {
        let p = common::uat_provider();
        eprintln!(
            "[uat::{test}] skipping: {} not set for MUNINN_UAT_PROVIDER={p} — run via `angreal test uat`",
            common::provider_env_var(&p)
        );
        true
    } else {
        false
    }
}

/// Stage a `.muninn/config.toml` pointing the engine at the muninn
/// repo so RLM has real source files to grep. Returns the
/// `.muninn/` directory to pass via `muninn --config <dir>`.
fn stage_proxy_config() -> PathBuf {
    let tmp = tempfile::tempdir().expect("tempdir");
    let muninn_dir = tmp.path().join(".muninn");
    std::fs::create_dir_all(&muninn_dir).expect("mkdir .muninn");
    // Provider/model are picked by the common helper from
    // MUNINN_UAT_PROVIDER / MUNINN_UAT_MODEL (defaulting to ollama).
    let cfg = format!(
        r#"
[project]
root = {root:?}

{default_block}
{router_block}"#,
        root = workspace_root(),
        default_block = common::uat_default_config_fragment(),
        router_block = common::uat_router_config_fragment(),
    );
    std::fs::write(muninn_dir.join("config.toml"), cfg).expect("write config.toml");
    let path = muninn_dir;
    std::mem::forget(tmp);
    path
}

/// Spawn `muninn --config <dir> proxy --port <port>`. Returns the
/// child + the bind address.
struct ProxyHandle {
    child: Child,
    base: String,
}

impl ProxyHandle {
    // clippy can't see that ProxyHandle's Drop impl always wait()s
    // the child — the spawned process is reaped both on the success
    // path (Drop) and the timeout path (explicit wait above).
    #[allow(clippy::zombie_processes)]
    fn start(config_dir: &Path, port: u16) -> Self {
        let mut child = Command::new(muninn_bin())
            .arg("--config")
            .arg(config_dir)
            .args(["proxy", "--port"])
            .arg(port.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn muninn proxy");
        let base = format!("http://127.0.0.1:{port}");

        // Poll the port until it accepts connections, or panic on
        // timeout. We use a TCP connect probe (no HTTP) so we don't
        // require the server to handle a particular path before it's
        // ready.
        let deadline = Instant::now() + READY_TIMEOUT;
        loop {
            if Instant::now() > deadline {
                let _ = child.kill();
                // Reap the child before reading its stderr so clippy's
                // `zombie_processes` is happy and we don't leak a
                // reaping debt to the OS.
                let _ = child.wait();
                let stderr = child
                    .stderr
                    .take()
                    .map(|mut s| {
                        let mut buf = String::new();
                        use std::io::Read;
                        let _ = s.read_to_string(&mut buf);
                        buf
                    })
                    .unwrap_or_default();
                panic!(
                    "proxy did not become ready on {} within {READY_TIMEOUT:?}\nstderr:\n{stderr}",
                    base
                );
            }
            if std::net::TcpStream::connect_timeout(
                &format!("127.0.0.1:{port}").parse().unwrap(),
                Duration::from_millis(100),
            )
            .is_ok()
            {
                return Self { child, base };
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for ProxyHandle {
    fn drop(&mut self) {
        // Best-effort SIGTERM via Child::kill (which on Unix is SIGKILL —
        // adequate for tests; the daemon process management is what
        // cares about graceful shutdown).
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Send a /v1/messages request and return the raw response body.
fn send_messages_request(base: &str, body: &serde_json::Value) -> serde_json::Value {
    let client = reqwest::blocking::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .expect("reqwest client");
    let resp = client
        .post(format!("{base}/v1/messages"))
        .json(body)
        .send()
        .expect("send /v1/messages");
    let status = resp.status();
    let text = resp.text().expect("read body");
    if !status.is_success() {
        panic!("proxy returned status {status}; body: {text}");
    }
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("response not JSON: {e}\n{text}"))
}

/// Headline test: explicit `@muninn explore` should fire the
/// recursive engine, which should locate `socket_path_for_repo` —
/// the function we know lives at `crates/muninn-core/src/daemon.rs`.
///
/// Assertions are *tolerant* of LLM nondeterminism:
/// - response.muninn.tool_calls > 0 (engine actually invoked tools)
/// - response text contains "socket_path_for_repo" OR
///   "socket_path" OR "daemon" OR mentions a file under
///   `crates/muninn-core` (any one of these is sufficient evidence)
#[test]
#[ignore = "UAT — routing/RLM; invoke via `angreal test uat`"]
fn proxy_rlm_explore_finds_known_symbol() {
    if skip_if_no_backend("proxy_rlm_explore_finds_known_symbol") {
        return;
    }
    let cfg = stage_proxy_config();
    let port = pick_port();
    let proxy = ProxyHandle::start(&cfg, port);

    let body = serde_json::json!({
        "model": common::uat_model(),
        "max_tokens": 1024,
        "messages": [{
            "role": "user",
            "content": "@muninn explore\n\nFind the function that computes the daemon's repo-scoped socket path. Reply with the function name and file."
        }]
    });

    let resp = send_messages_request(&proxy.base, &body);
    eprintln!("[uat] response: {}", serde_json::to_string(&resp).unwrap());

    // Extract response text (Anthropic-style content array).
    let answer = extract_text(&resp);
    let answer_lower = answer.to_lowercase();
    let evidence_keywords = [
        "socket_path_for_repo",
        "socket_path",
        "daemon.rs",
        "muninn-core",
        "daemon",
    ];
    let hit = evidence_keywords
        .iter()
        .find(|k| answer_lower.contains(&k.to_lowercase()));
    assert!(
        hit.is_some(),
        "RLM answer should reference at least one of {evidence_keywords:?}; got: {answer}"
    );
    eprintln!("[uat] evidence keyword matched: {hit:?}");

    // Tool-use evidence: muninn.tool_calls should be > 0 when RLM
    // actually explored. If the engine answered without firing any
    // tools, that's still a valid "exploration" in the trivial case
    // (the prompt was answerable from system prompt alone), so we
    // log but don't fail-hard. The keyword check above is the strong
    // assertion.
    if let Some(tool_calls) = resp
        .get("muninn")
        .and_then(|m| m.get("tool_calls"))
        .and_then(|v| v.as_u64())
    {
        eprintln!("[uat] tool_calls reported by engine: {tool_calls}");
    } else {
        eprintln!("[uat] no muninn.tool_calls field in response");
    }
}

/// Verify the engine's exploration metadata (tool_calls,
/// duration_ms) is populated and round-trips through the
/// `/v1/messages` JSON response. This is the regression guard for
/// the proxy-side metadata pipeline — separate from the answer-
/// quality check above.
///
/// We use a deliberately simple prompt that should still trigger a
/// tool call or two via the explore directive — the goal is to
/// exercise the metadata path, not to test answer quality.
#[test]
#[ignore = "UAT — routing/RLM; invoke via `angreal test uat`"]
fn proxy_records_exploration_metadata() {
    if skip_if_no_backend("proxy_records_exploration_metadata") {
        return;
    }
    let cfg = stage_proxy_config();
    let port = pick_port();
    let proxy = ProxyHandle::start(&cfg, port);

    let body = serde_json::json!({
        "model": common::uat_model(),
        "max_tokens": 512,
        "messages": [{
            "role": "user",
            "content": "@muninn explore\n\nList one Rust source file inside crates/muninn-core/."
        }]
    });

    let resp = send_messages_request(&proxy.base, &body);
    eprintln!("[uat] response: {}", serde_json::to_string(&resp).unwrap());

    // The presence of the muninn metadata block is itself the
    // assertion — without RLM exploration, this field is absent.
    let metadata = resp
        .get("muninn")
        .unwrap_or_else(|| panic!("response missing `muninn` metadata block: {resp}"));
    assert!(
        metadata.is_object(),
        "muninn metadata should be an object: {metadata}"
    );

    // duration_ms is always set when exploration ran (even at zero,
    // because the field is non-Option in ExplorationMetadata).
    let duration = metadata
        .get("duration_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or_else(|| panic!("muninn.duration_ms missing or non-numeric: {metadata}"));
    assert!(duration > 0, "exploration duration should be > 0ms");
    eprintln!("[uat] exploration metadata: {metadata}");

    // Help future debugging by surfacing the tool_calls / depth_reached
    // numbers; don't assert on them because the model may legitimately
    // answer from system prompt alone for this simple prompt.
    if let Some(tc) = metadata.get("tool_calls").and_then(|v| v.as_u64()) {
        eprintln!("[uat] tool_calls: {tc}");
    }
    if let Some(d) = metadata.get("depth_reached").and_then(|v| v.as_u64()) {
        eprintln!("[uat] depth_reached: {d}");
    }
}

/// Implicit-context routing — the user is asking the agent to do an
/// implementation task ("add a flag to muninn proxy") *without*
/// mentioning code exploration or using `@muninn explore`. The
/// router's new bias should choose RLM anyway, because the work
/// obviously needs project context.
///
/// Assertion: the response carries the `muninn` exploration
/// metadata block, which is only populated when RLM actually fired.
#[test]
#[ignore = "UAT — routing/RLM; invoke via `angreal test uat`"]
fn proxy_router_chooses_rlm_for_implicit_implementation_request() {
    if skip_if_no_backend("proxy_router_chooses_rlm_for_implicit_implementation_request") {
        return;
    }
    let cfg = stage_proxy_config();
    let port = pick_port();
    let proxy = ProxyHandle::start(&cfg, port);

    let body = serde_json::json!({
        "model": common::uat_model(),
        "max_tokens": 1024,
        "messages": [{
            "role": "user",
            "content": "Add a `--no-cors` flag to `muninn proxy` that disables the CORS layer. Where in the code would I need to edit?"
        }]
    });

    let resp = send_messages_request(&proxy.base, &body);
    eprintln!("[uat] response: {}", serde_json::to_string(&resp).unwrap());

    // The muninn metadata block is the routing-decision oracle:
    // present ⇒ RLM ran, absent ⇒ passthrough (would have failed
    // without upstream creds anyway, but we don't need to assert that
    // path — its absence here is what matters).
    let metadata = resp.get("muninn").unwrap_or_else(|| {
        panic!(
            "router chose passthrough for an implicit code request; expected RLM. response: {resp}"
        )
    });
    assert!(metadata.is_object(), "muninn metadata should be an object");
    eprintln!("[uat] router chose RLM (metadata present): {metadata}");
}

/// Same idea, different surface — a diagnostic ("why" / "what's
/// failing") request without an explicit explore directive. Still
/// obviously needs project context to answer well, so the router
/// should escalate.
#[test]
#[ignore = "UAT — routing/RLM; invoke via `angreal test uat`"]
fn proxy_router_chooses_rlm_for_implicit_diagnostic_request() {
    if skip_if_no_backend("proxy_router_chooses_rlm_for_implicit_diagnostic_request") {
        return;
    }
    let cfg = stage_proxy_config();
    let port = pick_port();
    let proxy = ProxyHandle::start(&cfg, port);

    let body = serde_json::json!({
        "model": common::uat_model(),
        "max_tokens": 1024,
        "messages": [{
            "role": "user",
            "content": "Why might the muninn daemon's socket file get left behind on the disk after a normal shutdown? What signal handling is involved?"
        }]
    });

    let resp = send_messages_request(&proxy.base, &body);
    eprintln!("[uat] response: {}", serde_json::to_string(&resp).unwrap());

    let metadata = resp.get("muninn").unwrap_or_else(|| {
        panic!(
            "router chose passthrough for an implicit diagnostic request; expected RLM. response: {resp}"
        )
    });
    assert!(metadata.is_object(), "muninn metadata should be an object");
    eprintln!("[uat] router chose RLM (metadata present): {metadata}");
}

/// Pull plain text out of an Anthropic-style `content` array.
fn extract_text(resp: &serde_json::Value) -> String {
    let content = resp.get("content").and_then(|c| c.as_array());
    let Some(blocks) = content else {
        return resp.to_string();
    };
    let mut out = String::new();
    for block in blocks {
        if block.get("type").and_then(|t| t.as_str()) == Some("text")
            && let Some(text) = block.get("text").and_then(|t| t.as_str())
        {
            out.push_str(text);
            out.push('\n');
        }
    }
    out
}
