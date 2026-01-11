---
id: set-up-tree-sitter-infrastructure
level: task
title: "Set up tree-sitter infrastructure with LanguageConfig pattern"
short_code: "PROJEC-T-0003"
created_at: 2026-01-08T03:02:50.131951+00:00
updated_at: 2026-01-08T13:50:09.292228+00:00
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

# Set up tree-sitter infrastructure with LanguageConfig pattern

*This template includes sections for various types of tasks. Delete sections that don't apply to your specific use case.*

## Parent Initiative **[CONDITIONAL: Assigned Task]**

[[PROJEC-I-0001]]

## Objective

Set up the tree-sitter parsing infrastructure that supports multiple languages. This includes the core parser abstraction, language configuration pattern (adapted from narsil-mcp), and lazy initialization for efficient resource usage.

## Acceptance Criteria

## Acceptance Criteria

## Acceptance Criteria

## Acceptance Criteria

- [ ] Add tree-sitter dependencies to muninn-graph Cargo.toml
- [ ] `Language` enum for supported languages (Rust, Python, C, Cpp)
- [ ] `LanguageConfig` struct holding tree-sitter language + compiled queries
- [ ] `LazyLanguageConfig` with `OnceLock` for deferred query compilation
- [ ] `Parser` struct that manages language configs and parses files
- [ ] File extension to language mapping
- [ ] Integration test parsing a simple file

## Implementation Notes

### Location
`crates/muninn-graph/src/parser.rs`

### Dependencies to Add (Cargo.toml)
```toml
[dependencies]
tree-sitter = "0.24"
tree-sitter-rust = "0.24"
tree-sitter-python = "0.23"
tree-sitter-c = "0.23"
tree-sitter-cpp = "0.23"
```

### Core Types (adapted from narsil-mcp)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    Rust,
    Python,
    C,
    Cpp,
}

impl Language {
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext {
            "rs" => Some(Language::Rust),
            "py" => Some(Language::Python),
            "c" | "h" => Some(Language::C),
            "cpp" | "cc" | "cxx" | "hpp" | "hxx" => Some(Language::Cpp),
            _ => None,
        }
    }
    
    pub fn tree_sitter_language(&self) -> tree_sitter::Language {
        match self {
            Language::Rust => tree_sitter_rust::LANGUAGE.into(),
            Language::Python => tree_sitter_python::LANGUAGE.into(),
            Language::C => tree_sitter_c::LANGUAGE.into(),
            Language::Cpp => tree_sitter_cpp::LANGUAGE.into(),
        }
    }
}

pub struct LanguageConfig {
    pub language: tree_sitter::Language,
    pub symbols_query: tree_sitter::Query,
    pub imports_query: tree_sitter::Query,
    pub calls_query: tree_sitter::Query,
}

pub struct LazyLanguageConfig {
    language: Language,
    config: OnceLock<LanguageConfig>,
}

pub struct Parser {
    ts_parser: tree_sitter::Parser,
    configs: HashMap<Language, LazyLanguageConfig>,
}
```

### Parser Interface

```rust
impl Parser {
    pub fn new() -> Self;
    pub fn parse_file(&mut self, path: &Path) -> Result<ParsedFile>;
    pub fn parse_source(&mut self, source: &str, lang: Language) -> Result<ParsedFile>;
}

pub struct ParsedFile {
    pub language: Language,
    pub tree: tree_sitter::Tree,
    pub source: String,
}
```

### Reference
- narsil-mcp `parser.rs` lines 1-200 for LanguageConfig pattern
- tree-sitter Rust bindings documentation

### Dependencies
- Depends on: PROJEC-T-0001, PROJEC-T-0002 (for Symbol/Edge types used in extraction)

## Status Updates

*To be added during implementation*