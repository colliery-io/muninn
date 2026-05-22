//! muninn-graph: Code graph infrastructure
//!
//! Symbol/edge schema, on-disk graph store (graphqlite), the build
//! pipeline that drives the vendored narsil call-graph extractor,
//! and the file watcher.
//!
//! Extraction itself lives in [`muninn_narsil_vendor`]. This crate
//! owns the adapter that converts narsil's `CallNode`/`CallEdge` into
//! our `Symbol`/`Edge` types and persists them through `GraphStore`.

pub mod builder;
pub mod doc_store;
pub mod edges;
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
pub use store::{GraphStats, GraphStore, StoreError};
pub use symbols::{Symbol, SymbolKind, Visibility};
pub use watcher::{FileEvent, FileWatcher, WatchError, WatcherConfig};
