---
id: implement-rustdoc-json-extraction
level: task
title: "Implement rustdoc JSON extraction (cargo rustdoc --output-format json)"
short_code: "PROJEC-T-0050"
created_at: 2026-01-19T14:24:28.399839+00:00
updated_at: 2026-01-19T16:05:17.901757+00:00
parent: PROJEC-I-0010
blocked_by: []
archived: true

tags:
  - "#task"
  - "#phase/completed"


exit_criteria_met: false
initiative_id: PROJEC-I-0010
---

# Implement rustdoc JSON extraction (cargo rustdoc --output-format json)

*This template includes sections for various types of tasks. Delete sections that don't apply to your specific use case.*

## Parent Initiative **[CONDITIONAL: Assigned Task]**

[[PROJEC-I-0010]]

## Objective

Implement rustdoc JSON extraction functionality to parse documentation from Rust crates for indexing in the DocStore.

## Acceptance Criteria

## Acceptance Criteria

## Acceptance Criteria

- [x] `RustdocExtractor` struct that runs `cargo rustdoc --output-format json`
- [x] Support for nightly Rust toolchain (auto-detects and uses `cargo +nightly`)
- [x] Parse rustdoc JSON into `ExtractedItem` structs
- [x] Extract functions, structs, enums, traits, modules, type aliases, constants
- [x] Capture doc text, signatures, visibility, and full paths
- [x] Convert to `DocChunkInput` for storage in DocStore
- [x] Unit tests for extraction logic
- [x] Integration test with real crate download

## Implementation Notes

### Files Created/Modified
- `crates/muninn-graph/src/registry/rustdoc.rs` - Main implementation
- `crates/muninn-graph/src/registry/mod.rs` - Added exports
- `crates/muninn-graph/Cargo.toml` - Added `rustdoc-types = "0.56"`

### Key Types
- `RustdocExtractor` - Runs cargo rustdoc and generates JSON
- `ExtractedItem` - Parsed documentation item with path, type, docs, signature, visibility
- `ItemVisibility` - Public, Crate, Restricted, Private
- `RustdocError` - Error type for rustdoc operations

### Key Functions
- `generate_json(crate_path)` - Run rustdoc and return JSON path
- `extract_docs_from_json(json_path)` - Parse JSON into ExtractedItem vector
- `extract_docs_from_crate(krate)` - Extract from parsed Crate struct
- `items_to_chunks(items)` - Convert to DocChunkInput for storage

### Technical Notes
- Rustdoc JSON output requires nightly Rust (`-Z unstable-options`)
- Method attempts `cargo +nightly rustdoc` first, falls back to default toolchain
- Uses `rustdoc-types` crate v0.56 for JSON parsing
- Methods are represented as `ItemEnum::Function` (not a separate Method variant)
- Only public items are extracted by default

## Status Updates

### 2026-01-19
- Created `rustdoc.rs` module with full implementation
- Added `rustdoc-types = "0.56"` dependency
- Fixed API mismatches with rustdoc-types:
  - Removed `ItemEnum::Method` (methods are Functions)
  - Fixed `GenericParamDef.name` type (String, not Option<String>)
- Added nightly toolchain detection (`cargo +nightly rustdoc`)
- All 4 unit tests pass
- Integration test passes with nightly Rust installed
- **TASK COMPLETE**