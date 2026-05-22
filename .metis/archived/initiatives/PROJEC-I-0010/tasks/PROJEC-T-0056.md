---
id: implement-search-docs-rlm-tool
level: task
title: "Implement search_docs RLM tool with on-demand indexing"
short_code: "PROJEC-T-0056"
created_at: 2026-01-19T14:26:11.564224+00:00
updated_at: 2026-01-19T20:42:27.563315+00:00
parent: PROJEC-I-0010
blocked_by: []
archived: true

tags:
  - "#task"
  - "#phase/completed"


exit_criteria_met: false
initiative_id: PROJEC-I-0010
---

# Implement search_docs RLM tool with on-demand indexing

## Parent Initiative

[[PROJEC-I-0010]]

## Objective

Implement RLM tools for searching library documentation and on-demand indexing of Rust crates and Python packages. These tools enable the RLM engine to query documentation from indexed libraries and trigger indexing of new libraries as needed.

## Acceptance Criteria

## Acceptance Criteria

## Acceptance Criteria

## Acceptance Criteria

- [x] Create `search_docs` tool for searching documentation in indexed libraries
- [x] Create `index_crate` tool for on-demand Rust crate indexing from crates.io
- [x] Create `index_package` tool for on-demand Python package indexing from PyPI
- [x] Create `list_libraries` tool to list all indexed libraries
- [x] Tools follow existing `Tool` trait pattern from `muninn-rlm`
- [x] All tests pass



## Implementation Notes

### Technical Approach

Created `doc_tools.rs` module following the existing tool pattern in `muninn-rlm`. The tools use:

- `SharedDocStore` (Arc<Mutex<DocStore>>) for thread-safe access to the documentation database
- `tokio::task::spawn_blocking` for indexing operations (which use blocking I/O)
- Existing `RustDocIndexer` and `PyDocIndexer` from `muninn-graph`

### Key Types

```rust
pub type SharedDocStore = Arc<Mutex<DocStore>>;

// Tools
pub struct SearchDocsTool { store: SharedDocStore }
pub struct IndexCrateTool { store: SharedDocStore, work_dir: Option<PathBuf> }
pub struct IndexPackageTool { store: SharedDocStore, work_dir: Option<PathBuf>, python: String }
pub struct ListLibrariesTool { store: SharedDocStore }

// Factory
pub fn create_doc_tools(store: SharedDocStore) -> Vec<Box<dyn Tool>>
```

### Dependencies

- PROJEC-T-0052: Rust doc indexer
- PROJEC-T-0054: Python griffe extractor
- PROJEC-T-0055: Python doc indexer

## Status Updates

### Session 1 (2026-01-19)
- Created `doc_tools.rs` module in `muninn-rlm` crate
- Implemented 4 tools:
  1. **`SearchDocsTool`** (`search_docs`) - Search documentation in indexed libraries
  2. **`IndexCrateTool`** (`index_crate`) - On-demand indexing of Rust crates from crates.io
  3. **`IndexPackageTool`** (`index_package`) - On-demand indexing of Python packages from PyPI
  4. **`ListLibrariesTool`** (`list_libraries`) - List all indexed libraries
- Added `SharedDocStore` type and `wrap_doc_store()` helper
- Added `create_doc_tools()` factory function
- Registered module and exports in `lib.rs`
- All 287 unit tests pass, 10 integration tests pass
- Files created:
  - `crates/muninn-rlm/src/doc_tools.rs`
- Files modified:
  - `crates/muninn-rlm/src/lib.rs` (added module and exports)