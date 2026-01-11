//! Trace data structures for engine operations.
//!
//! This module contains serializable trace data captured during RLM exploration.
//! These structures are used for observability, debugging, and performance analysis.

use serde::Serialize;

/// Trace data captured at the start of an RLM exploration cycle.
#[derive(Debug, Clone, Serialize)]
pub struct RlmCycleTraceData {
    /// Model used for exploration.
    pub model: String,
    /// Whether the request was marked as recursive.
    pub is_recursive: bool,
    /// Number of messages in the original request.
    pub initial_message_count: usize,
    /// System prompt (if any).
    pub system_prompt: Option<String>,
}

/// Trace data for a single LLM iteration within exploration.
#[derive(Debug, Clone, Serialize)]
pub struct RlmIterationTraceData {
    /// Current depth in the exploration.
    pub depth: u32,
    /// Whether this is the last turn before depth limit.
    pub is_last_turn: bool,
    /// Number of messages sent to LLM.
    pub message_count: usize,
    /// Time to get LLM response (ms).
    pub llm_latency_ms: u64,
    /// Input tokens used.
    pub input_tokens: u32,
    /// Output tokens used.
    pub output_tokens: u32,
    /// Stop reason from LLM.
    pub stop_reason: Option<String>,
}

/// Trace data for tool execution.
#[derive(Debug, Clone, Serialize)]
pub struct ToolExecutionTraceData {
    /// Tool name.
    pub tool_name: String,
    /// Tool call ID.
    pub tool_id: String,
    /// Tool input (JSON).
    pub input: serde_json::Value,
    /// Whether the execution succeeded.
    pub success: bool,
    /// Output (truncated if large).
    pub output_preview: String,
    /// Execution time (ms).
    pub execution_time_ms: u64,
}

/// Trace data for exploration completion.
#[derive(Debug, Clone, Serialize)]
pub struct RlmCompletionTraceData {
    /// How the exploration terminated.
    pub termination_reason: String,
    /// Final depth reached.
    pub depth_reached: u32,
    /// Total tool calls made.
    pub tool_calls: u32,
    /// Total tokens used.
    pub tokens_used: u64,
    /// Total duration (ms).
    pub duration_ms: u64,
    /// Whether a final answer was extracted.
    pub has_final_answer: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cycle_trace_serialization() {
        let data = RlmCycleTraceData {
            model: "test-model".to_string(),
            is_recursive: true,
            initial_message_count: 3,
            system_prompt: Some("Be helpful".to_string()),
        };

        let json = serde_json::to_string(&data).unwrap();
        assert!(json.contains("test-model"));
        assert!(json.contains("true"));
    }

    #[test]
    fn test_iteration_trace_serialization() {
        let data = RlmIterationTraceData {
            depth: 2,
            is_last_turn: false,
            message_count: 5,
            llm_latency_ms: 1500,
            input_tokens: 100,
            output_tokens: 50,
            stop_reason: Some("end_turn".to_string()),
        };

        let json = serde_json::to_string(&data).unwrap();
        assert!(json.contains("1500"));
    }

    #[test]
    fn test_tool_execution_trace_serialization() {
        let data = ToolExecutionTraceData {
            tool_name: "read_file".to_string(),
            tool_id: "tool_123".to_string(),
            input: serde_json::json!({"path": "/test.rs"}),
            success: true,
            output_preview: "file contents...".to_string(),
            execution_time_ms: 50,
        };

        let json = serde_json::to_string(&data).unwrap();
        assert!(json.contains("read_file"));
    }

    #[test]
    fn test_completion_trace_serialization() {
        let data = RlmCompletionTraceData {
            termination_reason: "end_turn".to_string(),
            depth_reached: 3,
            tool_calls: 5,
            tokens_used: 10000,
            duration_ms: 5000,
            has_final_answer: true,
        };

        let json = serde_json::to_string(&data).unwrap();
        assert!(json.contains("end_turn"));
        assert!(json.contains("10000"));
    }
}
