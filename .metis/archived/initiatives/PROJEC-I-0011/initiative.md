---
id: hook-mcp-integration-layer-for
level: initiative
title: "Hook + MCP Integration Layer for Claude Code"
short_code: "PROJEC-I-0011"
created_at: 2026-05-19T15:22:56.090238+00:00
updated_at: 2026-05-20T20:35:35.888935+00:00
parent: PROJEC-V-0001
blocked_by: []
archived: true

tags:
  - "#initiative"
  - "#phase/completed"


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

## Post-Completion Status Updates

### 2026-05-21 — v1 RLM-focus finalization

The initiative shipped as designed, but two pieces of scope were **refined / dropped** after Claude Code's behavior surfaced friction during dogfooding. Capturing the decisions here so the initiative reflects what actually shipped in v1.

#### What changed vs the original design

1. **PreToolUse hook → UserPromptSubmit hook.** The original design fired the decision model per-`Grep`/`Read`/`Glob` and returned `passthrough` | `augment` | `rewrite`. In practice, even after a `rewrite` injection, Claude would still re-grep "to verify" — the off-load didn't actually off-load. Replaced with a **once-per-turn UserPromptSubmit hook** that runs the existing router as a cheap gate, drives the recursive engine on the configured local/cheap backend when the router picks RLM, and injects the result as `additionalContext` framed as an answer ("muninn has already explored — do not re-grep"). This matches REQ-002's intent (off-load discovery) at a more useful granularity. PROJEC-T-0070 (the `muninn hook decide` subcommand) is effectively superseded; the surviving hook command is `muninn hook submit`.

2. **Memory dropped from v1.** `recall_memory` / `record_memory` had no real write source in v1 — they would always return empty for users. REQ-001's memory tools were removed from the trait, the daemon dispatch, the MCP surface, and the test stubs. `muninn-rlm::memory_tools` deleted. Memory comes back when there's a concrete write story (likely a separate initiative).

3. **`search_docs` gated out of v1's MCP surface.** Infrastructure landed via [[PROJEC-I-0010]] (DocStore + indexers + internal RLM tool), but the agent-facing MCP tool is deferred — muninn v1 is positioned as an RLM, with dependency-doc retrieval as a "next chapter" context-injection surface. See [[PROJEC-T-0062]].

#### What shipped (matches REQ/NFR)

- **REQ-001 (MCP server with curated surface):** `muninn mcp` exposes `search_code` + `query_graph` over stdio. Both wired against real stores. `explore`, `recall_memory`, `search_docs` deliberately not advertised — see decisions above and `crates/muninn-core/src/mcp.rs` design notes.
- **REQ-002 (sanctioned hook):** UserPromptSubmit hook in `plugins/muninn-cc/`. Router-gated RLM with answer-shaped inject.
- **REQ-003 (budgeted decision model, silent passthrough on failure):** Router runs against `[router]` provider/model. Outer cap default raised to 240s (the RLM exploration on a local/cheap model regularly takes 20–60s for code-shaped prompts — 30s was producing silent quota-style passthroughs in dogfooding). `MUNINN_HOOK_DEADLINE_MS` env var lets UAT shrink the cap; the matching CC `hooks.json` timeout is 245s. Router LLM failures now log at `error!` level so they're distinguishable from real passthrough decisions.
- **REQ-004 (engine boundary):** `MuninnEngine` trait in `muninn-core`. Proxy, hook, and MCP all consume `Arc<dyn MuninnEngine>` via `DaemonClient`.
- **REQ-005 (install/config CLI):** `muninn install-cc` / `muninn uninstall-cc` ship.
- **NFR-001:** N/A in current form — the new UserPromptSubmit shape is not per-tool-call, so the 100ms p50 budget doesn't apply. The new budget is "user is already waiting for Claude's first token, 240s of pre-exploration is acceptable when it replaces Claude's own exploration."
- **NFR-002:** verified by 5 UAT tests in `crates/muninn/tests/user_prompt_submit.rs` covering daemon-unreachable, explicit `@muninn passthrough`, happy path, provider error mid-RLM, and outer-cap timeout backstop.
- **NFR-003:** unchanged.

#### Bugs found and fixed during finalization

- **`ensure_daemon` did not propagate `--config`** — the parent process's `--config` flag was swallowed when `ensure_daemon` spawned the child `muninn daemon start`, so users running with `--config` got a daemon reading config from CWD. Fixed by adding `ensure_daemon_with_args` and threading `--config` through.
- **Ollama and Groq backends ignored per-request `model`** — backend always used `OllamaConfig.model` / `GroqConfig.model`, defeating tier-config (router/RLM) model overrides. Anthropic backend already serialized the whole request and was fine. Fixed by adding `pick_model(request.model, config.model)`. Added regression tests in `groq.rs`.
- **`[ollama] max_retries` not exposed** — hardcoded 3 × 500ms backoff was burning the hook budget on flapping backends. Added to `OllamaProviderConfig` with a documented "set to 0 to fail fast" note.

#### New env vars / config surfaces

- `MUNINN_HOOK_TEST_SOCKET` — overrides hook's daemon-socket discovery (UAT only).
- `MUNINN_HOOK_DEADLINE_MS` — overrides the 240s outer cap (UAT only).
- `[ollama] max_retries` — user-facing config knob.

#### Test coverage

- `crates/muninn/tests/user_prompt_submit.rs` — 5 UAT covering the failure-mode contract.
- `crates/muninn/tests/mcp_protocol.rs` — `mcp_tools_call_search_code_returns_filesystem_hits` and `mcp_tools_call_query_graph_returns_graph_payload` assert real engine results (no more "not yet wired" sentinel tests).
- `angreal ci` green; full UAT green when Ollama Cloud quota permits.

#### What's intentionally still open

- `MuninnEngine::explore` (lightweight DTO) still returns "not yet wired" — callers use `complete` with a recursive `MuninnConfig` instead. Not blocking v1.
- `query_graph` for `kind=references` is the only graph kind that returns an error rather than results (no store-level support). Callers/callees/defines all work.
- [[PROJEC-T-0045]] (router trace truncation, P1 bug) is unrelated to this initiative and still open.