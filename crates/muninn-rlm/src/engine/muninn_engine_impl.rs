//! `MuninnEngine` trait implementation for [`RecursiveEngine`].
//!
//! This wires the adapter-neutral trait from `muninn-core` to muninn-rlm's
//! existing recursive exploration engine. Status:
//! - `complete` — load-bearing; delegates to [`RecursiveEngine::complete`].
//! - `search_code` — wired against [`crate::fs_tools::SearchFilesTool`].
//! - `query_graph` — wired against the optional graph store for
//!   `Callers` / `Callees` / `Defines`. `References` still returns
//!   an explicit "not yet implemented" error.
//! - `explore` — still stubbed; will land when the lightweight DTO
//!   has a clearer story.
//!
//! `search_docs` is no longer on the trait — it was dropped from
//! v1's externally-exposed surface to keep muninn focused on RLM.
//! The doc store + indexer infra still exists and the RLM's
//! internal exploration tools may use it.

use async_trait::async_trait;

use muninn_core::{
    CompletionRequest, CompletionResponse, ExploreRequest, ExploreResult, GraphQuery, GraphResult,
    MuninnCoreError, MuninnEngine, SearchQuery, SearchResult, error::Result as CoreResult,
};

use super::RecursiveEngine;
use crate::error::{BudgetExceededError, RlmError};

#[async_trait]
impl MuninnEngine for RecursiveEngine {
    async fn complete(&self, request: CompletionRequest) -> CoreResult<CompletionResponse> {
        // Disambiguate against the trait method we're defining — call the
        // inherent method on RecursiveEngine, not ourselves.
        RecursiveEngine::complete(self, request)
            .await
            .map_err(rlm_to_core)
    }

    async fn search_code(&self, query: SearchQuery) -> CoreResult<SearchResult> {
        let Some(work_dir) = self.work_dir.as_ref() else {
            return Err(MuninnCoreError::internal(
                "search_code: engine has no work_dir configured",
            ));
        };
        let tool = crate::fs_tools::SearchFilesTool::with_fs(work_dir, self.file_system.clone());
        tool.run_search(query).await
    }

    async fn explore(&self, _request: ExploreRequest) -> CoreResult<ExploreResult> {
        Err(MuninnCoreError::internal(
            "explore: lightweight DTO path is not yet wired; use `complete` with a recursive `MuninnConfig` for now",
        ))
    }

    async fn query_graph(&self, query: GraphQuery) -> CoreResult<GraphResult> {
        let Some(store) = self.graph_store.as_ref() else {
            return Err(MuninnCoreError::internal(
                "query_graph: engine has no graph store configured",
            ));
        };
        run_graph_query(store, query)
    }
}

