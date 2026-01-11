---
id: code-graph-infrastructure-parser
level: initiative
title: "Code Graph Infrastructure: Parser, Data Model, and Storage"
short_code: "PROJEC-I-0001"
created_at: 2026-01-08T02:26:41.958620+00:00
updated_at: 2026-01-08T18:28:47.763028+00:00
parent: PROJEC-V-0001
blocked_by: []
archived: true

tags:
  - "#initiative"
  - "#phase/completed"


exit_criteria_met: false
estimated_complexity: L
strategy_id: NULL
initiative_id: code-graph-infrastructure-parser
---

# Code Graph Infrastructure: Parser, Data Model, and Storage Initiative

*This template includes sections for various types of initiatives. Delete sections that don't apply to your specific use case.*

## Context

Muninn requires a code graph to enable intelligent context selection for agentic coding. The graph must understand code structure (what symbols exist), relationships (who calls whom, what imports what), and special patterns (FFI, macros, generated code).

**Reference Documents:**
- ADR-002 (PROJEC-A-0002): Defines the node types, edge types, and query requirements
- narsil-mcp analysis: Identified reusable patterns for tree-sitter parsing and symbol extraction

**Key Constraints:**
- Must support Rust (priority 1), Python (priority 2), C/C++ (priority 3)
- Storage via graphqlite with Cypher queries
- File watcher for continuous per-file rebuild (no batch Merkle tree approach)
- Cache stored in `.muninn/cache/` (gitignored derived data per ADR-001)

## Goals & Non-Goals

**Goals:**
- Build tree-sitter parsing infrastructure supporting Rust, Python, C/C++
- Define and implement the symbol data model (node types per ADR-002)
- Define and implement edge types including FFI, dynamic calls, and generated code tracking
- Integrate graphqlite for graph storage with Cypher query support
- Implement file watcher for continuous incremental rebuild
- Expose graph via MCP tools for Claude Code integration

**Non-Goals:**
- Visualization/rendering of the graph (YAGNI per prior discussion)
- Support for languages beyond Rust/Python/C/C++ in this initiative
- Vector embeddings (separate concern, uses sqlite-vec)
- Session memory or curated knowledge (ADR-001 Layer 2/3)

## Requirements

### Node Types (from ADR-002)
| Type | Description |
|------|-------------|
| File | Source file as container |
| Module | Logical grouping (Rust mod, Python module, C/C++ translation unit) |
| Class | OOP class |
| Struct | Data structure |
| Interface | Abstract interface (trait, protocol, ABC) |
| Enum | Enumeration type |
| Function | Standalone function |
| Method | Function attached to type |
| Variable | Module-level variable/constant |
| Type | Type alias or typedef |
| Macro | Macro definition |

### Edge Types (from ADR-002)
| Edge | Description |
|------|-------------|
| CONTAINS | Parent contains child (File→Function) |
| IMPORTS | Import/use relationship |
| CALLS | Direct function call |
| CALLS_DYNAMIC | Dynamic dispatch (vtable, reflection, dlopen) |
| CALLS_FFI | Foreign function interface (extern "C", PyO3, ctypes) |
| CALLS_API | External API call (HTTP, gRPC) |
| INHERITS | Class inheritance |
| IMPLEMENTS | Interface/trait implementation |
| USES_TYPE | Type reference in signature/body |
| INSTANTIATES | Object/struct construction |
| REFERENCES | Variable/constant reference |
| EXPANDS_TO | Macro expansion |
| GENERATED_BY | Code generation tracking |

### Functional Requirements
- REQ-001: Parse Rust source files extracting all node types
- REQ-002: Parse Python source files extracting classes, functions, imports
- REQ-003: Parse C/C++ source files extracting structs, functions, includes
- REQ-004: Detect Rust FFI patterns (extern blocks, PyO3 attributes)
- REQ-005: Detect Python FFI patterns (ctypes, cffi)
- REQ-006: Store graph in graphqlite with defined schema
- REQ-007: Support Cypher queries for graph traversal
- REQ-008: Watch file system for changes and rebuild affected nodes
- REQ-009: Expose MCP tools for graph queries

### Non-Functional Requirements
- NFR-001: Parse 10k file repository in under 60 seconds on cold start
- NFR-002: Incremental update within 500ms of file change
- NFR-003: Graph database under 100MB for typical repository

## Use Cases

### UC-1: Find All Callers of a Function
- **Actor**: Claude Code agent (via MCP)
- **Scenario**: Agent needs to understand impact of changing `parse_config()`
- **Query**: `MATCH (caller)-[:CALLS]->(f:Function {name: 'parse_config'}) RETURN caller`
- **Expected Outcome**: List of functions that call `parse_config` with file locations

