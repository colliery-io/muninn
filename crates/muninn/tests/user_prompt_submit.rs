//! UAT — UserPromptSubmit hook silent-passthrough contract.
//!
//! The UserPromptSubmit design is a two-stage filter:
//! 1. Router decision (cheap LLM call): passthrough vs rlm
//! 2. On rlm: drive recursive exploration on the local model and
//!    inject the result as `additionalContext`, framed as an answer
//!    rather than advisory context.
//!
//! End-to-end exercise of that path requires the hook to reach a
//! test daemon, which currently goes through the repo-scoped socket
//! discovery — so it isn't trivial to drive against an isolated
//! tempdir socket without a hook-socket override. The cross-process
//! happy path will land alongside that override.
//!
//! What this file covers today: `submit_returns_passthrough_when_daemon_unreachable`
//! — sanity check that the hook degrades cleanly when the daemon
//! socket the hook resolves doesn't match a live daemon.

mod common;

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use muninn_rlm::daemon::is_alive;

const ALIVE_TIMEOUT: Duration = Duration::from_secs(15);
const DEAD_TIMEOUT: Duration = Duration::from_secs(10);
const POLL_INTERVAL: Duration = Duration::from_millis(50);

fn muninn_bin() -> &'static str {
    env!("CARGO_BIN_EXE_muninn")
}

fn isolated_socket() -> PathBuf {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("muninn.sock");
    std::mem::forget(dir);
    path
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

fn wait_alive(socket: &std::path::Path) {
    let deadline = Instant::now() + ALIVE_TIMEOUT;
    while Instant::now() < deadline {
        if smol_block(is_alive(socket)) {
            return;
        }
        std::thread::sleep(POLL_INTERVAL);
    }
    panic!(
        "daemon did not come alive at {} within {ALIVE_TIMEOUT:?}",
        socket.display()
    );
}

fn wait_dead(socket: &std::path::Path) {
    let deadline = Instant::now() + DEAD_TIMEOUT;
    while Instant::now() < deadline {
        if !smol_block(is_alive(socket)) {
            return;
        }
        std::thread::sleep(POLL_INTERVAL);
    }
    eprintln!(
        "[uat] daemon still alive at {} after {DEAD_TIMEOUT:?}; continuing anyway",
        socket.display()
    );
}

/// Tiny single-shot block-on for a future. We're in a synchronous
/// `#[test]` so we need to coax our async helpers back into the
/// blocking world. tokio's runtime would be overkill here.
fn smol_block<F: std::future::Future>(f: F) -> F::Output {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    rt.block_on(f)
}

struct DaemonGuard {
    socket: PathBuf,
}

impl DaemonGuard {
    #[allow(clippy::zombie_processes)] // wait()-ed via daemon stop
    fn start_with_config(socket: PathBuf, config_dir: Option<&std::path::Path>) -> Self {
        let mut cmd = Command::new(muninn_bin());
        if let Some(d) = config_dir {
            cmd.args(["--config".as_ref(), d.as_os_str()]);
        }
        let status = cmd
            .args(["daemon", "ensure", "--socket"])
            .arg(&socket)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("daemon ensure");
        assert!(status.success(), "daemon ensure failed: {status:?}");
        wait_alive(&socket);
        Self { socket }
    }
}

impl Drop for DaemonGuard {
    fn drop(&mut self) {
        let _ = Command::new(muninn_bin())
            .args(["daemon", "stop", "--socket"])
            .arg(&self.socket)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        wait_dead(&self.socket);
    }
}

/// Repo root, derived from `CARGO_MANIFEST_DIR` (which points at
/// `crates/muninn/` for this test binary). Used to populate
/// `[project] root` in staged configs so the daemon's filesystem
/// tools grep the real workspace, not the empty tempdir.
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("canonicalize repo root")
}

/// Stage an isolated `.muninn/config.toml` with the given TOML body.
/// Returns the `.muninn/` directory path (suitable for `--config`);
/// leaks the temp dir for the test lifetime. The body should NOT
/// include `[project]` — this helper prepends it so the daemon
/// operates against the real workspace.
fn staged_config_dir(toml_body: &str) -> PathBuf {
    let dir = tempfile::tempdir().expect("tempdir for config");
    let muninn_dir = dir.path().join(".muninn");
    std::fs::create_dir_all(&muninn_dir).expect("mkdir .muninn");
    let root = repo_root();
    let full = format!(
        "[project]\nroot = {root:?}\n\n{toml_body}",
        root = root.display().to_string(),
    );
    std::fs::write(muninn_dir.join("config.toml"), full).expect("write config.toml");
    std::mem::forget(dir);
    muninn_dir
}

