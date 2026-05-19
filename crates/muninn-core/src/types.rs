//! Engine request/response types.
//!
//! These DTOs cross both the local-IPC wire (adapter ↔ daemon) and the
//! MCP wire (Claude Code ↔ MCP server). Treat them as a stable contract:
//! additive changes are fine (new optional fields), renames and removals
//! are breaking.
//!
//! Types stay deliberately small. If a field is "nice to have," leave it
//! out until an adapter actually needs it.

use serde::{Deserialize, Serialize};

// ─────────────────────────────────────────────────────────────────────────────
// search_code
// ─────────────────────────────────────────────────────────────────────────────

/// Text/regex search over the working tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchQuery {
    /// The pattern to search for. Treated as a regex when `is_regex` is
    /// true; otherwise a literal substring.
    pub pattern: String,
    /// When true, `pattern` is a regex; otherwise a literal substring.
    #[serde(default)]
    pub is_regex: bool,
    /// Optional path-glob filter (e.g. `src/**/*.rs`). When unset, the
    /// engine searches the whole working tree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_glob: Option<String>,
    /// Optional language tag filter (e.g. `"rust"`, `"python"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Maximum hits to return. `None` lets the engine pick a default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// A single hit returned by `search_code`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchHit {
    pub path: String,
    pub line: u32,
    /// The matching line (or surrounding snippet if the engine widens it).
    pub snippet: String,
}

/// Aggregated result of a `search_code` call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchResult {
    pub hits: Vec<SearchHit>,
    /// `true` if the engine truncated results to satisfy `limit`.
    #[serde(default)]
    pub truncated: bool,
}

// ─────────────────────────────────────────────────────────────────────────────
// explore (recursive)
// ─────────────────────────────────────────────────────────────────────────────

/// Kicks off a recursive LLM-driven exploration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExploreRequest {
    /// The high-level question the agent wants answered (e.g. "how does
    /// auth work in this repo?").
    pub question: String,
    /// Optional seed paths to bias initial exploration.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub seed_paths: Vec<String>,
    /// Optional caller-supplied budget override. `None` defers to engine
    /// defaults (which adapters typically configure once per daemon).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_depth: Option<u32>,
}

/// Final answer plus the trail of evidence the engine gathered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExploreResult {
    /// The engine's synthesized answer.
    pub answer: String,
    /// Files/symbols the engine consulted while answering.
    #[serde(default)]
    pub evidence: Vec<String>,
    /// `true` if the engine hit its budget before finishing.
    #[serde(default)]
    pub truncated: bool,
}

// ─────────────────────────────────────────────────────────────────────────────
// recall_memory / record_memory
// ─────────────────────────────────────────────────────────────────────────────

/// Lookup against the engine's persistent memory store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryQuery {
    /// Natural-language or keyword query. The engine picks the retrieval
    /// strategy (embedding, keyword, hybrid).
    pub query: String,
    /// Maximum hits to return.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// A single match against the memory store.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryHit {
    /// Opaque identifier; stable across the lifetime of the store.
    pub id: String,
    /// The remembered content (markdown).
    pub content: String,
    /// Engine-assigned relevance score in `[0.0, 1.0]`.
    pub score: f32,
}

/// A new entry to persist in the memory store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryItem {
    /// Markdown content to remember.
    pub content: String,
    /// Optional source tag (file path, ADR id, etc.) — helps the engine
    /// rank / dedupe later.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// search_docs
// ─────────────────────────────────────────────────────────────────────────────

/// Search the indexed library documentation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocsQuery {
    /// The natural-language query (e.g. "how do tokio joinsets work").
    pub query: String,
    /// Optional ecosystem filter (e.g. `"rust"`, `"python"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ecosystem: Option<String>,
    /// Optional library filter (e.g. `"tokio"`, `"requests"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub library: Option<String>,
    /// Maximum hits to return.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// A single docs-search hit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DocsHit {
    pub library: String,
    pub version: String,
    pub item_path: String,
    pub snippet: String,
    pub score: f32,
}

/// Aggregated docs-search result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DocsResult {
    pub hits: Vec<DocsHit>,
}

// ─────────────────────────────────────────────────────────────────────────────
// query_graph
// ─────────────────────────────────────────────────────────────────────────────

/// What kind of graph relationship to chase from the `target`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphQueryKind {
    /// Symbols that call `target`.
    Callers,
    /// Symbols `target` calls.
    Callees,
    /// Definitions of `target`.
    Defines,
    /// References to `target`.
    References,
}

/// A graph query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphQuery {
    /// Symbol name or `file:line` location to query.
    pub target: String,
    pub kind: GraphQueryKind,
    /// Maximum hops; `None` defers to engine defaults.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_hops: Option<u32>,
}

/// A node in the result graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphNode {
    /// Stable id within this result (e.g. fully-qualified symbol name).
    pub id: String,
    /// Source location, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
}

/// An edge between two [`GraphNode`]s.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphEdge {
    pub from: String,
    pub to: String,
    /// Edge kind (e.g. `"calls"`, `"defines"`).
    pub kind: String,
}

/// Aggregated graph-query result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphResult {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_query_roundtrips_minimal() {
        let q = SearchQuery {
            pattern: "fn main".into(),
            is_regex: false,
            path_glob: None,
            language: None,
            limit: None,
        };
        let s = serde_json::to_string(&q).unwrap();
        // Optional fields omitted from the wire form.
        assert!(!s.contains("path_glob"));
        let q2: SearchQuery = serde_json::from_str(&s).unwrap();
        assert_eq!(q, q2);
    }

    #[test]
    fn search_query_roundtrips_full() {
        let q = SearchQuery {
            pattern: "fn .*".into(),
            is_regex: true,
            path_glob: Some("src/**/*.rs".into()),
            language: Some("rust".into()),
            limit: Some(50),
        };
        let s = serde_json::to_string(&q).unwrap();
        let q2: SearchQuery = serde_json::from_str(&s).unwrap();
        assert_eq!(q, q2);
    }

    #[test]
    fn graph_query_kind_serializes_snake_case() {
        let q = GraphQuery {
            target: "foo::bar".into(),
            kind: GraphQueryKind::Callers,
            max_hops: None,
        };
        let s = serde_json::to_string(&q).unwrap();
        assert!(s.contains("\"callers\""));
    }

    #[test]
    fn memory_item_source_optional() {
        let item = MemoryItem {
            content: "remember this".into(),
            source: None,
        };
        let s = serde_json::to_string(&item).unwrap();
        assert!(!s.contains("source"));
    }
}
