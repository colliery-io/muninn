//! `MuninnEngine` trait implementation for [`RecursiveEngine`].
//!
//! This wires the adapter-neutral trait from `muninn-core` to muninn-rlm's
//! existing recursive exploration engine. The rich `complete()` method is
//! the load-bearing one — it's what the proxy adapter calls — and it
//! delegates directly to [`RecursiveEngine::complete`].
//!
//! The lightweight MCP-shaped methods (`search_code`, `explore`,
//! `recall_memory`, `record_memory`, `search_docs`, `query_graph`) are
//! stubbed for now. Wiring them requires direct handles to the underlying
//! stores (graph, doc, memory) which the current `RecursiveEngine`
//! reaches only through `ToolEnvironment`. Those wirings are tracked as
//! follow-up tasks under PROJEC-I-0011 and intentionally not bundled
//! into this commit (see PROJEC-T-0065 status notes).

use async_trait::async_trait;

use muninn_core::{
    CompletionRequest, CompletionResponse, DocsQuery, DocsResult, ExploreRequest, ExploreResult,
    GraphQuery, GraphResult, MemoryHit, MemoryItem, MemoryQuery, MuninnCoreError, MuninnEngine,
    SearchQuery, SearchResult, error::Result as CoreResult,
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

    async fn search_code(&self, _query: SearchQuery) -> CoreResult<SearchResult> {
        Err(MuninnCoreError::internal(
            "search_code is not yet wired to RecursiveEngine — see PROJEC-T-0065 status notes",
        ))
    }

    async fn explore(&self, _request: ExploreRequest) -> CoreResult<ExploreResult> {
        Err(MuninnCoreError::internal(
            "explore is not yet wired to RecursiveEngine via the lightweight DTO — see PROJEC-T-0065 status notes",
        ))
    }

    async fn recall_memory(&self, _query: MemoryQuery) -> CoreResult<Vec<MemoryHit>> {
        Err(MuninnCoreError::internal(
            "recall_memory is not yet wired to RecursiveEngine — see PROJEC-T-0065 status notes",
        ))
    }

    async fn record_memory(&self, _item: MemoryItem) -> CoreResult<()> {
        Err(MuninnCoreError::internal(
            "record_memory is not yet wired to RecursiveEngine — see PROJEC-T-0065 status notes",
        ))
    }

    async fn search_docs(&self, _query: DocsQuery) -> CoreResult<DocsResult> {
        Err(MuninnCoreError::internal(
            "search_docs is not yet wired to RecursiveEngine — see PROJEC-T-0065 status notes",
        ))
    }

    async fn query_graph(&self, _query: GraphQuery) -> CoreResult<GraphResult> {
        Err(MuninnCoreError::internal(
            "query_graph is not yet wired to RecursiveEngine — see PROJEC-T-0065 status notes",
        ))
    }
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
