---
id: implement-query-graph-kind
level: task
title: "Implement query_graph kind=references via graph store"
short_code: "PROJEC-T-0078"
created_at: 2026-05-21T15:15:12.180305+00:00
updated_at: 2026-05-21T15:15:12.180305+00:00
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

# Implement query_graph kind=references via graph store

## Objective

`MuninnEngine::query_graph` currently supports `kind = callers | callees | defines` against the real graph store; `kind = references` returns an explicit "not yet implemented at the store level" error. Add a `find_references(node_id)` API on `muninn-graph::GraphStore` and wire it through the trait impl so all four `GraphQueryKind` variants work end-to-end over MCP.

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
- **User Value**: `references` rounds out the graph-traversal surface — "show me everywhere this type is mentioned" is one of the most natural code-navigation questions, and right now it errors instead of returning results. With it wired, the agent gets a complete `query_graph` tool that doesn't have hidden gaps.
- **Business Value**: Modest. The other three kinds cover most use cases; this is the polish that makes the MCP tool's schema actually match its implementation.
- **Effort Estimate**: S — likely a Cypher query against the underlying graphqlite store similar to `find_callers` / `find_implementations`, then a one-line dispatch in `run_graph_query`.

## Acceptance Criteria

- [ ] `GraphStore::find_references(node_id)` added with the same shape as `find_callers` (returns `Vec<graphqlite::Value>`).
- [ ] `MuninnEngine::query_graph` with `kind: References` returns real nodes + edges (edge kind `"references"`), not the current placeholder error.
- [ ] MCP UAT extended: an additional branch in `mcp_tools_call_query_graph_returns_graph_payload` (or a sibling test) calls `kind: "references"` against a known target and asserts the response shape.
- [ ] Documentation update in `crates/muninn-core/src/types.rs` removes the implicit "references doesn't work" caveat from the GraphQueryKind doc-comment.

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