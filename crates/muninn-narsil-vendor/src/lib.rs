//! Vendored code-graph extraction from [narsil-mcp](https://github.com/postrv/narsil-mcp).
//!
//! See `NOTICE.md` at the crate root for the upstream commit, license
//! attribution, and the rationale for vendoring rather than depending.
//!
//! ## Scope of the vendor
//!
//! We took a focused subset of narsil's `src/`: the parts that
//! extract symbols and call graphs from source via tree-sitter. We
//! did NOT vendor narsil's RDF/SPARQL layer, its MCP server, its
//! security scanner, its embeddings/neural code, its LSP integration,
//! its frontend, or its persistence layer. Those concerns are handled
//! upstream of this crate by muninn's own infrastructure.
//!
//! ## What's exposed
//!
//! - [`symbols`] — `Symbol` and `SymbolKind` (narsil's, not muninn's)
//! - [`parser`] — `LanguageParser` over tree-sitter
//! - [`extract`] — extraction helpers
//! - [`callgraph`] — `CallGraph`, `CallNode`, `CallEdge`, scope-hint resolver
//! - [`incremental`] — Merkle-tree-based per-file change detection
//!
//! Consumers (currently `muninn-graph`) adapt these types to muninn's
//! graph store at the boundary.

pub mod callgraph;
pub mod extract;
pub mod incremental;
pub mod parser;
pub mod symbols;

/// Re-export tree-sitter so downstream crates can name `Tree` /
/// `Node` / etc. without taking their own dep on tree-sitter (which
/// would otherwise conflict via the `links = "tree-sitter"` rule
/// if the version drifts).
pub use tree_sitter;
