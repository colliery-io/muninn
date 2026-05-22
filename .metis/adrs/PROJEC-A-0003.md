---
id: 003-hook-mcp-integration-model-as
level: adr
title: "Hook + MCP integration model as primary Claude Code surface; proxy intercept retained as first-class"
number: 3
short_code: "PROJEC-A-0003"
created_at: 2026-05-19T15:22:55.968809+00:00
updated_at: 2026-05-19T16:15:50.576862+00:00
decision_date: 
decision_maker: dylan.storey@gmail.com
parent: 
archived: false

tags:
  - "#adr"
  - "#phase/decided"


exit_criteria_met: false
initiative_id: NULL
---

# ADR-3: Hook + MCP integration model as primary Claude Code surface; proxy intercept retained as first-class

## Context

Muninn's current Claude Code integration is a **transparent HTTP proxy** (`crates/muninn-rlm/src/proxy.rs` + `passthrough.rs` + `router.rs`) that masquerades as the Anthropic Messages API. Claude Code is pointed at muninn instead of `api.anthropic.com`; muninn either rewrites OAuth bearer tokens and forwards the request (passthrough) or hijacks it for recursive exploration via a different backend (RLM mode).

This realizes the vision principle "Invisible by default" — but it does so by being an *unsanctioned man-in-the-middle*:

1. **Fragility.** The integration relies on the shape of Anthropic's Messages API (streaming SSE schema, tool-use block structure, `cache_control` semantics, header set), the Claude Code → API contract (OAuth bearer format, model id strings), and the assumption that these contracts won't shift. Each upstream change is a potential breakage we have to chase.
2. **ToS posture.** Even where the intent is benign — providing better context to the user's own agent on the user's own machine — proxying an authenticated session through software that rewrites requests/responses sits in grey territory with respect to Anthropic's ToS. We do not want muninn's value proposition to be coupled to that risk.
3. **Architectural lock-in.** Tying the design to "look like Anthropic" limits us. Hooks and MCP are Claude Code's *first-class* extension surfaces — designed exactly for the kind of context augmentation muninn does — and we're not using them.

At the same time, the proxy genuinely serves users who are *not* on Claude Code: anyone driving muninn from a raw OpenAI- or Anthropic-compatible client (custom agents, scripts, other IDE plugins) benefits from a backend-side intercept. Ripping it out would close off that audience.

## Decision

Adopt a **two-surface architecture**:

1. **Primary surface (Claude Code): hooks + MCP.**
   - Expose muninn's capabilities (`search_code`, `explore`, `recall_memory`, `search_docs`, graph queries) as an **MCP server**. Claude Code calls them explicitly when it wants context — sanctioned, durable, no protocol mimicry.
   - Install a **PreToolUse hook plugin** that intercepts `Grep` / `Read` / `Glob` (and possibly `Bash` searches). The hook runs a small **decision model** that decides per-call whether to (a) let the call through unchanged, (b) augment the result with muninn-derived context (related symbols, prior memory, callers/callees), or (c) rewrite the call to muninn's smarter equivalent and short-circuit.
   - Optional **UserPromptSubmit hook** for coarse, per-turn memory injection when the prompt clearly references project state ("the auth module", "that ADR about caching", etc.).

2. **Secondary surface (everything else): the existing proxy intercept, kept first-class.**
   - Continues to support raw OpenAI/Anthropic-compatible clients that don't speak hooks or MCP.
   - Stays maintained, tested, and released alongside the hook+MCP surface — not deprecated.
   - The router/RLM engine becomes the shared core: both surfaces are thin adapters over the same recursive-exploration engine, tool environment, and memory store.

## Alternatives Analysis

| Option | Pros | Cons | Risk Level | Implementation Cost |
|--------|------|------|------------|-------------------|
| Status quo: proxy-only | Already works; one code path; truly invisible to the agent | Fragile to upstream API changes; ToS-grey; ignores CC's native extension model | High | Zero new cost, ongoing maintenance debt |
| MCP-only | Idiomatic, sanctioned, lowest fragility | Loses implicit augmentation (only fires when the agent explicitly calls muninn tools); agents that don't speak MCP can't benefit | Low | Medium |
| Hooks-only | Implicit augmentation on every Grep/Read/Glob without agent cooperation | Hooks are CC-specific; non-CC agents get nothing; no path for explicit "ask muninn" calls | Medium | Medium |
| **Hooks + MCP (primary) + proxy (secondary)** — chosen | Sanctioned primary path; implicit *and* explicit augmentation; preserves non-CC users; isolates ToS/fragility risk to an opt-in secondary surface | Two surfaces to maintain; need a clean shared engine boundary; decision-model latency budget must be tight | Medium | Medium-High |
| Rip out the proxy entirely | Smallest surface area; cleanest ToS posture | Cuts off non-CC users overnight; throws away working code | Low for code, High for users | Low (negative — deletion) |

## Rationale

The hook+MCP path matches the *intent* of "Invisible by default" — the agent doesn't have to know about muninn to benefit — while using the surfaces Anthropic explicitly provides for that intent. Hooks deliver the implicit augmentation the proxy was giving us (silently making `Grep` smarter); MCP delivers the explicit one (the agent asks for code search and gets muninn's recursive exploration). Together they recover essentially all the value of the proxy intercept without the fragility or ToS posture.

We keep the proxy because:
- It still serves non-Claude-Code users (the vision is "any agent / any OpenAI-compatible client", not "Claude Code only").
- The engine underneath (recursive exploration, memory, tools) is the actual value; the proxy is just one adapter on top. Removing the adapter would discard a working integration path with real users.
- Maintaining both forces a clean engine/adapter boundary, which is healthy architecture regardless.

The decision-model component (tiny, fast model deciding per tool-call whether to augment) is the load-bearing new piece. The router we already have ([[llm-based-router]] / PROJEC-T-0042) is the closest precedent — same shape, different decision surface.

## Consequences

### Positive
- Primary Claude Code integration moves onto sanctioned extension surfaces; upstream changes to the Anthropic API no longer break the default user.
- Cleaner separation between **engine** (recursive exploration, memory, tools — the value) and **adapters** (proxy, MCP server, hook plugin — the surfaces).
- Opens the door to other agent ecosystems (Cursor, Continue, etc.) once MCP/hook adapters are pluggable.
- ToS risk is now confined to the opt-in proxy path for users who explicitly choose it.

### Negative
- Two integration surfaces to maintain, test, and document instead of one.
- The PreToolUse hook adds latency to every Grep/Read/Glob — the decision model must be small and fast (likely the same tier as the current router model). Budget: well under 100ms p50 or users will turn it off.
- Requires building and shipping a Claude Code plugin (new artifact type for muninn), with its own update/distribution story.
- Existing users who configured muninn-as-proxy will need migration guidance; default install advice changes.

### Neutral
- The recursive-exploration engine itself doesn't change. This is an integration-layer pivot, not a core-algorithm pivot.
- Backend abstraction (Groq/Anthropic/Ollama/local) is unaffected and shared across both surfaces.

## Review Schedule

### Review Triggers
- Anthropic ships a sanctioned proxy/gateway extension point that obsoletes the intercept.
- Hook latency budget proves unachievable in practice (decision-model p50 > 200ms on representative hardware) — re-evaluate whether implicit augmentation is worth the cost.
- Non-Claude-Code adoption of muninn falls to near-zero — proxy surface may become deletable.

### Scheduled Review
- **Next Review Date**: 2026-11-19 (6 months) or on first hook+MCP release, whichever comes first.
- **Review Criteria**: Are both surfaces in active use? Is the engine boundary actually shared, or have they diverged?