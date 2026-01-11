---
id: implement-rust-parser-with-symbol
level: task
title: "Implement Rust parser with symbol extraction"
short_code: "PROJEC-T-0004"
created_at: 2026-01-08T03:02:50.206250+00:00
updated_at: 2026-01-08T14:00:43.046106+00:00
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

# Implement Rust parser with symbol extraction

*This template includes sections for various types of tasks. Delete sections that don't apply to your specific use case.*

## Parent Initiative **[CONDITIONAL: Assigned Task]**

[[PROJEC-I-0001]]

## Objective

Implement Rust-specific symbol extraction using tree-sitter queries. This is the first language parser and establishes patterns for subsequent language implementations.

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

- [ ] Tree-sitter queries for Rust symbol extraction (structs, enums, functions, traits, impls, mods)
- [ ] Extract visibility (pub, pub(crate), etc.) from Rust AST
- [ ] Extract function signatures
- [ ] Extract doc comments (/// and //!)
- [ ] Detect `use` statements for IMPORTS edges
- [ ] Detect function calls for CALLS edges
- [ ] Detect extern blocks for FFI edge markers
- [ ] Unit tests with sample Rust code
- [ ] Integration test parsing muninn-graph's own source

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
`crates/muninn-graph/src/lang/rust.rs`

### Tree-Sitter Queries

```scheme
;; Structs
(struct_item
  name: (type_identifier) @name
  body: (field_declaration_list)?) @struct

;; Enums
(enum_item
  name: (type_identifier) @name) @enum

;; Functions
(function_item
  name: (identifier) @name
  parameters: (parameters) @params
  return_type: (_)? @return) @function

;; Traits
(trait_item
  name: (type_identifier) @name) @trait

;; Impl blocks
(impl_item
  trait: (type_identifier)? @trait_name
  type: (type_identifier) @type_name) @impl

;; Use statements
(use_declaration
  argument: (_) @path) @use

;; Function calls
(call_expression
  function: (_) @callee) @call

;; Extern blocks (FFI)
(foreign_mod
  (extern_crate_declaration)?) @extern
```

### Extraction Logic

```rust
pub struct RustExtractor;

impl RustExtractor {
    pub fn extract_symbols(tree: &Tree, source: &str) -> Vec<Symbol>;
    pub fn extract_imports(tree: &Tree, source: &str) -> Vec<Import>;
    pub fn extract_calls(tree: &Tree, source: &str) -> Vec<Call>;
    pub fn extract_ffi_markers(tree: &Tree, source: &str) -> Vec<FFIMarker>;
}
```

### Reference
- narsil-mcp `parser.rs` Rust query patterns (lines 300-500)
- tree-sitter-rust node types

### Dependencies
- Depends on: PROJEC-T-0001, PROJEC-T-0002, PROJEC-T-0003

## Status Updates

*To be added during implementation*