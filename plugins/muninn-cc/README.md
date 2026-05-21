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
- A running muninn daemon (the binary starts one automatically on first
  use via `muninn daemon ensure`).
- A muninn `.muninn/config.toml` configured for at least one provider.
  See the top-level README for the tiered-config defaults.

The hook is engineered for **silent passthrough on any failure**: if
`muninn` is missing, errors, or times out, Claude Code processes the
user's prompt with no injection. The plugin never blocks the user's
turn (NFR-002).

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
