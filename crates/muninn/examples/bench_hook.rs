//! Hook latency benchmark — PROJEC-T-0074.
//!
//! Spawns `muninn hook decide` N times against the configured backend
//! and reports end-to-end wall-clock percentiles. Measures the same
//! thing Claude Code measures when its PreToolUse hook fires: total
//! subprocess time, including binary start-up + tokio runtime + LLM
//! call + decision parsing.
//!
//! Output is a single JSON object on stdout so CI / regression
//! tracking can diff runs across commits. Designed to be invoked via
//! `angreal bench hook` (which wraps with `sops exec-env` so
//! credentials are loaded from `tests/secrets/uat.enc.yaml`).
//!
//! ## Usage
//!
//! ```sh
//! cargo run --release --example bench_hook -- --iters 100
//! ```
//!
//! ## Notes
//!
//! - Default 100 iterations. Override via `--iters N`.
//! - First iteration is treated as a warm-up and recorded separately so
//!   the steady-state percentiles aren't polluted by cargo's
//!   first-run filesystem cache misses or the LLM's cold prompt cache.
//! - Failures (non-zero exit, decode error) are counted but the test
//!   doesn't abort — we want to see whether NFR-002 silent-passthrough
//!   holds under repeated load.

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const CANNED_PAYLOAD: &str = r#"{
    "session_id": "bench",
    "transcript_path": "/dev/null",
    "tool_name": "Grep",
    "tool_input": {"pattern": "fn main", "path": "crates/muninn-core/src"}
}"#;

#[derive(Debug, Clone, Copy)]
struct Args {
    iters: usize,
    warmup: usize,
}

fn parse_args() -> Args {
    let mut iters = 100usize;
    let mut warmup = 1usize;
    let mut argv = std::env::args().skip(1);
    while let Some(a) = argv.next() {
        match a.as_str() {
            "--iters" => iters = argv.next().expect("--iters needs N").parse().expect("N"),
            "--warmup" => warmup = argv.next().expect("--warmup needs N").parse().expect("N"),
            "--help" | "-h" => {
                eprintln!(
                    "bench_hook --iters N --warmup N\n\
                     Default: --iters 100 --warmup 1"
                );
                std::process::exit(0);
            }
            other => {
                eprintln!("unknown arg: {other}");
                std::process::exit(2);
            }
        }
    }
    Args { iters, warmup }
}

/// Run a single `muninn hook decide` invocation and return
/// (elapsed, exit_code, stdout_len).
fn run_once(exe: &std::path::Path) -> (Duration, i32, usize) {
    let start = Instant::now();
    let mut child = Command::new(exe)
        .args(["hook", "decide"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn muninn hook decide");
    child
        .stdin
        .as_mut()
        .expect("child stdin")
        .write_all(CANNED_PAYLOAD.as_bytes())
        .expect("write stdin");
    let out = child.wait_with_output().expect("wait child");
    let elapsed = start.elapsed();
    (elapsed, out.status.code().unwrap_or(-1), out.stdout.len())
}

/// Pre-computed nearest-rank percentile of an already-sorted slice.
fn percentile_ms(sorted_ms: &[f64], p: f64) -> f64 {
    if sorted_ms.is_empty() {
        return 0.0;
    }
    // Nearest-rank: ceil(p/100 * n)
    let n = sorted_ms.len() as f64;
    let rank = (p / 100.0 * n).ceil() as usize;
    sorted_ms[rank.saturating_sub(1).min(sorted_ms.len() - 1)]
}

fn locate_binary() -> std::path::PathBuf {
    // Prefer the release binary if it exists; otherwise the debug one.
    // We don't auto-build to keep the benchmark honest about whatever
    // the user has compiled.
    let workspace_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(std::path::PathBuf::from)
        .expect("locate workspace root from CARGO_MANIFEST_DIR");
    for profile in ["release", "debug"] {
        let candidate = workspace_root.join("target").join(profile).join("muninn");
        if candidate.exists() {
            return candidate;
        }
    }
    panic!(
        "no muninn binary found under {}/target/{{release,debug}}/. \
         Build first: cargo build --release",
        workspace_root.display()
    );
}

fn main() {
    let args = parse_args();
    let exe = locate_binary();

    eprintln!(
        "bench_hook: binary={} iters={} warmup={}",
        exe.display(),
        args.iters,
        args.warmup
    );

    // Warm up — drop these samples from the steady-state stats.
    let mut warmup_ms: Vec<f64> = Vec::with_capacity(args.warmup);
    for _ in 0..args.warmup {
        let (e, code, _) = run_once(&exe);
        warmup_ms.push(e.as_secs_f64() * 1000.0);
        if code != 0 {
            eprintln!("warmup: hook decide exited {code}");
        }
    }

    // Measured iterations.
    let mut samples_ms: Vec<f64> = Vec::with_capacity(args.iters);
    let mut nonzero_exits = 0usize;
    let mut nonempty_outputs = 0usize;
    let overall_start = Instant::now();
    for i in 0..args.iters {
        let (e, code, stdout_len) = run_once(&exe);
        samples_ms.push(e.as_secs_f64() * 1000.0);
        if code != 0 {
            nonzero_exits += 1;
        }
        if stdout_len > 0 {
            nonempty_outputs += 1;
        }
        if i % 10 == 9 {
            eprintln!(
                "  {}/{} iters; last={:.1}ms, running mean={:.1}ms",
                i + 1,
                args.iters,
                samples_ms.last().copied().unwrap_or(0.0),
                samples_ms.iter().sum::<f64>() / samples_ms.len() as f64
            );
        }
    }
    let total_wall = overall_start.elapsed();

    let mut sorted = samples_ms.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p50 = percentile_ms(&sorted, 50.0);
    let p95 = percentile_ms(&sorted, 95.0);
    let p99 = percentile_ms(&sorted, 99.0);
    let min = sorted.first().copied().unwrap_or(0.0);
    let max = sorted.last().copied().unwrap_or(0.0);
    let mean = if samples_ms.is_empty() {
        0.0
    } else {
        samples_ms.iter().sum::<f64>() / samples_ms.len() as f64
    };

    // NFR-001 from PROJEC-I-0011: hook overhead p50 ≤ 100 ms on
    // commodity hardware. We *report* the verdict but don't fail —
    // the benchmark's job is measurement, not gatekeeping.
    let nfr_001_p50_ms: f64 = 100.0;
    let nfr_001_pass = p50 <= nfr_001_p50_ms;

    let report = serde_json::json!({
        "binary": exe.display().to_string(),
        "iters": args.iters,
        "warmup": args.warmup,
        "warmup_ms": warmup_ms,
        "stats_ms": {
            "min": min,
            "max": max,
            "mean": mean,
            "p50": p50,
            "p95": p95,
            "p99": p99,
        },
        "exit_status": {
            "nonzero_exits": nonzero_exits,
            "nonempty_outputs": nonempty_outputs,
        },
        "nfr_001": {
            "p50_budget_ms": nfr_001_p50_ms,
            "p50_actual_ms": p50,
            "pass": nfr_001_pass,
        },
        "total_wall_s": total_wall.as_secs_f64(),
    });
    println!("{}", serde_json::to_string_pretty(&report).unwrap());

    eprintln!(
        "\nNFR-001 (p50 ≤ {:.0} ms): {} (actual p50 = {:.1} ms)",
        nfr_001_p50_ms,
        if nfr_001_pass { "PASS" } else { "FAIL" },
        p50,
    );
}
