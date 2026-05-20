# muninn-cc

Claude Code plugin that augments `Grep` / `Read` / `Glob` tool calls with
muninn's persistent context.

## What it does

On every `Grep`, `Read`, or `Glob` Claude Code is about to issue, the
plugin's PreToolUse hook hands the call to a local muninn daemon. A small
"decision model" running inside muninn picks one of three outcomes:

- **Passthrough** — the tool runs unchanged. No latency cost beyond the
  decision-model call.
- **Augment** — the tool runs normally, and muninn attaches an
  `additionalContext` block (related symbols, callers/callees, prior
  memory) capped at ~2 KB.
- **Rewrite** — muninn short-circuits the original tool and returns the
  answer from one of its engine methods (`search_code`, `query_graph`,
  …) instead.

The plugin itself is intentionally thin — a shell entry that shells out
to the `muninn` binary. All real work happens inside muninn so updating
the binary updates the plugin's behavior.

## Requirements

- The `muninn` binary on `PATH`.
- A running muninn daemon (the binary starts one automatically on first
  use via `muninn daemon ensure`).
- A muninn `.muninn/config.toml` configured for at least one provider.
  See the top-level README for the tiered-config defaults.

The hook is engineered for **silent passthrough on any failure**: if
`muninn` is missing, errors, or times out, Claude Code runs the original
tool unchanged. The plugin never blocks the user's tool call. (See
NFR-002 in PROJEC-I-0011.)

## Installation

The recommended path is the `muninn install-cc` CLI, shipping in
PROJEC-T-0072. Until then, point Claude Code at the plugin directly by
adding this repo as a local plugin source.

## Layout

```
plugins/muninn-cc/
├── .claude-plugin/
│   └── plugin.json          # plugin manifest (name, version, metadata)
├── hooks/
│   ├── hooks.json           # registers the PreToolUse hook on Grep|Read|Glob
│   └── pre-tool-use.sh      # shell entry point — shells out to `muninn hook decide`
└── README.md                # this file
```

## See also

- ADR-0003 — why muninn uses hooks + MCP as the primary Claude Code
  integration surface.
- PROJEC-I-0011 — the broader hook + MCP integration initiative.
- PROJEC-T-0070 — the `muninn hook decide` subcommand the script
  delegates to.
- PROJEC-T-0072 — the `muninn install-cc` installer.
