---
id: implement-python-parser-with
level: task
title: "Implement Python parser with symbol extraction"
short_code: "PROJEC-T-0008"
created_at: 2026-01-08T03:02:57.651058+00:00
updated_at: 2026-01-08T18:28:37.509202+00:00
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

# Implement Python parser with symbol extraction

*This template includes sections for various types of tasks. Delete sections that don't apply to your specific use case.*

## Parent Initiative **[CONDITIONAL: Assigned Task]**

[[PROJEC-I-0001]]

## Objective

Implement Python-specific symbol extraction using tree-sitter queries. Extract classes, functions, imports, and detect FFI patterns (ctypes, cffi).

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

- [ ] Tree-sitter queries for Python (classes, functions, methods, decorators)
- [ ] Extract docstrings as doc_comment
- [ ] Extract `import` and `from...import` statements
- [ ] Detect function calls
- [ ] Detect ctypes patterns (`cdll.LoadLibrary`, `CFUNCTYPE`)
- [ ] Detect cffi patterns (`ffi.cdef`, `ffi.dlopen`)
- [ ] Handle decorated functions (@staticmethod, @classmethod, @property)
- [ ] Unit tests with sample Python code
- [ ] Integration test with real Python project

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
`crates/muninn-graph/src/lang/python.rs`

### Tree-Sitter Queries

```scheme
;; Classes
(class_definition
  name: (identifier) @name
  body: (block) @body) @class

;; Functions
(function_definition
  name: (identifier) @name
  parameters: (parameters) @params
  return_type: (_)? @return
  body: (block) @body) @function

;; Methods (functions inside class)
(class_definition
  body: (block
    (function_definition
      name: (identifier) @method_name))) @method

;; Imports
(import_statement
  name: (dotted_name) @module) @import

(import_from_statement
  module_name: (dotted_name) @module
  name: (dotted_name) @name) @from_import

;; Function calls
(call
  function: (_) @callee) @call

;; Decorators
(decorated_definition
  (decorator) @decorator) @decorated
```

### FFI Detection Patterns

```scheme
;; ctypes loading
(call
  function: (attribute
    object: (_)
    attribute: (identifier) @method)
  (#match? @method "LoadLibrary|CDLL|WinDLL")) @ctypes_load

;; cffi
(call
  function: (attribute
    attribute: (identifier) @method)
  (#match? @method "cdef|dlopen|verify")) @cffi_call
```

### PythonExtractor Interface

```rust
pub struct PythonExtractor;

impl PythonExtractor {
    pub fn extract_symbols(tree: &Tree, source: &str) -> Vec<Symbol>;
    pub fn extract_imports(tree: &Tree, source: &str) -> Vec<Import>;
    pub fn extract_calls(tree: &Tree, source: &str) -> Vec<Call>;
    pub fn extract_ffi_markers(tree: &Tree, source: &str) -> Vec<FFIMarker>;
}
```

### Python-Specific Considerations
- Docstrings: first string literal in function/class body
- Visibility: leading underscore convention (`_private`, `__dunder__`)
- No explicit types for older Python code

### Reference
- narsil-mcp `parser.rs` Python query patterns

### Dependencies
- Depends on: PROJEC-T-0001, PROJEC-T-0002, PROJEC-T-0003

## Status Updates

*To be added during implementation*