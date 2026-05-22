---
id: wire-muninnengine-explore
level: task
title: "Wire MuninnEngine::explore lightweight DTO path"
short_code: "PROJEC-T-0077"
created_at: 2026-05-21T15:15:10.934414+00:00
updated_at: 2026-05-21T15:15:10.934414+00:00
parent: 
blocked_by: []
archived: false

tags:
  - "#task"
  - "#phase/backlog"
  - "#tech-debt"


exit_criteria_met: false
initiative_id: NULL
---

# Wire MuninnEngine::explore lightweight DTO path

## Objective

`MuninnEngine::explore(ExploreRequest) → ExploreResult` is on the trait but its impl in `crates/muninn-rlm/src/engine/muninn_engine_impl.rs` returns an "not yet wired" error. Callers that want recursive exploration today go through `complete()` with a `MuninnConfig::recursive()` instead.

Wire the lightweight DTO path so adapters can reach exploration without constructing a full `CompletionRequest`. The hook's `submit_inner` would be the obvious first consumer — it currently builds a `CompletionRequest` with the RLM-instruction prefix, which is awkward when a structured `ExploreRequest { prompt, budget?, work_dir? }` would say the same thing more directly.

### Type
- [ ] Bug
- [ ] Feature
- [x] Tech Debt
- [ ] Chore

### Priority
- [ ] P0
- [ ] P1
- [x] P2
- [ ] P3

### Impact Assessment **[CONDITIONAL: Bug]**
- **Affected Users**: {Number/percentage of users affected}
- **Reproduction Steps**: 
  1. {Step 1}
  2. {Step 2}
  3. {Step 3}
- **Expected vs Actual**: {What should happen vs what happens}

### Business Justification **[CONDITIONAL: Feature]**
- **User Value**: {Why users need this}
- **Business Value**: {Impact on metrics/revenue}
- **Effort Estimate**: {Rough size - S/M/L/XL}

### Technical Debt Impact
- **Current Problems**: `explore` is on the public trait but unusable; readers of `muninn-core/src/lib.rs` see the method and reasonably expect it to work. Anyone wiring a non-Claude adapter will trip on it.
- **Benefits of Fixing**: One coherent surface — `explore` for "I want recursive exploration with a structured budget," `complete` for "I want full chat-completion semantics." The hook's `submit_inner` simplifies a bit.
- **Risk Assessment**: Low — the wiring is mechanical (delegate to `RecursiveEngine::complete` with a synthesized `CompletionRequest`). Risk is mostly schema decisions about `ExploreRequest`'s shape.

## Acceptance Criteria

- [ ] `MuninnEngine::explore(ExploreRequest)` returns a real `ExploreResult` against `RecursiveEngine`, not the "not yet wired" error.
- [ ] `ExploreRequest` has a clear minimal shape (`prompt`, optional `budget`, optional `work_dir` override). Document in `muninn-core/src/types.rs`.
- [ ] Unit test in `engine/muninn_engine_impl.rs` covering happy path against a `MockBackend`.
- [ ] (Optional) `submit_inner` migrated to use `explore` instead of building a `CompletionRequest` with the embedded RLM instruction.

## Test Cases **[CONDITIONAL: Testing Task]**

{Delete unless this is a testing task}

### Test Case 1: {Test Case Name}
- **Test ID**: TC-001
- **Preconditions**: {What must be true before testing}
- **Steps**: 
  1. {Step 1}
  2. {Step 2}
  3. {Step 3}
- **Expected Results**: {What should happen}
- **Actual Results**: {To be filled during execution}
- **Status**: {Pass/Fail/Blocked}

### Test Case 2: {Test Case Name}
- **Test ID**: TC-002
- **Preconditions**: {What must be true before testing}
- **Steps**: 
  1. {Step 1}
  2. {Step 2}
- **Expected Results**: {What should happen}
- **Actual Results**: {To be filled during execution}
- **Status**: {Pass/Fail/Blocked}

## Documentation Sections **[CONDITIONAL: Documentation Task]**

{Delete unless this is a documentation task}

### User Guide Content
- **Feature Description**: {What this feature does and why it's useful}
- **Prerequisites**: {What users need before using this feature}
- **Step-by-Step Instructions**:
  1. {Step 1 with screenshots/examples}
  2. {Step 2 with screenshots/examples}
  3. {Step 3 with screenshots/examples}

### Troubleshooting Guide
- **Common Issue 1**: {Problem description and solution}
- **Common Issue 2**: {Problem description and solution}
- **Error Messages**: {List of error messages and what they mean}

### API Documentation **[CONDITIONAL: API Documentation]**
- **Endpoint**: {API endpoint description}
- **Parameters**: {Required and optional parameters}
- **Example Request**: {Code example}
- **Example Response**: {Expected response format}

## Implementation Notes **[CONDITIONAL: Technical Task]**

{Keep for technical tasks, delete for non-technical. Technical details, approach, or important considerations}

### Technical Approach
{How this will be implemented}

### Dependencies
{Other tasks or systems this depends on}

### Risk Considerations
{Technical risks and mitigation strategies}

## Status Updates **[REQUIRED]**

*To be added during implementation*