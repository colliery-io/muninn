//! Engine error type.
//!
//! `MuninnCoreError` is intentionally a coarse, adapter-friendly enum.
//! Adapters map these variants to their wire format (HTTP status, MCP
//! tool error, hook passthrough); the engine's internal failure detail
//! lives in the optional `detail` string rather than a deep variant
//! hierarchy.

use thiserror::Error;

/// Result alias used across the trait surface.
pub type Result<T> = std::result::Result<T, MuninnCoreError>;

/// Error type returned by every [`crate::MuninnEngine`] method.
#[derive(Debug, Error)]
pub enum MuninnCoreError {
    /// A required input was missing or malformed.
    #[error("invalid request: {0}")]
    InvalidRequest(String),

    /// The requested resource (file, symbol, library, memory id) was not
    /// found.
    #[error("not found: {0}")]
    NotFound(String),

    /// A configured limit (depth, tokens, tool calls, wall clock, …) was
    /// exceeded during processing.
    #[error("budget exceeded: {0}")]
    BudgetExceeded(String),

    /// The configured backend (LLM, embedding model, …) returned an
    /// error or was unreachable.
    #[error("backend error: {0}")]
    Backend(String),

    /// I/O failure reading from the working tree, the graph store, the
    /// doc store, or the memory store.
    #[error("storage error: {0}")]
    Storage(String),

    /// Catch-all for engine-internal failures that don't fit a more
    /// specific variant. Adapters typically treat this as a 5xx-equivalent.
    #[error("internal error: {0}")]
    Internal(String),
}

impl MuninnCoreError {
    /// Convenience for adapters that want a single helper to wrap an
    /// arbitrary error string as an internal failure.
    pub fn internal(msg: impl Into<String>) -> Self {
        MuninnCoreError::Internal(msg.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_includes_variant_payload() {
        let e = MuninnCoreError::NotFound("symbol foo".into());
        assert_eq!(e.to_string(), "not found: symbol foo");
    }

    #[test]
    fn internal_helper_constructs_internal_variant() {
        let e = MuninnCoreError::internal("kaboom");
        assert!(matches!(e, MuninnCoreError::Internal(_)));
    }
}