### UC-2: Trace FFI Boundary
- **Actor**: Claude Code agent
- **Scenario**: Agent investigating a crash that may cross FFI boundary
- **Query**: `MATCH (rust)-[:CALLS_FFI]->(c) RETURN rust, c`
- **Expected Outcome**: All Rust functions calling into C/C++ code

### UC-3: Find Implementations of Trait
- **Actor**: Claude Code agent
- **Scenario**: Agent needs to understand all types implementing `Iterator`
- **Query**: `MATCH (t)-[:IMPLEMENTS]->(trait:Interface {name: 'Iterator'}) RETURN t`
- **Expected Outcome**: All structs/types that implement the trait

### UC-4: Discover Generated Code
- **Actor**: Claude Code agent
- **Scenario**: Agent should not modify generated files
- **Query**: `MATCH (f:File)-[:GENERATED_BY]->(gen) RETURN f, gen`
- **Expected Outcome**: Files marked as generated with their generator source

## Architecture

### Overview

```
┌─────────────────────────────────────────────────────────────┐
│                     MCP Tool Layer                          │
│  (graph_query, find_callers, find_implementations, etc.)    │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                    Query Interface                          │
│              (Cypher queries via graphqlite)                │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                    Graph Storage                            │
│           (graphqlite: .muninn/cache/graph.db)              │
└─────────────────────────────────────────────────────────────┘
                              ▲
                              │
┌─────────────────────────────────────────────────────────────┐
│                   Graph Builder                             │
│     (transforms parsed symbols into graph nodes/edges)      │
└─────────────────────────────────────────────────────────────┘
                              ▲
                              │
┌─────────────────────────────────────────────────────────────┐
│                 Language Parsers                            │
│  ┌─────────┐  ┌─────────┐  ┌─────────┐                     │
│  │  Rust   │  │ Python  │  │  C/C++  │                     │
│  │ Parser  │  │ Parser  │  │ Parser  │                     │
│  └─────────┘  └─────────┘  └─────────┘                     │
│         (tree-sitter with per-language queries)            │
└─────────────────────────────────────────────────────────────┘
                              ▲
                              │
┌─────────────────────────────────────────────────────────────┐
│                   File Watcher                              │
│          (notify crate, debounced events)                   │
└─────────────────────────────────────────────────────────────┘
```

### Components

**1. File Watcher** (adapt from narsil-mcp `persist.rs`)
- Uses `notify` crate for cross-platform file watching
- Debounces events (300ms buffer)
- Filters to source file extensions only
- Triggers per-file rebuild on change

**2. Language Parsers** (adapt from narsil-mcp `parser.rs`)
- `LanguageConfig` struct with tree-sitter language + queries
- `LazyLanguageConfig` with `OnceLock` for deferred compilation
- Per-language symbol extraction queries
- Extended for FFI pattern detection

**3. Graph Builder** (new)
- Transforms `Symbol` structs into graph nodes
- Resolves cross-file references to create edges
- Handles special cases: FFI, macros, generated code

**4. Graph Storage** (new, graphqlite)
- Schema for node types and edge types
- Rust bindings to graphqlite
- Stored at `.muninn/cache/graph.db`

**5. Query Interface** (new)
- Cypher query execution
- Common query patterns as helper functions
- MCP tool wrappers

## Detailed Design

### Data Structures

**Symbol (adapted from narsil-mcp):**
```rust
pub enum SymbolKind {
    File, Module, Class, Struct, Interface, Enum,
    Function, Method, Variable, Type, Macro,
}

pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub file_path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub signature: Option<String>,
    pub qualified_name: Option<String>,
    pub doc_comment: Option<String>,
    pub visibility: Visibility,  // NEW: public/private/crate
}
```

**Edge Types:**
```rust
pub enum EdgeKind {
    Contains,
    Imports { path: String },
    Calls { call_type: CallType },
    Inherits,
    Implements,
    UsesType,
    Instantiates,
    References,
    ExpandsTo,
    GeneratedBy { generator: String },
}

pub enum CallType {
    Direct,
    Method,
    StaticMethod,
    Dynamic,    // vtable, reflection
    FFI,        // extern, PyO3, ctypes
    API,        // HTTP, gRPC
}
```

### Tree-Sitter Query Extensions

**Rust FFI Detection (extend narsil queries):**
```scheme
;; extern "C" blocks
(foreign_mod) @ffi.block

;; PyO3 function attributes
(attribute_item
  (attribute
    (identifier) @attr (#match? @attr "pyfunction|pymethods"))) @pyo3.attr

;; libloading/dlopen patterns
(call_expression
  function: (field_expression
    field: (field_identifier) @method (#eq? @method "get")))
  @dynamic.load
```

