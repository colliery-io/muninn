//! UAT smoke tests against a real LLM backend.
//!
//! Every test in this file is `#[ignore]` — they exist to validate the
//! hook + daemon + provider wiring against the live Ollama Cloud
//! (or whatever provider the user configures), not to run on every
//! `cargo test`. Invoke them via:
//!
//! ```sh
//! angreal test uat
//! ```
//!
//! which decrypts `tests/secrets/uat.enc.yaml` via `sops exec-env` and
//! exposes the keys as env vars to the test runner. If the encrypted
//! bundle doesn't exist, the task falls back to whatever's already in
//! the shell env — fine for ad-hoc local runs.
//!
//! Tests skip cleanly when the required credentials are missing so a
//! plain `cargo test -- --ignored` doesn't fail noisily on a fresh
//! checkout.

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Return true and print a friendly skip line when the required env
/// var isn't set; the caller can early-return.
fn skip_if_missing(var: &str, test_name: &str) -> bool {
    if std::env::var_os(var).is_none() {
        eprintln!("[uat::{test_name}] skipping: {var} not set — run via `angreal test uat`");
        true
    } else {
        false
    }
}

/// Drive `muninn hook decide` end-to-end against the real default
/// backend (Ollama Cloud + `gemma4:31b` from the tiered config).
///
/// Asserts:
/// - the binary exits 0 within the hook's 500 ms internal budget +
///   a generous wall-clock cushion for cold start,
/// - stdout is either empty (passthrough) or valid JSON with the
///   expected `hookSpecificOutput` envelope (augment).
///
/// The decision-model's actual choice (passthrough vs. augment) is
/// non-deterministic, so we don't assert *which* path it took — only
/// that the response shape is well-formed.
#[test]
#[ignore = "UAT — runs against real LLM; invoke via `angreal test uat`"]
fn hook_decide_against_real_default_backend() {
    if skip_if_missing("OLLAMA_API_KEY", "hook_decide_against_real_default_backend") {
        return;
    }

    // CARGO_BIN_EXE_<name> points at the just-built binary inside the
    // same package — only available because this test lives in
    // crates/muninn/tests/.
    let exe = env!("CARGO_BIN_EXE_muninn");

    // Realistic CC PreToolUse payload.
    let stdin_payload = serde_json::json!({
        "session_id": "uat-session",
        "transcript_path": "/dev/null",
        "tool_name": "Grep",
        "tool_input": {
            "pattern": "fn main",
            "path": "crates/muninn-core/src"
        }
    })
    .to_string();

    let start = Instant::now();
    let mut child = Command::new(exe)
        .args(["hook", "decide"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn muninn hook decide");

    child
        .stdin
        .as_mut()
        .expect("hook subprocess stdin")
        .write_all(stdin_payload.as_bytes())
        .expect("write hook input");

    let output = child
        .wait_with_output()
        .expect("wait for muninn hook decide");
    let elapsed = start.elapsed();

    // The binary should never exit non-zero from the hook path — NFR-002
    // says every failure collapses to silent passthrough (exit 0, empty
    // stdout).
    assert!(
        output.status.success(),
        "muninn hook decide exited {:?}; stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr),
    );

    // Wall-clock bound: 500 ms internal cap + generous cushion for
    // cold start + sops exec-env wrapping. If we blow past this we
    // either hit a network hang (network bug, escalate) or the timeout
    // didn't fire (logic bug — fix decide_inner).
    let wall_budget = Duration::from_secs(10);
    assert!(
        elapsed < wall_budget,
        "muninn hook decide took {elapsed:?} (budget {wall_budget:?})"
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    if stdout.trim().is_empty() {
        // Passthrough — most common decision in v1.
        eprintln!("[uat] hook decide chose passthrough in {elapsed:?}");
    } else {
        // Augment — verify the response envelope is valid JSON with
        // the expected shape.
        let v: serde_json::Value = serde_json::from_str(stdout.trim())
            .unwrap_or_else(|e| panic!("hook decide stdout is not JSON: {e}: {stdout:?}"));
        assert!(
            v.get("hookSpecificOutput").is_some(),
            "augment response missing hookSpecificOutput: {v}"
        );
        let hso = &v["hookSpecificOutput"];
        assert_eq!(
            hso.get("hookEventName").and_then(|x| x.as_str()),
            Some("PreToolUse"),
        );
        assert!(
            hso.get("additionalContext").is_some(),
            "augment response missing additionalContext: {v}"
        );
        eprintln!(
            "[uat] hook decide chose augment in {elapsed:?}: {}",
            hso.get("additionalContext")
                .and_then(|x| x.as_str())
                .unwrap_or("(non-string context)")
        );
    }
}

/// Probe that the binary's CLI surface is intact — runs
/// `muninn --help` and verifies the new `hook` and `mcp` subcommands
/// are advertised. Doesn't need network access.
///
/// Marked `#[ignore]` so it rides alongside the rest of the UAT
/// suite, but it's also useful as a sanity check that the binary
/// builds with all expected features enabled.
#[test]
#[ignore = "UAT — runs the muninn binary; invoke via `angreal test uat`"]
fn cli_advertises_hook_and_mcp_subcommands() {
    let exe = env!("CARGO_BIN_EXE_muninn");
    let output = Command::new(exe)
        .arg("--help")
        .output()
        .expect("invoke muninn --help");
    assert!(
        output.status.success(),
        "muninn --help exited {:?}",
        output.status
    );
    let help = String::from_utf8_lossy(&output.stdout);
    for sub in &["hook", "mcp", "daemon"] {
        assert!(
            help.contains(sub),
            "muninn --help does not advertise the {sub} subcommand:\n{help}"
        );
    }
}
