---
id: use-cc-s-transcript-path-to-give
level: task
title: "Use CC's transcript_path to give UserPromptSubmit hook conversation history"
short_code: "PROJEC-T-0079"
created_at: 2026-05-21T18:38:39.847153+00:00
updated_at: 2026-05-21T18:38:39.847153+00:00
parent: 
blocked_by: []
archived: false

tags:
  - "#task"
  - "#phase/backlog"
  - "#feature"


exit_criteria_met: false
initiative_id: NULL
---

# Use CC's transcript_path to give UserPromptSubmit hook conversation history

## Objective

The UserPromptSubmit hook is per-turn stateless: it only receives the current user prompt via CC's hook input JSON, with no conversation history. This means prompts containing cross-turn references like *"check again"*, *"fix that"*, or *"like the previous one"* hit the RLM with no anchor, producing generic non-answers. Caught during the v1 live UAT — the architectural limit, not a code bug.

CC's hook input does include a `transcript_path` field pointing at the per-session JSONL transcript on disk. We currently parse it into `UserPromptInput` but ignore it. Reading the last N turns from that file would give the RLM enough context to resolve most cross-turn anaphora.

### Type
- [ ] Bug
- [x] Feature
- [ ] Tech Debt
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

### Business Justification
- **User Value**: Today the hook gives unhelpful answers for short referential prompts ("check again", "fix that"). With history, the RLM can resolve "that" to the actual prior code/file/symbol, and the inject becomes useful on conversational follow-ups rather than only first-turn questions.
- **Business Value**: Materially raises the hit-rate of useful injects in real CC sessions — most CC turns are NOT brand-new questions, they're follow-ups. Without this the hook silently passes through on the majority of real-world turns even when it could help.
- **Effort Estimate**: S — read the JSONL, take the last N records, format as `Vec<Message>`, pass to `CompletionRequest::messages` instead of a single-message vec. The engine already truncates downstream to `RLM_CONTEXT_USER_MESSAGES = 3`, so we don't need our own bounding.

## Acceptance Criteria

- [ ] `UserPromptInput` parses `transcript_path` field (already does — verify and remove `#[serde(skip)]` if present).
- [ ] `submit_inner` reads the transcript JSONL (last ~5 records of role=user|assistant) and builds a `Vec<Message>` ending with the current prompt + RLM-instruction wrapper.
- [ ] Bounded read: open with a deadline (50ms?) and a max byte count so a corrupt or huge transcript can't blow the hook's budget.
- [ ] On any read failure, fall back to single-message behavior — keep NFR-002 silent-passthrough invariant.
- [ ] New UAT test in `crates/muninn/tests/user_prompt_submit.rs`: write a fake transcript file, set `transcript_path` in the hook input, assert the injected answer references content from the prior turn.

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