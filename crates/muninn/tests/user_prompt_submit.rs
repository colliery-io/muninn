//! UAT — UserPromptSubmit hook and engine memory plumbing.
//!
//! The current UserPromptSubmit design is a two-stage filter:
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
//! What this file covers today:
//!
//! 1. `daemon_memory_round_trip_via_ipc` — engine-level regression
//!    test that proves `recall_memory` / `record_memory` work through
//!    the daemon IPC boundary. Memory is no longer a v1 user feature,
//!    but the trait methods are wired and exercised here so the
//!    plumbing doesn't bit-rot before v2.
//! 2. `submit_returns_passthrough_when_daemon_unreachable` — sanity
//!    check that the hook degrades cleanly when the daemon socket
//!    the hook resolves doesn't match a live daemon.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use muninn_rlm::daemon::{DaemonClient, is_alive};

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
    let has_any = std::env::var_os("OLLAMA_API_KEY").is_some()
        || std::env::var_os("GROQ_API_KEY").is_some()
        || std::env::var_os("ANTHROPIC_API_KEY").is_some();
    if !has_any {
        eprintln!(
            "[uat::{test}] skipping: no backend credentials in env — run via `angreal test uat`"
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
    fn start(socket: PathBuf) -> Self {
        let status = Command::new(muninn_bin())
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

/// Run `muninn --config <none> hook submit` with the given payload on
/// stdin against an already-running daemon at `socket`. Returns
/// (exit code, stdout, stderr).
fn run_hook_submit(socket: &std::path::Path, stdin_payload: &str) -> (i32, String, String) {
    // Stage a temp `.muninn/config.toml` that points the hook at our
    // test daemon's socket. The hook decides its socket from
    // `config_dir`'s parent (the "repo root"), so we set
    // `project.root` to whatever — what matters is that the hook
    // resolves the daemon socket to the same path we started.
    //
    // To keep this simple we directly override via the daemon
    // discovery path: muninn's hook code uses
    // `socket_path_for_repo(repo_root)`. We can't easily inject our
    // test socket via that path, BUT — the hook `submit_inner`
    // builds its retrieval call against `hook_socket_path()`, which
    // is repo-scoped. So we need to make the hook target a socket
    // we control.
    //
    // The cleanest way for a test: spin up the daemon at the repo-
    // scoped path that the hook would naturally pick, *not* at a
    // custom temp path. Reuse the workspace-root-based default.
    //
    // Override path: pass --config <temp> so the hook computes
    // socket_path_for_repo(<temp parent>) — and we matched that in
    // our own daemon's --socket arg.
    let mut child = Command::new(muninn_bin())
        .args(["hook", "submit"])
        // We *don't* pass --socket here — `muninn hook submit` reads
        // the socket from `hook_socket_path(...)` based on config_dir.
        // Instead, we'll set the env var the hook honors. (For
        // simplicity, set a custom XDG_RUNTIME_DIR so the repo-scoped
        // socket lands at our chosen path.)
        .env("MUNINN_HOOK_TEST_SOCKET", socket)
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

/// Sanity test: the hook resolves a *repo-scoped* socket path that
/// doesn't match the isolated tempdir socket the test daemon binds
/// to — so the daemon-alive probe in `submit_inner` returns false
/// and the hook degrades to passthrough (exit 0, empty stdout).
///
/// This proves the silent-passthrough contract holds when the
/// daemon at the canonical path is absent — which is the most
/// common failure path in production too. The "hook actually fires
/// RLM and injects a preamble" path waits on a hook-socket
/// override that lets tests bring up a daemon at a path the hook
/// will pick.
#[test]
#[ignore = "UAT — UserPromptSubmit silent-passthrough; invoke via `angreal test uat`"]
fn submit_returns_passthrough_when_daemon_unreachable() {
    if skip_if_no_backend("submit_returns_passthrough_when_daemon_unreachable") {
        return;
    }
    let sock = isolated_socket();
    let _daemon = DaemonGuard::start(sock.clone());

    let (code, stdout, stderr) = run_hook_submit(
        &sock,
        r#"{"prompt":"how does the daemon socket path work in this repo?"}"#,
    );
    assert_eq!(code, 0, "hook submit should exit 0; stderr: {stderr}");
    // Empty store → no augment block → passthrough.
    assert!(
        stdout.trim().is_empty(),
        "expected passthrough (empty stdout); got {stdout:?}"
    );
}

/// Trait-level round-trip via the daemon IPC. This is the one we
/// actually run in UAT today, because it avoids the hook-socket
/// override issue: we drive the daemon directly via DaemonClient and
/// exercise the same code path the hook uses (recall_memory through
/// the daemon).
///
/// 1. Bring up a daemon.
/// 2. Connect a DaemonClient.
/// 3. record_memory("setsid is used to detach the daemon child",
///    source="ADR-0003").
/// 4. recall_memory("setsid") — expect 1 hit whose content matches.
/// 5. record_memory again with identical content — idempotent.
/// 6. recall again — still 1 hit.
#[test]
#[ignore = "UAT — UserPromptSubmit; invoke via `angreal test uat`"]
fn daemon_memory_round_trip_via_ipc() {
    if skip_if_no_backend("daemon_memory_round_trip_via_ipc") {
        return;
    }
    let sock = isolated_socket();
    let _daemon = DaemonGuard::start(sock.clone());

    smol_block(async {
        use muninn_core::types::{MemoryItem, MemoryQuery};
        use muninn_rlm::MuninnEngine;

        let client = DaemonClient::connect(&sock).await.expect("connect");

        let initial = client
            .recall_memory(MemoryQuery {
                query: "setsid".into(),
                limit: Some(10),
            })
            .await
            .expect("recall initial");
        assert!(initial.is_empty(), "store should start empty");

        client
            .record_memory(MemoryItem {
                content: "setsid is used to detach the daemon child from the parent's session"
                    .into(),
                source: Some("ADR-0003".into()),
            })
            .await
            .expect("record");

        let hits = client
            .recall_memory(MemoryQuery {
                query: "setsid".into(),
                limit: Some(10),
            })
            .await
            .expect("recall after record");
        assert_eq!(hits.len(), 1, "expected 1 hit, got {hits:?}");
        assert!(hits[0].content.contains("setsid"));

        // Idempotency: same content+source should not duplicate.
        client
            .record_memory(MemoryItem {
                content: "setsid is used to detach the daemon child from the parent's session"
                    .into(),
                source: Some("ADR-0003".into()),
            })
            .await
            .expect("record dup");
        let hits = client
            .recall_memory(MemoryQuery {
                query: "setsid".into(),
                limit: Some(10),
            })
            .await
            .expect("recall after dup");
        assert_eq!(hits.len(), 1, "duplicate record should not duplicate hits");

        eprintln!("[uat] memory round-trip OK: {hits:?}");
    });
}