/// Translate a [`GraphQuery`] into the appropriate
/// [`muninn_graph::GraphStore`] call(s) and shape the result into a
/// [`GraphResult`]. Each branch:
/// - resolves the `target` (treated first as an existing node id; if
///   no node has that id, falls back to `find_by_name`),
/// - calls the matching store method,
/// - synthesizes one edge per returned node, oriented per the query kind.
///
/// `Defines` returns the resolved definition node(s) with no edges.
/// `References` is not yet implemented at the store level — surface a
/// clear error rather than fake-empty results.
fn run_graph_query(
    store: &crate::graph_tools::SharedGraphStore,
    query: muninn_core::types::GraphQuery,
) -> muninn_core::error::Result<muninn_core::types::GraphResult> {
    use muninn_core::MuninnCoreError;
    use muninn_core::types::{GraphEdge, GraphNode, GraphQueryKind, GraphResult};

    let guard = store
        .lock()
        .map_err(|_| MuninnCoreError::internal("graph store mutex poisoned"))?;

    // Resolve target → node id. Try id-as-given first (cheap has_node
    // check), then name lookup.
    let resolved_id = match guard.has_node(&query.target) {
        Ok(true) => query.target.clone(),
        _ => {
            let matches = guard
                .find_by_name(&query.target)
                .map_err(|e| MuninnCoreError::internal(format!("graph find_by_name: {e}")))?;
            if matches.is_empty() {
                return Ok(GraphResult {
                    nodes: vec![],
                    edges: vec![],
                });
            }
            match extract_id(&matches[0]) {
                Some(id) => id,
                None => {
                    return Err(MuninnCoreError::internal("graph node missing id property"));
                }
            }
        }
    };

    let (raw_nodes, edge_kind): (Vec<graphqlite::Value>, &str) = match query.kind {
        GraphQueryKind::Callers => {
            let v = guard
                .find_callers(&resolved_id)
                .map_err(|e| MuninnCoreError::internal(format!("graph find_callers: {e}")))?;
            (v, "calls")
        }
        GraphQueryKind::Callees => {
            let v = guard
                .find_callees(&resolved_id)
                .map_err(|e| MuninnCoreError::internal(format!("graph find_callees: {e}")))?;
            (v, "calls")
        }
        GraphQueryKind::Defines => {
            let v = guard
                .find_by_name(&query.target)
                .map_err(|e| MuninnCoreError::internal(format!("graph find_by_name: {e}")))?;
            (v, "defines")
        }
        GraphQueryKind::References => {
            return Err(MuninnCoreError::internal(
                "query_graph: references is not yet implemented at the store level",
            ));
        }
    };

    let mut nodes = vec![GraphNode {
        id: resolved_id.clone(),
        location: None,
    }];
    let mut edges = Vec::new();
    for raw in &raw_nodes {
        let id = extract_id(raw).unwrap_or_else(|| String::from("<unknown>"));
        let location = extract_location(raw);
        // For Callers: edge points from caller → target.
        // For Callees and Defines: edge points from target → callee/def.
        let (from, to) = match query.kind {
            GraphQueryKind::Callers => (id.clone(), resolved_id.clone()),
            _ => (resolved_id.clone(), id.clone()),
        };
        nodes.push(GraphNode { id, location });
        edges.push(GraphEdge {
            from,
            to,
            kind: edge_kind.to_string(),
        });
    }
    Ok(GraphResult { nodes, edges })
}

/// Pull the `id` string out of a graphqlite node Value. Mirrors the
/// extraction logic the existing tool-side helper uses.
fn extract_id(value: &graphqlite::Value) -> Option<String> {
    if let graphqlite::Value::Object(map) = value {
        if let Some(graphqlite::Value::Object(props)) = map.get("properties") {
            if let Some(graphqlite::Value::String(id)) = props.get("id") {
                return Some(id.clone());
            }
        }
        if let Some(graphqlite::Value::String(id)) = map.get("id") {
            return Some(id.clone());
        }
    }
    None
}

/// Best-effort `file:line` extraction for the [`GraphNode::location`]
/// field. Returns `None` when the node doesn't carry that property.
fn extract_location(value: &graphqlite::Value) -> Option<String> {
    let graphqlite::Value::Object(map) = value else {
        return None;
    };
    let props = match map.get("properties") {
        Some(graphqlite::Value::Object(p)) => p,
        _ => map,
    };
    let file = match props.get("file") {
        Some(graphqlite::Value::String(s)) => s.clone(),
        _ => return None,
    };
    let line = match props.get("line") {
        Some(graphqlite::Value::Integer(i)) => *i,
        _ => return Some(file),
    };
    Some(format!("{file}:{line}"))
}

/// Map a muninn-rlm `RlmError` to the adapter-neutral `MuninnCoreError`.
fn rlm_to_core(e: RlmError) -> MuninnCoreError {
    match e {
        RlmError::Backend(s) | RlmError::Network(s) => MuninnCoreError::Backend(s),
        RlmError::ToolExecution(s) => MuninnCoreError::Internal(format!("tool execution: {s}")),
        RlmError::BudgetExceeded(b) => MuninnCoreError::BudgetExceeded(format_budget(&b)),
        RlmError::InvalidRequest(s) => MuninnCoreError::InvalidRequest(s),
        RlmError::Serialization(s) => MuninnCoreError::Internal(format!("serialization: {s}")),
        RlmError::Config(s) => MuninnCoreError::Internal(format!("config: {s}")),
        RlmError::Protocol(s) => MuninnCoreError::Internal(format!("protocol: {s}")),
        RlmError::Internal(s) => MuninnCoreError::Internal(s),
    }
}

fn format_budget(b: &BudgetExceededError) -> String {
    format!(
        "{:?} budget exceeded (limit={}, actual={})",
        b.budget_type, b.limit, b.actual
    )
}
