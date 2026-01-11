---
id: router-trace-data-truncates
level: task
title: "Router trace data truncates/corrupts user message content"
short_code: "PROJEC-T-0045"
created_at: 2026-01-10T19:48:59.758834+00:00
updated_at: 2026-01-10T20:06:48.513091+00:00
parent: 
blocked_by: []
archived: false

tags:
  - "#task"
  - "#bug"
  - "#phase/active"


exit_criteria_met: false
strategy_id: NULL
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

## Acceptance Criteria **[REQUIRED]**

- [ ] Router trace data captures full user message content
- [ ] Verified with messages of various lengths (10 chars, 100 chars, 1000 chars)
- [ ] Integration test added to verify trace data accuracy
- [ ] No truncation or corruption of message content in JSON serialization

## Implementation Notes **[CONDITIONAL: Technical Task]**

### Evidence
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
- `RouterTraceData` struct (line ~31)
- `get_last_user_message()` method (line ~397)
- Trace emission at line ~281

### Technical Approach
1. Add debug logging in `get_last_user_message()` to see raw content before truncation
2. Check `Content::to_text()` implementation in `types.rs` (line ~194)
3. Verify serde serialization doesn't truncate
4. Test with various message lengths

### Dependencies
None - standalone bug fix

### Risk Considerations
- Trace files may grow larger with full message content (acceptable tradeoff)
- May need to add optional truncation for very long messages (>1000 chars)