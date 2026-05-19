---
id: hook-mcp-integration-layer-for
level: initiative
title: "Hook + MCP Integration Layer for Claude Code"
short_code: "PROJEC-I-0011"
created_at: 2026-05-19T15:22:56.090238+00:00
updated_at: 2026-05-19T17:50:28.701397+00:00
parent: PROJEC-V-0001
blocked_by: []
archived: false

tags:
  - "#initiative"
  - "#phase/active"


exit_criteria_met: false
estimated_complexity: L
initiative_id: hook-mcp-integration-layer-for
---

# Hook + MCP Integration Layer for Claude Code

## Context

Today, muninn's Claude Code integration is the HTTP proxy in `crates/muninn-rlm/src/proxy.rs` that pretends to be the Anthropic Messages API. This realizes "Invisible by default" but is fragile (depends on Anthropic's unsanctioned API contract staying stable) and ToS-grey (rewrites authenticated sessions in transit). See [[003-hook-mcp-integration-model-as]] (PROJEC-A-0003) for the full rationale.

This initiative builds the **sanctioned** integration path: an MCP server plus a Claude Code hook plugin, sharing the existing recursive-exploration engine. The proxy stays first-class for non-Claude-Code clients.

## Goals & Non-Goals

**Goals:**
- Ship an MCP server exposing muninn's core capabilities (`search_code`, `explore`, `recall_memory`, `search_docs`, graph queries) so Claude Code can invoke them explicitly.
- Ship a Claude Code plugin with a `PreToolUse` hook on `Grep` / `Read` / `Glob` that runs a small decision model and either augments, rewrites, or passes the call through. This is the "implicit augmentation" the proxy used to provide.
- Refactor the engine boundary so proxy, MCP server, and hook plugin are three thin adapters over a shared `RecursiveEngine` + `ToolEnvironment` + memory store.
- Preserve the existing proxy as a fully-supported secondary surface for non-CC clients.

**Non-Goals:**
- Rewriting the recursive-exploration algorithm. The engine stays as-is; only its callers change.
- Building plugin/MCP support for other agent ecosystems (Cursor, Continue). The architecture should leave that door open but this initiative ships CC only.
- Deprecating the proxy. Per ADR-0003 it remains first-class.
- Replacing the existing `crates/muninn-rlm/src/mcp.rs` scaffolding if it already does the job — assess first, evolve if it does, rebuild only if necessary.

## Requirements

### Functional
- REQ-001: MCP server exposes at least: `search_code`, `explore` (recursive), `recall_memory`, `search_docs`, `code_graph_query`. Tool schemas documented and stable.
- REQ-002: PreToolUse hook fires on `Grep`, `Read`, `Glob`; the decision model returns one of `{passthrough, augment, rewrite}` plus an optional context payload.
- REQ-003: Decision model is configurable (provider/model in `[router]`-style config) and budget-bounded (timeout, max tokens). Default p50 < 100ms, hard timeout 500ms — on timeout, fall through to passthrough so the user is never blocked.
- REQ-004: The shared engine has a documented public boundary; proxy, MCP, hook plugin all consume it without reaching into each other's internals.
- REQ-005: Plugin distribution: installable via Claude Code's plugin mechanism. `muninn` CLI gains a command to install/uninstall/configure the plugin against a target CC config.

### Non-Functional
- NFR-001: Hook overhead at the 50th percentile must not exceed 100ms on commodity hardware (M-series Mac, mid-range Linux desktop) using the default decision model.
- NFR-002: Hook failure modes (decision model down, MCP server unreachable) must degrade to silent passthrough — never block the user's tool call.
- NFR-003: Privacy invariant unchanged: no code or prompts leave the user's machine unless they configured a remote backend.

## Architecture

### Overview

