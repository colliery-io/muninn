//! UAT — daemon lifecycle round-trip.
//!
//! Exercises the `muninn daemon` machinery against a real OS: Unix
//! socket bind, detached child via `setsid`, PID file handling,
//! SIGTERM-then-SIGKILL escalation in `stop`, and `ensure`
//! auto-respawn after a hard kill.
//!
//! Each test uses a unique socket path under a per-test tempdir so
//! parallel runs don't collide with each other or with the user's
//! real daemon.
//!
//! Gated on `#[ignore]` because:
//! - `muninn daemon start` builds a real engine (needs OLLAMA_API_KEY
//!   or equivalent), so this is UAT-pipeline territory.
//! - Subprocess spawns and SIGKILL aren't appropriate in a default
//!   `cargo test` run.
//!
//! Invoke via `angreal test uat`, which decrypts the secrets bundle
//! and runs all `#[ignore]`'d UAT crates.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const POLL_INTERVAL: Duration = Duration::from_millis(50);
const ALIVE_TIMEOUT: Duration = Duration::from_secs(15);
const DEAD_TIMEOUT: Duration = Duration::from_secs(10);

fn muninn_bin() -> &'static str {
    env!("CARGO_BIN_EXE_muninn")
}

/// Create a unique tempdir + socket path that survives the test
/// (leaked so cleanup happens via the OS at process exit).
fn isolated_socket() -> PathBuf {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("muninn.sock");
    std::mem::forget(dir);
    path
}

/// `muninn daemon status --socket <sock>` — returns "alive" or "dead".
fn status(socket: &std::path::Path) -> String {
    let out = Command::new(muninn_bin())
        .args(["daemon", "status", "--socket"])
        .arg(socket)
        .output()
        .expect("daemon status");
    assert!(
        out.status.success(),
        "daemon status exited {:?}",
        out.status
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Format is "<alive|dead>\t<path>"; we just want the verb.
    stdout
        .split_whitespace()
        .next()
        .expect("status verb")
        .to_string()
}

/// Block until the socket reports `alive` or the timeout fires.
fn wait_alive(socket: &std::path::Path, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if status(socket) == "alive" {
            return;
        }
        std::thread::sleep(POLL_INTERVAL);
    }
    panic!(
        "daemon did not come alive at {} within {timeout:?}",
        socket.display()
    );
}

/// Block until the socket reports `dead` or the timeout fires.
fn wait_dead(socket: &std::path::Path, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if status(socket) == "dead" {
            return;
        }
        std::thread::sleep(POLL_INTERVAL);
    }
    panic!(
        "daemon did not go dead at {} within {timeout:?}",
        socket.display()
    );
}

/// Skip-with-message when credentials aren't around.
fn skip_if_no_backend(test: &str) -> bool {
    let has_any = std::env::var_os("OLLAMA_API_KEY").is_some()
        || std::env::var_os("GROQ_API_KEY").is_some()
        || std::env::var_os("ANTHROPIC_API_KEY").is_some();
    if !has_any {
        eprintln!(
            "[uat::{test}] skipping: no backend credentials in env — \
             run via `angreal test uat`"
        );
        true
    } else {
        false
    }
}

/// Full lifecycle:
///   status=dead → ensure → status=alive → stop → status=dead.
#[test]
#[ignore = "UAT — daemon lifecycle; invoke via `angreal test uat`"]
fn daemon_full_lifecycle() {
    if skip_if_no_backend("daemon_full_lifecycle") {
        return;
    }
    let sock = isolated_socket();
    assert_eq!(status(&sock), "dead", "fresh tempdir should be dead");

    let ensure = Command::new(muninn_bin())
        .args(["daemon", "ensure", "--socket"])
        .arg(&sock)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("daemon ensure");
    assert!(ensure.success(), "daemon ensure exited {ensure:?}");

    wait_alive(&sock, ALIVE_TIMEOUT);
    let pid_path = {
        let mut p = sock.as_os_str().to_owned();
        p.push(".pid");
        PathBuf::from(p)
    };
    assert!(pid_path.exists(), "PID file should exist while alive");

    let stop = Command::new(muninn_bin())
        .args(["daemon", "stop", "--socket"])
        .arg(&sock)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("daemon stop");
    assert!(stop.success(), "daemon stop exited {stop:?}");

    wait_dead(&sock, DEAD_TIMEOUT);
    assert!(!sock.exists(), "socket should be unlinked on stop");
    assert!(!pid_path.exists(), "PID file should be unlinked on stop");
}

