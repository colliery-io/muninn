<p align="center">
  <img src="image.png" alt="Muninn" width="400">
</p>

<h1 align="center">Muninn</h1>

<p align="center">
  <strong>Privacy-first recursive context gateway for agentic coding</strong>
</p>

<p align="center">
  <a href="#installation">Installation</a> •
  <a href="#quick-start">Quick Start</a> •
  <a href="#how-it-works">How It Works</a> •
  <a href="#license">License</a>
</p>

---

Named for Odin's raven of Memory — Muninn enables AI coding agents to understand large codebases without sacrificing privacy, burning through your token budget, or suffering from session amnesia.

**Built for developers on Claude Pro or Max plans** who want to stretch their token budgets further. Muninn offloads expensive codebase exploration to fast, cheap models (like Groq) so Claude only sees what matters.

## The Problem

AI coding assistants face a trilemma:

1. **Context limits kill productivity** — LLMs hallucinate when they can't see the full picture
2. **Privacy matters** — Many developers can't send proprietary code to cloud providers
3. **Persistent amnesia** — Every session starts fresh, re-learning the codebase from scratch

## The Solution

Muninn is a **recursive context gateway** that sits between AI coding agents (like Claude Code) and LLM backends.

**Context tokens become compute, not storage.**

Instead of stuffing millions of tokens into a prompt, Muninn uses Recursive Language Model (RLM) techniques to let the LLM programmatically explore and selectively retrieve only the context that matters.

## Installation

### Quick Install (Linux/macOS)

```bash
curl -fsSL https://raw.githubusercontent.com/colliery-io/muninn/main/install.sh | bash
```

### From Source

Requires Rust 1.85+ (nightly):

```bash
git clone https://github.com/colliery-io/muninn.git
cd muninn
cargo build --release
```

### Local Inference with Ollama

For fully local inference, Muninn supports [Ollama](https://ollama.ai):

```bash
# Install Ollama, then:
ollama serve
ollama pull gpt-oss:20b  # or any model you prefer
```

Then configure Muninn to use it:
```toml
[rlm]
provider = "ollama"
model = "gpt-oss:20b"
```

## Quick Start

1. Create a `.muninn/config.toml` in your project:

```toml
# Router uses a fast model to classify requests
[router]
provider = "groq"
model = "llama-3.1-8b-instant"

# RLM uses a capable model for code exploration
[rlm]
provider = "groq"
model = "qwen/qwen3-32b"

# API keys (or use GROQ_API_KEY / ANTHROPIC_API_KEY env vars)
[groq]
api_key = "your-groq-api-key"

# For local inference with Ollama:
# [rlm]
# provider = "ollama"
# model = "gpt-oss:20b"
```

2. Run Claude Code through Muninn:

```bash
muninn claude
```

That's it. Muninn starts the proxy, launches Claude Code with the correct configuration, and intelligently routes requests through RLM when codebase exploration is needed.

## How It Works

```
┌─────────────┐     ┌─────────────┐     ┌─────────────┐
│ Claude Code │────▶│   Muninn    │────▶│ LLM Backend │
└─────────────┘     │   (Proxy)   │     └─────────────┘
                    │             │
                    │  ┌───────┐  │
                    │  │Router │  │  "Does this need
                    │  └───┬───┘  │   code exploration?"
                    │      │      │
                    │  ┌───▼───┐  │
                    │  │  RLM  │  │  Recursive exploration
                    │  │Engine │  │  with tools (grep, read, etc.)
                    │  └───────┘  │
                    └─────────────┘
```

### Routing

The router decides whether requests need RLM processing or can pass through directly:

- **Passthrough**: Simple commands, log analysis, follow-up questions
- **RLM**: "How does authentication work?", "Find the router implementation"

Force routing with triggers:
- `@muninn explore` — Force RLM exploration
- `@muninn passthrough` — Force direct passthrough

## Running the Proxy Standalone

While `muninn claude` handles everything automatically, you can also run the proxy standalone:

```bash
# Start proxy on a specific port
muninn proxy --port 8080

# With verbose logging
muninn proxy --port 8080 -v

# With a specific routing strategy
muninn proxy --port 8080 --router always-passthrough
```

By default, `--port 0` auto-selects an available port. Specify a port explicitly when you need a predictable address.

## Direct Claude Access (OpenAI-Compatible)

If you have a Claude MAX subscription ($100/month or $200/month), you're paying for flat-rate inference — but that's normally only accessible through Claude Code or claude.ai.

Muninn unlocks your MAX subscription for any OpenAI-compatible tool or library:

```
POST /v1/chat/completions
```

**Why use this?**

- **Flat-rate billing**: Your MAX subscription includes inference costs — no per-token API charges
- **Use any client**: Connect Cursor, Continue, Aider, or any tool that speaks OpenAI format
- **No RLM overhead**: Bypass the router entirely for simple, direct Claude access
- **Your existing tools**: Libraries like LangChain, LlamaIndex, or custom scripts just work

First, authenticate once with `muninn oauth`, then point your tools at `http://localhost:8080/v1/chat/completions`.

### Request Format (OpenAI-style)

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

**Note on Tools**: Tools are fully supported and passed through to Claude. While not required for OAuth validation, any tools you include in your request will work normally.

### Response Format (Anthropic-style)

The response is returned in Anthropic's native format:

```json
{
  "id": "msg_01XYZ...",
  "type": "message",
  "role": "assistant",
  "content": [
    {
      "type": "text",
      "text": "Hello! How can I help you today?"
    }
  ],
  "model": "claude-sonnet-4-20250514",
  "stop_reason": "end_turn",
  "usage": {
    "input_tokens": 25,
    "output_tokens": 15
  }
}
```

When Claude uses tools, the response includes tool use blocks:

```json
{
  "id": "msg_01XYZ...",
  "type": "message",
  "role": "assistant",
  "content": [
    {
      "type": "tool_use",
      "id": "toolu_01ABC...",
      "name": "get_weather",
      "input": {"location": "San Francisco"}
    }
  ],
  "model": "claude-sonnet-4-20250514",
  "stop_reason": "tool_use",
  "usage": {
    "input_tokens": 50,
    "output_tokens": 35
  }
}
```

### Example Usage

```bash
# First, authenticate with Claude MAX
muninn oauth

# Start the proxy on a specific port (default auto-selects a random port)
muninn proxy --port 8080

# Simple request
curl -X POST http://localhost:8080/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "claude-sonnet-4-20250514",
    "messages": [{"role": "user", "content": "Hello!"}],
    "max_tokens": 100
  }'

# Request with tools
curl -X POST http://localhost:8080/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "claude-sonnet-4-20250514",
    "messages": [{"role": "user", "content": "What is the weather in SF?"}],
    "max_tokens": 1024,
    "tools": [{
      "name": "get_weather",
      "description": "Get weather for a location",
      "input_schema": {
        "type": "object",
        "properties": {
          "location": {"type": "string", "description": "City name"}
        },
        "required": ["location"]
      }
    }]
  }'
```

For streaming responses, set `"stream": true` and the response will be returned as Server-Sent Events (SSE).

## Configuration

Muninn stores data in `.muninn/` within your project:

```
.muninn/
├── sessions/           # Per-session logs and traces
│   └── 2026-01-11T17-34-52_a3f2/
│       ├── muninn.log
│       ├── traces.jsonl
│       └── session.json
└── config.toml         # Optional configuration
```

## License

[Apache License 2.0](LICENSE)

---

<p align="center">
  <sub>Built by <a href="https://github.com/colliery-io">Colliery</a></sub>
</p>