```
┌─────────────────┐   ┌──────────────────┐   ┌─────────────────────────┐
│ Claude Code     │   │ Other agents /   │   │ Direct CLI / scripts    │
│ (hooks + MCP)   │   │ OpenAI-compat    │   │                         │
└────────┬────────┘   └────────┬─────────┘   └──────────┬──────────────┘
         │                     │                        │
         ▼                     ▼                        ▼
   ┌──────────┐          ┌───────────┐            ┌──────────┐
   │ Hook     │          │ Proxy     │            │ CLI cmd  │
   │ plugin   │          │ intercept │            │ adapters │
   └────┬─────┘          └─────┬─────┘            └────┬─────┘
        │                      │                       │
        │   ┌──────────────────┼───────────────────┐   │
        │   │                  │                   │   │
        ▼   ▼                  ▼                   ▼   ▼
              ┌──────────────────────────────┐
              │ Shared RecursiveEngine /     │
              │ ToolEnvironment / Memory     │
              │ (today: muninn-rlm engine)   │
              └──────────────────────────────┘
                            │
              ┌──────────────────────────────┐
              │ Backends (Groq, Anthropic,   │
              │ Ollama local, Ollama Cloud)  │
              └──────────────────────────────┘

   MCP server is its own process/transport, also calling the
   shared engine — drawn separately:

   Claude Code  ──MCP──►  muninn-mcp  ──►  shared engine
```

### Components

1. **Engine boundary refactor.** Extract a `MuninnEngine` trait (or equivalent) from the current `RecursiveEngine` so adapters depend on the interface, not the concrete type. Today proxy reaches into engine internals directly; tighten that.
2. **MCP server (`muninn mcp` or similar).** Evolves `crates/muninn-rlm/src/mcp.rs`. Exposes the tool surface listed in REQ-001. Runs as a stdio MCP server invoked by Claude Code's MCP config.
3. **Hook plugin (`plugins/muninn-cc/`).** A Claude Code plugin with a `PreToolUse` hook (and optional `UserPromptSubmit`). Hook handler calls a tiny decision model, then either returns the original tool call, an augmented result, or a rewritten call targeting an MCP tool.
4. **Decision model.** A small/fast LLM (default: Ollama-served small instruct model) prompted with the tool call + lightweight project context, returning a structured `{decision, payload}` JSON. Same architectural pattern as the existing `router.rs` — likely shares plumbing.
5. **Install/config CLI.** `muninn install-cc` / `muninn install-mcp` (final names TBD) that wires the plugin and MCP server into a target Claude Code config and writes the requisite `.claude/` entries.

## Detailed Design

### Engine boundary

Define a `MuninnEngine` **trait** in a new `muninn-core` crate that has no dependency on any adapter (proxy / MCP / hook). Adapters depend on the trait; the concrete `RecursiveEngine` impl lives where it does today (or migrates to `muninn-core` if cleanest). Minimum public surface — keep small, grow only on demand:

```rust
#[async_trait]
trait MuninnEngine: Send + Sync {
    async fn search_code(&self, q: SearchQuery) -> Result<SearchResult>;
    async fn explore(&self, q: ExploreRequest) -> Result<ExploreResult>;
    async fn recall_memory(&self, q: MemoryQuery) -> Result<Vec<MemoryHit>>;
    async fn record_memory(&self, item: MemoryItem) -> Result<()>;
    async fn search_docs(&self, q: DocsQuery) -> Result<DocsResult>;
    async fn query_graph(&self, q: GraphQuery) -> Result<GraphResult>;
}
```

All adapter code consumes `Arc<dyn MuninnEngine>`. The existing `proxy.rs` is refactored to route through this trait instead of reaching into engine internals.

### Decision model — prompt + output schema

Input to the decision model (kept tiny to hit NFR-001 budget):
- `tool_name` (one of `Grep`, `Read`, `Glob`).
- `tool_args` (the literal arguments — the regex/path/pattern).
- `recent_tool_history` (last 3–5 tool calls in this CC turn, names + truncated args only).
- `top_k_memory_hints` (k=3, surfaced via `recall_memory` keyed on `tool_args` + recent prompt).

Hard budget: 256 input tokens / 64 output tokens. Use the provider's JSON mode where available.

Output JSON schema:
```json
{
  "decision": "passthrough" | "augment" | "rewrite",
  "rationale": "string, <=120 chars, optional",
  "rewrite": {                       // only when decision == "rewrite"
    "tool": "search_code" | "explore",
    "args": { ... }                  // matches MCP tool schema
  },
  "augment_hint": "string, <=200 chars, optional"  // only when decision == "augment"; steers what to attach
}
```

On parse failure, timeout, or any error: treat as `passthrough` (NFR-002).

### Augmentation payload format

When the hook decides `augment`, the original tool call runs unchanged and produces its normal result. The hook then appends a **Muninn context block** to the result the agent sees:

