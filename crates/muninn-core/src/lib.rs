//! Adapter-neutral engine interface for muninn.
//!
//! `muninn-core` defines the [`MuninnEngine`] trait and the request/response
//! types that cross the engine boundary. Adapters — the existing proxy, the
//! upcoming MCP server, and the Claude Code hook plugin — all consume
//! `Arc<dyn MuninnEngine>` rather than depending on the concrete recursive
//! engine implementation in `muninn-rlm`.
//!
//! This crate must have **no dependency on `muninn-rlm` or any adapter
//! crate**. Dependencies flow inward toward `muninn-core`; nothing here
//! flows the other way.
//!
//! The trait is deliberately small. Grow it only when an adapter
//! genuinely needs a new method — not for symmetry. The DTOs cross both
//! the local IPC wire (between adapter processes and the daemon) and the
//! MCP wire (between Claude Code and the MCP server), so changing them
//! later is expensive.

pub mod error;
pub mod mcp;
pub mod types;

pub use error::{MuninnCoreError, Result};
pub use mcp::{McpToolSchema, SchemaStability, tool_schemas};
pub use types::{
    DocsHit, DocsQuery, DocsResult, ExploreRequest, ExploreResult, GraphEdge, GraphNode,
    GraphQuery, GraphQueryKind, GraphResult, MemoryHit, MemoryItem, MemoryQuery, SearchHit,
    SearchQuery, SearchResult,
};

use std::sync::Arc;

use async_trait::async_trait;

/// Adapter-neutral engine API.
///
/// Implementations are owned by a daemon process and reached by adapters
/// (proxy, MCP server, hook plugin) over local IPC. The trait is
/// object-safe and downstream consumers should hold `Arc<dyn MuninnEngine>`.
///
/// All methods are async and return [`Result`] so adapters can map engine
/// failures to their respective wire formats (HTTP, MCP, hook JSON)
/// uniformly.
#[async_trait]
pub trait MuninnEngine: Send + Sync {
    /// Text/regex code search over the working tree, with optional path
    /// and language filters.
    async fn search_code(&self, query: SearchQuery) -> Result<SearchResult>;

    /// Recursive exploration: the LLM-driven loop that walks the codebase
    /// to answer a high-level question. This is the expensive entry
    /// point — adapters should expose it only where bounded budget makes
    /// sense.
    async fn explore(&self, request: ExploreRequest) -> Result<ExploreResult>;

    /// Look up prior notes/decisions/observations that the engine has
    /// stored about this repo. Used by the hook to attach context to
    /// implicit tool calls.
    async fn recall_memory(&self, query: MemoryQuery) -> Result<Vec<MemoryHit>>;

    /// Persist a new memory item (a fact, a decision, an observation).
    /// Called by the engine itself during exploration, and by the hook
    /// when it observes something worth remembering.
    async fn record_memory(&self, item: MemoryItem) -> Result<()>;

    /// Search indexed library documentation. Returns ranked chunks.
    async fn search_docs(&self, query: DocsQuery) -> Result<DocsResult>;

    /// Query the code graph (callers, callees, defines, references, …)
    /// for a given symbol or location.
    async fn query_graph(&self, query: GraphQuery) -> Result<GraphResult>;
}

/// A shared, object-safe handle to a [`MuninnEngine`]. Adapters consume
/// this type alias rather than naming `Arc<dyn MuninnEngine>` directly.
pub type SharedEngine = Arc<dyn MuninnEngine>;
