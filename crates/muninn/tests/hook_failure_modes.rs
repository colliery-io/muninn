//! Failure-mode integration tests for `muninn hook decide` and the
//! `muninn-cc` PreToolUse hook script.
//!
//! NFR-002 says the hook MUST NEVER block the user's tool call. Every
//! failure path — missing config, bogus provider, no credentials,
//! unparseable stdin, unreachable backend, malformed model output —
//! has to collapse to "exit 0, stdout empty," which Claude Code reads
//! as "allow the original tool unchanged."
//!
//! These tests drive a `muninn hook decide` subprocess with various
//! deliberately-broken environments and assert that contract holds.
//! They also exercise `plugins/muninn-cc/hooks/pre-tool-use.sh` with
//! a stubbed `muninn` on PATH to cover the hook-script side of the
//! same contract.
//!
//! Tests don't run under `cargo test` by default — wall-clock spends
//! and a few of them touching the network make them better suited to
//! the same `angreal test uat` pipeline that runs the real-backend
//! smoke. They're `#[ignore]`'d for that reason. CI can enable them
//! by passing `--ignored` to its test runner.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

// ─────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────

/// Generous outer wall-clock budget. The hook's own internal cap is
/// 500 ms, but a cold subprocess start (cargo's just-built binary,
/// load tokio runtime, parse config) typically dominates that.
const WALL_BUDGET: Duration = Duration::from_secs(10);

/// Stage a `.muninn/config.toml` under a fresh tempdir and return the
/// path to the `.muninn` directory so the binary can be pointed at it
/// via `--config`. The TempDir is leaked because Rust's test harness
/// reclaims temp space at process exit.
fn staged_config_dir(toml: &str) -> PathBuf {
    let tmp = tempfile::tempdir().expect("create tempdir");
    let muninn_dir = tmp.path().join(".muninn");
    std::fs::create_dir_all(&muninn_dir).expect("mkdir .muninn");
    std::fs::write(muninn_dir.join("config.toml"), toml).expect("write config.toml");
    let path = muninn_dir;
    std::mem::forget(tmp);
    path
}

/// Spawn `muninn hook decide` with the given stdin payload and config
/// override, returning (exit_code, stdout, stderr, elapsed).
struct HookRun {
    code: i32,
    stdout: String,
    stderr: String,
    elapsed: Duration,
}

