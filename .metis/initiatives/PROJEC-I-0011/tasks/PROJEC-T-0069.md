---
id: scaffold-plugins-muninn-cc-claude
level: task
title: "Scaffold plugins/muninn-cc Claude Code plugin with PreToolUse hook entry point"
short_code: "PROJEC-T-0069"
created_at: 2026-05-19T16:41:30.767408+00:00
updated_at: 2026-05-19T16:41:30.767408+00:00
parent: PROJEC-I-0011
blocked_by: []
archived: false

tags:
  - "#task"
  - "#phase/todo"


exit_criteria_met: false
initiative_id: PROJEC-I-0011
---

# Scaffold plugins/muninn-cc Claude Code plugin with PreToolUse hook entry point

## Parent Initiative

[[hook-mcp-integration-layer-for-claude-code]] (PROJEC-I-0011)

## Objective

Create the Claude Code plugin at `plugins/muninn-cc/` — plugin manifest + the PreToolUse hook entry-point script. The plugin is intentionally *thin*: the hook script reads CC's hook input from stdin and shells out to `muninn hook decide` (PROJEC-T-0070) for all real work, then returns the appropriate response to CC.

## Acceptance Criteria

- [ ] Directory `plugins/muninn-cc/` with the CC plugin manifest (`plugin.json` or current equivalent) and a `hooks/` subdirectory.
- [ ] PreToolUse hook registered for `Grep`, `Read`, `Glob`.
- [ ] Hook script (POSIX shell or small Rust binary — choose pragmatically) reads CC's hook input JSON from stdin, invokes `muninn hook decide`, returns the hook response CC expects.
- [ ] On any error in the hook chain (muninn binary missing, exit non-zero, malformed output): return "allow original tool call unchanged" — silent passthrough per NFR-002.
- [ ] Plugin loads cleanly in a real CC session (manual verification); hook fires on a real `Grep`.
- [ ] README in `plugins/muninn-cc/` describing purpose, dependency on the `muninn` binary, and pointer to install-cc (PROJEC-T-0072).

## Dependencies

- Scaffold itself has none, but the hook is inert without PROJEC-T-0070.

## Implementation Notes

- Prefer shell over compiled binary for the hook entry — shorter cold start, no per-platform builds. The Rust work happens inside `muninn hook decide`.
- Document the exact CC hook input/output schema versions the plugin targets, with a fallback if CC ships a breaking change.
- Keep the plugin self-contained: anything CC needs to load it lives inside `plugins/muninn-cc/`.

## Status Updates

*To be added during implementation.*
