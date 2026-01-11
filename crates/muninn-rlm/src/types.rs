//! Core types for RLM gateway.
//!
//! These types are designed to be compatible with the Anthropic Messages API
//! while supporting Muninn-specific extensions for recursive exploration.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// System prompt - can be a string or array of text blocks.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SystemPrompt {
    /// Simple string prompt.
    Text(String),
    /// Array of text blocks (for cache control).
    Blocks(Vec<SystemBlock>),
}

/// A text block in a system prompt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemBlock {
    /// The text content.
    pub text: String,
    /// Block type (always "text").
    #[serde(rename = "type")]
    pub block_type: String,
    /// Optional cache control.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<CacheControl>,
}

impl SystemPrompt {
    /// Get the text content of the system prompt.
    pub fn to_text(&self) -> String {
        match self {
            SystemPrompt::Text(s) => s.clone(),
            SystemPrompt::Blocks(blocks) => blocks
                .iter()
                .map(|b| b.text.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }
}

/// A completion request compatible with Anthropic Messages API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionRequest {
    /// The model to use for completion.
    pub model: String,

    /// The messages in the conversation.
    pub messages: Vec<Message>,

    /// Maximum tokens to generate.
    pub max_tokens: u32,

    /// System prompt (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<SystemPrompt>,

    /// Tools available for the model to use.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolDefinition>,

    /// How the model should use tools.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ToolChoice>,

    /// Whether to stream the response.
    #[serde(default)]
    pub stream: bool,

    /// Temperature for sampling (0.0 to 1.0).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,

    /// Top-p sampling parameter.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,

    /// Top-k sampling parameter.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,

    /// Stop sequences.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stop_sequences: Vec<String>,

    /// Muninn-specific configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub muninn: Option<MuninnConfig>,

    /// Additional metadata.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, serde_json::Value>,

    /// Extended thinking configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<serde_json::Value>,
}

impl CompletionRequest {
    /// Create a new completion request with the given model and messages.
    pub fn new(model: impl Into<String>, messages: Vec<Message>, max_tokens: u32) -> Self {
        Self {
            model: model.into(),
            messages,
            max_tokens,
            system: None,
            tools: Vec::new(),
            tool_choice: None,
            stream: false,
            temperature: None,
            top_p: None,
            top_k: None,
            stop_sequences: Vec::new(),
            muninn: None,
            metadata: HashMap::new(),
            thinking: None,
        }
    }

    /// Set the system prompt.
    pub fn with_system(mut self, system: impl Into<String>) -> Self {
        self.system = Some(SystemPrompt::Text(system.into()));
        self
    }

    /// Add tools to the request.
    pub fn with_tools(mut self, tools: Vec<ToolDefinition>) -> Self {
        self.tools = tools;
        self
    }

    /// Enable streaming.
    pub fn with_streaming(mut self) -> Self {
        self.stream = true;
        self
    }

    /// Set Muninn configuration.
    pub fn with_muninn(mut self, config: MuninnConfig) -> Self {
        self.muninn = Some(config);
        self
    }
}

/// A message in the conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// The role of the message author.
    pub role: Role,

    /// The content of the message.
    pub content: Content,
}

impl Message {
    /// Create a user message with text content.
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: Content::Text(text.into()),
        }
    }

    /// Create an assistant message with text content.
    pub fn assistant(text: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: Content::Text(text.into()),
        }
    }

    /// Create an assistant message with content blocks.
    pub fn assistant_blocks(blocks: Vec<ContentBlock>) -> Self {
        Self {
            role: Role::Assistant,
            content: Content::Blocks(blocks),
        }
    }

    /// Create a user message with tool results.
    pub fn tool_results(results: Vec<ToolResultBlock>) -> Self {
        Self {
            role: Role::User,
            content: Content::Blocks(results.into_iter().map(|r| r.into()).collect()),
        }
    }
}

/// The role of a message author.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

/// Message content - either a simple string or structured blocks.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Content {
    /// Simple text content.
    Text(String),
    /// Structured content blocks.
    Blocks(Vec<ContentBlock>),
}

