<p align="center">
  <img src="image.png" alt="Muninn" width="400">
</p>

<h1 align="center">Muninn</h1>

<p align="center">
  <strong>Privacy-first recursive context gateway for agentic coding</strong>
</p>

<p align="center">
  <a href="#installation">Installation</a> •
  <a href="#using-muninn-with-claude-code">Quick Start</a> •
  <a href="#configuration">Configuration</a> •
  <a href="#tested-backends-and-known-flakiness">Tested Backends</a> •
  <a href="#how-it-works">How It Works</a>
</p>

---

Named for Odin's raven of Memory — Muninn enables AI coding agents to understand large codebases without sacrificing privacy, burning through your token budget, or suffering from session amnesia.

**Built for developers on Claude Pro or Max plans** who want to stretch their token budgets further. Muninn offloads expensive codebase exploration to fast, cheap models so Claude only sees what matters.

## The Problem

AI coding assistants face a trilemma:

1. **Context limits kill productivity** — LLMs hallucinate when they can't see the full picture
2. **Privacy matters** — Many developers can't send proprietary code to cloud providers
3. **Persistent amnesia** — Every session starts fresh, re-learning the codebase from scratch

## How muninn plugs into Claude Code

Muninn ships as a Claude Code plugin: a UserPromptSubmit hook that
pre-answers each user prompt via a cheap local model and injects the
result, plus an MCP server exposing `search_code` and `query_graph`.
Both surfaces are backed by a single local daemon and use CC's
native extension points — no HTTP intercept, no protocol mimicry.

## Installation

### Step 1 — install the binary

```bash
# Linux/macOS prebuilt:
curl -fsSL https://raw.githubusercontent.com/colliery-io/muninn/main/install.sh | bash

# Or from source (Rust 1.85+, workspace edition 2024):
git clone https://github.com/colliery-io/muninn.git
cd muninn
cargo install --path crates/muninn --locked
```

`--locked` is important: it tells cargo to use the committed
`Cargo.lock` rather than re-resolving dependencies fresh. Without
it you may pick up newer upstream patch releases than what muninn
has been tested against (some have published breaking changes
within a `0.x.y` line).

Verify with `muninn --version`. The installer drops the binary in `~/.local/bin/`; make sure that's on your `PATH`.

### Step 2 — initialize the project

Inside the repo you want muninn to know about:

```bash
muninn init
```

This creates `.muninn/config.toml` with a sensible tiered config (Ollama Cloud + `gemma4:31b` as the default for router and RLM), plus the `.muninn/` directory where muninn keeps its graph index, sessions, and traces. `.muninn/` is per-developer state — keep it gitignored.

### Step 3 — provide a backend credential

