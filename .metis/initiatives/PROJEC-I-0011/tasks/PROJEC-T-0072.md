---
id: implement-muninn-install-cc-and
level: task
title: "Implement muninn install-cc and uninstall-cc CLI commands"
short_code: "PROJEC-T-0072"
created_at: 2026-05-19T16:41:35.462224+00:00
updated_at: 2026-05-19T16:41:35.462224+00:00
parent: PROJEC-I-0011
blocked_by: []
archived: false

tags:
  - "#task"
  - "#phase/todo"


exit_criteria_met: false
initiative_id: PROJEC-I-0011
---

# Implement muninn install-cc and uninstall-cc CLI commands

## Parent Initiative

[[hook-mcp-integration-layer-for-claude-code]] (PROJEC-I-0011)

## Objective

Add CLI commands that wire the muninn-cc plugin and the MCP server into a target Claude Code configuration: install registers the plugin path with CC and adds an `mcp.json` entry pointing at `muninn mcp`. Uninstall reverses both cleanly. This is how users adopt the new integration without hand-editing CC config files.

## Acceptance Criteria

- [ ] `muninn install-cc [--global|--project]` installs the plugin + MCP entry. Default scope: project. `--global` writes user-level CC config.
- [ ] Install is idempotent: re-running doesn't double-register.
- [ ] Install fails clearly when CC isn't detected (no config dir found), with a pointer to docs.
- [ ] `muninn uninstall-cc [--global|--project]` removes both registrations. Leaves other CC config untouched.
- [ ] Backup: install writes a `.bak` copy of any file it modifies before editing.
- [ ] `--dry-run` prints what would change without writing.
- [ ] Integration test against a fixture CC config: install, verify diffs, uninstall, verify clean restoration.

## Dependencies

- PROJEC-T-0068 (MCP server must exist to register)
- PROJEC-T-0069 (plugin must exist to install)

## Implementation Notes

- Target the current CC config layout but isolate path/format knowledge into one module — CC will change config locations over time.
- Project-scope install writes into `.claude/` in the current repo (consistent with `.muninn/` placement).
- Be conservative editing JSON config: preserve formatting and comments where possible.

## Status Updates

*To be added during implementation.*
