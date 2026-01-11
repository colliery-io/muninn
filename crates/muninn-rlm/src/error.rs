//! Error types for the RLM gateway.

use thiserror::Error;

/// Result type alias for RLM operations.
pub type Result<T> = std::result::Result<T, RlmError>;

/// Errors that can occur in the RLM gateway.
#[derive(Debug, Error)]
pub enum RlmError {
    /// Error from the LLM backend.
    #[error("Backend error: {0}")]
    Backend(String),

    /// Error during tool execution.
    #[error("Tool execution error: {0}")]
    ToolExecution(String),

    /// Budget exceeded during exploration.
    #[error("Budget exceeded: {0}")]
    BudgetExceeded(BudgetExceededError),

    /// Invalid request.
    #[error("Invalid request: {0}")]
    InvalidRequest(String),

    /// HTTP/network error.
    #[error("Network error: {0}")]
    Network(String),

    /// Serialization/deserialization error.
    #[error("Serialization error: {0}")]
    Serialization(String),

    /// Configuration error.
    #[error("Configuration error: {0}")]
    Config(String),

    /// Internal error.
    #[error("Internal error: {0}")]
    Internal(String),

    /// Protocol error (MCP, etc.).
    #[error("Protocol error: {0}")]
    Protocol(String),
}

/// Details about which budget was exceeded.
#[derive(Debug, Clone)]
pub struct BudgetExceededError {
    /// The type of budget that was exceeded.
    pub budget_type: BudgetType,
    /// The limit that was set.
    pub limit: u64,
    /// The actual value that exceeded the limit.
    pub actual: u64,
}

impl std::fmt::Display for BudgetExceededError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:?} budget exceeded: {} > {}",
            self.budget_type, self.actual, self.limit
        )
    }
}

/// Types of budgets that can be exceeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetType {
    /// Total tokens across all calls.
    Tokens,
    /// Wall-clock time.
    Duration,
    /// Recursion depth.
    Depth,
    /// Number of tool calls.
    ToolCalls,
}

impl From<reqwest::Error> for RlmError {
    fn from(e: reqwest::Error) -> Self {
        RlmError::Network(e.to_string())
    }
}

impl From<serde_json::Error> for RlmError {
    fn from(e: serde_json::Error) -> Self {
        RlmError::Serialization(e.to_string())
    }
}

impl From<std::io::Error> for RlmError {
    fn from(e: std::io::Error) -> Self {
        RlmError::Internal(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = RlmError::Backend("connection failed".to_string());
        assert_eq!(err.to_string(), "Backend error: connection failed");

        let budget_err = RlmError::BudgetExceeded(BudgetExceededError {
            budget_type: BudgetType::Tokens,
            limit: 100_000,
            actual: 150_000,
        });
        assert!(budget_err.to_string().contains("Tokens"));
        assert!(budget_err.to_string().contains("150000"));
    }

    #[test]
    fn test_budget_exceeded_display() {
        let err = BudgetExceededError {
            budget_type: BudgetType::Depth,
            limit: 10,
            actual: 15,
        };
        assert_eq!(err.to_string(), "Depth budget exceeded: 15 > 10");
    }
}