```
─── Muninn context ───
Related symbols: foo::bar (crates/x/src/y.rs:42), …
Callers: alpha (a.rs:10), beta (b.rs:88)
Prior memory:
  - 2026-04-02 ADR-0001 declares this module owns auth
─────────────────────
```

Implementation: Claude Code's PreToolUse hook (or paired PostToolUse) returns an `additionalContext` payload that CC concatenates to the tool result. Hard cap: 2KB per augmentation to avoid context bloat. Content is generated by calling `recall_memory` + `query_graph` against the engine — no separate LLM call for augmentation itself, only retrieval.

When the hook decides `rewrite`, it short-circuits the original tool, calls the named engine method directly, and returns its result as if the tool had produced it. No "Muninn context" header — the result *is* the muninn result.

### Plugin packaging

Single repo. New top-level directory `plugins/muninn-cc/` in the muninn workspace containing the CC plugin manifest + hook scripts. The plugin is *thin* — it shells out to the `muninn` binary for all real work (decision model call, augmentation retrieval). Versioned together with the muninn binary; release artifacts ship both. No separate publication pipeline.

### MCP transport

**Stdio.** It's CC's default and the simplest to wire. Local HTTP only if we hit a concrete stdio limitation (none anticipated). The MCP server is the `muninn` binary invoked as `muninn mcp` (subcommand to add).

### Memory access from MCP vs. hook

Both surfaces talk to a **single `muninn` daemon process** that owns the engine and memory store. The MCP server (stdio process spawned by CC) and the hook scripts (also short-lived processes) both connect to this daemon via a local IPC (Unix socket; named pipe on Windows). This:

- Avoids double-locking SQLite (one writer).
- Lets the embedding model / graph index stay warm across calls.
- Gives a single observability surface for traces.

If no daemon is running when an adapter is invoked, the adapter auto-starts one in the background (idempotent — `muninn daemon ensure` or similar).

### Decision-model provider

Inherits from the tiered config landing in [[add-ollama-cloud-provider-and-make]] (PROJEC-T-0063). By default, the decision model uses `[default]` — i.e. on free-tier Ollama Cloud it's `gemma4:31b` doing triple duty (router, RLM, hook decision). Users hitting NFR-001 issues override `[hook_decision]` (new section, same inheritance shape) with a smaller/faster model.

## Alternatives Considered

Covered in [[003-hook-mcp-integration-model-as]] (PROJEC-A-0003). Summary:
- Status quo proxy-only — rejected (fragile, ToS-grey).
- MCP-only — rejected (loses implicit augmentation).
- Hooks-only — rejected (no explicit-recall surface, CC-only).
- Rip out proxy — rejected (cuts off non-CC users).

## Implementation Plan

Phased; decomposition into tasks deferred until exit of discovery → design (Metis HITL gate).

### Phase 1 — Design (current)
- Resolve open questions in Detailed Design above.
- Write the engine-boundary spec.
- Prototype the decision model prompt and measure latency on representative tool calls.

### Phase 2 — Engine boundary + MCP server
- Refactor engine into adapter-friendly shape.
- Build the MCP server surface (REQ-001).
- Both proxy and MCP go through the new boundary; no behavior change for proxy users.

### Phase 3 — Hook plugin
- Build the CC plugin with `PreToolUse` hook (REQ-002, REQ-003).
- Ship a default decision model config that hits NFR-001.
- Install/uninstall CLI commands (REQ-005).

### Phase 4 — Hardening + docs
- Failure-mode tests (NFR-002).
- Migration guide: "muninn-as-proxy" users → "muninn-as-plugin" users.
- README/docs reorganized around two surfaces.

### Dependencies
- [[add-ollama-cloud-provider-and-make]] (PROJEC-T-0063) lands first or in parallel — the decision model will likely run on Ollama by default, and we want the cloud option available.
- Existing `mcp.rs` scaffolding (assess in Phase 1).

## Exit Criteria

- All REQ/NFR satisfied with tests.
- Both surfaces (hook+MCP and proxy) covered by integration tests in `tests/`.
- A real Claude Code session using the plugin demonstrates implicit augmentation on `Grep`/`Read` and explicit calls to `search_code` via MCP.
- Documentation explains both surfaces and when to pick which.