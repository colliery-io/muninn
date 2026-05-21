---
id: implement-hybrid-search-semantic
level: task
title: "Implement hybrid search (semantic + FTS with RRF fusion)"
short_code: "PROJEC-T-0048"
created_at: 2026-01-19T14:24:02.060447+00:00
updated_at: 2026-01-19T20:42:50.397470+00:00
parent: PROJEC-I-0010
blocked_by: []
archived: true

tags:
  - "#task"
  - "#phase/completed"


exit_criteria_met: false
initiative_id: PROJEC-I-0010
---

# Implement hybrid search (semantic + FTS with RRF fusion)

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

## Status Updates **[REQUIRED]**

### 2026-01-19: Implementation Complete

Implemented hybrid search combining FTS5 and semantic search with RRF (Reciprocal Rank Fusion).

**New Types Added:**
- `ScoredChunk` - DocChunk with relevance score
- `SearchMode` - Enum for Fts/Semantic/Hybrid modes
- `RRF_K` constant (60.0) - Standard RRF parameter

**New Functions:**
- `search_semantic()` - Stub for embedding-based search (returns empty until sqlite-vec integrated)
- `search_hybrid()` - Main hybrid search combining FTS + semantic with RRF fusion
- `search()` - Convenience method using hybrid mode with FTS fallback
- `rrf_fusion()` - Reciprocal Rank Fusion algorithm implementation
- `chunks_to_scored()` - Convert chunks to scored chunks with RRF scores

**RRF Algorithm:**
```
RRF_score(d) = Σ 1 / (k + rank(d))
```
- Items appearing in both FTS and semantic results get boosted
- No score normalization needed between systems
- Robust to outliers

**Current Behavior:**
- Hybrid mode falls back to FTS-only when no embeddings available
- Semantic search returns empty (stub) until sqlite-vec is integrated
- All search methods return `ScoredChunk` with relevance scores

**Tests Added:**
- `test_search_hybrid_fts_mode` - FTS-only mode works
- `test_search_hybrid_default_mode` - Hybrid falls back to FTS
- `test_search_convenience_method` - search() convenience method
- `test_rrf_fusion_algorithm` - RRF algorithm correctness
- `test_search_semantic_stub` - Semantic stub returns empty

**Test Results:** All 12 doc_store tests pass

**Files Modified:**
- `crates/muninn-graph/src/doc_store.rs` (added hybrid search)
- `crates/muninn-graph/src/lib.rs` (exported new types)