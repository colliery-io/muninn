---
id: define-symbol-and-symbolkind-types
level: task
title: "Define Symbol and SymbolKind types in muninn-graph"
short_code: "PROJEC-T-0001"
created_at: 2026-01-08T03:02:49.986702+00:00
updated_at: 2026-01-08T13:30:53.222645+00:00
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

# Define Symbol and SymbolKind types in muninn-graph

*This template includes sections for various types of tasks. Delete sections that don't apply to your specific use case.*

## Parent Initiative **[CONDITIONAL: Assigned Task]**

[[PROJEC-I-0001]]

## Objective

Define the core data types for representing code symbols extracted from source files. These types form the foundation of the code graph and are used by parsers, graph storage, and query interfaces.

## Acceptance Criteria

## Acceptance Criteria

## Acceptance Criteria

## Acceptance Criteria

- [ ] `SymbolKind` enum with all node types from ADR-002 (File, Module, Class, Struct, Interface, Enum, Function, Method, Variable, Type, Macro)
- [ ] `Symbol` struct with fields: name, kind, file_path, start_line, end_line, signature, qualified_name, doc_comment, visibility
- [ ] `Visibility` enum (Public, Private, Crate, Restricted)
- [ ] Derive Serialize/Deserialize for all types
- [ ] Unit tests for type construction and serialization
- [ ] Documentation comments on all public types

## Implementation Notes

### Location
`crates/muninn-graph/src/symbols.rs`

### Types to Define

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SymbolKind {
    File,
    Module,
    Class,
    Struct,
    Interface,  // trait, protocol, ABC
    Enum,
    Function,
    Method,
    Variable,   // module-level constant/variable
    Type,       // type alias, typedef
    Macro,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Visibility {
    Public,
    Private,
    Crate,              // pub(crate)
    Restricted(String), // pub(in path)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub file_path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub signature: Option<String>,
    pub qualified_name: Option<String>,
    pub doc_comment: Option<String>,
    pub visibility: Visibility,
}
```

### Dependencies
- `serde` with derive feature (already in workspace)

### Reference
- narsil-mcp `symbols.rs` for base pattern
- ADR-002 for node type definitions

## Status Updates

*To be added during implementation*