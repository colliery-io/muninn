---
id: dependency-documentation-context
level: initiative
title: "Dependency Documentation Context: Fetch, Index, and Search Library Docs"
short_code: "PROJEC-I-0010"
created_at: 2026-01-16T02:52:13.935372+00:00
updated_at: 2026-01-19T14:32:18.538235+00:00
parent: PROJEC-V-0001
blocked_by: []
archived: false

tags:
  - "#initiative"
  - "#phase/active"


exit_criteria_met: false
estimated_complexity: M
strategy_id: NULL
initiative_id: dependency-documentation-context
---

# Dependency Documentation Context: Self-Hosted Doc Extraction Pipeline

## Context

Muninn's RLM currently provides intelligent context selection from the user's codebase via tools like `read_file`, `grep`, and `graph_query`. However, when working with dependencies, the RLM lacks access to library documentation - leading to hallucinated APIs, outdated patterns, or generic advice.

Example: When writing axum handlers, the RLM should be able to search axum's documentation to understand extractors, routing patterns, and middleware - not just guess based on training data.

## Goals & Non-Goals

**Goals:**
- Enable RLM to search and retrieve dependency documentation during exploration
- **Own the entire pipeline** - fetch source from registries, extract docs ourselves
- Support Rust (crates.io → rustdoc JSON) and Python (PyPI → griffe/AST)
- Local semantic search with embeddings (same infrastructure as code)
- Fully offline-capable once indexed
- Aligned with Muninn's "rebuildable from source" principle

**Non-Goals:**
- Third-party API dependencies for core functionality (Context7 users can add via MCP if desired)
- npm/JavaScript ecosystem (future scope)
- Real-time sync (manual or TTL-based refresh is sufficient)

## Architecture

### Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                        RLM Exploration                          │
│                              │                                  │
│                    search_docs(library, query)                  │
│                              │                                  │
│                              ▼                                  │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │        Local Index (.muninn/muninn.db)                   │   │
│  │                                                         │   │
│  │  doc_chunks table:                                      │   │
│  │  • library, version, item_path, doc_text, embedding     │   │
│  │  • FTS5 for keyword search                              │   │
│  │                                                         │   │
│  │  doc_libraries table:                                   │   │
│  │  • library, ecosystem, version, indexed_at              │   │
│  └─────────────────────────────────────────────────────────┘   │
│                              │                                  │
│                    not indexed?                                 │
│                              │                                  │
│         ┌────────────────────┴────────────────────┐             │
│         ▼                                         ▼             │
│  ┌─────────────────────┐                ┌─────────────────────┐ │
│  │   Rust Pipeline     │                │   Python Pipeline   │ │
│  │                     │                │                     │ │
│  │ 1. crates.io API    │                │ 1. PyPI JSON API    │ │
│  │    → source tarball │                │    → sdist tarball  │ │
│  │ 2. Extract to temp  │                │ 2. Extract to temp  │ │
│  │ 3. rustdoc --json   │                │ 3. griffe extract   │ │
│  │ 4. Parse Crate JSON │                │ 4. Parse docstrings │ │
│  │ 5. Chunk + embed    │                │ 5. Chunk + embed    │ │
│  │ 6. Store locally    │                │ 6. Store locally    │ │
│  └─────────────────────┘                └─────────────────────┘ │
│                                                                 │
│  Supplementary sources (checked first):                        │
│  • {lib-homepage}/llms.txt or /llms-full.txt                   │
└─────────────────────────────────────────────────────────────────┘
```

### Data Flow

1. **Query**: RLM calls `search_docs("tokio", "spawn task")`
2. **Index check**: Is `tokio` in `doc_libraries`?
3. **Index hit**: Semantic + FTS search against `doc_chunks`
4. **Index miss**: 
   - Detect ecosystem (Rust crate vs Python package)
   - Run appropriate extraction pipeline
   - Chunk and embed
   - Store in local index
   - Then search
5. **Pre-indexing**: `muninn docs index tokio` runs pipeline upfront

### Rust Extraction Pipeline

**Source**: crates.io API

```
GET https://crates.io/api/v1/crates/{crate}
→ versions[0].dl_path → download source tarball
```

**Extraction**:
```bash
# Generate JSON documentation
cargo rustdoc --manifest-path {extracted}/Cargo.toml -- --output-format json
# Output: target/doc/{crate}.json
```

**JSON Structure** (rustdoc_json_types::Crate):
- `index`: Map of item ID → Item (structs, functions, traits, etc.)
- Each Item has: `name`, `docs` (the doc comment), `inner` (type-specific data)
- Traverse to extract all public API documentation

### Python Extraction Pipeline

**Source**: PyPI JSON API

```
GET https://pypi.org/pypi/{package}/json
→ urls[] where packagetype == "sdist" → download tarball
```

**Extraction** (using griffe):
```python
import griffe

# Load package from extracted source
package = griffe.load("{package_name}", search_paths=[extracted_path])

# Traverse modules, classes, functions
for module in package.modules.values():
    for obj in module.members.values():
        # obj.docstring contains parsed docstring
        # obj.signature contains type hints