**Generated Code Detection:**
```scheme
;; File-level comments indicating generation
(line_comment) @comment (#match? @comment "Generated|AUTO-GENERATED|DO NOT EDIT")
```

### graphqlite Schema

```sql
-- Nodes
CREATE TABLE nodes (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    name TEXT NOT NULL,
    file_path TEXT NOT NULL,
    start_line INTEGER,
    end_line INTEGER,
    signature TEXT,
    qualified_name TEXT,
    visibility TEXT
);

-- Edges  
CREATE TABLE edges (
    source_id TEXT NOT NULL,
    target_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    metadata TEXT,  -- JSON for edge-specific data
    FOREIGN KEY (source_id) REFERENCES nodes(id),
    FOREIGN KEY (target_id) REFERENCES nodes(id)
);

-- Indexes for common queries
CREATE INDEX idx_nodes_kind ON nodes(kind);
CREATE INDEX idx_nodes_name ON nodes(name);
CREATE INDEX idx_nodes_file ON nodes(file_path);
CREATE INDEX idx_edges_source ON edges(source_id);
CREATE INDEX idx_edges_target ON edges(target_id);
CREATE INDEX idx_edges_kind ON edges(kind);
```

## Testing Strategy

### Unit Testing
- Parser tests with sample source files per language
- Graph builder tests with known symbol relationships
- Query tests against fixture databases

### Integration Testing
- End-to-end: file change → watcher → parser → graph update → query verification
- Test repositories: small (100 files), medium (1k files), large (10k files)

### Test Fixtures from narsil-mcp
The narsil-mcp repository includes test cases in `parser.rs` that cover:
- Rust: structs, functions, impls, traits, enums, mods
- Python: classes, functions
- C++: namespaces, classes, structs, enums
- And more languages

These can serve as a starting point for Muninn's test suite.

## Alternatives Considered

### 1. Use narsil-mcp Directly
**Rejected**: narsil-mcp is a complete MCP server with 76 tools, many unrelated to Muninn's needs. It uses DashMap for in-memory storage rather than graphqlite, and lacks FFI/macro/generated-code tracking.

### 2. LSP-Based Symbol Extraction
**Rejected**: Language servers (rust-analyzer, pylsp) provide rich symbol info but require running language-specific servers. Tree-sitter is lighter weight, consistent across languages, and sufficient for structural analysis.

### 3. Merkle Tree Change Detection (narsil approach)
**Rejected**: The Merkle tree in `incremental.rs` is elegant for batch diffing but adds complexity. File watcher with continuous per-file rebuild is simpler and meets our latency requirements.

### 4. In-Memory Graph (DashMap like narsil)
**Rejected**: graphqlite provides persistence, Cypher queries, and handles larger codebases. DashMap would require custom serialization and query implementation.

### 5. Full AST Storage
**Rejected**: Storing complete ASTs would explode storage requirements. Symbol-level extraction with edge relationships captures the essential structure.

## Implementation Plan

### Phase 1: Foundation
- [ ] Set up Rust workspace with Cargo.toml
- [ ] Adapt `symbols.rs` from narsil-mcp (extend SymbolKind)
- [ ] Adapt `parser.rs` LanguageConfig pattern
- [ ] Implement Rust parser with basic symbol extraction
- [ ] Unit tests for Rust parsing

### Phase 2: Graph Storage
- [ ] Integrate graphqlite crate
- [ ] Implement schema (nodes, edges, indexes)
- [ ] Graph builder: Symbol → Node insertion
- [ ] Graph builder: Edge creation for CONTAINS, IMPORTS
- [ ] Basic Cypher query wrapper

### Phase 3: File Watching
- [ ] Adapt AsyncFileWatcher from narsil-mcp
- [ ] Implement debounced event handling
- [ ] Per-file incremental update logic
- [ ] Integration test: file change → graph update

### Phase 4: Call Graph Edges
- [ ] CALLS edge detection (direct, method, static)
- [ ] CALLS_FFI detection (extern blocks)
- [ ] CALLS_DYNAMIC detection (trait objects)
- [ ] Transitive query support

### Phase 5: Additional Languages
- [ ] Python parser (classes, functions, imports)
- [ ] Python FFI detection (ctypes, cffi)
- [ ] C/C++ parser (structs, functions, includes)

### Phase 6: Advanced Features
- [ ] IMPLEMENTS edge (trait impls)
- [ ] EXPANDS_TO edge (macro tracking)
- [ ] GENERATED_BY edge (generated code detection)
- [ ] MCP tool exposure

### Dependencies
- `tree-sitter` + language grammars (tree-sitter-rust, tree-sitter-python, tree-sitter-c, tree-sitter-cpp)
- `graphqlite` (Rust bindings)
- `notify` (file watching)
- `tokio` (async runtime)
- `serde` (serialization)