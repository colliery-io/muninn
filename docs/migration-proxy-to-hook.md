# Migration: proxy-only → hook + MCP

If you've been running muninn as an HTTP proxy in front of Claude Code,
this page walks you through enabling the new **hook + MCP** integration
that uses CC's native extension points. **Both surfaces are
first-class** — you can keep the proxy running for other tooling and
add the hook + MCP path on top, or switch entirely. See
[ADR-0003](../.metis/adrs/PROJEC-A-0003.md) for the rationale.

## TL;DR

```sh
# 1. Make sure muninn is on PATH and current.
which muninn

# 2. Register the MCP server with Claude Code (project scope; --global also works).
muninn install-cc

# 3. From inside a CC session in this repo, install the PreToolUse hook plugin.
/plugin add-source ./plugins/muninn-cc

# 4. (Optional) Pin a faster decision model for the hot Grep/Read/Glob path.
$EDITOR .muninn/config.toml   # add [hook_decision] with a small/fast model
```

That's the whole migration. The proxy keeps working unchanged — nothing
about your existing `muninn proxy` setup gets touched.

## What changed under the hood

| Surface | What you ran before | What you can run now |
|---|---|---|
| Claude Code | `muninn claude` (proxy intercept) | Direct CC session + `muninn install-cc` for MCP + `/plugin add-source ./plugins/muninn-cc` for the PreToolUse hook |
| Other agents (Cursor, Continue, Aider, custom OpenAI/Anthropic clients) | `muninn proxy --port 8080` | **Same.** No changes here. |
| Engine internals | Proxy held a `RecursiveEngine` directly | All adapters consume `Arc<dyn MuninnEngine>` via a local daemon. Same engine, cleaner boundary. |

The new surfaces are layered on top of the existing engine via a small
local daemon (`muninn daemon`) — the proxy now reaches the same engine
through the same trait abstraction. No re-implementation, no
duplicate logic.

## Why migrate at all?

1. **Sanctioned extension points.** The hook + MCP path uses Claude
   Code's own plugin and MCP machinery — no protocol mimicry, no
   intercepting authenticated sessions. ToS-safe by construction.
2. **Implicit context augmentation.** Per-Grep / per-Read decisions
   can attach Muninn context (related symbols, callers, prior memory)
   to the agent's tool results without the agent having to know
   muninn exists.
3. **Explicit MCP tools.** When the agent does want to ask muninn
   directly — "search docs for tokio joinsets", "who calls this
   function" — the MCP server surfaces `search_code`, `query_graph`,
   `recall_memory`, and `search_docs` as first-class tools.
4. **No proxy bypass needed for non-Claude calls.** Muninn doesn't
   sit in the request path on the CC side anymore — you can leave
   `muninn proxy` running for your other tools without it interfering
   with CC.

## Step-by-step

### 1. Verify your muninn build is current

The hook + MCP path needs the subcommands introduced in this
initiative:

```sh
muninn --help 2>&1 | grep -E "hook|mcp|install-cc|daemon"
```

You should see `hook`, `mcp`, `install-cc`, `uninstall-cc`, and
`daemon` listed. If not, rebuild from the current source.

### 2. Register the MCP server

```sh
cd <your project>
muninn install-cc
```

This creates (or updates) `.mcp.json` at the repo root with a single
entry pointing at `muninn mcp`. Pre-existing `mcpServers` entries are
preserved; only the `muninn` key is touched. Re-running is a no-op if
the entry already matches.

Options:

| Flag | Effect |
|---|---|
| `--global` | Write to `~/.claude.json` instead of `.mcp.json`. Applies to every CC session for your user. |
| `--dry-run` | Print what would change without writing. |

Roll back with `muninn uninstall-cc [--global]`.

### 3. Install the PreToolUse plugin

This is the piece that lets muninn augment / rewrite Grep / Read /
Glob calls. From inside a Claude Code session in this repo, run:

```
/plugin add-source ./plugins/muninn-cc
```

(Exact incantation depends on your CC version. See
[`plugins/muninn-cc/README.md`](../plugins/muninn-cc/README.md).)

The plugin is intentionally thin — its shell entry delegates every
real decision to `muninn hook decide`. If the muninn binary is
missing or errors, the hook silently passes the tool call through.
This is the **NFR-002 contract**: muninn never blocks your tool call.

### 4. (Optional) Tune the decision model

