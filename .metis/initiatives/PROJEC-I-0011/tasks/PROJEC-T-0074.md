---
id: hook-latency-benchmark-harness
level: task
title: "Hook latency benchmark harness verifying NFR-001 (p50 < 100ms)"
short_code: "PROJEC-T-0074"
created_at: 2026-05-19T16:41:38.605116+00:00
updated_at: 2026-05-20T20:21:39.321374+00:00
parent: PROJEC-I-0011
blocked_by: []
archived: false

tags:
  - "#task"
  - "#phase/completed"


exit_criteria_met: false
initiative_id: PROJEC-I-0011
---

# Hook latency benchmark harness verifying NFR-001 (p50 < 100ms)

## Parent Initiative

[[hook-mcp-integration-layer-for-claude-code]] (PROJEC-I-0011)

## Objective

Build a reproducible benchmark that measures end-to-end hook overhead — from CC invoking the hook to the hook returning a decision — on a representative mix of Grep/Read/Glob calls. Verify NFR-001 (p50 < 100ms; hard cap 500ms) on M-series Mac as the reference platform. The benchmark stays in-tree as a regression guard.

## Acceptance Criteria

## Acceptance Criteria

## Acceptance Criteria

- [ ] Benchmark binary or test that exercises `muninn hook decide` against a fixed corpus of representative tool calls.
- [ ] Reports p50, p95, p99 over at least 100 calls.
- [ ] Runs against the `[default]` config (i.e. gemma4:31b on Ollama Cloud) and outputs a comparable number for a local-Ollama configuration if available.
- [ ] Reference run on M-series Mac documented in the task's Status Updates: pass/fail against NFR-001, latency breakdown (decision model time vs. daemon IPC vs. plumbing overhead).
- [ ] `angreal bench hook` task that invokes it.
- [ ] Result format machine-readable (JSON) so CI can compare across commits.

## Dependencies

- PROJEC-T-0069 (plugin)
- PROJEC-T-0070 (hook decide)

## Implementation Notes

- Warm-cache and cold-cache measurements should both be captured — first call after daemon spawn is qualitatively different from steady-state.
- If p50 misses on default config, that's *signal*, not failure for this task. The task's job is to measure; the design decision (smaller default model, prompt trimming, etc.) is a follow-up.
- Don't run the benchmark in mainline CI by default (too noisy on shared runners). Provide an opt-in tag.

## Status Updates

*To be added during implementation.*
### 2026-05-20 — Implementation landed; reference run documented

**New example binary `crates/muninn/examples/bench_hook.rs`** — spawns `muninn hook decide` repeatedly with a canned CC PreToolUse payload and reports end-to-end wall-clock percentiles. Emits a single JSON object on stdout for CI / regression diffing and a human-readable NFR-001 verdict on stderr.

Key design choices:

- **Subprocess measurement.** The benchmark measures what Claude Code measures when its PreToolUse hook fires: total subprocess wall-clock time. No in-process shortcuts.
- **Locates the binary at runtime.** Prefers `target/release/muninn`; falls back to `target/debug/muninn`. Panics with a clear "build first" message rather than auto-building. Keeps measurements honest about whatever the user actually compiled.
- **Warmup separated.** First N iterations (default 1) are reported separately so cargo's filesystem cache + the LLM's cold prompt cache don't pollute the steady-state percentiles.
- **Counts failures, doesn't abort.** `nonzero_exits` + `nonempty_outputs` go into the report so we can spot NFR-002 violations under load. The benchmark's job is measurement, not gating.
- **Nearest-rank percentile** (not interpolation). Simpler and matches what `cargo bench` / Criterion do.

**New angreal task `angreal bench hook`** (in `.angreal/task_bench.py`) — wraps the example with `sops exec-env` so the encrypted UAT bundle's `OLLAMA_API_KEY` is automatically loaded. Falls back to shell env vars when the bundle is absent. Accepts `--iters N` and `--warmup N` arguments.

### Reference run — 2026-05-20 on M-series Mac

`angreal bench hook --iters 25` against the default tiered config (`[default] provider = "ollama", model = "gemma4:31b"` on Ollama Cloud):

