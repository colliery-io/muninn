---
id: add-llms-txt-fetch-and-indexing
level: task
title: "Add llms.txt fetch and indexing support (optional fast-path)"
short_code: "PROJEC-T-0060"
created_at: 2026-01-19T14:27:38.680394+00:00
updated_at: 2026-01-19T20:32:37.759214+00:00
parent: PROJEC-I-0010
blocked_by: []
archived: true

tags:
  - "#task"
  - "#phase/completed"


exit_criteria_met: false
initiative_id: PROJEC-I-0010
---

# Add llms.txt fetch and indexing support (optional fast-path)

*This template includes sections for various types of tasks. Delete sections that don't apply to your specific use case.*

## Parent Initiative **[CONDITIONAL: Assigned Task]**

[[PROJEC-I-0010]]

## Objective **[REQUIRED]**

{Clear statement of what this task accomplishes}

## Backlog Item Details **[CONDITIONAL: Backlog Item]**

{Delete this section when task is assigned to an initiative}

### Type
- [ ] Bug - Production issue that needs fixing
- [ ] Feature - New functionality or enhancement  
- [ ] Tech Debt - Code improvement or refactoring
- [ ] Chore - Maintenance or setup work

### Priority
- [ ] P0 - Critical (blocks users/revenue)
- [ ] P1 - High (important for user experience)
- [ ] P2 - Medium (nice to have)
- [ ] P3 - Low (when time permits)

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

### Technical Debt Impact **[CONDITIONAL: Tech Debt]**
- **Current Problems**: {What's difficult/slow/buggy now}
- **Benefits of Fixing**: {What improves after refactoring}
- **Risk Assessment**: {Risks of not addressing this}

## Acceptance Criteria

## Acceptance Criteria

## Acceptance Criteria

## Acceptance Criteria **[REQUIRED]**

- [ ] {Specific, testable requirement 1}
- [ ] {Specific, testable requirement 2}
- [ ] {Specific, testable requirement 3}

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

## Status Updates

### 2026-01-19 - Design Considerations

**User raised valid skepticism about llms.txt utility** given that we already have:
- rustdoc JSON extraction for Rust (structured API docs)
- griffe-based extraction for Python (structured docstrings)

**llms.txt value proposition (optional fast-path)**:
1. **Curated content**: Maintainer-selected docs optimized for LLMs
2. **Guides/tutorials**: Conceptual docs not in API references
3. **No build tools**: Skip cargo rustdoc/griffe dependencies
4. **Fast indexing**: Just HTTP fetch + markdown parse

**Limitations**:
- Limited adoption (primarily Mintlify-hosted docs)
- Often just links requiring further fetching
- Variable quality
- Less structured than API docs

**Decision**: Implement as optional supplement, not replacement. Use case:
- Quick indexing of documentation sites that provide llms.txt
- Complementary to source-based indexing
- User explicitly chooses when to use it

### Implementation Progress

- Created `llmstxt.rs` parser module with:
  - `LlmsTxtParser::parse()` - parse llms.txt markdown format
  - `LlmsTxtFetcher::fetch()` - HTTP fetch with URL normalization
  - `LlmsTxtIndexer` - stores parsed docs in DocStore
  - Unit tests for parsing
- Added `Web` ecosystem and `Page`/`Guide` item types to `doc_store.rs`
- Added `muninn docs index-llms` CLI command with:
  - `--fast` flag for description-only indexing (no link fetching)
  - `--max-links` for limiting number of links to fetch
- Fixed tokio runtime panic by wrapping blocking HTTP calls in `spawn_blocking`

### Testing Results

```
$ muninn docs index-llms "https://mintlify.com/docs/llms.txt" --fast --db /tmp/test.db
INFO muninn: Opening doc store at /tmp/test.db
INFO muninn: Indexing llms.txt from https://mintlify.com/docs/llms.txt (fast mode)...
INFO muninn: Successfully indexed 'Mintlify'
INFO muninn:   159 links found, 100 indexed, 0 failed

$ muninn docs search Mintlify "components" --db /tmp/test.db
INFO muninn: Searching 'components' in Mintlify vllms.txt (web)...
1. Mintlify::React (page)
   Build interactive and reusable elements with React components.
2. Mintlify::Tree (page)
   Use tree components to display hierarchical file and folder structures.
3. Mintlify::Frames (page)
   Add visual emphasis with styled frames around images and other components.
INFO muninn: Found 3 results
```

### Files Modified

- `crates/muninn-graph/src/registry/llmstxt.rs` (NEW) - Parser, fetcher, indexer
- `crates/muninn-graph/src/registry/mod.rs` - Exports llmstxt module
- `crates/muninn-graph/src/doc_store.rs` - Added Web ecosystem, Page/Guide types
- `crates/muninn/src/main.rs` - Added index-llms command, spawn_blocking fixes

### Completion Status

All acceptance criteria met:
- [x] llms.txt parser for markdown format
- [x] HTTP fetcher with URL normalization
- [x] DocStore integration with Web ecosystem
- [x] CLI command: `muninn docs index-llms`
- [x] Fast mode for description-only indexing
- [x] Fixed tokio runtime panic with spawn_blocking
- [x] All tests passing (11/11 muninn tests)