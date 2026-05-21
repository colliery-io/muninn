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
- A running muninn daemon. The hook itself does **not** auto-spawn
  one — cold-start cost would blow the per-turn budget — so start
  it ahead of time with `muninn daemon ensure` (or rely on `muninn
  mcp --ensure` if you're using the MCP server alongside).
- A muninn `.muninn/config.toml` configured for at least one provider.
  See the top-level README for the tiered-config defaults.

The hook is engineered for **silent passthrough on any failure**: if
`muninn` is missing, the daemon is unreachable, the backend errors,
or the deadline fires, Claude Code processes the user's prompt with
no injection. The plugin never blocks the user's turn (NFR-002).

The hook's outer deadline defaults to 240 seconds — recursive
exploration on a local/cheap model regularly takes 20–60 s for
code-shaped prompts, and the user is already waiting for Claude's
first token, so a tighter cap silently squashed useful injections
in practice. Override with `MUNINN_HOOK_DEADLINE_MS` (env var) when
testing failure modes.

## Installation

The recommended path is the `muninn install-cc` CLI. Until then,
point Claude Code at the plugin directly by adding this repo as a
local plugin source.

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
