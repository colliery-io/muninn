---
id: implement-griffe-based-python
level: task
title: "Implement griffe-based Python docstring extraction"
short_code: "PROJEC-T-0054"
created_at: 2026-01-19T14:25:34.923245+00:00
updated_at: 2026-01-19T16:33:17.493120+00:00
parent: PROJEC-I-0010
blocked_by: []
archived: false

tags:
  - "#task"
  - "#phase/completed"


exit_criteria_met: false
strategy_id: NULL
initiative_id: PROJEC-I-0010
---

# Implement griffe-based Python docstring extraction

*This template includes sections for various types of tasks. Delete sections that don't apply to your specific use case.*

## Parent Initiative **[CONDITIONAL: Assigned Task]**

[[PROJEC-I-0010]]

## Objective

Create a griffe-based Python documentation extractor that calls the griffe Python tool via subprocess, parses its JSON output, and converts extracted documentation to `DocChunkInput` for storage in DocStore.

## Acceptance Criteria

## Acceptance Criteria

- [x] Create `pydoc.rs` module with `GriffeExtractor` struct
- [x] Parse griffe JSON output for modules, classes, functions, methods
- [x] Extract docstrings, signatures, and type annotations
- [x] Convert to `DocChunkInput` for DocStore storage
- [x] Create `py_indexer.rs` with `PyDocIndexer` for full pipeline
- [x] All tests pass

## Implementation Notes

### Technical Approach

Created two new modules:

1. **`pydoc.rs`** - Griffe-based documentation extractor
   - `GriffeExtractor` struct that runs `python -m griffe dump <package>`
   - Parses griffe JSON output (modules, classes, functions, methods)
   - Extracts docstrings, signatures with type annotations
   - `items_to_chunks()` function converts to `DocChunkInput`

2. **`py_indexer.rs`** - Full indexing pipeline
   - `PyDocIndexer` combines PyPI download with griffe extraction
   - `index_package()` - download from PyPI and index
   - `index_local()` - index local package without download
   - `index_batch()` - index multiple packages

### Key Types

```rust
pub struct GriffeExtractor {
    python: String,      // Python executable
    flags: Vec<String>,  // Additional griffe flags
}

pub struct ExtractedPyItem {
    path: String,              // e.g., "requests.api.get"
    item_type: ItemType,       // Module, Class, Function, Method
    docstring: String,
    signature: Option<String>, // "def get(url: str, **kwargs) -> Response"
    parent_class: Option<String>,
}

pub struct PyDocIndexer {
    pypi_client: PyPiClient,
    config: PyIndexerConfig,
}
```

### Dependencies

- Requires griffe Python package: `pip install griffe`
- Uses PyPI client from PROJEC-T-0053

## Status Updates

### Session 1 (2026-01-19)
- Created `pydoc.rs` with `GriffeExtractor` for griffe-based extraction
- Implemented griffe JSON parsing for modules, classes, functions, methods
- Created `py_indexer.rs` with `PyDocIndexer` for full pipeline
- All 97 unit tests pass, 5 integration tests pass, 6 doc tests pass
- Files created:
  - `crates/muninn-graph/src/registry/pydoc.rs`
  - `crates/muninn-graph/src/registry/py_indexer.rs`
- Files modified:
  - `crates/muninn-graph/src/registry/mod.rs` (added exports)