---
id: hook-latency-benchmark-harness
level: task
title: "Hook latency benchmark harness verifying NFR-001 (p50 < 100ms)"
short_code: "PROJEC-T-0074"
created_at: 2026-05-19T16:41:38.605116+00:00
updated_at: 2026-05-19T16:41:38.605116+00:00
parent: PROJEC-I-0011
blocked_by: []
archived: false

tags:
  - "#task"
  - "#phase/todo"


exit_criteria_met: false
initiative_id: PROJEC-I-0011
---

# Hook latency benchmark harness verifying NFR-001 (p50 < 100ms)

## Parent Initiative

[[hook-mcp-integration-layer-for-claude-code]] (PROJEC-I-0011)

## Objective

Build a reproducible benchmark that measures end-to-end hook overhead — from CC invoking the hook to the hook returning a decision — on a representative mix of Grep/Read/Glob calls. Verify NFR-001 (p50 < 100ms; hard cap 500ms) on M-series Mac as the reference platform. The benchmark stays in-tree as a regression guard.

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