/// Default test config: both router and rlm on whatever provider
/// `MUNINN_UAT_PROVIDER` selects (default: ollama). Same stack the
/// routing UAT exercises.
fn isolated_config_dir() -> PathBuf {
    let cfg = format!(
        r#"
{default_block}
{router_block}enabled = true

[rlm]

[budget]
max_tokens = 50000
max_depth = 5
max_tool_calls = 20
max_duration_secs = 120
"#,
        default_block = common::uat_default_config_fragment(),
        router_block = common::uat_router_config_fragment(),
    );
    staged_config_dir(&cfg)
}

/// Spawn `muninn hook submit` with `stdin_payload` on stdin. If
/// `socket` is Some, pin the hook's daemon socket to that path via
/// `MUNINN_HOOK_TEST_SOCKET`. If `config_dir` is Some, pass it as
/// `--config`. Extra `envs` are applied last (so tests can override
/// e.g. `MUNINN_HOOK_DEADLINE_MS`). Returns (exit code, stdout, stderr).
fn run_hook_submit(
    socket: Option<&std::path::Path>,
    config_dir: Option<&std::path::Path>,
    envs: &[(&str, &str)],
    stdin_payload: &str,
) -> (i32, String, String) {
    let mut cmd = Command::new(muninn_bin());
    if let Some(d) = config_dir {
        cmd.args(["--config".as_ref(), d.as_os_str()]);
    }
    cmd.args(["hook", "submit"]);
    if let Some(s) = socket {
        cmd.env("MUNINN_HOOK_TEST_SOCKET", s);
    } else {
        cmd.env_remove("MUNINN_HOOK_TEST_SOCKET");
    }
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn muninn hook submit");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(stdin_payload.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// Hook degrades cleanly when the daemon socket it resolves doesn't
/// match a live daemon — the most common production failure path.
/// Point the hook at an isolated tempdir socket that has nothing
/// listening on it; the `is_alive` probe in `submit_inner` returns
/// false and the hook exits 0 with empty stdout.
#[test]
#[ignore = "UAT — UserPromptSubmit silent-passthrough; invoke via `angreal test uat`"]
fn submit_returns_passthrough_when_daemon_unreachable() {
    let sock = isolated_socket();
    // No DaemonGuard — the socket path is bare. is_alive should fail
    // fast and the hook should degrade to passthrough without ever
    // needing a backend.
    let (code, stdout, stderr) = run_hook_submit(
        Some(&sock),
        None,
        &[],
        r#"{"prompt":"how does the daemon socket path work in this repo?"}"#,
    );
    assert_eq!(code, 0, "hook submit should exit 0; stderr: {stderr}");
    assert!(
        stdout.trim().is_empty(),
        "expected passthrough (empty stdout); got {stdout:?}"
    );
}

/// Floor-case passthrough: `@muninn passthrough` short-circuits the
/// router gate so the hook returns empty stdout without contacting
/// the daemon or any backend. Proves the explicit user override
/// works end-to-end.
#[test]
#[ignore = "UAT — UserPromptSubmit explicit-passthrough; invoke via `angreal test uat`"]
fn submit_honors_explicit_passthrough_marker() {
    let (code, stdout, stderr) = run_hook_submit(
        None,
        None,
        &[],
        r#"{"prompt":"@muninn passthrough just answer normally please"}"#,
    );
    assert_eq!(code, 0, "hook submit should exit 0; stderr: {stderr}");
    assert!(
        stdout.trim().is_empty(),
        "expected passthrough (empty stdout); got {stdout:?}"
    );
}

/// Happy path: daemon alive, RLM produces a FINAL answer, and the
/// hook emits a well-formed `additionalContext` envelope with the
/// answer-shaped framing.
///
/// We force the RLM branch with the `@muninn explore` text trigger
/// rather than relying on the router's LLM judgment — the routing
/// UAT covers "router picks RLM for code-shaped prompts"
/// independently, and we don't want that non-determinism flaking
/// the inject-envelope assertion here.
#[test]
#[ignore = "UAT — UserPromptSubmit happy path; invoke via `angreal test uat`"]
fn submit_injects_answer_envelope_for_code_question() {
    if skip_if_no_backend("submit_injects_answer_envelope_for_code_question") {
        return;
    }
    let sock = isolated_socket();
    let cfg = isolated_config_dir();
    let _daemon = DaemonGuard::start_with_config(sock.clone(), Some(&cfg));

    // Routing UAT data shows gemma4:31b exploration regularly takes
    // 20–55s for prompts of this shape — we're testing the inject
    // envelope, not the production 30s budget, so widen the cap.
    let (code, stdout, stderr) = run_hook_submit(
        Some(&sock),
        Some(&cfg),
        &[
            ("MUNINN_HOOK_DEADLINE_MS", "90000"),
            // Surface backend errors in stderr so the assertions below
            // can distinguish "test infrastructure issue" (quota,
            // 5xx) from "real regression in the hook path."
            ("RUST_LOG", "muninn=debug,muninn_rlm=debug,muninn_core=info"),
        ],
        r#"{"prompt":"@muninn explore which function computes the daemon's repo-scoped socket path and what file is it in?"}"#,
    );
    assert_eq!(code, 0, "hook submit should exit 0; stderr: {stderr}");

    // Backend quota / rate-limit exhaustion is infrastructure, not a
    // regression: NFR-002 says we degrade cleanly, and the empty
    // stdout proves we did. Skip with a loud breadcrumb so it's
    // distinguishable from a real failure.
    if stderr.contains("429") || stderr.contains("weekly usage limit") {
        eprintln!(
            "[uat::happy] backend reported quota/rate-limit; hook degraded cleanly. \
             Skipping inject-envelope assertion. stderr:\n{stderr}"
        );
        return;
    }

    let trimmed = stdout.trim();
    assert!(
        !trimmed.is_empty(),
        "expected an injection envelope; got empty stdout. stderr:\n{stderr}"
    );

    let v: serde_json::Value = serde_json::from_str(trimmed)
        .unwrap_or_else(|e| panic!("stdout is not JSON: {e}: {trimmed:?}"));
    let hso = &v["hookSpecificOutput"];
    assert_eq!(
        hso.get("hookEventName").and_then(|x| x.as_str()),
        Some("UserPromptSubmit"),
        "wrong hookEventName: {v}"
    );
    let ctx = hso
        .get("additionalContext")
        .and_then(|x| x.as_str())
        .unwrap_or_else(|| panic!("missing additionalContext: {v}"));
    assert!(
        ctx.contains("muninn turn-start answer"),
        "additionalContext missing answer-shaped framing: {ctx:?}"
    );
    // The framing should steer the downstream agent toward using
    // muninn's answer as the starting point. Original wording was
    // "Do NOT re-grep" which proved too overbearing in live use; we
    // assert the softer phrasing now, but keep the test so cosmetic
    // refactors can't silently drop the steer entirely.
    assert!(
        ctx.contains("Prefer it as your starting point"),
        "additionalContext missing starting-point steer: {ctx:?}"
    );
    // Also pin the one-shot contract — the agent needs to know the
    // inject doesn't recur unless the user re-triggers, otherwise
    // it might wait for muninn on follow-up turns or duplicate work.
    assert!(
        ctx.contains("one-shot") && ctx.contains("@muninn explore"),
        "additionalContext missing one-shot priming + re-trigger guidance: {ctx:?}"
    );
    // The answer body should at least mention one of the symbols
    // involved in the daemon socket path — proves the RLM produced
    // something substantive rather than a stub. We accept either
    // `socket_path_for_repo` (the canonical core helper) or
    // `resolve_daemon_socket` (the main.rs wrapper that calls it)
    // because different models genuinely pick different correct
    // entry points when asked "where is the repo-scoped socket
    // path computed?"
    let mentions_socket_symbol = ctx.contains("socket_path_for_repo")
        || ctx.contains("resolve_daemon_socket");
    assert!(
        mentions_socket_symbol,
        "answer body missing any socket-path symbol the prompt asked about: {ctx:?}"
    );
    eprintln!("[uat] submit injected envelope ({} bytes)", ctx.len());
}

/// Mid-flight provider error: router picks RLM (forced via the
/// `@muninn explore` trigger so we don't need a working router
/// backend), then the daemon's RLM call reaches a TCP port nothing
/// is listening on. Each attempt fails synchronously with
/// `ECONNREFUSED`; the Ollama backend's retry policy
/// (3 attempts × 500ms backoff) exhausts in ~1.5s, after which the
/// error bubbles out of `submit_inner` and the hook collapses to
/// passthrough. Proves the retry-exhaustion path surfaces as a
/// clean `Err` rather than hanging the budget.
#[test]
#[ignore = "UAT — UserPromptSubmit retry-exhaustion path; invoke via `angreal test uat`"]
fn submit_handles_provider_error_during_rlm() {
    // Bind then immediately drop the listener so the OS frees the
    // port. Any subsequent connect to that port gets ECONNREFUSED.
    // (A small risk of port reuse by another process is acceptable
    // for an `#[ignore]`'d UAT.)
    let dead_port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").expect("bind dead-port probe");
        let p = l.local_addr().expect("listener addr").port();
        drop(l);
        p
    };

    let cfg = staged_config_dir(&format!(
        r#"
[default]
provider = "ollama"
model = "gemma4:31b"

[ollama]
base_url = "http://127.0.0.1:{dead_port}"
api_key = "fake-key-for-test"

[router]
strategy = "llm"
enabled = true

[rlm]

[budget]
max_tokens = 50000
max_depth = 5
max_tool_calls = 20
max_duration_secs = 60
"#
    ));
    let sock = isolated_socket();
    let _daemon = DaemonGuard::start_with_config(sock.clone(), Some(&cfg));

    let start = Instant::now();
    let (code, stdout, stderr) = run_hook_submit(
        Some(&sock),
        Some(&cfg),
        &[],
        r#"{"prompt":"@muninn explore which function computes the daemon's repo-scoped socket path?"}"#,
    );
    let elapsed = start.elapsed();

    assert_eq!(
        code, 0,
        "hook should exit 0 even when RLM provider errors; stderr: {stderr}"
    );
    assert!(
        stdout.trim().is_empty(),
        "expected passthrough when RLM provider errors; got stdout: {stdout:?}"
    );
    // Retry exhaustion should complete in ~1.5s; allow generous
    // headroom for subprocess startup. Hard upper bound well under
    // the 30s outer cap proves the error path bubbles up cleanly
    // instead of being swept up by the timeout backstop.
    assert!(
        elapsed < Duration::from_secs(10),
        "hook took {elapsed:?} — connection-refused retries should exhaust well under 10s"
    );
    eprintln!("[uat] provider-error path: exit 0, empty stdout, elapsed {elapsed:?}");
}

