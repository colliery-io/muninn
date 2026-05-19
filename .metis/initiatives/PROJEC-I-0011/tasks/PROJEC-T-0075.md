---
id: docs-reorganization-around-two
level: task
title: "Docs reorganization around two surfaces plus migration guide for proxy-only users"
short_code: "PROJEC-T-0075"
created_at: 2026-05-19T16:41:39.662406+00:00
updated_at: 2026-05-19T16:41:39.662406+00:00
parent: PROJEC-I-0011
blocked_by: []
archived: false

tags:
  - "#task"
  - "#phase/todo"


exit_criteria_met: false
initiative_id: PROJEC-I-0011
---

# Docs reorganization around two surfaces plus migration guide for proxy-only users

## Parent Initiative

[[hook-mcp-integration-layer-for-claude-code]] (PROJEC-I-0011)

## Objective

Restructure top-level docs so the primary (hook+MCP for Claude Code) and secondary (proxy for non-CC clients) surfaces are clearly separated, with explicit guidance on when to pick which. Ship a migration guide walking existing muninn-as-proxy users through enabling the plugin and MCP server side-by-side configs.

## Acceptance Criteria

- [ ] README rewritten with two clearly labeled top-level paths: "Using muninn with Claude Code (recommended)" and "Using muninn with other clients (proxy)".
- [ ] Each path has a copy-pasteable getting-started block.
- [ ] New page `docs/migration-proxy-to-hook.md` (or equivalent) showing:
  - what the proxy-only setup looks like today,
  - what the hook+MCP setup looks like,
  - whether to keep proxy enabled in parallel (yes by default — both first-class),
  - troubleshooting common issues during the switch.
- [ ] Vision principles in `PROJEC-V-0001` still reflect reality; if "Invisible by default" needs a nuance, note it.
- [ ] All references to "the proxy" in user-facing docs say which surface they mean.
- [ ] Internal architecture diagram (`docs/architecture.md` or equivalent) updated to show the engine/adapter split.

## Dependencies

- PROJEC-T-0072 (install-cc — the recommended path docs reference it)
- All upstream implementation tasks must be far enough along that the docs reflect shipped behavior, not aspiration.

## Implementation Notes

- This is the user-facing capstone for the initiative. It's where most users will form their first impression of the new architecture.
- Resist the urge to document every internal detail. The reader wants to know: what do I do, what changes for me, what's the rollback if something breaks.
- Cross-link [[003-hook-mcp-integration-model-as]] (PROJEC-A-0003) from the migration page for users who want the "why".

## Status Updates

*To be added during implementation.*