impl Content {
    /// Get the text content if this is simple text.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Content::Text(s) => Some(s),
            Content::Blocks(_) => None,
        }
    }

    /// Get the content blocks.
    pub fn blocks(&self) -> Vec<ContentBlock> {
        match self {
            Content::Text(s) => vec![ContentBlock::Text {
                text: s.clone(),
                cache_control: None,
            }],
            Content::Blocks(blocks) => blocks.clone(),
        }
    }

    /// Extract all text from the content.
    pub fn to_text(&self) -> String {
        match self {
            Content::Text(s) => {
                tracing::trace!(content_type = "Text", length = s.len(), "Content::to_text");
                s.clone()
            }
            Content::Blocks(blocks) => {
                tracing::trace!(
                    content_type = "Blocks",
                    block_count = blocks.len(),
                    block_types = ?blocks.iter().map(|b| match b {
                        ContentBlock::Text { .. } => "Text",
                        ContentBlock::ToolUse { .. } => "ToolUse",
                        ContentBlock::ToolResult { .. } => "ToolResult",
                        ContentBlock::Thinking { .. } => "Thinking",
                    }).collect::<Vec<_>>(),
                    "Content::to_text"
                );
                blocks
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::Text {
                            text,
                            cache_control: _,
                        } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("")
            }
        }
    }
}

/// Cache control for prompt caching.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CacheControl {
    /// Ephemeral cache control.
    Ephemeral,
}

/// A content block in a message.
///
/// Note: For passthrough mode, the proxy uses raw JSON to handle all content types
/// including thinking blocks, images, etc. This enum is only used for RLM mode
/// where we only need to handle text and tool-related blocks.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    /// Text content.
    Text {
        /// The text content.
        text: String,
        /// Optional cache control.
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControl>,
    },
    /// Tool use request from the assistant.
    ToolUse {
        /// Unique ID for this tool use.
        id: String,
        /// Name of the tool to use.
        name: String,
        /// Input arguments for the tool.
        input: serde_json::Value,
        /// Optional cache control.
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControl>,
    },
    /// Tool result from the user.
    ToolResult {
        /// ID of the tool use this is a result for.
        tool_use_id: String,
        /// The result content (optional).
        #[serde(skip_serializing_if = "Option::is_none")]
        content: Option<ToolResultContent>,
        /// Whether the tool execution resulted in an error.
        #[serde(default)]
        is_error: bool,
        /// Optional cache control.
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControl>,
    },
    /// Extended thinking content from the assistant.
    Thinking {
        /// The thinking content.
        thinking: String,
        /// Signature for verification.
        signature: String,
    },
}

/// Tool result content - can be a string or array of content blocks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ToolResultContent {
    Text(String),
    Blocks(Vec<serde_json::Value>),
}

/// Convenience struct for creating tool use blocks.
///
/// This can be converted to a ContentBlock using Into/From.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolUseBlock {
    /// Unique ID for this tool use.
    pub id: String,
    /// Name of the tool to use.
    pub name: String,
    /// Input arguments for the tool.
    pub input: serde_json::Value,
}

impl From<ToolUseBlock> for ContentBlock {
    fn from(block: ToolUseBlock) -> Self {
        ContentBlock::ToolUse {
            id: block.id,
            name: block.name,
            input: block.input,
            cache_control: None,
        }
    }
}

/// Convenience struct for creating tool result blocks.
///
/// This can be converted to a ContentBlock using Into/From.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResultBlock {
    /// ID of the tool use this is a result for.
    pub tool_use_id: String,
    /// The result content (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<ToolResultContent>,
    /// Whether the tool execution resulted in an error.
    #[serde(default)]
    pub is_error: bool,
}

impl ToolResultBlock {
    /// Create a successful tool result.
    pub fn success(tool_use_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            tool_use_id: tool_use_id.into(),
            content: Some(ToolResultContent::Text(content.into())),
            is_error: false,
        }
    }

    /// Create an error tool result.
    pub fn error(tool_use_id: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            tool_use_id: tool_use_id.into(),
            content: Some(ToolResultContent::Text(error.into())),
            is_error: true,
        }
    }
}

impl From<ToolResultBlock> for ContentBlock {
    fn from(block: ToolResultBlock) -> Self {
        ContentBlock::ToolResult {
            tool_use_id: block.tool_use_id,
            content: block.content,
            is_error: block.is_error,
            cache_control: None,
        }
    }
}

impl ContentBlock {
    /// Create a text content block.
    pub fn text(content: impl Into<String>) -> Self {
        ContentBlock::Text {
            text: content.into(),
            cache_control: None,
        }
    }

    /// Create a tool use content block.
    pub fn tool_use(
        id: impl Into<String>,
        name: impl Into<String>,
        input: serde_json::Value,
    ) -> Self {
        ContentBlock::ToolUse {
            id: id.into(),
            name: name.into(),
            input,
            cache_control: None,
        }
    }

    /// Create a successful tool result block.
    pub fn tool_result_success(tool_use_id: impl Into<String>, content: impl Into<String>) -> Self {
        ContentBlock::ToolResult {
            tool_use_id: tool_use_id.into(),
            content: Some(ToolResultContent::Text(content.into())),
            is_error: false,
            cache_control: None,
        }
    }

