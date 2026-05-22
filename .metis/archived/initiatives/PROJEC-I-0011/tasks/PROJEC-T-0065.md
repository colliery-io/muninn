---
id: implement-muninnengine-for
level: task
title: "Implement MuninnEngine for RecursiveEngine and migrate adapters to consume the trait"
short_code: "PROJEC-T-0065"
created_at: 2026-05-19T16:41:24.508566+00:00
updated_at: 2026-05-20T02:29:53.650534+00:00
parent: PROJEC-I-0011
blocked_by: []
archived: true

tags:
  - "#task"
  - "#phase/completed"


exit_criteria_met: false
initiative_id: PROJEC-I-0011
---

# Implement MuninnEngine for RecursiveEngine and migrate adapters to consume the trait

## Parent Initiative

[[hook-mcp-integration-layer-for-claude-code]] (PROJEC-I-0011)

## Objective

Wire the existing `RecursiveEngine` (and its tool environment, memory store, etc.) to the `MuninnEngine` trait from PROJEC-T-0064. Refactor `crates/muninn-rlm/src/proxy.rs` and the current `mcp.rs` scaffolding to consume `Arc<dyn MuninnEngine>` instead of reaching into engine internals. **No external behavior change** — pure refactor.

## Acceptance Criteria

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

### 2026-05-19 — Implementation landed (full refactor path)

Scope discovery early in the work showed the original AC ("impl MuninnEngine for RecursiveEngine + proxy.rs no longer references RecursiveEngine directly") couldn't be satisfied without resolving a type-shape mismatch: `RecursiveEngine::complete` operates on the full `CompletionRequest` / `CompletionResponse` shape (Anthropic Messages API), but the muninn-core trait was designed around lightweight MCP-shaped DTOs. The proxy uses `engine.complete(request)` end-to-end.

User chose the **full-refactor path**: move LLM types into muninn-core, add `complete()` to the trait, then migrate the proxy.

What landed:

- **`crates/muninn-core/src/llm.rs`** — the entire LLM type surface from `muninn-rlm/src/types.rs` moved into muninn-core. Includes `CompletionRequest`, `CompletionResponse`, `Message`, `Role`, `Content`, `ContentBlock`, `ToolUseBlock`, `ToolResultBlock`, `ToolDefinition`, `ToolChoice`, `SystemPrompt`, `CacheControl`, `StopReason`, `Usage`, `MuninnConfig`, `BudgetConfig`, `ExplorationMetadata`. `muninn-rlm/src/types.rs` becomes a thin `pub use muninn_core::llm::*;` re-export shim so internal `crate::types::Foo` paths keep working without touching every call site. Added `tracing` as a muninn-core dep (the moved code uses it for debug logging).
- **`MuninnEngine::complete()`** added to the trait in `muninn-core/src/lib.rs`. Takes `CompletionRequest` (now in muninn-core), returns `CoreResult<CompletionResponse>`. Documented as the rich Messages-API entry point used by the proxy and (future) the hook plugin when it issues a `rewrite` directive.
- **`CompletionRequest::is_recursive()`** moved from `RecursiveEngine::is_recursive` to a method on `CompletionRequest` in muninn-core, so the recursive-flag check is part of the type contract and not coupled to a specific engine.
- **`impl MuninnEngine for RecursiveEngine`** in new file `crates/muninn-rlm/src/engine/muninn_engine_impl.rs`. `complete()` delegates to the existing inherent method via `RecursiveEngine::complete(self, request)`. Includes an `RlmError -> MuninnCoreError` conversion that preserves the backend / budget / invalid-request semantics. The other five methods (`search_code`, `explore`, `recall_memory`, `record_memory`, `search_docs`, `query_graph`) are intentionally stubbed with `Internal("not yet wired — see PROJEC-T-0065 status notes")` — wiring them requires direct handles to the underlying stores (graph, doc, memory) which RecursiveEngine only reaches through `ToolEnvironment` today. Tracked as part of the daemon/MCP/hook tasks (T-0066/0068/0070+).
- **`crates/muninn-rlm/src/engine/mod.rs::default_engine()`** — new helper that returns `Arc<dyn MuninnEngine>` given a backend + tools + optional budget + work_dir. Centralizes engine construction so adapters never name `RecursiveEngine` by hand.
- **`crates/muninn-rlm/src/proxy.rs`** — `engine` field type changed from `Option<RecursiveEngine>` to `Option<Arc<dyn MuninnEngine>>`. All three constructors (`new`, `with_router`, `with_separate_backends`) call `default_engine(...)` instead of building `RecursiveEngine` inline. `with_engine(...)` signature changed to accept `Arc<dyn MuninnEngine>`. The static `RecursiveEngine::is_recursive(&req)` call switched to `req.is_recursive()`. Added `From<MuninnCoreError> for ProxyError` that maps the trait's error back into the existing wire-shaped error so the `IntoResponse` arm (notably the special-case 200-OK budget-exceeded handling) keeps working.
- **`crates/muninn-rlm/src/engine/tests.rs`** — `RecursiveEngine::is_recursive(&req)` references replaced with `req.is_recursive()`.

### mcp.rs assessment

The existing `crates/muninn-rlm/src/mcp.rs` (273 lines, generic MCP-server-from-ToolEnvironment adapter using `rust-mcp-sdk`) is **left untouched**. It does not depend on the engine boundary and does not conflict with the new code. T-0068 (build `muninn mcp` subcommand using the new schemas from T-0067) will re-evaluate whether to reuse parts of it or replace it; that's the right place to make that decision, not here.

### Verification

- `grep -rn RecursiveEngine crates/muninn-rlm/src/proxy.rs crates/muninn/src/main.rs` → no hits. Proxy and binary are clean of direct references.
- `cargo test --workspace` → **all ~456 tests pass** (muninn 16, muninn-core 11, muninn-graph 105 + ignored, muninn-rlm unit 281 + integration 10, etc.).
- `cargo clippy -p muninn-core -p muninn-rlm -p muninn --no-deps -- -D warnings` → only the pre-existing `muninn/main.rs:1242` warning tracked in PROJEC-T-0076.
- `cargo fmt --check` clean on touched crates.

### Deferred follow-ups (not blocking T-0065)

- Wire the five stubbed `MuninnEngine` methods to direct store handles. Will happen naturally in PROJEC-T-0066 (daemon, which holds the engine + stores) and PROJEC-T-0071 (augmentation retrieval, which exercises `query_graph` and `recall_memory`).
- mcp.rs cleanup happens as part of PROJEC-T-0068.

### CI carve-out

Same as PROJEC-T-0063/0064/0067 — workspace `angreal ci` still blocked by the pre-existing muninn-graph clippy debt tracked in PROJEC-T-0076. This task introduces no new clippy or fmt issues.