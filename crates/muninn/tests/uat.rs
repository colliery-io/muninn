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

use std::process::Command;

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
