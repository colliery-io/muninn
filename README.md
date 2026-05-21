<p align="center">
  <img src="image.png" alt="Muninn" width="400">
</p>

<h1 align="center">Muninn</h1>

<p align="center">
  <strong>Privacy-first recursive context gateway for agentic coding</strong>
</p>

<p align="center">
  <a href="#installation">Installation</a> •
  <a href="#using-muninn-with-claude-code-recommended">Quick Start (Claude Code)</a> •
  <a href="#using-muninn-with-other-clients-proxy">Other Clients</a> •
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

## Two integration surfaces

Muninn ships **two ways** to plug into your agent. They share the same engine; pick whichever fits your setup.

| Surface | When to use | What it gives you |
|---|---|---|
| **Hook + MCP (Claude Code)** | You're using Claude Code (the primary recommendation). | A UserPromptSubmit hook that pre-answers each user prompt via a cheap local model and injects the result, plus an MCP server exposing `search_code`, `query_graph`, `search_docs`. Backed by a single local daemon. Sanctioned by CC's own extension points. |
| **Proxy (everyone else)** | Cursor / Continue / Aider / any OpenAI- or Anthropic-compatible client. | A drop-in HTTP proxy that intercepts requests and routes them through a recursive exploration engine when appropriate. Same engine, different adapter. |

See [ADR-0003](.metis/adrs/PROJEC-A-0003.md) for the rationale behind keeping both.

## Installation

### Quick Install (Linux/macOS)

```bash
curl -fsSL https://raw.githubusercontent.com/colliery-io/muninn/main/install.sh | bash
```

### From Source

Requires Rust 1.85+ (workspace edition 2024):

```bash
git clone https://github.com/colliery-io/muninn.git
cd muninn
cargo build --release
```

### Provider credentials

Muninn's tiered config defaults to Ollama Cloud + `gemma4:31b` for both router and recursive exploration. Get an [Ollama Cloud](https://ollama.com) API key and export it:

```bash
export OLLAMA_API_KEY="..."
```

To use a different provider (Groq, Anthropic, local Ollama, …), see [Configuration](#configuration).

## Using muninn with Claude Code (recommended)

This path uses CC's native hook + MCP extension points. No HTTP intercept, no protocol mimicry — muninn shows up as a regular plugin and a regular MCP server.

### 1. Register the MCP server

In your repo, run:

```bash
muninn install-cc
```

This writes a `.mcp.json` at the repo root with:

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

Use `--dry-run` to preview without writing. The corresponding uninstall is `muninn uninstall-cc [--global]`.

### 2. Install the UserPromptSubmit plugin

From inside a Claude Code session in this repo:

```
/plugin add-source ./plugins/muninn-cc
```

The plugin's UserPromptSubmit hook fires once per user turn: a cheap router model decides whether the prompt needs exploration; if it does, muninn drives its recursive exploration loop on the configured local/cheap backend and injects the result as `additionalContext`, framed as the answer for Claude to deliver. **Failure mode is always silent passthrough** — the hook never blocks the user's turn. See [`plugins/muninn-cc/README.md`](plugins/muninn-cc/README.md).

### Available MCP tools

Once installed, Claude Code can call:

- **`search_code`** — ranked, scoped text/regex matches in the working tree
- **`query_graph`** — callers / callees / definitions / references via the code graph
- **`search_docs`** — indexed library documentation (crates.io / PyPI)

Full schema reference: [`docs/mcp-tools.md`](docs/mcp-tools.md).

## Using muninn with other clients (proxy)

If you're not on Claude Code — Cursor, Continue, Aider, custom scripts hitting OpenAI- or Anthropic-compatible endpoints — muninn's HTTP proxy is the equivalent surface. Same engine, same intelligence; different way of getting requests to it.

### Running the proxy

```bash
# Start proxy on a specific port (default auto-selects)
muninn proxy --port 8080

# With verbose logging
muninn proxy --port 8080 -v

# Force routing strategy
muninn proxy --port 8080 --router always-passthrough
```

### Routing

The router decides whether requests need RLM processing or can pass through directly:

- **Passthrough**: Simple commands, log analysis, follow-up questions
- **RLM**: "How does authentication work?", "Find the router implementation"

Force routing with triggers in the prompt:
- `@muninn explore` — Force RLM exploration
- `@muninn passthrough` — Force direct passthrough

### Direct Claude access via OpenAI-compatible API

If you have a Claude MAX subscription, the proxy unlocks it for any OpenAI-compatible tool:

```
POST /v1/chat/completions
```

First, authenticate once with `muninn oauth`, then point your tools at `http://localhost:8080/v1/chat/completions`.

**Why use this?**

- **Flat-rate billing**: Your MAX subscription includes inference costs — no per-token API charges
- **Use any client**: Connect Cursor, Continue, Aider, or any tool that speaks OpenAI format
- **No RLM overhead**: Bypass the router entirely for simple, direct Claude access

#### Request format (OpenAI-style)

```json
{
  "model": "claude-sonnet-4-20250514",
  "messages": [
    {"role": "system", "content": "You are a helpful assistant."},
    {"role": "user", "content": "Hello!"}
  ],
  "max_tokens": 1024,
  "stream": false
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `model` | string | Yes | Claude model ID (e.g., `claude-sonnet-4-20250514`) |
| `messages` | array | Yes | Conversation messages with `role` and `content` |
| `max_tokens` | integer | Yes | Maximum tokens to generate |
| `stream` | boolean | No | Enable streaming (default: `false`) |
| `temperature` | float | No | Sampling temperature (0.0-1.0) |
| `top_p` | float | No | Nucleus sampling parameter |
| `stop_sequences` | array | No | Stop sequences |
| `tools` | array | No | Tool definitions (passed through to Claude) |
| `tool_choice` | object | No | Tool choice configuration |

Tools are fully supported and passed through to Claude.

Response is returned in Anthropic's native format (see streaming notes below for SSE).

#### Example

```bash
muninn oauth                      # one-time
muninn proxy --port 8080          # in a terminal

curl -X POST http://localhost:8080/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "claude-sonnet-4-20250514",
    "messages": [{"role": "user", "content": "Hello!"}],
    "max_tokens": 100
  }'
