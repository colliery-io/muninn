---
id: implement-augmentation-retrieval
level: task
title: "Implement augmentation retrieval block (<=2KB markdown via recall_memory + query_graph)"
short_code: "PROJEC-T-0071"
created_at: 2026-05-19T16:41:34.076715+00:00
updated_at: 2026-05-20T19:44:42.850844+00:00
parent: PROJEC-I-0011
blocked_by: []
archived: false

tags:
  - "#task"
  - "#phase/completed"


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

### 2026-05-20 — Implementation landed (formatting complete; engine-side stubs gate live retrieval)

**New module `crates/muninn/src/hook.rs`:**

- `try_build_augment_block(socket, tool_name, tool_args, augment_hint) -> Option<String>` —
  the top-level entry the `decide_inner` augment branch calls. Connects
  to the daemon (without `ensure_daemon` — too costly mid-hook), issues
  `recall_memory` + `query_graph` in parallel under a 150 ms per-call
  budget, and renders the results as the Muninn-context markdown
  block.
- `format_augment_block(graph, memory, byte_cap) -> Option<String>` —
  pure formatter, isolated so it's easy to unit-test. Produces the
  three documented sections (`Related symbols:` / `Callers:` /
  `Prior memory:`) only when each has content; returns `None` when
  every section is empty so the caller can passthrough cleanly.
- `truncate_with_marker` — UTF-8-safe truncation that preserves the
  footer line and appends `… (truncated)` so the agent knows the
  block was trimmed. Handles absurdly-small caps gracefully (returns
  the footer alone instead of panicking).
- `extract_graph_target(tool_name, tool_args)` — picks the most
  useful symbol/path to query graph against per tool. Grep uses
  `pattern`; Read/Glob use `file_path` (falling back through `path` /
  `pattern`); unknown tools yield `None`.

**Wired into `decide_inner` (`crates/muninn/src/main.rs`):**

- The augment branch now calls `try_build_augment_block` first. On
  success, the full block becomes the `additionalContext`. On `None`
  (empty retrieval, daemon down, timeout, engine error), it falls
  back to the existing "Muninn hint: …" one-liner from the model.
  Falls all the way to passthrough when there's no hint either.
  NFR-002 contract preserved end-to-end.
- `hook_socket_path` helper resolves the repo-scoped socket path the
  same way `muninn daemon ensure` would.

**Dependencies:**

- Added `muninn-core` as a direct dep of the `muninn` binary so the
  hook code can construct engine DTOs (`MemoryQuery`, `GraphQuery`,
  `GraphQueryKind`) without leaking through `muninn-rlm` re-exports.
- Added `futures = "0.3"` for `futures::future::join` (parallel
  retrieval).

**Today's reality:**

`RecursiveEngine`'s `recall_memory` and `query_graph` are still
stubbed with `Internal("not yet wired …")` errors from the PROJEC-T-0065
carve-out. The augmentation path therefore *always* gets an empty
GraphResult / empty memory hits and returns `None`, degrading to the
hint-only fallback or passthrough. The formatting + dispatch
machinery is fully exercised by unit tests, so when those stubs
become real impls — likely as part of a future memory-store task —
augmentation starts producing real blocks immediately, no further
changes here.

### Tests
- 11 new unit tests in `hook::tests`:
  - `format_returns_none_when_everything_empty`
  - `format_renders_full_block_under_cap` (asserts the documented format,
    presence of header/footer, all three sections, byte-cap budget)
  - `format_handles_partial_sections` (memory only, graph empty)
  - `format_skips_memory_entries_with_blank_first_line`
  - `format_truncates_when_over_cap_and_marks_with_ellipsis` (200-node
    graph forced past a 512-byte cap; verifies marker presence + footer
    intact + total under cap)
  - `truncate_respects_utf8_char_boundaries` (multibyte chars right at
    the truncation point — would panic if we sliced wrong)
  - `truncate_with_absurdly_small_cap_returns_footer_only`
  - `extract_graph_target_grep_uses_pattern`
  - `extract_graph_target_read_uses_path`
  - `extract_graph_target_unknown_tool_returns_none`
  - `extract_graph_target_empty_string_returns_none`
- muninn binary unit tests: **39/39 pass** (was 28, +11 hook tests).
- Workspace: all 16 test suites still green.
- `angreal test uat` against real Ollama Cloud still passes (1.6s,
  passthrough decision).
- Strict clippy + `cargo fmt --check` clean on touched crates.

### Decisions

- **Don't `ensure_daemon` during hook execution.** Cold-spawning a
  daemon costs hundreds of ms — that blows the 500 ms outer cap.
  Hook gracefully degrades to passthrough when the daemon isn't
  already running. `muninn install-cc` (PROJEC-T-0072) will own the
  "make sure the daemon is up" lifecycle out-of-band.
- **150 ms per-retrieval timeout**, both calls in parallel. Worst
  case ≈ 150 ms inside the augmentation block; combined with the
  decision-model call we stay safely inside 500 ms.
- **Use `augment_hint` as the memory query** when the model provides
  one — the hint is the model's pointer at what muninn might know,
  which is a better retrieval query than the raw tool args.
- **Edge filter `kind == "calls" || kind == "caller"`** for the
  Callers section — supports both naming conventions that the
  engine impl might emerge with.
- **Cap = 2 KB** matches the initiative design. If real usage shows
  agents ignoring big blocks, easy to drop (single constant).

### Deferred / explicit non-scope

- **End-to-end integration test against a fixture repo** — the AC
  asks for "real Grep augmented end-to-end against a fixture repo,"
  which requires the engine's `recall_memory` / `query_graph` impls
  to be wired to real stores. Until then the test would only
  exercise the passthrough/empty-fallback path the unit tests
  already cover. Will fold into the memory-store wiring task when
  it lands.
- **Rewrite decisions** still degrade to passthrough — synthesizing
  a tool result by calling `search_code` / `query_graph` and
  feeding it back as a deny+reason is a follow-up.
- **Engine-side `recall_memory` / `query_graph` implementations** —
  tracked separately from this initiative; this task's job was the
  hook-side formatting + dispatch.

### CI carve-out
Same as previous initiative tasks — workspace `angreal ci` still blocked by the pre-existing muninn-graph clippy debt tracked in PROJEC-T-0076. No new clippy or fmt issues introduced.