/// `ensure` is idempotent — running it twice when alive doesn't error
/// and doesn't restart the daemon.
#[test]
#[ignore = "UAT — daemon lifecycle; invoke via `angreal test uat`"]
fn daemon_ensure_is_idempotent_when_alive() {
    if skip_if_no_backend("daemon_ensure_is_idempotent_when_alive") {
        return;
    }
    let sock = isolated_socket();
    let pid_path: PathBuf = {
        let mut p = sock.as_os_str().to_owned();
        p.push(".pid");
        PathBuf::from(p)
    };

    // First ensure brings it up.
    let _ = Command::new(muninn_bin())
        .args(["daemon", "ensure", "--socket"])
        .arg(&sock)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("first ensure");
    wait_alive(&sock, ALIVE_TIMEOUT);
    let pid_before = std::fs::read_to_string(&pid_path).expect("read pid");

    // Second ensure should be a no-op — no fresh spawn, PID stays.
    let _ = Command::new(muninn_bin())
        .args(["daemon", "ensure", "--socket"])
        .arg(&sock)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("second ensure");
    let pid_after = std::fs::read_to_string(&pid_path).expect("read pid");
    assert_eq!(
        pid_before.trim(),
        pid_after.trim(),
        "ensure-when-alive should not respawn"
    );

    // Cleanup.
    let _ = Command::new(muninn_bin())
        .args(["daemon", "stop", "--socket"])
        .arg(&sock)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    wait_dead(&sock, DEAD_TIMEOUT);
}

/// `stop` on a dead daemon is idempotent — reports the "no PID file"
/// case as a benign no-op (exit 0) rather than erroring.
#[test]
#[ignore = "UAT — daemon lifecycle; invoke via `angreal test uat`"]
fn daemon_stop_is_idempotent_when_dead() {
    if skip_if_no_backend("daemon_stop_is_idempotent_when_dead") {
        return;
    }
    let sock = isolated_socket();
    assert_eq!(status(&sock), "dead");

    let out = Command::new(muninn_bin())
        .args(["daemon", "stop", "--socket"])
        .arg(&sock)
        .output()
        .expect("daemon stop on dead");
    assert!(
        out.status.success(),
        "stop on dead daemon should exit 0 (got {:?}); stderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr),
    );
}

/// Kill the daemon process out-of-band (SIGKILL the PID we wrote) and
/// verify `ensure` brings up a fresh one. Models the
/// "daemon crashed, adapter hits `ensure`" recovery path.
#[test]
#[ignore = "UAT — daemon lifecycle; invoke via `angreal test uat`"]
#[cfg(unix)]
fn daemon_auto_respawn_after_hard_kill() {
    if skip_if_no_backend("daemon_auto_respawn_after_hard_kill") {
        return;
    }
    let sock = isolated_socket();
    let pid_path: PathBuf = {
        let mut p = sock.as_os_str().to_owned();
        p.push(".pid");
        PathBuf::from(p)
    };

    // First ensure.
    let _ = Command::new(muninn_bin())
        .args(["daemon", "ensure", "--socket"])
        .arg(&sock)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("first ensure");
    wait_alive(&sock, ALIVE_TIMEOUT);
    let pid_str = std::fs::read_to_string(&pid_path).expect("pid file");
    let pid: i32 = pid_str.trim().parse().expect("parse pid");
    let original_pid = pid;

    // SIGKILL the daemon directly — no cleanup possible, simulating a
    // crash.
    // SAFETY: standard libc::kill FFI call with a valid signal number.
    let rc = unsafe { libc::kill(pid, libc::SIGKILL) };
    assert_eq!(
        rc,
        0,
        "SIGKILL should succeed; errno {:?}",
        std::io::Error::last_os_error()
    );

    wait_dead(&sock, DEAD_TIMEOUT);

    // Second ensure should spawn a fresh process.
    let _ = Command::new(muninn_bin())
        .args(["daemon", "ensure", "--socket"])
        .arg(&sock)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("respawn ensure");
    wait_alive(&sock, ALIVE_TIMEOUT);
    let new_pid: i32 = std::fs::read_to_string(&pid_path)
        .expect("new pid file")
        .trim()
        .parse()
        .expect("parse new pid");
    assert_ne!(
        new_pid, original_pid,
        "respawned daemon should have a fresh PID (was {original_pid}, still {new_pid})"
    );

    // Cleanup.
    let _ = Command::new(muninn_bin())
        .args(["daemon", "stop", "--socket"])
        .arg(&sock)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    wait_dead(&sock, DEAD_TIMEOUT);
}
