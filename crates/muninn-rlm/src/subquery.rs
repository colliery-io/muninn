//! Sub-query spawning and context isolation.
//!
//! This module provides the ability to spawn isolated sub-queries during
//! recursive exploration. Sub-queries have their own context, budget, and
//! can be used to decompose complex questions.

use std::sync::Arc;

use crate::backend::LLMBackend;
use crate::engine::{EngineConfig, EngineDeps, RecursiveEngine};
use crate::error::Result;
use crate::tools::ToolEnvironment;
use crate::types::{BudgetConfig, CompletionRequest, Message, MuninnConfig, ToolDefinition};

/// Configuration for spawning a sub-query.
#[derive(Debug, Clone)]
pub struct SubQuery {
    /// The question for the sub-query to answer.
    pub question: String,

    /// Optional system prompt for the sub-query.
    pub system: Option<String>,

    /// Tools available to the sub-query (empty = all tools).
    pub allowed_tools: Vec<String>,

    /// Budget allocation for this sub-query.
    pub budget: BudgetConfig,

    /// Whether to summarize results before returning.
    pub summarize: bool,

    /// Model to use (if different from parent).
    pub model: Option<String>,
}

impl SubQuery {
    /// Create a new sub-query with the given question.
    pub fn new(question: impl Into<String>) -> Self {
        Self {
            question: question.into(),
            system: None,
            allowed_tools: Vec::new(),
            budget: Self::default_sub_budget(),
            summarize: false,
            model: None,
        }
    }

    /// Default budget for sub-queries (more restrictive than parent).
    pub fn default_sub_budget() -> BudgetConfig {
        BudgetConfig {
            max_tokens: Some(20_000),
            max_duration_secs: Some(60),
            max_depth: Some(3),
            max_tool_calls: Some(10),
        }
    }

    /// Set the system prompt.
    pub fn with_system(mut self, system: impl Into<String>) -> Self {
        self.system = Some(system.into());
        self
    }

    /// Restrict to specific tools.
    pub fn with_allowed_tools(mut self, tools: Vec<String>) -> Self {
        self.allowed_tools = tools;
        self
    }

    /// Set the budget.
    pub fn with_budget(mut self, budget: BudgetConfig) -> Self {
        self.budget = budget;
        self
    }

    /// Enable result summarization.
    pub fn with_summarization(mut self) -> Self {
        self.summarize = true;
        self
    }

    /// Set a specific model.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }
}

/// Result from a sub-query execution.
#[derive(Debug, Clone)]
pub struct SubQueryResult {
    /// The answer from the sub-query.
    pub answer: String,

    /// Tokens used by the sub-query.
    pub tokens_used: u64,

    /// Tool calls made by the sub-query.
    pub tool_calls: u32,

    /// Maximum depth reached.
    pub depth_reached: u32,
}

/// Sub-query executor that manages isolated exploration sessions.
pub struct SubQueryExecutor {
    backend: Arc<dyn LLMBackend>,
    tools: Arc<dyn ToolEnvironment>,
    parent_model: String,
}

impl SubQueryExecutor {
    /// Create a new sub-query executor.
    pub fn new(
        backend: Arc<dyn LLMBackend>,
        tools: Arc<dyn ToolEnvironment>,
        parent_model: String,
    ) -> Self {
        Self {
            backend,
            tools,
            parent_model,
        }
    }

