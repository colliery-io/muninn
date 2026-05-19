---
id: create-muninn-core-crate-with
level: task
title: "Create muninn-core crate with MuninnEngine trait and supporting types"
short_code: "PROJEC-T-0064"
created_at: 2026-05-19T16:41:22.947928+00:00
updated_at: 2026-05-19T16:41:22.947928+00:00
parent: PROJEC-I-0011
blocked_by: []
archived: false

tags:
  - "#task"
  - "#phase/todo"


exit_criteria_met: false
initiative_id: PROJEC-I-0011
---

# Create muninn-core crate with MuninnEngine trait and supporting types

## Parent Initiative

[[hook-mcp-integration-layer-for-claude-code]] (PROJEC-I-0011)

## Objective

Establish the adapter-neutral core of the workspace. Create a new `crates/muninn-core/` crate that defines the `MuninnEngine` trait and its supporting request/response types. This crate must have **no dependency on `muninn-rlm` or any adapter** — proxy, MCP, and hook all depend on it, not the other way around.

## Acceptance Criteria

- [ ] New crate `crates/muninn-core/` added to the workspace `Cargo.toml`.
- [ ] `MuninnEngine` async trait defined with these six methods, matching the signatures in [[hook-mcp-integration-layer-for-claude-code]]:
  - `search_code(SearchQuery) -> Result<SearchResult>`
  - `explore(ExploreRequest) -> Result<ExploreResult>`
  - `recall_memory(MemoryQuery) -> Result<Vec<MemoryHit>>`
  - `record_memory(MemoryItem) -> Result<()>`
  - `search_docs(DocsQuery) -> Result<DocsResult>`
  - `query_graph(GraphQuery) -> Result<GraphResult>`
- [ ] Request/response types defined with `serde` derives — they cross the IPC and MCP wire boundaries.
- [ ] Crate's own error type (`MuninnCoreError`) with conversions usable by adapters.
- [ ] Dependency direction enforced: `cargo tree -p muninn-core` shows no `muninn-rlm` or adapter crate.
- [ ] `angreal ci` passes.

## Dependencies

None — this is the foundation everything else builds on.

## Implementation Notes

- Use `async_trait` for object safety; downstream consumes `Arc<dyn MuninnEngine>`.
- Resist the temptation to add a seventh method "for symmetry." Grow on demand only.
- Types should be minimal and stable — they appear in the MCP tool schema (PROJEC-T-0067) and the IPC wire format (PROJEC-T-0066), so changing them later is expensive.

## Status Updates

*To be added during implementation.*
