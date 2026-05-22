---
id: define-and-document-mcp-tool
level: task
title: "Define and document MCP tool schemas for the engine surface"
short_code: "PROJEC-T-0067"
created_at: 2026-05-19T16:41:27.683837+00:00
updated_at: 2026-05-20T02:03:33.050460+00:00
parent: PROJEC-I-0011
blocked_by: []
archived: true

tags:
  - "#task"
  - "#phase/completed"


exit_criteria_met: false
initiative_id: PROJEC-I-0011
---

# Define and document MCP tool schemas for the engine surface

## Parent Initiative

[[hook-mcp-integration-layer-for-claude-code]] (PROJEC-I-0011)

## Objective

Lock the wire shape of the MCP tools Claude Code (and other MCP clients) will see. Each tool maps 1:1 to a `MuninnEngine` method but exposes a clean, agent-friendly schema — names, descriptions, argument types, return shape. These schemas are the contract; changes are breaking changes for plugin users.

## Acceptance Criteria

## Acceptance Criteria

- [ ] Schemas defined for: `search_code`, `recall_memory`, `search_docs`, `query_graph`. (Open call: should `explore` be public-facing via MCP or hook-internal only? Decide and document.)
- [ ] Each schema includes: name, one-paragraph description aimed at agents, argument schema (JSON Schema), return schema, and 1–2 example calls.
- [ ] Schemas align with the `muninn-core` types from PROJEC-T-0064 — derive from a single source of truth (`schemars` or hand-maintained mapping).
- [ ] Schemas live in a single, discoverable module used by the MCP server (PROJEC-T-0068).
- [ ] Documentation page in `docs/` explains what each tool does, when an agent should call which, and how they relate to the hook-rewrite path.
- [ ] Stability note in docs: which fields are stable vs. experimental.

## Dependencies

- PROJEC-T-0064 (types these schemas describe).

## Implementation Notes

- Tool descriptions matter more than people think — they're what the agent's planner sees. Write them like prompt fragments, not API docs. "Use this when you need …" framing.
- Prefer fewer, richer tools over many narrow ones.
- If `explore` is hook-only, don't expose it via MCP and drop to four tools.

## Status Updates

### 2026-05-19 — Implementation landed

- New module `crates/muninn-core/src/mcp.rs` with `McpToolSchema` (name, description, input_schema, output_schema, examples, stability) and a `tool_schemas()` function returning every exposed tool.
- Tools exposed: **`search_code`, `query_graph`, `recall_memory`, `search_docs`** (four total).
- `JsonSchema` derives added to every DTO in `types.rs` via `schemars` 0.8. Input/output schemas are derived from the muninn-core types — no hand-maintained duplicate. `recall_memory` wraps its return as `{ "hits": [...] }` because the trait returns `Vec<MemoryHit>` and MCP tool outputs need to be a single object.
- **`explore` is intentionally not exposed via MCP** — the recursive engine is the expensive code path and an LLM planner is prone to invoking it for vague questions and blowing through budget. The hook plugin (PROJEC-T-0070) drives `explore` directly via the trait when its decision model determines a rewrite is warranted. Rationale captured in the module-level doc and reinforced by a unit test (`tool_schemas_do_not_expose_explore`).
- Stability levels documented: tool names + descriptions + documented field sets are **Stable**; numeric scoring details are best-effort.
- New documentation page `docs/mcp-tools.md` with when-to-use guidance, per-tool field tables, examples, and cross-links to ADR-0003 / initiative / source.
- Tests: 5 new schema tests covering tool list, `explore` absence, every-schema-has-description-and-properties, every-example-is-an-object, and full JSON roundtrip. Plus the original 6 type/error tests still pass. Total 11/11.
- Strict clippy on muninn-core: clean. `cargo fmt --check -p muninn-core`: clean.

### CI carve-out
Same as PROJEC-T-0063/0064: workspace `angreal ci` still blocked by the pre-existing muninn-graph clippy debt tracked in PROJEC-T-0076. This task's code is clean end-to-end.