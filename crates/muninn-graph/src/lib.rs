//! muninn-graph: Code graph infrastructure
//!
//! This crate provides the core code graph functionality for Muninn:
//! - Symbol extraction from source code via tree-sitter
//! - Graph storage and querying
//! - Graph building from source files
//! - File watching for incremental updates

pub mod builder;
pub mod edges;
pub mod lang;
pub mod parser;
pub mod store;
pub mod symbols;
pub mod watcher;

pub use builder::{BuildError, BuildStats, GraphBuilder};
pub use edges::{CallType, Edge, EdgeKind};
pub use lang::python::PythonExtractor;
pub use lang::rust::{Call, FFIMarker, Import, RustExtractor};
pub use parser::{Language, ParseError, ParsedFile, Parser};
pub use store::{GraphStats, GraphStore, StoreError};
pub use symbols::{Symbol, SymbolKind, Visibility};
pub use watcher::{FileEvent, FileWatcher, WatchError, WatcherConfig};