    /// Create an error tool result block.
    pub fn tool_result_error(tool_use_id: impl Into<String>, error: impl Into<String>) -> Self {
        ContentBlock::ToolResult {
            tool_use_id: tool_use_id.into(),
            content: Some(ToolResultContent::Text(error.into())),
            is_error: true,
            cache_control: None,
        }
    }
}

/// Definition of a tool available to the model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    /// Name of the tool.
    pub name: String,

    /// Description of what the tool does.
    pub description: String,

    /// JSON Schema for the tool's input parameters.
    pub input_schema: serde_json::Value,
}

impl ToolDefinition {
    /// Create a new tool definition.
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        input_schema: serde_json::Value,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            input_schema,
        }
    }
}

/// How the model should choose which tool to use.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolChoice {
    /// Model decides whether to use tools.
    Auto,
    /// Model must use a tool.
    Any,
    /// Model must use a specific tool.
    Tool { name: String },
    /// Model should not use tools.
    None,
}

/// A completion response from the model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionResponse {
    /// Unique ID for this response.
    pub id: String,

    /// The type of response (always "message").
    #[serde(rename = "type", default = "default_message_type")]
    pub response_type: String,

    /// The role (always "assistant").
    pub role: Role,

    /// The content blocks in the response.
    pub content: Vec<ContentBlock>,

    /// The model that generated the response.
    pub model: String,

    /// Why the model stopped generating.
    pub stop_reason: Option<StopReason>,

    /// Token usage statistics.
    pub usage: Usage,

    /// Muninn exploration metadata.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub muninn: Option<ExplorationMetadata>,
}

fn default_message_type() -> String {
    "message".to_string()
}

impl CompletionResponse {
    /// Create a new completion response.
    pub fn new(
        id: impl Into<String>,
        model: impl Into<String>,
        content: Vec<ContentBlock>,
        stop_reason: StopReason,
        usage: Usage,
    ) -> Self {
        Self {
            id: id.into(),
            response_type: "message".to_string(),
            role: Role::Assistant,
            content,
            model: model.into(),
            stop_reason: Some(stop_reason),
            usage,
            muninn: None,
        }
    }

    /// Get all tool use blocks from the response.
    pub fn tool_uses(&self) -> Vec<ToolUseBlock> {
        self.content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::ToolUse {
                    id,
                    name,
                    input,
                    cache_control: _,
                } => Some(ToolUseBlock {
                    id: id.clone(),
                    name: name.clone(),
                    input: input.clone(),
                }),
                _ => None,
            })
            .collect()
    }

    /// Get the text content from the response.
    pub fn text(&self) -> String {
        self.content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text {
                    text,
                    cache_control: _,
                } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("")
    }

    /// Check if the response contains tool use requests.
    pub fn has_tool_use(&self) -> bool {
        self.content
            .iter()
            .any(|block| matches!(block, ContentBlock::ToolUse { .. }))
    }
}

/// Why the model stopped generating.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// Natural end of response.
    EndTurn,
    /// Model wants to use a tool.
    ToolUse,
    /// Hit max_tokens limit.
    MaxTokens,
    /// Hit a stop sequence.
    StopSequence,
}

/// Token usage statistics.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    /// Tokens in the input.
    pub input_tokens: u32,
    /// Tokens in the output.
    pub output_tokens: u32,
    /// Tokens used for caching (if applicable).
    #[serde(default)]
    pub cache_creation_input_tokens: u32,
    /// Tokens read from cache (if applicable).
    #[serde(default)]
    pub cache_read_input_tokens: u32,
}

impl Usage {
    /// Create new usage statistics.
    pub fn new(input_tokens: u32, output_tokens: u32) -> Self {
        Self {
            input_tokens,
            output_tokens,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
        }
    }

    /// Total tokens used.
    pub fn total(&self) -> u32 {
        self.input_tokens + self.output_tokens
    }
}

/// Muninn-specific configuration for recursive exploration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MuninnConfig {
    /// Whether to enable recursive exploration.
    #[serde(default)]
    pub recursive: bool,

    /// Budget constraints for exploration.
    #[serde(default)]
    pub budget: BudgetConfig,

    /// Whether to include exploration metadata in response.
    #[serde(default = "default_true")]
    pub include_metadata: bool,
}

fn default_true() -> bool {
    true
}

impl Default for MuninnConfig {
    fn default() -> Self {
        Self {
            recursive: false,
            budget: BudgetConfig::default(),
            include_metadata: true, // Include metadata by default
        }
    }
}

