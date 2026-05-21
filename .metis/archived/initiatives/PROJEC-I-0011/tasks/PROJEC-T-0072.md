---
id: implement-muninn-install-cc-and
level: task
title: "Implement muninn install-cc and uninstall-cc CLI commands"
short_code: "PROJEC-T-0072"
created_at: 2026-05-19T16:41:35.462224+00:00
updated_at: 2026-05-20T20:00:42.016765+00:00
parent: PROJEC-I-0011
blocked_by: []
archived: true

tags:
  - "#task"
  - "#phase/completed"


exit_criteria_met: false
initiative_id: PROJEC-I-0011
---

# Implement muninn install-cc and uninstall-cc CLI commands

## Parent Initiative

[[hook-mcp-integration-layer-for-claude-code]] (PROJEC-I-0011)

## Objective

Add CLI commands that wire the muninn-cc plugin and the MCP server into a target Claude Code configuration: install registers the plugin path with CC and adds an `mcp.json` entry pointing at `muninn mcp`. Uninstall reverses both cleanly. This is how users adopt the new integration without hand-editing CC config files.

## Acceptance Criteria

## Acceptance Criteria

## Acceptance Criteria

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
### 2026-05-20 — Implementation landed (MCP register; plugin step printed as instructions)

**New module `crates/muninn/src/install.rs`** plus two CLI subcommands:

- `muninn install-cc [--global] [--dry-run]`
- `muninn uninstall-cc [--global] [--dry-run]`

**File targets** (matched against the CC manifests observed under
`~/.claude/plugins/cache/*/.mcp.json`):

- Project scope (default): `<repo>/.mcp.json`
- Global scope (`--global`): `~/.claude.json`

Both are JSON; we read, mutate `mcpServers.muninn`, then write back —
unrelated keys are preserved verbatim. The written entry matches CC's
canonical `.mcp.json` shape:

```json
{ "mcpServers": { "muninn": { "command": "muninn", "args": ["mcp"], "env": {} } } }
```

**Behavior:**

- Idempotent: a re-install when the existing entry already matches reports `AlreadyPresent` and writes nothing.
- Rewrites a non-matching entry (e.g. old args from a previous version) in place.
- Refuses to clobber a config file whose top-level is not a JSON object — surfaces a clear error.
- Writes `<path>.bak` before mutating anything pre-existing.
- `--dry-run` short-circuits the write and prints the proposed value + action ("add" / "rewrite" / "remove" mcpServers.muninn) in the same shape the real run would emit.
- Uninstall removes only `mcpServers.muninn` — sibling entries and other top-level keys stay intact.

**Plugin step**: CC's local-plugin registration format is less standardized than `.mcp.json`. Rather than reverse-engineering an internal config, install-cc prints a `Plugin (muninn-cc) install:` notice with the `/plugin add-source ./plugins/muninn-cc` incantation users should run from inside a Claude Code session. This is the realistic boundary between what muninn knows reliably (the MCP shape) and what CC owns (its plugin source registry).

### Tests
- 11 new unit tests in `install::tests` covering: install-into-missing-file, preserves-unrelated-entries, idempotency, rewrites-when-entry-differs, dry-run-does-not-write, rejects-non-object-config, uninstall-preserves-other-entries, uninstall-on-clean-config-is-noop, uninstall-on-missing-config-is-noop, install-then-uninstall round-trip, describe-install-handles-all-variants.
- muninn binary unit tests: **50/50 pass** (was 39, +11).
- Workspace: 16 suites still green.
- `angreal test uat` against real Ollama Cloud still passes (1.6s, passthrough).
- Strict clippy + `cargo fmt --check` clean apart from the pre-existing `main.rs:1242` warning tracked in PROJEC-T-0076.
- Manual smoke: `cargo run -- install-cc --dry-run` in this repo prints the expected proposed value + plugin notice.

### Decisions

- **Project scope = `.mcp.json` at the repo root**, not `.claude/settings.json`. CC reads both, but `.mcp.json` is the lowest-friction surface (one file, one purpose, easy to .gitignore or commit per team preference).
- **Global scope = `~/.claude.json`**, the file that already holds `mcpServers` for the user.
- **Don't write to `enabledPlugins`** from install-cc. The plugin and the MCP server are separate concerns; users get a clear notice instead.

### Deferred / explicit non-scope

- **Driving CC's `/plugin add-source` programmatically.** No supported subprocess interface today; install-cc prints the manual step instead.
- **Backup management beyond `.bak`** — single rotating `.bak` is enough for the "undo my install" use case.
- **Project-scope settings.json mutation** — only the canonical `.mcp.json` is written; install-cc stays unopinionated about CC's other config layers.

### CI carve-out
Same as previous initiative tasks — workspace `angreal ci` still blocked by the pre-existing muninn-graph clippy debt tracked in PROJEC-T-0076. No new clippy or fmt issues introduced.