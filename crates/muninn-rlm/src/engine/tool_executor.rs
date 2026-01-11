//! Tool call execution and result handling.
//!
//! This module provides the `ToolExecutor` for executing tool calls
//! requested by the LLM during exploration.

use std::sync::Arc;
use std::time::Instant;

use crate::error::Result;
use crate::tools::ToolEnvironment;
use crate::types::{CompletionResponse, ToolResultBlock, ToolResultContent};

use super::trace::ToolExecutionTraceData;

/// Executes tool calls and collects results.
///
/// The executor handles tool execution errors gracefully by returning
/// error results to the LLM rather than aborting exploration.
#[derive(Clone)]
pub struct ToolExecutor {
    tools: Arc<dyn ToolEnvironment>,
}

impl ToolExecutor {
    /// Create a new tool executor with the given tool environment.
    pub fn new(tools: Arc<dyn ToolEnvironment>) -> Self {
        Self { tools }
    }

    /// Execute all tool use requests from a response.
    ///
    /// Tool errors are returned as error results to the LLM rather than
    /// aborting the exploration - this allows the model to learn and adapt.
    pub async fn execute_tools(
        &self,
        response: &CompletionResponse,
    ) -> Result<Vec<ToolResultBlock>> {
        let tool_uses = response.tool_uses();
        let mut results = Vec::with_capacity(tool_uses.len());

        for tool_use in tool_uses {
            let tool_start = Instant::now();
            let (result, success, output_preview) = match self.tools.execute_tool(&tool_use).await {
                Ok(result) => {
                    let preview = Self::extract_result_preview(&result.content, 500);
                    (result, true, preview)
                }
                Err(e) => {
                    // Return error as tool result so LLM can learn and adapt
                    let error_result = ToolResultBlock::error(&tool_use.id, e.to_string());
                    let preview = Self::truncate_string(&e.to_string(), 500);
                    (error_result, false, preview)
                }
            };
            let execution_time_ms = tool_start.elapsed().as_millis() as u64;

            // Trace the tool execution
            let tool_data = ToolExecutionTraceData {
                tool_name: tool_use.name.clone(),
                tool_id: tool_use.id.clone(),
                input: tool_use.input.clone(),
                success,
                output_preview,
                execution_time_ms,
            };
            muninn_tracing::start_span_with_data("tool_execution", &tool_data);
            muninn_tracing::end_span_ok();

            results.push(result);
        }

        Ok(results)
    }

    /// Extract a preview from tool result content.
    fn extract_result_preview(content: &Option<ToolResultContent>, max_len: usize) -> String {
        match content {
            Some(ToolResultContent::Text(text)) => Self::truncate_string(text, max_len),
            Some(ToolResultContent::Blocks(blocks)) => {
                let json = serde_json::to_string(blocks).unwrap_or_default();
                Self::truncate_string(&json, max_len)
            }
            None => "[no content]".to_string(),
        }
    }

    /// Truncate a string for trace preview.
    fn truncate_string(content: &str, max_len: usize) -> String {
        if content.len() <= max_len {
            content.to_string()
        } else {
            format!(
                "{}... [truncated, {} total chars]",
                &content[..max_len],
                content.len()
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::MockToolEnvironment;
    use crate::types::{ContentBlock, StopReason, ToolDefinition, Usage};
    use serde_json::json;

    fn create_tool_response(tool_name: &str, tool_id: &str) -> CompletionResponse {
        CompletionResponse::new(
            "msg_1",
            "model",
            vec![ContentBlock::ToolUse {
                id: tool_id.to_string(),
                name: tool_name.to_string(),
                input: json!({"arg": "value"}),
                cache_control: None,
            }],
            StopReason::ToolUse,
            Usage::new(10, 10),
        )
    }

    #[tokio::test]
    async fn test_execute_single_tool() {
        let tools = Arc::new(MockToolEnvironment::new(vec![ToolDefinition::new(
            "test_tool",
            "A test tool",
            json!({}),
        )]));
        tools.set_response("test_tool", "tool result");

        let executor = ToolExecutor::new(tools.clone());
        let response = create_tool_response("test_tool", "t1");

        let results = executor.execute_tools(&response).await.unwrap();
        assert_eq!(results.len(), 1);
        assert!(!results[0].is_error);
        assert_eq!(tools.execution_count(), 1);
    }

    #[tokio::test]
    async fn test_execute_multiple_tools() {
        let tools = Arc::new(MockToolEnvironment::new(vec![
            ToolDefinition::new("tool_a", "A", json!({})),
            ToolDefinition::new("tool_b", "B", json!({})),
        ]));

        let executor = ToolExecutor::new(tools.clone());
        let response = CompletionResponse::new(
            "msg_1",
            "model",
            vec![
                ContentBlock::ToolUse {
                    id: "t1".to_string(),
                    name: "tool_a".to_string(),
                    input: json!({}),
                    cache_control: None,
                },
                ContentBlock::ToolUse {
                    id: "t2".to_string(),
                    name: "tool_b".to_string(),
                    input: json!({}),
                    cache_control: None,
                },
            ],
            StopReason::ToolUse,
            Usage::new(10, 10),
        );

        let results = executor.execute_tools(&response).await.unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(tools.execution_count(), 2);
    }

    #[test]
    fn test_truncate_string_short() {
        let result = ToolExecutor::truncate_string("short", 100);
        assert_eq!(result, "short");
    }

    #[test]
    fn test_truncate_string_long() {
        let long = "a".repeat(200);
        let result = ToolExecutor::truncate_string(&long, 50);
        assert!(result.contains("truncated"));
        assert!(result.contains("200 total chars"));
    }

    #[test]
    fn test_extract_result_preview_text() {
        let content = Some(ToolResultContent::Text("Hello world".to_string()));
        let preview = ToolExecutor::extract_result_preview(&content, 100);
        assert_eq!(preview, "Hello world");
    }

    #[test]
    fn test_extract_result_preview_none() {
        let preview = ToolExecutor::extract_result_preview(&None, 100);
        assert_eq!(preview, "[no content]");
    }

    #[test]
    fn test_extract_result_preview_blocks() {
        let content = Some(ToolResultContent::Blocks(vec![
            json!({"type": "text", "text": "Block content"}),
        ]));
        let preview = ToolExecutor::extract_result_preview(&content, 1000);
        assert!(preview.contains("Block content"));
    }
}
