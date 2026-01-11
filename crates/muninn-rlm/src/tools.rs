//! Tool environment trait and types.
//!
//! This module defines the abstraction for tool execution, which is used by
//! the recursive exploration engine to interact with the outside world.
//!
//! # Architecture
//!
//! - `Tool`: Individual tool implementation (read_file, search_code, etc.)
//! - `ToolRegistry`: Collection of tools, implements `ToolEnvironment`
//! - `ToolEnvironment`: Abstraction for executing tools (used by RLM engine)
//! - `ToolResult`: Structured result from tool execution with metadata

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

use crate::error::Result;
use crate::types::{ToolDefinition, ToolResultBlock, ToolUseBlock};

// ============================================================================
// Tool Trait and Result Types
// ============================================================================

/// A tool that can be executed by the RLM engine.
///
/// Tools are the building blocks of the tool environment. Each tool
/// provides a specific capability (reading files, searching code, etc.)
/// and can be registered with a `ToolRegistry`.
#[async_trait]
pub trait Tool: Send + Sync {
    /// Unique name for this tool.
    fn name(&self) -> &str;

    /// Human-readable description for the LLM.
    fn description(&self) -> &str;

    /// JSON Schema for the tool's input parameters.
    fn parameters_schema(&self) -> serde_json::Value;

    /// Execute the tool with the given parameters.
    async fn execute(&self, params: serde_json::Value) -> Result<ToolResult>;

    /// Whether this tool is internal-only (not exposed via MCP).
    ///
    /// Internal tools are used by the RLM engine for exploration but are
    /// not exposed to external agents (like Claude Code) via MCP. This
    /// prevents collisions with tools the agent already has (e.g., read_file).
    ///
    /// Default: false (tools are exposed by default)
    fn is_internal(&self) -> bool {
        false
    }

    /// Convert this tool to an Anthropic-compatible tool definition.
    fn to_definition(&self) -> ToolDefinition {
        ToolDefinition::new(self.name(), self.description(), self.parameters_schema())
    }
}

/// Result from executing a tool.
#[derive(Debug, Clone)]
pub struct ToolResult {
    /// The content returned by the tool.
    pub content: ToolContent,
    /// Metadata for context aggregation.
    pub metadata: ToolMetadata,
}

impl ToolResult {
    /// Create a successful text result.
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            content: ToolContent::Text(content.into()),
            metadata: ToolMetadata::default(),
        }
    }

    /// Create a successful JSON result.
    pub fn json(value: serde_json::Value) -> Self {
        Self {
            content: ToolContent::Json(value),
            metadata: ToolMetadata::default(),
        }
    }

    /// Create a file content result.
    pub fn file(
        path: impl Into<String>,
        content: impl Into<String>,
        language: Option<String>,
    ) -> Self {
        Self {
            content: ToolContent::FileContent {
                path: path.into(),
                content: content.into(),
                language,
            },
            metadata: ToolMetadata::default(),
        }
    }

    /// Create an error result.
    pub fn error(message: impl Into<String>, recoverable: bool) -> Self {
        Self {
            content: ToolContent::Error {
                message: message.into(),
                recoverable,
            },
            metadata: ToolMetadata::default(),
        }
    }

    /// Add metadata to this result.
    pub fn with_metadata(mut self, metadata: ToolMetadata) -> Self {
        self.metadata = metadata;
        self
    }

    /// Check if this result is an error.
    pub fn is_error(&self) -> bool {
        matches!(self.content, ToolContent::Error { .. })
    }

    /// Convert to a string representation for the LLM.
    pub fn to_string_content(&self) -> String {
        match &self.content {
            ToolContent::Text(s) => s.clone(),
            ToolContent::Json(v) => serde_json::to_string_pretty(v).unwrap_or_default(),
            ToolContent::FileContent {
                path,
                content,
                language,
            } => {
                let lang = language.as_deref().unwrap_or("");
                format!("```{} ({})\n{}\n```", lang, path, content)
            }
            ToolContent::Error { message, .. } => format!("Error: {}", message),
        }
    }

    /// Convert to a ToolResultBlock for the API.
    pub fn to_result_block(&self, tool_use_id: &str) -> ToolResultBlock {
        match &self.content {
            ToolContent::Error { message, .. } => ToolResultBlock::error(tool_use_id, message),
            _ => ToolResultBlock::success(tool_use_id, self.to_string_content()),
        }
    }
}

