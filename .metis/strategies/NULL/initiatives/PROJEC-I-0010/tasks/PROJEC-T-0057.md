---
id: implement-muninn-docs-index-cli
level: task
title: "Implement muninn docs index CLI command"
short_code: "PROJEC-T-0057"
created_at: 2026-01-19T14:27:18.128518+00:00
updated_at: 2026-01-19T17:26:27.408624+00:00
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

# Implement muninn docs index CLI command

## Parent Initiative

[[PROJEC-I-0010]]

## Objective

Add a `muninn docs` CLI subcommand that enables users to index documentation from Rust crates (crates.io) and Python packages (PyPI), list indexed libraries, and search documentation from the command line.

## Acceptance Criteria

## Acceptance Criteria

- [x] `muninn docs index-crate <name>` indexes a Rust crate from crates.io
- [x] `muninn docs index-package <name>` indexes a Python package from PyPI
- [x] `muninn docs list` shows all indexed libraries with metadata
- [x] `muninn docs search <library> <query>` searches documentation
- [x] All subcommands support `--db` option to specify custom database path
- [x] Help text is clear and includes usage examples
- [x] Tests pass

## Implementation Notes

### Technical Approach

Added a `Docs` subcommand to the existing CLI in `crates/muninn/src/main.rs` with four sub-subcommands:

1. **index-crate**: Uses `RustDocIndexer` from `muninn-graph` to download and index Rust crate documentation
2. **index-package**: Uses `PyDocIndexer` from `muninn-graph` to download and index Python package documentation
3. **list**: Opens the doc store and lists all indexed libraries in a table format
4. **search**: Performs FTS5 full-text search on the doc store and displays results

### Files Modified

- `crates/muninn/src/main.rs`:
  - Added imports for `DocStore`, `Ecosystem`, `PyDocIndexer`, `PyIndexerConfig`, `RustDocIndexer`, `IndexerConfig`
  - Added `Commands::Docs { command: DocsCommand }` variant
  - Added `DocsCommand` enum with `IndexCrate`, `IndexPackage`, `List`, `Search` variants
  - Added handler in main match statement for all four subcommands

### CLI Usage

```bash
# Index a Rust crate
muninn docs index-crate tokio --version 1.35.0

# Index a Python package
muninn docs index-package requests --python python3.12

# List indexed libraries
muninn docs list
muninn docs list --ecosystem rust

# Search documentation
muninn docs search tokio "spawn async task" -n 10
```

## Status Updates

### 2026-01-19

- Implemented all four docs subcommands in main.rs
- Fixed short flag conflict (`-v` for version vs global `--verbose`)
- Verified help text works for all subcommands
- All 11 muninn tests pass
- All muninn-rlm and muninn-graph tests pass