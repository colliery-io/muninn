---
id: llm-based-router
level: task
title: "LLM-Based Router"
short_code: "PROJEC-T-0042"
created_at: 2026-01-10T17:01:47.494494+00:00
updated_at: 2026-01-10T18:29:38.186423+00:00
parent: 
blocked_by: []
archived: false

tags:
  - "#task"
  - "#feature"
  - "#phase/completed"


exit_criteria_met: false
strategy_id: NULL
initiative_id: NULL
---

# LLM-Based Router

*This template includes sections for various types of tasks. Delete sections that don't apply to your specific use case.*

## Objective

Replace heuristic-based routing with LLM-based decision making. The router calls a configurable LLM to decide whether a request should use RLM (recursive exploration) or passthrough (direct to backend).

## Details

### Type
- [x] Feature - New functionality or enhancement  

### Priority
- [x] P1 - High (important for user experience)

### Business Justification
- **User Value**: Better routing decisions mean faster responses for simple queries (passthrough) and richer context for complex queries (RLM)
- **Business Value**: Enables data collection for future SLM fine-tuning
- **Effort Estimate**: M

## Acceptance Criteria

## Acceptance Criteria

## Acceptance Criteria

## Acceptance Criteria

- [ ] Router LLM is separately configurable from RLM LLM
- [ ] Router uses tool call to select "RLM" or "Passthrough"
- [ ] Request/decision pairs are logged for future training data
- [ ] Explicit `muninn.recursive` JSON flag forces RLM (for testing/automation)
- [ ] Text trigger `muninn.recursive=true` in message forces RLM (for savvy users, skips router LLM)
- [ ] Fallback to passthrough if router LLM fails

## Implementation Notes

### Flow
```
Request arrives
    ↓
Check request.muninn.recursive (JSON field)
    → true: force RLM (testing/automation)
    ↓
Check message for "muninn.recursive=true" (text trigger)
    → found: force RLM (savvy users, skip router LLM)
    ↓
Router LLM receives request context
    ↓
LLM calls route_decision tool with choice: "rlm" | "passthrough"
    ↓
Request forwarded to RLM engine or passthrough handler
```

### Technical Approach

1. **New config options** in proxy config:
   - `router_backend`: Backend URL for router LLM (can differ from RLM backend)
   - `router_model`: Model to use for routing decisions
   
2. **Route decision tool**:
   ```rust
   struct RouteDecisionTool;
   // Returns: { "route": "rlm" | "passthrough", "reason": "..." }
   ```

3. **Reuse existing infrastructure**:
   - Use same tool execution pattern as RLM engine
   - Router is essentially a single-turn RLM call with one tool
   
4. **Decision logging**:
   - Log request summary + decision to `.muninn/routing_decisions.jsonl`
   - Future: use this data to fine-tune an SLM

### Dependencies
- Existing tool execution infrastructure
- Proxy request handling

### Risk Considerations
- **Latency**: Router LLM adds latency to every request
  - Mitigation: Use fast model, consider caching similar requests
- **Cost**: Every request now costs router tokens
  - Mitigation: Keep router prompt minimal, use cheap model

## Status Updates **[REQUIRED]**

*To be added during implementation*