---
id: generate-compressed-docs-index-for
level: task
title: "Generate compressed docs index for context injection"
short_code: "PROJEC-T-0062"
created_at: 2026-01-31T04:48:07.760628+00:00
updated_at: 2026-01-31T04:48:07.760628+00:00
parent: 
blocked_by: []
archived: false

tags:
  - "#task"
  - "#phase/backlog"
  - "#feature"


exit_criteria_met: false
strategy_id: NULL
initiative_id: NULL
---

# Generate compressed docs index for context injection

## Objective

Add a command (`muninn docs gen-index`) that generates a compressed, single-line documentation index from already-indexed libraries and injects it into Muninn's forwarded context for the downstream LLM. This enables retrieval-led reasoning — the agent sees a pointer-based index of available docs and can then call `search_docs` to retrieve specific content on demand.

## Motivation

AI coding agents rely on pre-trained knowledge that may be outdated or misaligned with the specific version of a library in use. By injecting a docs index into the forwarded context, the downstream LLM:

- Knows exactly which libraries have local documentation available
- Can prefer retrieval (`search_docs`) over pre-training for version-specific APIs
- Gets directory-grouped file/item listings with minimal token overhead
- Avoids hallucinating APIs or suggesting deprecated patterns

Current pain points:
- Agents don't know which docs are indexed and available to search
- No signal to prefer retrieval over pre-trained knowledge
- Version mismatches between agent knowledge and project dependencies

## Relationship to PROJEC-I-0010

This builds on top of the existing dependency documentation indexing infrastructure (PROJEC-I-0010). That initiative handles fetching, extracting, and storing docs in the local index. This feature generates a compressed summary of what's indexed and injects it into the context Muninn forwards to the LLM.

## Backlog Item Details

### Type
- [x] Feature - New functionality or enhancement

### Priority
- [ ] P1 - High (important for user experience)

### Business Justification
- **User Value**: Downstream LLMs gain awareness of available local docs, leading to more accurate, version-matched responses
- **Business Value**: Key differentiator — Muninn becomes a retrieval-led reasoning gateway, not just a proxy
- **Effort Estimate**: M

## Proposed Design

### Index Generation Flow

```
┌─────────────────────────────────────────────────────────────────┐
│  1. ENUMERATE INDEXED LIBRARIES                                 │
│     └── Query doc_libraries table for all indexed libs          │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│  2. GROUP ITEMS BY LIBRARY + MODULE PATH                        │
│     ├── Query doc_chunks grouped by item_path prefix            │
│     └── Collapse into directory-style groupings                 │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│  3. COMPRESS INTO SINGLE-LINE FORMAT                            │
│     ├── Pipe-delimited segments                                 │
│     ├── Directory grouping eliminates path redundancy           │
│     └── Target: under 4KB for typical dependency sets           │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│  4. INJECT INTO FORWARDED CONTEXT                               │
│     ├── Include as system-level context in RLM prompt           │
│     ├── Instruct LLM to prefer search_docs over pre-training   │
│     └── Idempotent — regenerate on each session or on demand    │
└─────────────────────────────────────────────────────────────────┘
```

### Output Format

Single-line, pipe-delimited, minimal token overhead:

```
[Docs Index]|IMPORTANT: Prefer search_docs over pre-training for these libraries.|tokio@1.38.0(rust): {runtime,task,sync,net,io,time}|serde@1.0.203(rust): {ser,de,derive}|requests@2.32.3(python): {api,models,sessions,auth}|...
```

### Design Principles

| Principle | Rationale |
|-----------|-----------|
| Pointer-based | Index lists available modules; agent retrieves via search_docs |
| Directory grouping | Eliminates redundant path prefixes, compact representation |
| Single-line format | Minimal context window consumption |
| Injected, not file-based | Muninn controls the context directly — no static file needed |
| Version-tagged | Each library entry includes version for the LLM to reason about |

## Acceptance Criteria

- [ ] `muninn docs gen-index` generates a compressed index from all indexed libraries
- [ ] Index groups items by library and top-level module path
- [ ] Output is single-line, pipe-delimited, under 4KB for typical projects
- [ ] Index includes library name, version, and ecosystem for each entry
- [ ] Index can be injected into Muninn's forwarded context (system prompt or preamble)
- [ ] Index includes instruction nudging the LLM to prefer `search_docs` over pre-training
- [ ] Works with zero indexed libraries (produces empty/no-op index)

## Implementation Notes

### Technical Approach
- Query `doc_libraries` for all indexed libs (name, version, ecosystem)
- For each library, query `doc_chunks` and extract distinct top-level module paths from `item_path`
- Format into compressed single-line representation
- Expose as CLI command and as internal function callable by the RLM context builder

### Dependencies
- Requires PROJEC-I-0010 infrastructure (doc_libraries, doc_chunks tables populated)
- Requires Muninn's context forwarding mechanism to have an injection point

### Open Questions
- Should the index be auto-generated on every session, or only when explicitly requested?
- Should there be a token budget cap that truncates the index if it exceeds a threshold?
- Should the index include item counts per module to help the LLM gauge coverage depth?

## References

- Vercel's AGENTS.md approach: [next.js#88961](https://github.com/vercel/next.js/pull/88961)
- PROJEC-I-0010: Existing dependency documentation indexing initiative

## Status Updates

### 2026-05-21 — Deferred per v1 RLM-focus decision

Muninn v1 (see [[PROJEC-I-0011]] post-completion notes) positions as a recursive-language-model gateway — the agent-facing `search_docs` MCP tool was explicitly dropped from v1. This task is the natural next chapter once that scope reopens: a compressed docs-index injection is one of the "other context injection mechanisms" the v1 deferral note flags as future work. Stays in backlog until the v2 context-injection thesis is ready.