```

For streaming responses, set `"stream": true` and the response is returned as Server-Sent Events.

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

Worked example tuning for cost/quality:

```toml
[default]
provider = "ollama"
model = "gemma4:31b"

[router]
# Cheap/fast model for routing decisions. Inherits provider from [default].
model = "gemma4:9b"

[rlm]
# Bigger model for deep recursive exploration. Overrides both.
provider = "anthropic"
model = "claude-haiku-4-5-20251001"

[anthropic]
api_key = "sk-..."

[groq]
api_key = "gsk_..."
```

Switching providers entirely is one section:

```toml
[default]
provider = "groq"
model = "llama-3.1-8b-instant"

[groq]
api_key = "gsk_..."
```

### Local Inference with Ollama

For fully local inference, override the Ollama base URL:

```toml
[ollama]
base_url = "http://localhost:11434/v1"
```

```bash
ollama serve
ollama pull gemma4:31b
```

## How It Works

```
┌──────────────────────┐  UserPromptSubmit  ┌─────────────────────────┐
│  Claude Code         │ ─────hook────────▶ │ muninn-cc plugin        │
│                      │     tools/list     │ user-prompt-submit.sh   │
│                      │ ──────MCP───────┐  └────────────┬────────────┘
└──────────────────────┘                 │               │ shells out
                                         │               ▼
┌──────────────────────┐                 │     ┌──────────────────┐
│  Cursor / Continue / │                 │     │  muninn          │
│  Aider / custom      │                 │     │  hook submit     │
│  OpenAI-compatible   │ ─────HTTP─────▶ │     └────────┬─────────┘
└──────────────────────┘                 ▼              ▼
                            ┌──────────────────────────┐
                            │  muninn daemon           │
                            │   ┌─────────────────┐    │
                            │   │ MuninnEngine    │    │
                            │   │  search_code    │    │
                            │   │  query_graph    │    │
                            │   │  search_docs    │    │
                            │   │  explore (RLM)  │    │
                            │   │  complete       │    │
                            │   └────────┬────────┘    │
                            └────────────┼─────────────┘
                                         ▼
                            ┌──────────────────────────┐
                            │  LLM backend             │
                            │  (Ollama Cloud / Groq /  │
                            │   Anthropic / local)     │
                            └──────────────────────────┘
```

- **Hook + MCP**: once per user turn, a cheap router decides whether muninn should explore; on rlm, the recursive engine runs on the configured local backend and the result is injected as the answer for Claude to deliver.
- **Proxy**: HTTP intercept routes each chat-completions request through the same engine — same MuninnEngine trait, same daemon, just a different adapter.

## License

[Apache License 2.0](LICENSE)

---

<p align="center">
  <sub>Built by <a href="https://github.com/colliery-io">Colliery</a></sub>
</p>