The hot path through the hook fires per Grep / Read / Glob, so model
choice matters for end-to-end latency. By default the hook inherits
the tiered config's `[default]` provider / model — which is Ollama
Cloud + `gemma4:31b` out of the box. At time of writing the reference
benchmark on M-series Mac:

```
p50: 290 ms  p95: 508 ms  p99: 508 ms (= 500ms internal cap)
warmup (cold subprocess): ~990 ms
```

That misses the NFR-001 target of p50 ≤ 100 ms. Pin a smaller, faster
model for the hook:

```toml
# .muninn/config.toml
[hook_decision]
provider = "groq"
model = "llama-3.1-8b-instant"

[groq]
api_key = "gsk_..."  # or use GROQ_API_KEY env var
```

`[router]` and `[rlm]` stay on the heavier default — they don't fire
on the per-tool-call hot path.

To measure your own setup:

```sh
cargo build --release -p muninn
angreal bench hook --iters 100
```

The benchmark emits a JSON report on stdout (good for diffing across
commits) and a PASS/FAIL verdict against NFR-001 on stderr.

### 5. (Optional) Keep the proxy running

The hook + MCP path doesn't replace the proxy — they coexist. Keep
`muninn proxy` running if:

- You drive non-Claude-Code agents (Cursor, Continue, Aider, custom
  scripts) through OpenAI- or Anthropic-compatible endpoints.
- You're on Claude MAX and use the `/v1/chat/completions`
  pass-through for any tool that speaks OpenAI format.
- You want the existing `@muninn explore` / `@muninn passthrough`
  prompt triggers.

The proxy adapter now consumes the same `MuninnEngine` trait
internally, so a single muninn binary + daemon serves both surfaces
in parallel. No port conflicts, no duplicated state.

## Verification

After install, sanity-check the wiring:

```sh
# Daemon should auto-start on first hook/MCP invocation; you can also
# start it explicitly.
muninn daemon status
muninn daemon ensure
muninn daemon status      # should now report `alive`

# MCP server smoke test (Ctrl-C to exit).
muninn mcp --no-ensure < /dev/null

# Real end-to-end hook decision against the configured backend.
angreal test uat
```

The last command runs the same UAT smoke that's in CI:
spawns `muninn hook decide` with a canned CC payload and asserts
either passthrough (empty stdout) or a well-formed augment envelope,
within a 10 s wall-clock budget.

## Troubleshooting

### `muninn hook decide` always falls through to passthrough

The decision model can't always articulate a reason to augment, and
the system prompt biases it toward passthrough. That's by design — a
hook that augments every call would balloon the agent's context.

If you want to confirm the path is reaching the model at all:

```sh
echo '{"tool_name":"Grep","tool_input":{"pattern":"fn main"}}' \
  | muninn hook decide -v
```

The `-v` (verbose) flag emits decision-model timing and parsed
results to stderr.

### `muninn install-cc` says "already present" but CC doesn't see the server

Check that your CC session was started **after** `install-cc` ran —
CC reads `.mcp.json` at session start. Restart CC and look for
`muninn` in its MCP server list.

### Hook latency is too high

The benchmark in section 4 above is the source of truth. The most
common fix is pinning a smaller `[hook_decision]` model. Other
options: run a local quantized model (override `[ollama] base_url =
"http://localhost:11434/v1"`); or pre-warm the daemon and your
provider's prompt cache so steady-state runs faster.

### Daemon won't start

```sh
muninn daemon start -v
```

Reports binding errors and config-resolution issues to stderr. Most
common cause: a stale socket from a previous crashed daemon; the
binder unlinks it automatically, so `daemon start` should succeed on
the second attempt. If not, check that `$XDG_RUNTIME_DIR/muninn/` (or
`~/Library/Caches/muninn/` on macOS) is writable.

### I want to roll back

```sh
muninn uninstall-cc          # remove the MCP entry from .mcp.json
# Then in CC: `/plugin remove muninn-cc` (or equivalent for your version)
muninn daemon stop           # if the daemon is running
```

The proxy path is unaffected.

## See also

- [ADR-0003](../.metis/adrs/PROJEC-A-0003.md) — why hook + MCP became
  the primary surface and why the proxy stays first-class.
- [`docs/mcp-tools.md`](mcp-tools.md) — the curated MCP tool surface
  the agent sees.
- [`plugins/muninn-cc/README.md`](../plugins/muninn-cc/README.md) —
  hook plugin details and failure semantics.
- [PROJEC-I-0011](../.metis/initiatives/PROJEC-I-0011/initiative.md) —
  the initiative that built all of this.
