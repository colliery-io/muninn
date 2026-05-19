---
id: failure-mode-integration-tests-for
level: task
title: "Failure-mode integration tests for hook (silent passthrough on all faults)"
short_code: "PROJEC-T-0073"
created_at: 2026-05-19T16:41:37.037880+00:00
updated_at: 2026-05-19T16:41:37.037880+00:00
parent: PROJEC-I-0011
blocked_by: []
archived: false

tags:
  - "#task"
  - "#phase/todo"


exit_criteria_met: false
initiative_id: PROJEC-I-0011
---

# Failure-mode integration tests for hook (silent passthrough on all faults)

## Parent Initiative

[[hook-mcp-integration-layer-for-claude-code]] (PROJEC-I-0011)

## Objective

NFR-002 says the hook must degrade to silent passthrough on every conceivable failure — daemon down, decision model unreachable, MCP server crashed, malformed decision output, timeout. This task builds the integration test matrix that proves it. If any failure path blocks the user's tool call, this task fails.

## Acceptance Criteria

- [ ] Test: daemon not running and `daemon ensure` fails → hook returns passthrough; original tool runs.
- [ ] Test: daemon running but decision-model endpoint unreachable → hook returns passthrough within timeout budget.
- [ ] Test: decision model returns malformed JSON / unknown decision / invalid `rewrite.tool` → hook returns passthrough.
- [ ] Test: decision model exceeds the 500ms hard timeout → hook returns passthrough.
- [ ] Test: augmentation retrieval errors → augment branch falls back to passthrough (no half-formed block attached).
- [ ] Test: hook script itself crashes / non-zero exit → CC sees passthrough (validated via CC's hook contract behavior).
- [ ] All tests live in `tests/hook_failure_modes/` and run under `angreal test integration`.
- [ ] CI runs them; flake budget zero — passthrough must be deterministic.

## Dependencies

- PROJEC-T-0069 (plugin)
- PROJEC-T-0070 (hook decide)
- PROJEC-T-0071 (augment block — for the augment-failure case)

## Implementation Notes

- Use fault-injection rather than mocking where possible: actually kill the daemon, actually block the LLM endpoint with iptables/`socat` tricks or a stub server that returns garbage.
- Make sure the tests cover the *user-visible* behavior (original tool call still runs) — internal "we returned passthrough" assertions are necessary but not sufficient.

## Status Updates

*To be added during implementation.*