/// Content types that tools can return.
#[derive(Debug, Clone)]
pub enum ToolContent {
    /// Plain text content.
    Text(String),
    /// Structured JSON content.
    Json(serde_json::Value),
    /// File contents with path and optional language hint.
    FileContent {
        path: String,
        content: String,
        language: Option<String>,
    },
    /// Error with message and recoverability hint.
    Error { message: String, recoverable: bool },
}

/// Metadata for tool results, used by the context aggregator.
#[derive(Debug, Clone, Default)]
pub struct ToolMetadata {
    /// Source identifier (file path, query, etc.).
    pub source: Option<String>,
    /// Relevance hint (0.0 to 1.0).
    pub relevance: Option<f32>,
    /// Approximate token count.
    pub token_estimate: Option<usize>,
    /// Tags for categorization.
    pub tags: Vec<String>,
}

impl ToolMetadata {
    /// Create metadata with a source identifier.
    pub fn with_source(source: impl Into<String>) -> Self {
        Self {
            source: Some(source.into()),
            ..Default::default()
        }
    }

    /// Set the relevance score.
    pub fn with_relevance(mut self, relevance: f32) -> Self {
        self.relevance = Some(relevance.clamp(0.0, 1.0));
        self
    }

    /// Set the token estimate.
    pub fn with_tokens(mut self, tokens: usize) -> Self {
        self.token_estimate = Some(tokens);
        self
    }

    /// Add a tag.
    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.tags.push(tag.into());
        self
    }
}

// ============================================================================
// Tool Registry
// ============================================================================

/// Registry of tools that implements `ToolEnvironment`.
///
/// The registry holds a collection of tools and routes execution requests
/// to the appropriate tool based on the tool name.
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    /// Register a tool with the registry.
    pub fn register(&mut self, tool: impl Tool + 'static) {
        let name = tool.name().to_string();
        self.tools.insert(name, Arc::new(tool));
    }

    /// Register a tool (Arc version).
    pub fn register_arc(&mut self, tool: Arc<dyn Tool>) {
        let name = tool.name().to_string();
        self.tools.insert(name, tool);
    }

    /// Get a tool by name.
    pub fn get(&self, name: &str) -> Option<&Arc<dyn Tool>> {
        self.tools.get(name)
    }

    /// Get the number of registered tools.
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    /// Check if the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// Get all tool names.
    pub fn tool_names(&self) -> Vec<&str> {
        self.tools.keys().map(|s| s.as_str()).collect()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ToolEnvironment for ToolRegistry {
    async fn execute_tool(&self, tool_use: &ToolUseBlock) -> Result<ToolResultBlock> {
        if let Some(tool) = self.tools.get(&tool_use.name) {
            let result = tool.execute(tool_use.input.clone()).await?;
            Ok(result.to_result_block(&tool_use.id))
        } else {
            Ok(ToolResultBlock::error(
                &tool_use.id,
                format!("Tool '{}' is not registered", tool_use.name),
            ))
        }
    }

    fn available_tools(&self) -> Vec<ToolDefinition> {
        self.tools.values().map(|t| t.to_definition()).collect()
    }

    fn available_tools_external(&self) -> Vec<ToolDefinition> {
        self.tools
            .values()
            .filter(|t| !t.is_internal())
            .map(|t| t.to_definition())
            .collect()
    }
}

impl std::fmt::Debug for ToolRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolRegistry")
            .field("tools", &self.tool_names())
            .finish()
    }
}

/// Trait for tool execution environments.
///
/// Implementations provide the actual tool execution logic (reading files,
/// querying the code graph, executing code, etc.).
#[async_trait]
pub trait ToolEnvironment: Send + Sync {
    /// Execute a tool and return the result.
    async fn execute_tool(&self, tool_use: &ToolUseBlock) -> Result<ToolResultBlock>;