```

**Docstring Styles**: griffe handles Google, NumPy, and Sphinx styles automatically.

### llms.txt Supplementary Source

Before running extraction pipelines, check if library publishes llms.txt:

```
GET https://{library-homepage}/llms.txt
GET https://{library-homepage}/llms-full.txt
```

If found, use directly (already optimized for LLM consumption). Fall back to extraction pipeline if not available.

## Detailed Design

### Storage Schema

```sql
-- Indexed libraries metadata
CREATE TABLE doc_libraries (
    id INTEGER PRIMARY KEY,
    library TEXT NOT NULL UNIQUE,
    ecosystem TEXT NOT NULL,        -- "rust" or "python"
    version TEXT NOT NULL,
    source_url TEXT,                -- homepage for llms.txt check
    indexed_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- Documentation chunks with embeddings
CREATE TABLE doc_chunks (
    id INTEGER PRIMARY KEY,
    library_id INTEGER NOT NULL REFERENCES doc_libraries(id),
    item_path TEXT NOT NULL,        -- e.g., "tokio::spawn" or "requests.Session.get"
    item_type TEXT NOT NULL,        -- "function", "struct", "class", "method", "module"
    doc_text TEXT NOT NULL,
    signature TEXT,                 -- function/method signature with types
    embedding BLOB,                 -- sqlite-vec vector
    UNIQUE(library_id, item_path)
);

-- FTS5 for keyword search
CREATE VIRTUAL TABLE doc_chunks_fts USING fts5(
    item_path, doc_text,
    content='doc_chunks',
    content_rowid='id'
);

CREATE INDEX idx_doc_chunks_library ON doc_chunks(library_id);
```

### RLM Tool

**`search_docs(library: str, query: str, ecosystem: str)`**
- Check if library is indexed
- If not indexed: trigger extraction pipeline, then search
- Search: hybrid semantic (embedding) + keyword (FTS) with RRF fusion
- Return ranked documentation chunks

**Tool Definition (Anthropic format):**
```json
{
  "name": "search_docs",
  "description": "Search documentation for a library. Returns relevant API docs, examples, and type signatures.",
  "input_schema": {
    "type": "object",
    "properties": {
      "library": {
        "type": "string",
        "description": "Library/crate/package name (e.g., 'tokio', 'requests', 'serde')"
      },
      "query": {
        "type": "string", 
        "description": "What you want to find (e.g., 'async task spawning', 'HTTP POST with JSON')"
      },
      "ecosystem": {
        "type": "string",
        "enum": ["rust", "python"],
        "description": "Package ecosystem: 'rust' for crates.io, 'python' for PyPI"
      }
    },
    "required": ["library", "query", "ecosystem"]
  }
}
```

### CLI Commands

- `muninn docs index <ecosystem> <library>` - Index a library (e.g., `muninn docs index rust tokio`)
- `muninn docs search <ecosystem> <library> <query>` - Search indexed docs
- `muninn docs list` - Show indexed libraries with version and chunk count
- `muninn docs remove <library>` - Remove from index
- `muninn docs update <library>` - Re-index with latest version



### Chunking Strategy

**Per-item chunking** (not arbitrary text windows):
- Each function/method/struct/class = one chunk
- Preserves semantic boundaries naturally
- `item_path` provides hierarchy context
- Include signature + full docstring

**For large docstrings** (>512 tokens):
- Split on markdown headers within the docstring
- Preserve item_path prefix on each chunk

### Error Handling

| Scenario | Response |
|----------|----------|
| Library not found on registry | "Library '{library}' not found on {ecosystem} registry" |
| No sdist/source available | "Source not available for '{library}' - only wheels published" |
| Extraction failed | "Failed to extract docs: {error}" |
| Not indexed + user query | Index first, then search (may be slow on first access)

### Freshness Strategy

- **Version-based**: Store indexed version, compare against registry latest
- **Manual update**: `muninn docs update <library>` re-indexes
- **No auto-refresh**: User controls when to update (keeps behavior predictable)

## Alternatives Considered

### 1. Context7 API (cache-through)
- **Pros**: Simple, Context7 handles extraction and ranking
- **Cons**: Third-party dependency, query metadata sent externally, limited to their index
- **Decision**: Rejected - conflicts with Muninn's "no cloud dependencies" principle

### 2. llms.txt only
- **Pros**: Simplest, many libraries already publish it
- **Cons**: Coverage gaps, not all libraries have llms.txt
- **Decision**: Use as supplementary source, not primary

### 3. Scrape docs.rs / ReadTheDocs HTML
- **Pros**: Covers published docs without source access
- **Cons**: Fragile (HTML structure changes), no type signatures, lossy
- **Decision**: Rejected - source extraction is more robust and complete

### 4. Use existing doc generation tools (pdoc, rustdoc HTML)
- **Pros**: Battle-tested tools
- **Cons**: Generate HTML, would need to parse back to structured data
- **Decision**: Rejected - use JSON output (rustdoc) and AST parsing (griffe) directly

## Implementation Plan

### Phase 1: Schema & Core Infrastructure
- Add `doc_libraries` and `doc_chunks` tables
- Set up FTS5 virtual table
- Implement embedding storage with sqlite-vec
- Hybrid search (semantic + FTS with RRF)

### Phase 2: Rust Extraction Pipeline
- crates.io API client (fetch crate metadata, download source)
- Tarball extraction to temp directory
- Run `cargo rustdoc --output-format json`
- Parse rustdoc JSON, extract items with docs
- Chunk and embed, store in index

### Phase 3: Python Extraction Pipeline
- PyPI API client (fetch package metadata, download sdist)
- Tarball extraction to temp directory
- griffe-based extraction (or fallback to AST)
- Parse docstrings (Google/NumPy/Sphinx styles)
- Chunk and embed, store in index

### Phase 4: RLM Tool Integration
- Implement `search_docs` tool (ecosystem required)
- On-demand indexing (index if not found, then search)
- Add to tool registry

### Phase 5: CLI Commands
- `muninn docs index` - manual indexing
- `muninn docs search` - test searches
- `muninn docs list` - show indexed libraries
- `muninn docs remove` / `update` - management

### Phase 6: llms.txt Support (Optional Enhancement)
- Check library homepage for llms.txt before extraction
- If found, use directly (skip extraction pipeline)
- Chunk and embed the llms.txt content