---
id: define-and-document-mcp-tool
level: task
title: "Define and document MCP tool schemas for the engine surface"
short_code: "PROJEC-T-0067"
created_at: 2026-05-19T16:41:27.683837+00:00
updated_at: 2026-05-19T16:41:27.683837+00:00
parent: PROJEC-I-0011
blocked_by: []
archived: false

tags:
  - "#task"
  - "#phase/todo"


exit_criteria_met: false
initiative_id: PROJEC-I-0011
---

# Define and document MCP tool schemas for the engine surface

## Parent Initiative

[[hook-mcp-integration-layer-for-claude-code]] (PROJEC-I-0011)

## Objective

Lock the wire shape of the MCP tools Claude Code (and other MCP clients) will see. Each tool maps 1:1 to a `MuninnEngine` method but exposes a clean, agent-friendly schema — names, descriptions, argument types, return shape. These schemas are the contract; changes are breaking changes for plugin users.

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

*To be added during implementation.*