/// Outer-cap backstop: deliberately wedge the daemon's RLM backend
/// by pointing Ollama at a TCP listener that accepts connections
/// and never replies, then shrink the hook deadline via
/// `MUNINN_HOOK_DEADLINE_MS` to 2s. The outer `tokio::time::timeout`
/// in `run_hook_submit` must fire, cancel the inner future, and
/// emit empty stdout — proving NFR-002 holds against a hung
/// backend, not just an erroring one.
#[test]
#[ignore = "UAT — UserPromptSubmit timeout backstop; invoke via `angreal test uat`"]
fn submit_timeout_backstop_fires_on_hung_backend() {
    if skip_if_no_backend("submit_timeout_backstop_fires_on_hung_backend") {
        return;
    }

    // Bind a TCP listener on an ephemeral port that accepts but
    // never writes a response. The thread holds the connection open
    // for the duration of the test, so reqwest waits indefinitely
    // for the HTTP response that never arrives.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind stall listener");
    let port = listener.local_addr().expect("listener addr").port();
    let stall_thread = std::thread::spawn(move || {
        // Accept up to a handful of connections (reqwest may open
        // several over the test lifetime) and hold them. Drop on
        // shutdown so the OS reclaims them.
        let _held: Vec<std::net::TcpStream> = (0..8)
            .filter_map(|_| listener.accept().ok().map(|(s, _)| s))
            .collect();
        // Sleep so the listener thread keeps the sockets alive
        // until the test process exits.
        std::thread::sleep(Duration::from_secs(30));
    });

    let cfg = staged_config_dir(&format!(
        r#"
[default]
provider = "ollama"
model = "gemma4:31b"

[ollama]
base_url = "http://127.0.0.1:{port}"

[router]
strategy = "llm"
enabled = true

[rlm]

[budget]
max_tokens = 50000
max_depth = 5
max_tool_calls = 20
max_duration_secs = 60
"#
    ));
    let sock = isolated_socket();
    let _daemon = DaemonGuard::start_with_config(sock.clone(), Some(&cfg));

    let start = Instant::now();
    let (code, stdout, stderr) = run_hook_submit(
        Some(&sock),
        Some(&cfg),
        &[("MUNINN_HOOK_DEADLINE_MS", "2000")],
        r#"{"prompt":"@muninn explore tell me about the socket path resolution"}"#,
    );
    let elapsed = start.elapsed();

    assert_eq!(
        code, 0,
        "hook should exit 0 on outer-cap timeout; stderr: {stderr}"
    );
    assert!(
        stdout.trim().is_empty(),
        "expected passthrough on timeout; got stdout: {stdout:?}"
    );
    // The deadline is 2s; allow generous headroom for subprocess
    // startup and drop ordering. Hard upper bound is well under the
    // default 30s cap (proving the override is honored) and well
    // under any wait-forever bug (proving the timeout fires at all).
    assert!(
        elapsed < Duration::from_secs(10),
        "outer cap did not fire — hook took {elapsed:?} (deadline = 2s)"
    );
    eprintln!("[uat] timeout-backstop path: exit 0, empty stdout, elapsed {elapsed:?}");

    // Best-effort: the stall thread is detached for the rest of the
    // process lifetime. Don't bother joining.
    drop(stall_thread);
}