    /// Get definitions of all available tools (including internal ones).
    ///
    /// This returns all tools, including those marked as internal. Use
    /// `available_tools_external()` to get only tools that should be
    /// exposed to external agents via MCP.
    fn available_tools(&self) -> Vec<ToolDefinition>;

    /// Get definitions of tools that should be exposed externally (via MCP).
    ///
    /// This filters out internal tools that would collide with tools
    /// that external agents (like Claude Code) already have.
    ///
    /// Default implementation returns all tools (same as `available_tools()`).
    /// Implementations with internal tools should override this.
    fn available_tools_external(&self) -> Vec<ToolDefinition> {
        self.available_tools()
    }

    /// Get a subset of tools by name.
    fn filter_tools(&self, names: &[String]) -> Vec<ToolDefinition> {
        self.available_tools()
            .into_iter()
            .filter(|t| names.contains(&t.name))
            .collect()
    }

    /// Check if a specific tool is available.
    fn has_tool(&self, name: &str) -> bool {
        self.available_tools().iter().any(|t| t.name == name)
    }
}

/// A tool environment that has no tools.
///
/// Useful for pass-through mode where we don't want to execute any tools locally.
#[derive(Debug, Default)]
pub struct EmptyToolEnvironment;

#[async_trait]
impl ToolEnvironment for EmptyToolEnvironment {
    async fn execute_tool(&self, tool_use: &ToolUseBlock) -> Result<ToolResultBlock> {
        Ok(ToolResultBlock::error(
            &tool_use.id,
            format!("Tool '{}' is not available", tool_use.name),
        ))
    }

    fn available_tools(&self) -> Vec<ToolDefinition> {
        Vec::new()
    }
}

/// A composite tool environment that combines multiple environments.
pub struct CompositeToolEnvironment {
    environments: Vec<Arc<dyn ToolEnvironment>>,
    tool_map: HashMap<String, usize>,
}

impl CompositeToolEnvironment {
    /// Create a new composite environment from multiple environments.
    pub fn new(environments: Vec<Arc<dyn ToolEnvironment>>) -> Self {
        let mut tool_map = HashMap::new();

        for (idx, env) in environments.iter().enumerate() {
            for tool in env.available_tools() {
                // First environment to define a tool wins
                tool_map.entry(tool.name).or_insert(idx);
            }
        }

        Self {
            environments,
            tool_map,
        }
    }
}

#[async_trait]
impl ToolEnvironment for CompositeToolEnvironment {
    async fn execute_tool(&self, tool_use: &ToolUseBlock) -> Result<ToolResultBlock> {
        if let Some(&idx) = self.tool_map.get(&tool_use.name) {
            self.environments[idx].execute_tool(tool_use).await
        } else {
            Ok(ToolResultBlock::error(
                &tool_use.id,
                format!("Tool '{}' is not available", tool_use.name),
            ))
        }
    }

    fn available_tools(&self) -> Vec<ToolDefinition> {
        self.environments
            .iter()
            .flat_map(|e| e.available_tools())
            .collect()
    }
}

/// A mock tool environment for testing.
#[derive(Debug, Default)]
pub struct MockToolEnvironment {
    tools: Vec<ToolDefinition>,
    responses: std::sync::Mutex<HashMap<String, String>>,
    execution_log: std::sync::Mutex<Vec<ToolUseBlock>>,
}

