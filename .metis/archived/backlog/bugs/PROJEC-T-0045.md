---
id: router-trace-data-truncates
level: task
title: "Router trace data truncates/corrupts user message content"
short_code: "PROJEC-T-0045"
created_at: 2026-01-10T19:48:59.758834+00:00
updated_at: 2026-05-21T15:12:04.517756+00:00
parent: 
blocked_by: []
archived: true

tags:
  - "#task"
  - "#bug"
  - "#phase/completed"


exit_criteria_met: false
initiative_id: NULL
---

# Router trace data truncates/corrupts user message content

*This template includes sections for various types of tasks. Delete sections that don't apply to your specific use case.*

## Parent Initiative **[CONDITIONAL: Assigned Task]**

[[Parent Initiative]]

## Objective **[REQUIRED]**

Fix router trace data to capture full user message content instead of truncated/corrupted text.

## Backlog Item Details **[CONDITIONAL: Backlog Item]**

### Type
- [x] Bug - Production issue that needs fixing

### Priority
- [ ] P0 - Critical (blocks users/revenue)
- [x] P1 - High (important for user experience)
- [ ] P2 - Medium (nice to have)
- [ ] P3 - Low (when time permits)

### Impact Assessment **[CONDITIONAL: Bug]**
- **Affected Users**: All users - trace data is corrupted for every request
- **Reproduction Steps**: 
  1. Start muninn proxy with LLM router: `muninn claude -c`
  2. Send any request through the proxy
  3. Check trace file: `tail -1 .muninn/traces/2026-01-10.jsonl | jq '.spans[0].children[0].data.last_user_message'`
- **Expected vs Actual**: 
  - Expected: Full message text like "restarted, lets see what happens"
  - Actual: Truncated text like "count" or "quota"

## Acceptance Criteria

## Acceptance Criteria

## Acceptance Criteria

## Acceptance Criteria

## Acceptance Criteria **[REQUIRED]**

- [ ] Router trace data captures full user message content
- [ ] Verified with messages of various lengths (10 chars, 100 chars, 1000 chars)
- [ ] Integration test added to verify trace data accuracy
- [ ] No truncation or corruption of message content in JSON serialization

## Implementation Notes **[CONDITIONAL: Technical Task]**

### Evidence (as filed)
From `/Users/dstorey/Desktop/colliery/muninn/.muninn/traces/2026-01-10.jsonl`:
```json
{
  "last_user_message": "count",  // Should be full message
  "decision": "passthrough"
}
```

Multiple traces show 4-6 character strings ("count", "quota") instead of actual messages.

### Location
`crates/muninn-rlm/src/router.rs`:
- `RouterTraceData` struct (line ~51)
- `extract_routing_input()` (formerly the report's `get_last_user_message()`)
- Trace emission via `Router::finish()`

## Status Updates

### 2026-05-21 — Not a bug; observability fix shipped instead

**Investigation outcome:** the trace data is correct. The short values reported (`"count"`, `"ping"`, `"quota"`) are not truncations — they are exactly what Claude Code sent. Confirmation came from cross-referencing the trace entries against the contemporaneous raw-request log (`.muninn/debug/raw_requests.jsonl`):

```json
// Raw request matching the "count" trace
{
  "model": "claude-haiku-4-5-20251001",
  "request": {
    "max_tokens": 1,
    "messages": [{"content": "count", "role": "user"}]
  }
}
```

These are Claude Code's **connectivity-probe / token-count requests**: a one-word `content` (`"count"` / `"ping"`) against the haiku model with `max_tokens: 1`. CC sends them constantly alongside real turns. The router classified them as passthrough and recorded the (genuinely-short) user content. No code in muninn was truncating anything.

Verified by:
- `grep -F '"text":"count"' raw_requests.jsonl` → zero matches (no raw request had `"count"` as a block-form text).
- `grep -F '"content":"count"' raw_requests.jsonl` → matches in the string-form content, all paired with `max_tokens:1` and `claude-haiku-4-5-20251001`.
- The `extract_routing_input` / `strip_control_tags` pipeline preserves long user messages intact; backed by three new regression tests in `router.rs::tests`.

### What shipped instead

Observability + regression scaffolding so this misdiagnosis doesn't recur:

1. **`RouterTraceData.max_tokens`** — new field surfacing the request's `max_tokens` so probe traffic (`max_tokens: 1`) is distinguishable from real turns at a glance.
2. **Doc-comment on `last_user_message`** stating explicitly that the field is *not* truncated and short values reflect what the caller actually sent.
3. **Three regression tests** in `crates/muninn-rlm/src/router.rs::tests`:
   - `extract_routing_input_preserves_long_user_messages` (4kB+ string survives).
   - `extract_routing_input_preserves_blocks_form` (multi-block `Content::Blocks` reassembled correctly).
   - `extract_routing_input_strips_only_control_tags` (real content alongside `<system-reminder>` survives; only the tag is removed).

### Acceptance criteria — reassessed

- [x] Router trace data captures full user message content *(was already correct; verified)*.
- [x] Verified with messages of various lengths (4 kB / multi-block / mixed-with-control-tags) via the three new tests.
- [x] Regression tests added (unit-level in `router.rs::tests` — the contract lives at the extraction boundary).
- [x] No truncation or corruption of message content in JSON serialization *(re-confirmed; serde simply emits the `Option<String>` field).*

### What this leaves open

Nothing in `router.rs`. If trace-file size ever becomes a concern, an explicit `max_traced_message_chars` config knob is a reasonable future addition — but it would be opt-in, not a silent behavior change.

### Dependencies
None - standalone investigation.