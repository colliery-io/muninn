---
id: implement-muninnengine-for
level: task
title: "Implement MuninnEngine for RecursiveEngine and migrate adapters to consume the trait"
short_code: "PROJEC-T-0065"
created_at: 2026-05-19T16:41:24.508566+00:00
updated_at: 2026-05-19T16:41:24.508566+00:00
parent: PROJEC-I-0011
blocked_by: []
archived: false

tags:
  - "#task"
  - "#phase/todo"


exit_criteria_met: false
initiative_id: PROJEC-I-0011
---

# Implement MuninnEngine for RecursiveEngine and migrate adapters to consume the trait

## Parent Initiative

[[hook-mcp-integration-layer-for-claude-code]] (PROJEC-I-0011)

## Objective

Wire the existing `RecursiveEngine` (and its tool environment, memory store, etc.) to the `MuninnEngine` trait from PROJEC-T-0064. Refactor `crates/muninn-rlm/src/proxy.rs` and the current `mcp.rs` scaffolding to consume `Arc<dyn MuninnEngine>` instead of reaching into engine internals. **No external behavior change** — pure refactor.

## Acceptance Criteria

- [ ] `impl MuninnEngine for RecursiveEngine` exists and compiles.
- [ ] `proxy.rs` no longer references `RecursiveEngine` directly — only `Arc<dyn MuninnEngine>`.
- [ ] Existing `mcp.rs` is either rewired through the trait or removed (assess and decide; document choice in commit).
- [ ] All existing proxy integration tests pass unchanged.
- [ ] Manual end-to-end check: spin up the proxy against a real config and verify a passthrough call and an RLM exploration both still work.
- [ ] `angreal ci` passes.

## Dependencies

- PROJEC-T-0064 (muninn-core crate must exist).

## Implementation Notes

- Riskiest refactor in the initiative — the proxy has real users. Land it on its own, not bundled with behavior changes.
- If the trait surface from PROJEC-T-0064 isn't sufficient to express what the proxy needs, that's signal to extend the trait carefully — don't add escape-hatch downcasts.
- Watch for hidden coupling: anywhere the proxy peeks at engine state, that's a coupling to break.

## Status Updates

*To be added during implementation.*