The out-of-the-box config talks to Ollama Cloud. Get an [Ollama Cloud](https://ollama.com) API key (free tier works) and put it in your config:

```toml
# .muninn/config.toml
[ollama]
api_key = "..."
```

You can also export `OLLAMA_API_KEY` in your shell — but be aware: when Claude Code launches muninn's hook + MCP subprocesses, they may not inherit your interactive shell's environment (especially if you started CC from a desktop launcher rather than a terminal). Putting the key in `.muninn/config.toml` is the most reliable path. The same applies to `GROQ_API_KEY` / `ANTHROPIC_API_KEY`.

For Groq, Anthropic direct, or a local Ollama daemon, see [Configuration](#configuration).

That's the binary side done. Now wire it into your agent.

## Using muninn with Claude Code

This is the supported integration path. Muninn registers as a regular
plugin and a regular MCP server, sharing a single local daemon for
both surfaces.

### 1. Register the MCP server

From inside your repo:

```bash
muninn install-cc
```

This writes a `.mcp.json` at the repo root:

```json
{
  "mcpServers": {
    "muninn": { "command": "muninn", "args": ["mcp"], "env": {} }
  }
}
```

For a user-wide install (applies to every CC session):

```bash
muninn install-cc --global
```

Use `--dry-run` to preview without writing. Roll back with `muninn uninstall-cc [--global]`.

### 2. Install the UserPromptSubmit plugin

From inside Claude Code, add the muninn marketplace once, then install the plugin:

```
/plugin marketplace add colliery-io/muninn
/plugin install muninn-cc
```

Claude Code clones the repo to its plugin cache and loads the hook from there. To pull plugin updates later, run `/plugin marketplace update muninn` inside CC.

**Hacking on the plugin source?** If you're developing against a local checkout rather than the GitHub copy, load it directly with `--plugin-dir` at session start:

```bash
claude --plugin-dir /absolute/path/to/muninn/plugins/muninn-cc
```

`/reload-plugins` picks up plugin file edits without a full restart.

The plugin's UserPromptSubmit hook fires once per user turn: a cheap router model decides whether the prompt needs exploration; if it does, muninn drives its recursive exploration loop on the configured local/cheap backend and injects the result as `additionalContext`, framed as the answer for Claude to deliver. **Failure mode is always silent passthrough** — the hook never blocks the user's turn. See [`plugins/muninn-cc/README.md`](plugins/muninn-cc/README.md).

### 3. (No action needed) the daemon

You don't need to start the daemon explicitly. Both surfaces self-bootstrap:

- `muninn mcp` auto-ensures the daemon when CC connects on session start.
- The hook script (`user-prompt-submit.sh`) calls `muninn daemon ensure` ahead of `muninn hook submit`, which is a no-op when the daemon is already alive.

Whichever surface fires first that turn starts the daemon; subsequent calls reuse the same socket. If you want to inspect or control it manually: `muninn daemon status` / `muninn daemon ensure` / `muninn daemon stop`.

### Available MCP tools

Once installed, Claude Code can call:

- **`search_code`** — ranked, scoped text/regex matches in the working tree
- **`query_graph`** — callers / callees / definitions via the code graph

Full schema reference: [`docs/mcp-tools.md`](docs/mcp-tools.md). Other
context-injection surfaces (dependency docs, persistent memory) are
explicitly deferred from v1 — muninn v1 is positioned as an
RLM-driven hook with a minimal explicit-tool surface.

### Optional — index the code graph

`query_graph` returns empty results against a fresh `.muninn/graph.db`. To populate it for this repo:

```bash
muninn index
```

The `search_code` MCP tool works without the graph (it walks the filesystem directly). Indexing only unlocks `query_graph`.

## Configuration

Muninn stores data in `.muninn/` within your project:

```
.muninn/
├── config.toml         # tiered config (provider/model)
├── graph.db            # code graph
├── docs.db             # indexed library docs
└── sessions/           # per-session logs and traces
```

### Tiered config

`[default]` is the baseline. `[router]` and `[rlm]` each accept optional `provider` / `model` overrides; unset fields inherit from `[default]`. The minimal config is empty — defaults handle the rest.

Worked example — tier a cheap router with a stronger but still
cheap RLM, both within the same provider:

```toml
[default]
provider = "ollama"
model = "gemma4:31b"

[router]
# Cheap/fast model for routing decisions. Inherits provider from [default].
model = "gemma4:9b"

[ollama]
api_key = "..."
```

Or split across providers — `[router]` on Groq's fastest, `[rlm]` on
Ollama's bigger Gemma:

```toml
[default]
provider = "ollama"
model = "gemma4:31b"

[router]
provider = "groq"
model = "llama-3.1-8b-instant"

[ollama]
api_key = "..."

[groq]
api_key = "gsk_..."
```

Switching providers entirely is one section:

```toml
[default]
provider = "groq"
model = "qwen/qwen3-32b"

[router]
model = "llama-3.1-8b-instant"

[groq]
api_key = "gsk_..."
```

> **Don't put Anthropic (Claude) under the RLM.** The whole point of
> muninn is to keep expensive Claude-shaped inference on the Claude
> Code side and offload exploration to cheap models. The Anthropic
> adapter exists in the binary because earlier proxy work used it,
> but pointing `[rlm]` at Claude defeats the cost story.

### Local Inference with Ollama

For fully local inference, override the Ollama base URL:

```toml
[ollama]
base_url = "http://localhost:11434/v1"
# Optional: bound network retries. Default 3 × 500ms backoff. Set to 0
# to fail fast against a flapping or unreachable backend.
max_retries = 3
```

```bash
ollama serve
ollama pull gemma4:31b
```

## Tested backends and known flakiness

The muninn engine runs the LLM via OpenAI-shaped chat completions and
expects providers to return structured `tool_calls` in their
responses. Different models in different provider catalogs honor that
shape with different levels of reliability — the table below records
what we actually exercised end-to-end against muninn's UAT suite
(18 integration tests under `crates/muninn/tests/`, invoked via
`angreal test uat --provider <name>`).

| Provider | Model | UAT result | Notes |
|---|---|---|---|
| **Ollama Cloud** | `gemma4:31b` (RLM + router) | clean — default `[default]` config | The out-of-the-box configuration. Gemma respects the OpenAI tool-call shape; no observed format flakiness. |
| **Groq** | `qwen/qwen3-32b` (RLM) + `llama-3.1-8b-instant` (router) | clean on a fresh run; intermittent flakes recovered by retry | qwen3 occasionally emits its native `<tool_call>…</tool_call>` wrapper which Groq's strict validator rejects. Muninn's per-backend retry (`max_retries: 3`, 500 ms exponential backoff) resamples and recovers. |
| **Groq** | `openai/gpt-oss-120b` (RLM) | ~16/18 | Sporadic empty-content responses where the model emits text only in non-default harmony channels we don't currently parse. Usable but less reliable than qwen3 on Groq. |
| **Groq** | `llama-3.3-70b-versatile` (RLM) | retry-dependent | Emits `<function=name>{…}</function>` inline format on some prompt shapes; retry usually recovers but can exhaust on deterministic prompts. |
| **Groq** | `openai/gpt-oss-20b` (RLM) | **not recommended** | Deterministically leaks `<\|channel\|>commentary` harmony control tokens into tool names on common prompt shapes. Retry can't fix a deterministic format failure; pick one of the models above instead. |

> The Anthropic adapter is in the codebase and works, but we don't
> test or recommend it for the RLM tier — muninn's whole pitch is to
> keep the expensive Claude-shaped inference on the Claude Code side
> (where you're already paying for it via your Pro/Max plan) and run
> the recursive exploration loop on cheap Ollama-Cloud / Groq /
> local models. Putting Claude under the hood as the RLM erases the
> savings.

### Retry contract

Backend errors that match known model-format-flake patterns
(`"Failed to call a function"`, `"Failed to parse tool call
arguments"`, `"Server error: …"`) and transient network errors are
retried inside the backend client with exponential backoff. The retry
fires **at the LLM API call level**, not the engine or test level —
so when a flake recovers, the RLM's exploration loop never sees the
failure, and prior tool calls / model decisions in the current
exploration are preserved across the retry. Worst-case extra cost per
recovered flake: 3 LLM calls (the failing one plus up to 3 retries).

### Picking a model for your stack

- **You want the supported default**: keep the out-of-the-box config.
  Ollama Cloud + `gemma4:31b` is what we recommend and what UAT runs
  against in CI.
- **You want fast/cheap routing with stronger RLM**: tier them. The
  worked example in [Configuration](#tiered-config) shows
  `[router]` on a small model and `[rlm]` on a bigger one.
- **You're on Groq**: prefer `qwen/qwen3-32b` for the RLM and
  `llama-3.1-8b-instant` for the router. Other Groq models work
  but with varying reliability; see the table above.

## How It Works

```
┌──────────────────────┐  UserPromptSubmit  ┌─────────────────────────┐
│  Claude Code         │ ─────hook────────▶ │ muninn-cc plugin        │
│                      │     tools/list     │ user-prompt-submit.sh   │
│                      │ ──────MCP───────┐  └────────────┬────────────┘
└──────────────────────┘                 │               │ shells out
                                         │               ▼
                                         │     ┌──────────────────┐
                                         │     │  muninn          │
                                         │     │  hook submit     │
                                         │     └────────┬─────────┘
                                         ▼              ▼
                            ┌──────────────────────────┐
                            │  muninn daemon           │
                            │   ┌─────────────────┐    │
                            │   │ MuninnEngine    │    │
                            │   │  complete (RLM) │    │
                            │   │  search_code    │    │
                            │   │  query_graph    │    │
                            │   └────────┬────────┘    │
                            └────────────┼─────────────┘
                                         ▼
                            ┌──────────────────────────┐
                            │  LLM backend             │
                            │  (Ollama Cloud / Groq /  │
                            │   Anthropic / local)     │
                            └──────────────────────────┘
```

Once per user turn, a cheap router decides whether muninn should
explore; on RLM, the recursive engine runs on the configured local
backend and the result is injected as the answer for Claude to
deliver. The MCP server exposes `search_code` and `query_graph` as
on-demand tools. Both surfaces share one local daemon.

## License

[Apache License 2.0](LICENSE)

---

<p align="center">
  <sub>Built by <a href="https://github.com/colliery-io">Colliery</a></sub>
</p>
