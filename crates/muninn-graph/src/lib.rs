//! muninn-graph: Code graph infrastructure
//!
//! This crate provides the core code graph functionality for Muninn:
//! - Symbol extraction from source code via tree-sitter
//! - Graph storage and querying
//! - Graph building from source files
//! - File watching for incremental updates
//! - Registry clients for fetching package metadata and source

pub mod builder;
pub mod doc_store;
pub mod edges;
pub mod lang;
pub mod module_path;
pub mod parser;
pub mod registry;
pub mod store;
pub mod symbols;
pub mod watcher;

pub use builder::{BuildError, BuildStats, GraphBuilder};
pub use doc_store::{
    DocChunk, DocChunkInput, DocLibrary, DocStore, DocStoreError, Ecosystem, ItemType, ScoredChunk,
    SearchMode,
};
pub use edges::{CallType, Edge, EdgeKind};
pub use lang::python::PythonExtractor;
pub use lang::rust::{Call, FFIMarker, Import, RustExtractor};
pub use parser::{Language, ParseError, ParsedFile, Parser};
pub use store::{GraphStats, GraphStore, StoreError};
pub use symbols::{Symbol, SymbolKind, Visibility};
pub use watcher::{FileEvent, FileWatcher, WatchError, WatcherConfig};
