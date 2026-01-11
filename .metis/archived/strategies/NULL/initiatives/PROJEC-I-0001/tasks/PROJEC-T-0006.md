---
id: implement-graph-builder-symbol-to
level: task
title: "Implement graph builder (Symbol to Node/Edge)"
short_code: "PROJEC-T-0006"
created_at: 2026-01-08T03:02:57.510957+00:00
updated_at: 2026-01-08T18:28:37.371034+00:00
parent: PROJEC-I-0001
blocked_by: []
archived: true

tags:
  - "#task"
  - "#phase/completed"


exit_criteria_met: false
strategy_id: NULL
initiative_id: PROJEC-I-0001
---

# Implement graph builder (Symbol to Node/Edge)

*This template includes sections for various types of tasks. Delete sections that don't apply to your specific use case.*

## Parent Initiative **[CONDITIONAL: Assigned Task]**

[[PROJEC-I-0001]]

## Objective

Implement the graph builder that transforms parsed symbols and relationships into graph nodes and edges. This bridges the parser output and the graph storage layer.

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

## Acceptance Criteria

- [ ] `GraphBuilder` struct that coordinates parsing and storage
- [ ] Generate stable node IDs from file_path + name + kind
- [ ] Create CONTAINS edges (File -> symbols within)
- [ ] Create IMPORTS edges from use/import statements
- [ ] Create CALLS edges from function call extraction
- [ ] Resolve cross-file symbol references (best-effort)
- [ ] `build_file()` method: parse single file, update graph
- [ ] `rebuild_file()` method: delete old nodes/edges, rebuild
- [ ] Integration test: build graph from test fixtures

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

## Implementation Notes

### Location
`crates/muninn-graph/src/builder.rs`

### Node ID Generation
Stable IDs enable incremental updates:
```rust
fn generate_node_id(file_path: &str, name: &str, kind: &SymbolKind, line: usize) -> String {
    // Hash for stability across rebuilds
    format!("{}:{}:{}:{}", file_path, kind.as_str(), name, line)
}
```

### GraphBuilder Interface

```rust
pub struct GraphBuilder {
    parser: Parser,
    store: GraphStore,
}

impl GraphBuilder {
    pub fn new(store: GraphStore) -> Self;
    
    /// Parse and add a file to the graph
    pub fn build_file(&mut self, path: &Path) -> Result<BuildStats>;
    
    /// Remove old data and rebuild a file
    pub fn rebuild_file(&mut self, path: &Path) -> Result<BuildStats>;
    
    /// Build entire directory recursively
    pub fn build_directory(&mut self, path: &Path) -> Result<BuildStats>;
}

pub struct BuildStats {
    pub nodes_added: usize,
    pub edges_added: usize,
    pub parse_time_ms: u64,
    pub store_time_ms: u64,
}
```

### Cross-File Resolution
For CALLS edges where the target is in another file:
1. First pass: collect all symbols with qualified names
2. Second pass: resolve call targets to node IDs
3. Unresolved calls: store with target_id = "unresolved:{name}"

### Dependencies
- Depends on: PROJEC-T-0003 (Parser), PROJEC-T-0004 (Rust extractor), PROJEC-T-0005 (GraphStore)

## Status Updates

*To be added during implementation*