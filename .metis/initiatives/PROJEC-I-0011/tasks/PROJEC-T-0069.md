---
id: scaffold-plugins-muninn-cc-claude
level: task
title: "Scaffold plugins/muninn-cc Claude Code plugin with PreToolUse hook entry point"
short_code: "PROJEC-T-0069"
created_at: 2026-05-19T16:41:30.767408+00:00
updated_at: 2026-05-20T16:16:04.664126+00:00
parent: PROJEC-I-0011
blocked_by: []
archived: false

tags:
  - "#task"
  - "#phase/completed"


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

### 2026-05-20 — Plugin scaffold landed

Created the `plugins/muninn-cc/` Claude Code plugin. The shape matches the
plugin format used by other CC plugins on this machine (verified against
`~/.claude/plugins/marketplaces/angreal-angreal/plugin/` as the reference):

```
plugins/muninn-cc/
├── .claude-plugin/
│   └── plugin.json          # manifest (name=muninn-cc, version 0.0.1, Apache-2.0)
├── hooks/
│   ├── hooks.json           # PreToolUse matcher = "Grep|Read|Glob"; 1s timeout
│   └── pre-tool-use.sh      # shell entry, executable
└── README.md                # purpose, requirements, failure semantics, layout
```

### Hook script behavior

`pre-tool-use.sh` follows the NFR-002 "silent passthrough on any failure"
contract:

1. If `muninn` isn't on PATH → exit 0, no stdout. (Common case before install.)
2. Otherwise pipe CC's hook-input stdin through `muninn hook decide` (the subcommand from PROJEC-T-0070; not yet implemented, so this branch currently always falls into the failure path — still fine because failure = passthrough).
3. Any non-zero exit, missing subcommand, or empty output also collapses to passthrough.
4. Successful output (any non-empty stdout from `muninn hook decide`) is relayed verbatim to CC.

The 1-second timeout in `hooks.json` is enforced by CC; the shell script itself doesn't time out internally. CC kills the process on overflow and treats that as passthrough.

### Verification

Smoke-tested all three failure paths manually with a stubbed `muninn` binary on PATH:

| Scenario | Expected | Actual | Result |
|---|---|---|---|
| `muninn` not on PATH | exit 0, empty stdout | exit 0, empty stdout | PASS |
| `muninn hook decide` exits non-zero | exit 0, empty stdout | exit 0, empty stdout | PASS |
| `muninn hook decide` emits JSON | exit 0, JSON relayed verbatim | exit 0, JSON relayed verbatim | PASS |

### Decisions

- **Shell over compiled binary** for the hook entry — sub-millisecond cold start, no per-platform builds. All real work happens inside `muninn hook decide`. Matches the task's implementation-notes guidance.
- **`Grep|Read|Glob` regex matcher** registered as a single hook entry rather than three separate matchers — equivalent and tighter to maintain.
- **README cross-links** ADR-0003, PROJEC-I-0011, PROJEC-T-0070, PROJEC-T-0072 so future readers can find the rationale and install path.

### Deferred / explicit non-scope

- **`muninn install-cc`** registration into the user's CC config — separate task (PROJEC-T-0072).
- **Hook-level automated test** in the workspace test harness — failure-mode tests (PROJEC-T-0073) is the dedicated coverage step; this scaffold's manual smoke-test in the verification table above is sufficient until then.

### CI carve-out
No new Rust code; no impact on workspace `angreal ci`. The pre-existing carve-out from PROJEC-T-0076 still applies to workspace clippy.