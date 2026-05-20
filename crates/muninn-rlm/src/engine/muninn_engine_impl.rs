//! `MuninnEngine` trait implementation for [`RecursiveEngine`].
//!
//! This wires the adapter-neutral trait from `muninn-core` to muninn-rlm's
//! existing recursive exploration engine. The rich `complete()` method is
//! the load-bearing one — it's what the proxy adapter calls — and it
//! delegates directly to [`RecursiveEngine::complete`].
//!
//! Wiring status (as of the memory-store wiring commit):
//! - `complete` — fully wired; delegates to [`RecursiveEngine::complete`].
//! - `recall_memory` / `record_memory` — wired against the optional
//!   `EngineDeps::memory_store` (defaults to an [`InMemoryStore`] when
//!   constructed via [`crate::engine::default_engine`]).
//! - `search_code`, `explore`, `search_docs`, `query_graph` — still
//!   stubbed; require direct handles to the file system, recursive
//!   engine entry point, doc store, and graph store respectively.
//!   Tracked as follow-ups (see PROJEC-T-0065 status notes).

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

    async fn recall_memory(&self, query: MemoryQuery) -> CoreResult<Vec<MemoryHit>> {
        let Some(store) = self.memory_store.as_ref() else {
            return Err(MuninnCoreError::internal(
                "recall_memory: no memory store attached to RecursiveEngine \
                 (construct via default_engine_with_memory or attach via \
                 EngineDeps::with_memory_store)",
            ));
        };
        let limit = query.limit.unwrap_or(8) as usize;
        let entries = store
            .search(&query.query, limit)
            .map_err(|e| MuninnCoreError::Storage(format!("memory search: {e}")))?;
        Ok(entries
            .into_iter()
            .map(|e| MemoryHit {
                id: e.id,
                content: e.content,
                // MemoryEntry.relevance is already clamped to [0,1].
                score: e.relevance,
            })
            .collect())
    }

    async fn record_memory(&self, item: MemoryItem) -> CoreResult<()> {
        let Some(store) = self.memory_store.as_ref() else {
            return Err(MuninnCoreError::internal(
                "record_memory: no memory store attached to RecursiveEngine \
                 (construct via default_engine_with_memory or attach via \
                 EngineDeps::with_memory_store)",
            ));
        };
        // Synthesize an id from a SHA-256 of the content + source so
        // repeated record calls with the same content de-dup cleanly.
        // (The store's `store(...)` overwrites on id collision.)
        let id = {
            use sha2::{Digest, Sha256};
            let mut h = Sha256::new();
            h.update(item.content.as_bytes());
            if let Some(src) = item.source.as_deref() {
                h.update(b"\0");
                h.update(src.as_bytes());
            }
            let bytes = h.finalize();
            let hex: String = bytes.iter().take(12).map(|b| format!("{b:02x}")).collect();
            format!("mem_{hex}")
        };
        let category = item.source.clone().unwrap_or_else(|| "default".to_string());
        let entry = crate::memory_tools::MemoryEntry::new(id, item.content, category);
        store
            .store(entry)
            .map_err(|e| MuninnCoreError::Storage(format!("memory store: {e}")))?;
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::MockBackend;
    use crate::engine::{EngineConfig, EngineDeps};
    use crate::memory_tools::InMemoryStore;
    use crate::tools::EmptyToolEnvironment;
    use muninn_core::types::{MemoryItem, MemoryQuery};
    use std::sync::Arc;

    fn engine_with_memory() -> RecursiveEngine {
        let backend = Arc::new(MockBackend::new(vec![]));
        let tools = Arc::new(EmptyToolEnvironment);
        let store = Arc::new(InMemoryStore::new());
        let deps = EngineDeps::new(backend, tools).with_memory_store(store);
        RecursiveEngine::new(deps, EngineConfig::default())
    }

    fn engine_without_memory() -> RecursiveEngine {
        let backend = Arc::new(MockBackend::new(vec![]));
        let tools = Arc::new(EmptyToolEnvironment);
        let deps = EngineDeps::new(backend, tools);
        RecursiveEngine::new(deps, EngineConfig::default())
    }

    #[tokio::test]
    async fn recall_memory_returns_empty_for_fresh_store() {
        let engine = engine_with_memory();
        let hits = engine
            .recall_memory(MemoryQuery {
                query: "anything".into(),
                limit: Some(5),
            })
            .await
            .expect("recall should succeed against attached store");
        assert!(hits.is_empty());
    }

    #[tokio::test]
    async fn record_then_recall_round_trips_content() {
        let engine = engine_with_memory();
        engine
            .record_memory(MemoryItem {
                content: "the daemon uses setsid to detach the child".into(),
                source: Some("ADR-0003".into()),
            })
            .await
            .expect("record");
        let hits = engine
            .recall_memory(MemoryQuery {
                query: "setsid".into(),
                limit: Some(5),
            })
            .await
            .expect("recall");
        assert_eq!(hits.len(), 1, "expected 1 hit for 'setsid', got {hits:?}");
        assert!(hits[0].content.contains("setsid"));
        assert!(!hits[0].id.is_empty());
    }

    #[tokio::test]
    async fn record_is_idempotent_on_same_content_and_source() {
        let engine = engine_with_memory();
        for _ in 0..3 {
            engine
                .record_memory(MemoryItem {
                    content: "same thing".into(),
                    source: Some("test".into()),
                })
                .await
                .expect("record");
        }
        let hits = engine
            .recall_memory(MemoryQuery {
                query: "same".into(),
                limit: Some(10),
            })
            .await
            .expect("recall");
        // SHA-256-of-content+source as id => same id => store::store
        // overwrites => one entry survives.
        assert_eq!(hits.len(), 1);
    }

    #[tokio::test]
    async fn engine_without_store_returns_internal_error() {
        let engine = engine_without_memory();
        let err = engine
            .recall_memory(MemoryQuery {
                query: "x".into(),
                limit: None,
            })
            .await
            .unwrap_err();
        assert!(matches!(err, MuninnCoreError::Internal(_)));
        let err = engine
            .record_memory(MemoryItem {
                content: "x".into(),
                source: None,
            })
            .await
            .unwrap_err();
        assert!(matches!(err, MuninnCoreError::Internal(_)));
    }
}