fn run_hook(stdin_payload: &str, config_dir: &Path, env_overrides: &[(&str, &str)]) -> HookRun {
    let exe = env!("CARGO_BIN_EXE_muninn");

    let mut cmd = Command::new(exe);
    cmd.arg("--config")
        .arg(config_dir)
        .args(["hook", "decide"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // Apply env overrides last so they shadow any inherited value
    // (notably OLLAMA_API_KEY in the failure-mode tests that need it
    // to be absent).
    for (k, v) in env_overrides {
        if v.is_empty() {
            cmd.env_remove(k);
        } else {
            cmd.env(k, v);
        }
    }

    let start = Instant::now();
    let mut child = cmd.spawn().expect("spawn muninn hook decide");
    child
        .stdin
        .as_mut()
        .expect("child stdin")
        .write_all(stdin_payload.as_bytes())
        .expect("write stdin");
    let output = child.wait_with_output().expect("wait for child");
    let elapsed = start.elapsed();

    HookRun {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        elapsed,
    }
}

/// Assert the NFR-002 silent-passthrough contract: exit 0, empty
/// stdout, within wall-clock budget.
fn assert_silent_passthrough(label: &str, run: &HookRun) {
    assert_eq!(
        run.code, 0,
        "{label}: expected exit 0, got {}\nstderr: {}",
        run.code, run.stderr
    );
    assert!(
        run.stdout.trim().is_empty(),
        "{label}: expected empty stdout, got {:?}\nstderr: {}",
        run.stdout,
        run.stderr
    );
    assert!(
        run.elapsed < WALL_BUDGET,
        "{label}: took {:?} (budget {WALL_BUDGET:?})",
        run.elapsed
    );
}

// ─────────────────────────────────────────────────────────────────────
// `muninn hook decide` failure paths
// ─────────────────────────────────────────────────────────────────────

/// Provider name that doesn't match any backend → factory bails →
/// passthrough.
#[test]
#[ignore = "failure-mode integration; invoke via `angreal test uat` or `--ignored`"]
fn unknown_provider_falls_through() {
    let cfg = staged_config_dir(
        r#"
[default]
provider = "not-a-real-provider"
model = "doesnt-matter"
"#,
    );
    let run = run_hook(
        r#"{"tool_name":"Grep","tool_input":{"pattern":"x"}}"#,
        &cfg,
        // Clear known credentials so they don't accidentally satisfy
        // the factory's allowlist.
        &[
            ("OLLAMA_API_KEY", ""),
            ("GROQ_API_KEY", ""),
            ("ANTHROPIC_API_KEY", ""),
        ],
    );
    assert_silent_passthrough("unknown_provider", &run);
}

/// Configured provider needs credentials but none are available →
/// factory returns Ok(None) → passthrough.
#[test]
#[ignore = "failure-mode integration; invoke via `angreal test uat` or `--ignored`"]
fn missing_credentials_falls_through() {
    let cfg = staged_config_dir(
        r#"
[default]
provider = "groq"
model = "llama-3.1-8b-instant"
"#,
    );
    let run = run_hook(
        r#"{"tool_name":"Grep","tool_input":{"pattern":"x"}}"#,
        &cfg,
        &[
            ("GROQ_API_KEY", ""),
            ("OLLAMA_API_KEY", ""),
            ("ANTHROPIC_API_KEY", ""),
        ],
    );
    assert_silent_passthrough("missing_credentials", &run);
}

/// stdin isn't valid JSON → parse fails → passthrough.
#[test]
#[ignore = "failure-mode integration; invoke via `angreal test uat` or `--ignored`"]
fn malformed_stdin_falls_through() {
    let cfg = staged_config_dir(
        r#"
[default]
provider = "ollama"
model = "gemma4:31b"
"#,
    );
    // Provide a stub OLLAMA key so credentials don't gate; the parse
    // failure should kick in first.
    let run = run_hook(
        r#"not even close to JSON {{{"#,
        &cfg,
        &[("OLLAMA_API_KEY", "stub-not-used-because-parse-fails-first")],
    );
    assert_silent_passthrough("malformed_stdin", &run);
}

/// Empty stdin → parse fails → passthrough.
#[test]
#[ignore = "failure-mode integration; invoke via `angreal test uat` or `--ignored`"]
fn empty_stdin_falls_through() {
    let cfg = staged_config_dir(
        r#"
[default]
provider = "ollama"
model = "gemma4:31b"
"#,
    );
    let run = run_hook("", &cfg, &[("OLLAMA_API_KEY", "stub")]);
    assert_silent_passthrough("empty_stdin", &run);
}

/// Backend endpoint is unreachable (port 1 on localhost — virtually
/// guaranteed closed) → connection error → passthrough within budget.
#[test]
#[ignore = "failure-mode integration; invoke via `angreal test uat` or `--ignored`"]
fn unreachable_endpoint_falls_through() {
    let cfg = staged_config_dir(
        r#"
[default]
provider = "ollama"
model = "gemma4:31b"

[ollama]
base_url = "http://127.0.0.1:1/v1"
api_key = "anything"
"#,
    );
    let run = run_hook(
        r#"{"tool_name":"Grep","tool_input":{"pattern":"x"}}"#,
        &cfg,
        &[("OLLAMA_API_KEY", "anything")],
    );
    assert_silent_passthrough("unreachable_endpoint", &run);
}

// ─────────────────────────────────────────────────────────────────────
// `pre-tool-use.sh` failure paths
// ─────────────────────────────────────────────────────────────────────

/// Run the muninn-cc PreToolUse shell hook with the given `muninn`
/// stub script body installed on a controlled PATH. Returns (exit,
/// stdout). The hook script's stdin is fed `stdin_payload`.
struct ShellHookRun {
    code: i32,
    stdout: String,
}

fn run_shell_hook(stub_body: Option<&str>, stdin_payload: &str) -> ShellHookRun {
    let plugin_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent() // crates/
        .and_then(|p| p.parent()) // repo root
        .expect("repo root from CARGO_MANIFEST_DIR")
        .join("plugins/muninn-cc");
    let script = plugin_root.join("hooks/pre-tool-use.sh");

    // Build a temp PATH dir. If we have a stub, drop it there as
    // `muninn`. Otherwise the PATH won't contain a `muninn` binary
    // and the script's `command -v` check should bail.
    let tmp = tempfile::tempdir().expect("tempdir");
    if let Some(body) = stub_body {
        let stub_path = tmp.path().join("muninn");
        std::fs::write(&stub_path, body).expect("write stub");
        // Make it executable.
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&stub_path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&stub_path, perms).unwrap();
    }
    let tmp_path = tmp.path().to_path_buf();
    // Leak tempdir for the test's lifetime.
    std::mem::forget(tmp);

    let path_value = format!("{}:/usr/bin:/bin", tmp_path.display());
    let mut child = Command::new("/bin/bash")
        .arg(&script)
        .env_clear()
        .env("PATH", &path_value)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn pre-tool-use.sh");
    child
        .stdin
        .as_mut()
        .expect("child stdin")
        .write_all(stdin_payload.as_bytes())
        .expect("write stdin");
    let out = child.wait_with_output().expect("wait");
    ShellHookRun {
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
    }
}

#[test]
#[ignore = "failure-mode integration; invoke via `angreal test uat` or `--ignored`"]
fn shell_hook_no_muninn_on_path_is_passthrough() {
    let run = run_shell_hook(None, r#"{"tool_name":"Grep"}"#);
    assert_eq!(run.code, 0);
    assert!(
        run.stdout.is_empty(),
        "expected empty stdout, got {:?}",
        run.stdout
    );
}

#[test]
#[ignore = "failure-mode integration; invoke via `angreal test uat` or `--ignored`"]
fn shell_hook_muninn_nonzero_exit_is_passthrough() {
    let run = run_shell_hook(Some("#!/bin/sh\nexit 1\n"), r#"{"tool_name":"Grep"}"#);
    assert_eq!(run.code, 0);
    assert!(
        run.stdout.is_empty(),
        "expected empty stdout, got {:?}",
        run.stdout
    );
}

#[test]
#[ignore = "failure-mode integration; invoke via `angreal test uat` or `--ignored`"]
fn shell_hook_relays_muninn_stdout_unchanged() {
    // The hook script reads its own stdin and `muninn` inherits it.
    // Our stub just echoes back the canonical augment response.
    let stub = r#"#!/bin/sh
cat <<'PAYLOAD'
{"hookSpecificOutput":{"hookEventName":"PreToolUse","additionalContext":"hi"}}
PAYLOAD
"#;
    let run = run_shell_hook(Some(stub), r#"{"tool_name":"Grep"}"#);
    assert_eq!(run.code, 0);
    assert!(
        run.stdout.contains("\"additionalContext\":\"hi\""),
        "expected stub output relayed, got {:?}",
        run.stdout
    );
}

#[test]
#[ignore = "failure-mode integration; invoke via `angreal test uat` or `--ignored`"]
fn shell_hook_muninn_silent_success_is_passthrough() {
    // muninn exits 0 with no stdout (the "decision == passthrough"
    // case). Hook should also exit 0 with no stdout — not a separate
    // failure mode, but worth asserting the boundary.
    let run = run_shell_hook(Some("#!/bin/sh\nexit 0\n"), r#"{"tool_name":"Grep"}"#);
    assert_eq!(run.code, 0);
    assert!(
        run.stdout.is_empty(),
        "expected empty stdout, got {:?}",
        run.stdout
    );
}