impl MockToolEnvironment {
    /// Create a new mock environment with the given tools.
    pub fn new(tools: Vec<ToolDefinition>) -> Self {
        Self {
            tools,
            responses: std::sync::Mutex::new(HashMap::new()),
            execution_log: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// Set the response for a specific tool.
    pub fn set_response(&self, tool_name: impl Into<String>, response: impl Into<String>) {
        self.responses
            .lock()
            .unwrap()
            .insert(tool_name.into(), response.into());
    }

    /// Get all tool executions that occurred.
    pub fn executions(&self) -> Vec<ToolUseBlock> {
        self.execution_log.lock().unwrap().clone()
    }

    /// Get the number of tool executions.
    pub fn execution_count(&self) -> usize {
        self.execution_log.lock().unwrap().len()
    }
}

#[async_trait]
impl ToolEnvironment for MockToolEnvironment {
    async fn execute_tool(&self, tool_use: &ToolUseBlock) -> Result<ToolResultBlock> {
        self.execution_log.lock().unwrap().push(tool_use.clone());

        let responses = self.responses.lock().unwrap();
        if let Some(response) = responses.get(&tool_use.name) {
            Ok(ToolResultBlock::success(&tool_use.id, response))
        } else {
            Ok(ToolResultBlock::success(
                &tool_use.id,
                format!("Mock result for {}", tool_use.name),
            ))
        }
    }

    fn available_tools(&self) -> Vec<ToolDefinition> {
        self.tools.clone()
    }
}

/// A tool environment that can be shared across threads.
pub type SharedToolEnvironment = Arc<dyn ToolEnvironment>;

/// Helper to create common tool definitions.
pub mod common_tools {
    use super::*;
    use serde_json::json;

    /// Create a read_file tool definition.
    pub fn read_file() -> ToolDefinition {
        ToolDefinition::new(
            "read_file",
            "Read the contents of a file at the specified path.",
            json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "The path to the file to read"
                    }
                },
                "required": ["path"]
            }),
        )
    }

    /// Create a list_files tool definition.
    pub fn list_files() -> ToolDefinition {
        ToolDefinition::new(
            "list_files",
            "List files in a directory.",
            json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "The directory path to list"
                    },
                    "pattern": {
                        "type": "string",
                        "description": "Optional glob pattern to filter files"
                    }
                },
                "required": ["path"]
            }),
        )
    }

    /// Create a search_code tool definition.
    pub fn search_code() -> ToolDefinition {
        ToolDefinition::new(
            "search_code",
            "Search for code patterns in the codebase.",
            json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "The search query (regex supported)"
                    },
                    "path": {
                        "type": "string",
                        "description": "Optional path to limit search scope"
                    }
                },
                "required": ["query"]
            }),
        )
    }

    /// Create a query_graph tool definition.
    pub fn query_graph() -> ToolDefinition {
        ToolDefinition::new(
            "query_graph",
            "Query the code graph for symbols, relationships, and structure.",
            json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Cypher query to execute on the code graph"
                    }
                },
                "required": ["query"]
            }),
        )
    }

    /// Create a spawn_subquery tool definition.
    pub fn spawn_subquery() -> ToolDefinition {
        ToolDefinition::new(
            "spawn_subquery",
            "Spawn a sub-query to investigate a specific aspect in isolation.",
            json!({
                "type": "object",
                "properties": {
                    "question": {
                        "type": "string",
                        "description": "The question for the sub-query to answer"
                    },
                    "allowed_tools": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Tools available to the sub-query"
                    },
                    "summarize": {
                        "type": "boolean",
                        "description": "Whether to summarize results before returning"
                    }
                },
                "required": ["question"]
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn test_tool() -> ToolDefinition {
        ToolDefinition::new(
            "test_tool",
            "A test tool",
            json!({"type": "object", "properties": {}}),
        )
    }

    // Test implementation of Tool trait
    struct EchoTool;

    #[async_trait]
    impl Tool for EchoTool {
        fn name(&self) -> &str {
            "echo"
        }

        fn description(&self) -> &str {
            "Echoes the input message"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            json!({
                "type": "object",
                "properties": {
                    "message": { "type": "string" }
                },
                "required": ["message"]
            })
        }

        async fn execute(&self, params: serde_json::Value) -> Result<ToolResult> {
            let message = params
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("no message");
            Ok(ToolResult::text(format!("Echo: {}", message)))
        }
    }

    #[test]
    fn test_tool_result_text() {
        let result = ToolResult::text("hello world");
        assert!(!result.is_error());
        assert_eq!(result.to_string_content(), "hello world");
    }

    #[test]
    fn test_tool_result_json() {
        let result = ToolResult::json(json!({"key": "value"}));
        assert!(!result.is_error());
        let content = result.to_string_content();
        assert!(content.contains("key"));
        assert!(content.contains("value"));
    }

    #[test]
    fn test_tool_result_file() {
        let result = ToolResult::file("src/main.rs", "fn main() {}", Some("rust".to_string()));
        assert!(!result.is_error());
        let content = result.to_string_content();
        assert!(content.contains("```rust"));
        assert!(content.contains("src/main.rs"));
        assert!(content.contains("fn main()"));
    }

    #[test]
    fn test_tool_result_error() {
        let result = ToolResult::error("something went wrong", true);
        assert!(result.is_error());
        assert!(result.to_string_content().contains("Error:"));
    }

    #[test]
    fn test_tool_result_to_block() {
        let success = ToolResult::text("ok");
        let block = success.to_result_block("tool_1");
        assert!(!block.is_error);
        assert_eq!(
            block.content,
            Some(crate::types::ToolResultContent::Text("ok".to_string()))
        );

        let error = ToolResult::error("failed", false);
        let block = error.to_result_block("tool_2");
        assert!(block.is_error);
    }

    #[test]
    fn test_tool_metadata() {
        let meta = ToolMetadata::with_source("test.rs")
            .with_relevance(0.8)
            .with_tokens(100)
            .with_tag("code");

        assert_eq!(meta.source, Some("test.rs".to_string()));
        assert_eq!(meta.relevance, Some(0.8));
        assert_eq!(meta.token_estimate, Some(100));
        assert_eq!(meta.tags, vec!["code"]);
    }

    #[test]
    fn test_tool_metadata_clamps_relevance() {
        let meta = ToolMetadata::default().with_relevance(1.5);
        assert_eq!(meta.relevance, Some(1.0));

        let meta = ToolMetadata::default().with_relevance(-0.5);
        assert_eq!(meta.relevance, Some(0.0));
    }

    #[test]
    fn test_tool_to_definition() {
        let tool = EchoTool;
        let def = tool.to_definition();
        assert_eq!(def.name, "echo");
        assert_eq!(def.description, "Echoes the input message");
    }

    #[test]
    fn test_tool_registry_new() {
        let registry = ToolRegistry::new();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn test_tool_registry_register() {
        let mut registry = ToolRegistry::new();
        registry.register(EchoTool);

        assert_eq!(registry.len(), 1);
        assert!(registry.get("echo").is_some());
        assert!(registry.get("other").is_none());
    }

    #[tokio::test]
    async fn test_tool_registry_execute() {
        let mut registry = ToolRegistry::new();
        registry.register(EchoTool);

        let tool_use = ToolUseBlock {
            id: "t1".to_string(),
            name: "echo".to_string(),
            input: json!({"message": "hello"}),
        };

        let result = registry.execute_tool(&tool_use).await.unwrap();
        assert!(!result.is_error);
        assert_eq!(
            result.content,
            Some(crate::types::ToolResultContent::Text(
                "Echo: hello".to_string()
            ))
        );
    }

    #[tokio::test]
    async fn test_tool_registry_execute_unknown() {
        let registry = ToolRegistry::new();

        let tool_use = ToolUseBlock {
            id: "t1".to_string(),
            name: "unknown".to_string(),
            input: json!({}),
        };

        let result = registry.execute_tool(&tool_use).await.unwrap();
        assert!(result.is_error);
        match result.content.unwrap() {
            crate::types::ToolResultContent::Text(text) => {
                assert!(text.contains("not registered"));
            }
            _ => panic!("Expected text content"),
        }
    }

    #[test]
    fn test_tool_registry_available_tools() {
        let mut registry = ToolRegistry::new();
        registry.register(EchoTool);

        let tools = registry.available_tools();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "echo");
    }

    #[tokio::test]
    async fn test_empty_tool_environment() {
        let env = EmptyToolEnvironment;
        assert!(env.available_tools().is_empty());

        let tool_use = ToolUseBlock {
            id: "t1".to_string(),
            name: "foo".to_string(),
            input: json!({}),
        };

        let result = env.execute_tool(&tool_use).await.unwrap();
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn test_mock_tool_environment() {
        let env = MockToolEnvironment::new(vec![test_tool()]);
        env.set_response("test_tool", "success!");

        let tool_use = ToolUseBlock {
            id: "t1".to_string(),
            name: "test_tool".to_string(),
            input: json!({"arg": "value"}),
        };

        let result = env.execute_tool(&tool_use).await.unwrap();
        assert!(!result.is_error);
        assert_eq!(
            result.content,
            Some(crate::types::ToolResultContent::Text(
                "success!".to_string()
            ))
        );

        assert_eq!(env.execution_count(), 1);
        let executions = env.executions();
        assert_eq!(executions[0].name, "test_tool");
    }

    #[tokio::test]
    async fn test_composite_tool_environment() {
        let env1 = Arc::new(MockToolEnvironment::new(vec![ToolDefinition::new(
            "tool_a",
            "Tool A",
            json!({}),
        )]));
        env1.set_response("tool_a", "from env1");

        let env2 = Arc::new(MockToolEnvironment::new(vec![ToolDefinition::new(
            "tool_b",
            "Tool B",
            json!({}),
        )]));
        env2.set_response("tool_b", "from env2");

        let composite = CompositeToolEnvironment::new(vec![env1, env2]);

        // Check both tools are available
        let tools = composite.available_tools();
        assert_eq!(tools.len(), 2);

        // Execute tool_a
        let result = composite
            .execute_tool(&ToolUseBlock {
                id: "t1".to_string(),
                name: "tool_a".to_string(),
                input: json!({}),
            })
            .await
            .unwrap();
        assert_eq!(
            result.content,
            Some(crate::types::ToolResultContent::Text(
                "from env1".to_string()
            ))
        );

        // Execute tool_b
        let result = composite
            .execute_tool(&ToolUseBlock {
                id: "t2".to_string(),
                name: "tool_b".to_string(),
                input: json!({}),
            })
            .await
            .unwrap();
        assert_eq!(
            result.content,
            Some(crate::types::ToolResultContent::Text(
                "from env2".to_string()
            ))
        );
    }

    #[test]
    fn test_filter_tools() {
        let env = MockToolEnvironment::new(vec![
            ToolDefinition::new("a", "A", json!({})),
            ToolDefinition::new("b", "B", json!({})),
            ToolDefinition::new("c", "C", json!({})),
        ]);

        let filtered = env.filter_tools(&["a".to_string(), "c".to_string()]);
        assert_eq!(filtered.len(), 2);
        assert!(filtered.iter().any(|t| t.name == "a"));
        assert!(filtered.iter().any(|t| t.name == "c"));
    }

    #[test]
    fn test_has_tool() {
        let env = MockToolEnvironment::new(vec![test_tool()]);
        assert!(env.has_tool("test_tool"));
        assert!(!env.has_tool("other_tool"));
    }

    #[test]
    fn test_common_tools() {
        let read = common_tools::read_file();
        assert_eq!(read.name, "read_file");

        let list = common_tools::list_files();
        assert_eq!(list.name, "list_files");

        let search = common_tools::search_code();
        assert_eq!(search.name, "search_code");

        let graph = common_tools::query_graph();
        assert_eq!(graph.name, "query_graph");

        let subquery = common_tools::spawn_subquery();
        assert_eq!(subquery.name, "spawn_subquery");
    }

    // Test internal tool for filtering
    struct InternalTool;

    #[async_trait]
    impl Tool for InternalTool {
        fn name(&self) -> &str {
            "internal_tool"
        }

        fn description(&self) -> &str {
            "An internal-only tool"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            json!({"type": "object", "properties": {}})
        }

        fn is_internal(&self) -> bool {
            true // This tool should NOT be exposed via MCP
        }

        async fn execute(&self, _params: serde_json::Value) -> Result<ToolResult> {
            Ok(ToolResult::text("internal result"))
        }
    }

    #[test]
    fn test_internal_tools_filtered_from_external() {
        let mut registry = ToolRegistry::new();
        registry.register(EchoTool); // External tool (default)
        registry.register(InternalTool); // Internal tool

        // available_tools() returns ALL tools
        let all_tools = registry.available_tools();
        assert_eq!(all_tools.len(), 2);

        // available_tools_external() filters out internal tools
        let external_tools = registry.available_tools_external();
        assert_eq!(external_tools.len(), 1);
        assert_eq!(external_tools[0].name, "echo");
    }

    #[test]
    fn test_default_is_external() {
        // By default, tools are external (not internal)
        assert!(!EchoTool.is_internal());
    }
}