    /// Execute a sub-query with isolated context.
    pub async fn execute(&self, subquery: SubQuery) -> Result<SubQueryResult> {
        // Filter tools if specified
        let tools: Arc<dyn ToolEnvironment> = if subquery.allowed_tools.is_empty() {
            self.tools.clone()
        } else {
            Arc::new(FilteredToolEnvironment::new(
                self.tools.clone(),
                subquery.allowed_tools,
            ))
        };

        // Create isolated engine for this sub-query
        let deps = EngineDeps::new(self.backend.clone(), tools);
        let engine_config = EngineConfig::default().with_budget(subquery.budget.clone());
        let engine = RecursiveEngine::new(deps, engine_config);

        // Build the request
        let model = subquery.model.unwrap_or_else(|| self.parent_model.clone());
        let mut request =
            CompletionRequest::new(model, vec![Message::user(&subquery.question)], 4096)
                .with_muninn(MuninnConfig::recursive().with_budget(subquery.budget));

        if let Some(system) = subquery.system {
            request = request.with_system(system);
        }

        // Execute the sub-query
        let response = engine.complete(request).await?;

        // Extract the answer
        let answer = if subquery.summarize {
            // For now, just return the text. A real implementation would
            // make another call to summarize if the response is long.
            response.text()
        } else {
            response.text()
        };

        // Build result with metadata
        let metadata = response.muninn.unwrap_or_default();
        Ok(SubQueryResult {
            answer,
            tokens_used: metadata.tokens_used,
            tool_calls: metadata.tool_calls,
            depth_reached: metadata.depth_reached,
        })
    }
}

/// A tool environment that filters available tools.
struct FilteredToolEnvironment {
    inner: Arc<dyn ToolEnvironment>,
    allowed: Vec<String>,
}

impl FilteredToolEnvironment {
    fn new(inner: Arc<dyn ToolEnvironment>, allowed: Vec<String>) -> Self {
        Self { inner, allowed }
    }
}

#[async_trait::async_trait]
impl ToolEnvironment for FilteredToolEnvironment {
    async fn execute_tool(
        &self,
        tool_use: &crate::types::ToolUseBlock,
    ) -> Result<crate::types::ToolResultBlock> {
        if self.allowed.contains(&tool_use.name) {
            self.inner.execute_tool(tool_use).await
        } else {
            Ok(crate::types::ToolResultBlock::error(
                &tool_use.id,
                format!(
                    "Tool '{}' is not available in this sub-query",
                    tool_use.name
                ),
            ))
        }
    }

    fn available_tools(&self) -> Vec<ToolDefinition> {
        self.inner
            .available_tools()
            .into_iter()
            .filter(|t| self.allowed.contains(&t.name))
            .collect()
    }
}