impl MuninnConfig {
    /// Create a new Muninn config with recursive exploration enabled.
    pub fn recursive() -> Self {
        Self {
            recursive: true,
            budget: BudgetConfig::default(),
            include_metadata: true,
        }
    }

    /// Set the budget configuration.
    pub fn with_budget(mut self, budget: BudgetConfig) -> Self {
        self.budget = budget;
        self
    }
}

/// Budget configuration for recursive exploration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetConfig {
    /// Maximum total tokens across all recursive calls.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,

    /// Maximum wall-clock time in seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_duration_secs: Option<u64>,

    /// Maximum recursion depth.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_depth: Option<u32>,

    /// Maximum number of tool executions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tool_calls: Option<u32>,
}

impl Default for BudgetConfig {
    fn default() -> Self {
        Self {
            max_tokens: Some(100_000),
            max_duration_secs: Some(300),
            max_depth: Some(10),
            max_tool_calls: Some(50),
        }
    }
}

/// Metadata about recursive exploration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExplorationMetadata {
    /// Maximum depth reached during exploration.
    pub depth_reached: u32,
    /// Total tokens used across all calls.
    pub tokens_used: u64,
    /// Number of tool calls executed.
    pub tool_calls: u32,
    /// Total duration in milliseconds.
    pub duration_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_user() {
        let msg = Message::user("Hello");
        assert_eq!(msg.role, Role::User);
        assert_eq!(msg.content.as_text(), Some("Hello"));
    }

    #[test]
    fn test_message_assistant() {
        let msg = Message::assistant("Hi there");
        assert_eq!(msg.role, Role::Assistant);
        assert_eq!(msg.content.as_text(), Some("Hi there"));
    }

    #[test]
    fn test_completion_request_builder() {
        let request = CompletionRequest::new(
            "claude-sonnet-4-20250514",
            vec![Message::user("Hello")],
            1024,
        )
        .with_system("You are helpful.")
        .with_streaming();

        assert_eq!(request.model, "claude-sonnet-4-20250514");
        assert_eq!(request.max_tokens, 1024);
        assert!(request.system.is_some());
        assert!(request.stream);
    }

    #[test]
    fn test_completion_response_tool_uses() {
        let response = CompletionResponse {
            id: "msg_123".to_string(),
            response_type: "message".to_string(),
            role: Role::Assistant,
            content: vec![
                ContentBlock::Text {
                    text: "Let me help.".to_string(),
                    cache_control: None,
                },
                ContentBlock::ToolUse {
                    id: "tool_1".to_string(),
                    name: "read_file".to_string(),
                    input: serde_json::json!({"path": "/foo.rs"}),
                    cache_control: None,
                },
            ],
            model: "claude-sonnet-4-20250514".to_string(),
            stop_reason: Some(StopReason::ToolUse),
            usage: Usage::new(100, 50),
            muninn: None,
        };

        assert!(response.has_tool_use());
        let tool_uses = response.tool_uses();
        assert_eq!(tool_uses.len(), 1);
        assert_eq!(tool_uses[0].name, "read_file");
    }

    #[test]
    fn test_tool_result_block() {
        let success = ToolResultBlock::success("tool_1", "file contents here");
        assert!(!success.is_error);
        assert_eq!(
            success.content,
            Some(ToolResultContent::Text("file contents here".to_string()))
        );

        let error = ToolResultBlock::error("tool_2", "file not found");
        assert!(error.is_error);
    }

    #[test]
    fn test_serialize_deserialize_request() {
        let request = CompletionRequest::new(
            "claude-sonnet-4-20250514",
            vec![Message::user("Hello")],
            1024,
        )
        .with_muninn(MuninnConfig::recursive());

        let json = serde_json::to_string(&request).unwrap();
        let parsed: CompletionRequest = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.model, request.model);
        assert!(parsed.muninn.unwrap().recursive);
    }

    #[test]
    fn test_budget_config_default() {
        let budget = BudgetConfig::default();
        assert_eq!(budget.max_tokens, Some(100_000));
        assert_eq!(budget.max_duration_secs, Some(300));
        assert_eq!(budget.max_depth, Some(10));
        assert_eq!(budget.max_tool_calls, Some(50));
    }

    #[test]
    fn test_content_blocks() {
        let text = Content::Text("hello".to_string());
        assert_eq!(text.blocks().len(), 1);

        let blocks = Content::Blocks(vec![
            ContentBlock::Text {
                text: "one".to_string(),
                cache_control: None,
            },
            ContentBlock::Text {
                text: "two".to_string(),
                cache_control: None,
            },
        ]);
        assert_eq!(blocks.to_text(), "onetwo");
    }
}