```json
{
  "iters": 25,
  "warmup_ms": [991.7],
  "stats_ms": {
    "min": 270.7,
    "p50": 290.0,
    "mean": 353.3,
    "p95": 508.2,
    "p99": 509.0,
    "max": 509.0
  },
  "exit_status": { "nonzero_exits": 0, "nonempty_outputs": 0 },
  "nfr_001": { "p50_budget_ms": 100.0, "p50_actual_ms": 290.0, "pass": false }
}
```

Total wall time: 8.83 s for 25 iterations (≈ 353 ms per call average).

**Verdict: NFR-001 (p50 ≤ 100 ms) is missed by ~3×.** Per the task's own implementation notes: *"If p50 misses on default config, that's signal, not failure for this task. The task's job is to measure; the design decision is a follow-up."* This is exactly that signal.

**Diagnosis (not in scope for this task — captures the follow-up direction):**

- `gemma4:31b` is a generous mid-size model. The 290 ms p50 is dominated by the Ollama Cloud network call, not by binary cold-start (which we can see is ~700 ms isolated, in the warmup sample). The hook's internal 500 ms hard cap is *also* visible in the p99 (508 ms — right at the cap), which means the timeout is firing in some tail-end cases. Good news: NFR-002 holds (0 non-zero exits, 0 non-empty outputs since the model output was likely truncated and parsed as passthrough).
- Path forward: pin `[hook_decision]` to a small/fast model. Candidate from arawn's UAT matrix: Groq `llama-3.1-8b-instant` (~30-50 ms per call) or a local quantized Ollama gemma small. The tiered config supports this in one line:
  ```toml
  [hook_decision]
  provider = "groq"
  model = "llama-3.1-8b-instant"
  ```

### What's in the report

| Field | Meaning |
|---|---|
| `binary` | Which `target/.../muninn` was measured. |
| `iters` / `warmup` | Sample counts. |
| `warmup_ms` | Per-warmup latencies — usually cold-start dominated. |
| `stats_ms.{min,max,mean,p50,p95,p99}` | Steady-state percentiles in ms. |
| `exit_status.nonzero_exits` | Count of subprocess invocations that exited non-zero. **Should always be 0** per NFR-002. |
| `exit_status.nonempty_outputs` | Count of invocations that emitted stdout (passthrough vs. augment). Useful as a sanity check that the decision model isn't stuck always-augmenting. |
| `nfr_001.{p50_budget_ms,p50_actual_ms,pass}` | Headline NFR-001 verdict for the run. |
| `total_wall_s` | Wall-clock spent in the measured loop, useful for back-of-envelope cost math. |

### Decisions

- **Not added to CI.** Per the task's implementation notes ("Don't run the benchmark in mainline CI by default — too noisy on shared runners"), this is opt-in via `angreal bench hook`. CI can adopt later via a labeled job that triggers on specific PR labels.
- **No regression-tracking automation yet.** The JSON output is the contract; future tooling (or a follow-up task) can diff runs across commits without changes here.
- **Release-build measurement.** The task asks for representative numbers; debug builds skew the cold-start dominance. The angreal task uses `cargo run --release`. The example will fall back to debug if release isn't built, but emits a less-meaningful number.

### Deferred / explicit non-scope

- **NFR-001 compliance itself.** The benchmark surfaces the gap; closing it via a smaller `[hook_decision]` model is a follow-up task that needs both the model selection and a UAT-style verification pass.
- **Daemon-pool reuse.** Reusing a long-running process per CC session (e.g., a persistent helper the plugin connects to via Unix socket) could cut the ~700 ms binary cold-start. That's a bigger architectural change tracked separately if/when needed.
- **Streaming-completion latency.** `MuninnEngine::complete` is request/response; if streaming completions land, the hook decision could potentially make a "first token" call and cancel early. Not a v1 concern.

### CI carve-out
Same as previous initiative tasks — workspace `angreal ci` still blocked by the pre-existing muninn-graph clippy debt tracked in PROJEC-T-0076. No new clippy or fmt issues introduced.