/// Helper function to create a spawn_subquery tool definition.
pub fn spawn_subquery_tool() -> ToolDefinition {
    ToolDefinition::new(
        "spawn_subquery",
        "Spawn a sub-query to investigate a specific aspect in isolation. \
         Use this when you need to deeply explore a sub-topic without cluttering \
         the main conversation context.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "question": {
                    "type": "string",
                    "description": "The question for the sub-query to answer"
                },
                "allowed_tools": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Tools available to the sub-query (empty = all tools)"
                },
                "summarize": {
                    "type": "boolean",
                    "description": "Whether to summarize results before returning"
                },
                "max_depth": {
                    "type": "integer",
                    "description": "Maximum recursion depth for the sub-query"
                }
            },
            "required": ["question"]
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::MockBackend;
    use crate::tools::MockToolEnvironment;
    use crate::types::{ContentBlock, StopReason, Usage};

    #[tokio::test]
    async fn test_subquery_simple() {
        let responses = vec![crate::types::CompletionResponse::new(
            "sub_1",
            "model",
            vec![ContentBlock::Text {
                text: "Sub-query answer".to_string(),
                cache_control: None,
            }],
            StopReason::EndTurn,
            Usage::new(50, 30),
        )];

        let backend = Arc::new(MockBackend::new(responses));
        let tools = Arc::new(MockToolEnvironment::default());
        let executor = SubQueryExecutor::new(backend, tools, "test-model".to_string());

        let subquery = SubQuery::new("What is the answer?");
        let result = executor.execute(subquery).await.unwrap();

        assert_eq!(result.answer, "Sub-query answer");
        assert_eq!(result.tokens_used, 80); // 50 + 30
    }

    #[tokio::test]
    async fn test_subquery_with_filtered_tools() {
        let responses = vec![crate::types::CompletionResponse::new(
            "sub_1",
            "model",
            vec![ContentBlock::Text {
                text: "Done".to_string(),
                cache_control: None,
            }],
            StopReason::EndTurn,
            Usage::new(10, 10),
        )];

        let backend = Arc::new(MockBackend::new(responses));
        let tools = Arc::new(MockToolEnvironment::new(vec![
            ToolDefinition::new("tool_a", "A", serde_json::json!({})),
            ToolDefinition::new("tool_b", "B", serde_json::json!({})),
            ToolDefinition::new("tool_c", "C", serde_json::json!({})),
        ]));
        let executor = SubQueryExecutor::new(backend, tools, "test-model".to_string());

        let subquery = SubQuery::new("Question")
            .with_allowed_tools(vec!["tool_a".to_string(), "tool_c".to_string()]);

        let result = executor.execute(subquery).await.unwrap();
        assert_eq!(result.answer, "Done");
    }

    #[tokio::test]
    async fn test_subquery_with_custom_model() {
        let responses = vec![crate::types::CompletionResponse::new(
            "sub_1",
            "custom-model",
            vec![ContentBlock::Text {
                text: "Answer".to_string(),
                cache_control: None,
            }],
            StopReason::EndTurn,
            Usage::new(10, 10),
        )];

        let backend = Arc::new(MockBackend::new(responses));
        let tools = Arc::new(MockToolEnvironment::default());
        let executor = SubQueryExecutor::new(backend, tools, "default-model".to_string());

        let subquery = SubQuery::new("Question").with_model("custom-model");
        let result = executor.execute(subquery).await.unwrap();

        assert_eq!(result.answer, "Answer");
    }

    #[test]
    fn test_subquery_builder() {
        let subquery = SubQuery::new("Question")
            .with_system("Be concise")
            .with_allowed_tools(vec!["read_file".to_string()])
            .with_summarization()
            .with_budget(BudgetConfig {
                max_tokens: Some(5000),
                ..Default::default()
            });

        assert_eq!(subquery.question, "Question");
        assert_eq!(subquery.system, Some("Be concise".to_string()));
        assert_eq!(subquery.allowed_tools, vec!["read_file".to_string()]);
        assert!(subquery.summarize);
        assert_eq!(subquery.budget.max_tokens, Some(5000));
    }

    #[test]
    fn test_default_sub_budget() {
        let budget = SubQuery::default_sub_budget();
        assert_eq!(budget.max_tokens, Some(20_000));
        assert_eq!(budget.max_duration_secs, Some(60));
        assert_eq!(budget.max_depth, Some(3));
        assert_eq!(budget.max_tool_calls, Some(10));
    }

    #[test]
    fn test_spawn_subquery_tool() {
        let tool = spawn_subquery_tool();
        assert_eq!(tool.name, "spawn_subquery");
        assert!(tool.description.contains("sub-query"));
    }

    #[tokio::test]
    async fn test_filtered_tool_environment() {
        let inner = Arc::new(MockToolEnvironment::new(vec![
            ToolDefinition::new("allowed", "Allowed", serde_json::json!({})),
            ToolDefinition::new("blocked", "Blocked", serde_json::json!({})),
        ]));

        let filtered = FilteredToolEnvironment::new(inner, vec!["allowed".to_string()]);

        // Check available tools
        let tools = filtered.available_tools();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "allowed");

        // Try to execute blocked tool
        let tool_use = crate::types::ToolUseBlock {
            id: "t1".to_string(),
            name: "blocked".to_string(),
            input: serde_json::json!({}),
        };
        let result = filtered.execute_tool(&tool_use).await.unwrap();
        assert!(result.is_error);
        match result.content.unwrap() {
            crate::types::ToolResultContent::Text(text) => {
                assert!(text.contains("not available"));
            }
            _ => panic!("Expected text content"),
        }
    }
}
