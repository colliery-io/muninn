---
id: implement-augmentation-retrieval
level: task
title: "Implement augmentation retrieval block (<=2KB markdown via recall_memory + query_graph)"
short_code: "PROJEC-T-0071"
created_at: 2026-05-19T16:41:34.076715+00:00
updated_at: 2026-05-19T16:41:34.076715+00:00
parent: PROJEC-I-0011
blocked_by: []
archived: false

tags:
  - "#task"
  - "#phase/todo"


exit_criteria_met: false
initiative_id: PROJEC-I-0011
---

# Implement augmentation retrieval block (<=2KB markdown via recall_memory + query_graph)

## Parent Initiative

[[hook-mcp-integration-layer-for-claude-code]] (PROJEC-I-0011)

## Objective

When the decision model returns `augment`, the hook attaches a "Muninn context" markdown block to the tool result the agent sees. This task implements the retrieval and formatting: pull related symbols / callers / prior memory via `query_graph` and `recall_memory` on the engine, format as a compact markdown block, hard-cap at 2KB. **No LLM call in this path** — retrieval only.

## Acceptance Criteria

- [ ] Given a tool call (Grep/Read/Glob) and an `augment_hint`, produces a markdown block in the exact format from [[hook-mcp-integration-layer-for-claude-code]] Detailed Design (Related symbols / Callers / Prior memory).
- [ ] All retrieval goes through the daemon IPC — no direct DB access from the hook process.
- [ ] Hard cap: output ≤ 2KB. If retrieval returns more, truncate with a clear marker (`… (truncated)`).
- [ ] Empty results don't produce an empty block — the augment branch falls back to passthrough if there's nothing useful to attach.
- [ ] Unit tests: full block, partial block (some sections empty), oversized truncation, empty fallback.
- [ ] Integration test: real `Grep` augmented end-to-end against a fixture repo.

## Dependencies

- PROJEC-T-0065 (engine implements trait — needed for query_graph and recall_memory)
- PROJEC-T-0066 (daemon IPC)
- PROJEC-T-0070 (hook decide invokes this on `augment`)

## Implementation Notes

- 2KB is generous; if real usage shows agents ignoring or being confused by big blocks, drop the cap.
- Markdown because CC renders it; align with how CC displays tool results today.
- Consider dedup: if the augmentation just repeats what's already in the Grep result, prefer empty fallback.

## Status Updates

*To be added during implementation.*
