# muninn-cc

Claude Code plugin that pre-answers each user prompt with muninn's
recursive exploration on a cheap local model, so Claude Code itself
doesn't have to re-discover project context for every turn.

## What it does

On every `UserPromptSubmit`, the plugin's hook hands the prompt to a
local muninn daemon. A cheap router model decides whether the prompt
needs exploration; if it does, muninn drives its recursive exploration
loop on the configured local/cheap backend and returns the result. The
hook injects that result as `additionalContext`, framed as the answer
Claude should deliver — explicitly instructing Claude not to re-grep
or re-read the codebase to discover what muninn already found.

The plugin itself is intentionally thin — a shell entry that shells
out to the `muninn` binary. All real work happens inside muninn so
updating the binary updates the plugin's behavior.

## Requirements

- The `muninn` binary on `PATH`.
- A muninn `.muninn/config.toml` configured for at least one provider
  (run `muninn init` once per repo; see the top-level README for the
  tiered-config defaults).

The hook script runs `muninn daemon ensure` ahead of each turn —
idempotent when the daemon is already alive, so steady-state cost
is zero. First turn of a fresh CC session pays a ~1–2 s
daemon-startup cost.

The hook is engineered for **silent passthrough on any failure**: if
`muninn` is missing, the daemon won't start, the backend errors, or
the deadline fires, Claude Code processes the user's prompt with no
injection. The plugin never blocks the user's turn (NFR-002).

The hook's outer deadline defaults to 240 seconds — recursive
exploration on a local/cheap model regularly takes 20–60 s for
code-shaped prompts, and the user is already waiting for Claude's
first token, so a tighter cap silently squashed useful injections
in practice. Override with `MUNINN_HOOK_DEADLINE_MS` (env var) when
testing failure modes.

## Installation

Two pieces:

1. **MCP side** — `muninn install-cc` (in the repo root) writes the
   muninn entry into `.mcp.json` so CC sees the `search_code` /
   `query_graph` tools.
2. **Hook side** — restart Claude Code with `--plugin-dir` pointing
   at this directory:

   ```bash
   claude --plugin-dir /absolute/path/to/muninn/plugins/muninn-cc
   ```

   Use an absolute path. There is currently no slash command to add
   a local plugin to an already-running CC session; you must restart
   with the flag. After load, edits to `hooks.json` /
   `user-prompt-submit.sh` can be picked up live with `/reload-plugins`.

If you want to share this plugin with teammates, repackage it as a
plugin marketplace and use `/plugin marketplace add` + `/plugin
install`. The local-directory flow above is for development and
single-user installs.

## Layout

```
plugins/muninn-cc/
├── .claude-plugin/
│   └── plugin.json              # plugin manifest
├── hooks/
│   ├── hooks.json               # registers the UserPromptSubmit hook
│   └── user-prompt-submit.sh    # shell entry — shells out to `muninn hook submit`
└── README.md                    # this file
```

## See also

- ADR-0003 — why muninn uses hooks + MCP as the primary Claude Code
  integration surface.
- `muninn install-cc --help` — installer for the MCP server side.
