//! Test fixtures and builders for common test scenarios.
//!
//! Provides pre-built requests and responses to reduce test boilerplate.

use crate::backend::{ContentDelta, StreamEvent};
use crate::types::{
    CompletionRequest, CompletionResponse, ContentBlock, Message, MuninnConfig, StopReason,
    ToolDefinition, ToolResultBlock, Usage,
};

// ============================================================================
// Message Helpers
// ============================================================================

/// Create a user message with text content.
pub fn user_message(content: &str) -> Message {
    Message::user(content)
}

/// Create an assistant message with text content.
pub fn assistant_message(content: &str) -> Message {
    Message::assistant(content)
}

/// Create a tool result message.
pub fn tool_result_message(tool_use_id: &str, content: &str) -> Message {
    Message::tool_results(vec![ToolResultBlock::success(tool_use_id, content)])
}

/// Create an error tool result message.
pub fn tool_error_message(tool_use_id: &str, error: &str) -> Message {
    Message::tool_results(vec![ToolResultBlock::error(tool_use_id, error)])
}

// ============================================================================
// Request Helpers
// ============================================================================

/// Create a simple completion request.
pub fn simple_request() -> CompletionRequest {
    CompletionRequest::new("test-model", vec![Message::user("Hello")], 100)
}

/// Create a request with a system prompt.
pub fn request_with_system(system: &str) -> CompletionRequest {
    CompletionRequest::new("test-model", vec![Message::user("Hello")], 100).with_system(system)
}

/// Create a recursive exploration request.
pub fn recursive_request() -> CompletionRequest {
    CompletionRequest::new(
        "test-model",
        vec![Message::user("Analyze the codebase")],
        2048,
    )
    .with_muninn(MuninnConfig::recursive())
}

/// Create a request with specific tools.
pub fn request_with_tools(tool_names: &[&str]) -> CompletionRequest {
    let tools: Vec<ToolDefinition> = tool_names
        .iter()
        .map(|name| ToolDefinition::new(*name, format!("Tool: {}", name), serde_json::json!({})))
        .collect();

    CompletionRequest::new("test-model", vec![Message::user("Use tools")], 1000).with_tools(tools)
}

/// Create a simple text response.
pub fn text_response(content: &str) -> CompletionResponse {
    CompletionResponse::new(
        "msg_test",
        "test-model",
        vec![ContentBlock::Text {
            text: content.to_string(),
            cache_control: None,
        }],
        StopReason::EndTurn,
        Usage::new(10, 20),
    )
}

/// Create a response with custom usage.
pub fn text_response_with_usage(content: &str, input: u32, output: u32) -> CompletionResponse {
    CompletionResponse::new(
        "msg_test",
        "test-model",
        vec![ContentBlock::Text {
            text: content.to_string(),
            cache_control: None,
        }],
        StopReason::EndTurn,
        Usage::new(input, output),
    )
}

/// Create a tool use response.
pub fn tool_use_response(tool_name: &str, args: serde_json::Value) -> CompletionResponse {
    CompletionResponse::new(
        "msg_tool",
        "test-model",
        vec![ContentBlock::ToolUse {
            id: format!("tool_{}", uuid_v4_stub()),
            name: tool_name.to_string(),
            input: args,
            cache_control: None,
        }],
        StopReason::ToolUse,
        Usage::new(50, 30),
    )
}

/// Create a response with multiple tool uses.
pub fn multi_tool_response(tools: Vec<(&str, serde_json::Value)>) -> CompletionResponse {
    let content: Vec<ContentBlock> = tools
        .into_iter()
        .enumerate()
        .map(|(i, (name, args))| ContentBlock::ToolUse {
            id: format!("tool_{}", i),
            name: name.to_string(),
            input: args,
            cache_control: None,
        })
        .collect();

    CompletionResponse::new(
        "msg_multi_tool",
        "test-model",
        content,
        StopReason::ToolUse,
        Usage::new(100, 80),
    )
}

/// Create an error response (simulated by text with error indication).
pub fn error_response(error_msg: &str) -> CompletionResponse {
    CompletionResponse::new(
        "msg_error",
        "test-model",
        vec![ContentBlock::Text {
            text: format!("Error: {}", error_msg),
            cache_control: None,
        }],
        StopReason::EndTurn,
        Usage::new(5, 10),
    )
}

/// Create a response sequence for tool use followed by final answer.
pub fn tool_then_answer_responses(
    tool_name: &str,
    tool_args: serde_json::Value,
    final_answer: &str,
) -> Vec<CompletionResponse> {
    vec![
        tool_use_response(tool_name, tool_args),
        text_response(final_answer),
    ]
}

/// Create a sequence of streaming events for text content.
pub fn streaming_text_response(content: &str) -> Vec<StreamEvent> {
    vec![
        StreamEvent::MessageStart {
            id: format!("msg_{}", uuid_v4_stub()),
            model: "test-model".to_string(),
        },
        StreamEvent::ContentBlockStart {
            index: 0,
            content_type: "text".to_string(),
        },
        StreamEvent::ContentBlockDelta {
            index: 0,
            delta: ContentDelta::TextDelta(content.to_string()),
        },
        StreamEvent::ContentBlockStop { index: 0 },
        StreamEvent::MessageDelta {
            stop_reason: StopReason::EndTurn,
            usage: Usage::new(10, 20),
        },
        StreamEvent::MessageStop,
    ]
}

/// Create streaming events with chunked text (simulates real streaming).
pub fn streaming_text_chunked(chunks: &[&str]) -> Vec<StreamEvent> {
    let mut events = vec![
        StreamEvent::MessageStart {
            id: format!("msg_{}", uuid_v4_stub()),
            model: "test-model".to_string(),
        },
        StreamEvent::ContentBlockStart {
            index: 0,
            content_type: "text".to_string(),
        },
    ];

    for chunk in chunks {
        events.push(StreamEvent::ContentBlockDelta {
            index: 0,
            delta: ContentDelta::TextDelta(chunk.to_string()),
        });
    }

    events.push(StreamEvent::ContentBlockStop { index: 0 });
    events.push(StreamEvent::MessageDelta {
        stop_reason: StopReason::EndTurn,
        usage: Usage::new(10, chunks.len() as u32 * 5),
    });
    events.push(StreamEvent::MessageStop);

    events
}

/// Simple stub for generating unique-ish IDs in tests.
fn uuid_v4_stub() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    format!("{:08x}", COUNTER.fetch_add(1, Ordering::SeqCst))
}

/// Builder for creating custom completion responses.
pub struct ResponseBuilder {
    id: String,
    model: String,
    content: Vec<ContentBlock>,
    stop_reason: StopReason,
    usage: Usage,
}

impl ResponseBuilder {
    /// Create a new response builder.
    pub fn new() -> Self {
        Self {
            id: format!("msg_{}", uuid_v4_stub()),
            model: "test-model".to_string(),
            content: Vec::new(),
            stop_reason: StopReason::EndTurn,
            usage: Usage::new(10, 10),
        }
    }

    /// Set the response ID.
    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = id.into();
        self
    }

    /// Set the model name.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    /// Add text content.
    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.content.push(ContentBlock::Text {
            text: text.into(),
            cache_control: None,
        });
        self
    }

    /// Add tool use content.
    pub fn with_tool_use(mut self, name: impl Into<String>, input: serde_json::Value) -> Self {
        self.content.push(ContentBlock::ToolUse {
            id: format!("tool_{}", uuid_v4_stub()),
            name: name.into(),
            input,
            cache_control: None,
        });
        self.stop_reason = StopReason::ToolUse;
        self
    }

    /// Set the stop reason.
    pub fn with_stop_reason(mut self, reason: StopReason) -> Self {
        self.stop_reason = reason;
        self
    }

    /// Set usage statistics.
    pub fn with_usage(mut self, input: u32, output: u32) -> Self {
        self.usage = Usage::new(input, output);
        self
    }

    /// Build the completion response.
    pub fn build(self) -> CompletionResponse {
        CompletionResponse::new(
            self.id,
            self.model,
            self.content,
            self.stop_reason,
            self.usage,
        )
    }
}

impl Default for ResponseBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Request Builder
// ============================================================================

/// Builder for creating custom completion requests.
pub struct RequestBuilder {
    model: String,
    messages: Vec<Message>,
    max_tokens: u32,
    system: Option<String>,
    tools: Vec<ToolDefinition>,
    recursive: bool,
}

impl RequestBuilder {
    /// Create a new request builder with defaults.
    pub fn new() -> Self {
        Self {
            model: "test-model".to_string(),
            messages: Vec::new(),
            max_tokens: 1000,
            system: None,
            tools: Vec::new(),
            recursive: false,
        }
    }

    /// Set the model name.
    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    /// Add a message to the request.
    pub fn message(mut self, msg: Message) -> Self {
        self.messages.push(msg);
        self
    }

    /// Add a user message.
    pub fn user(self, content: impl Into<String>) -> Self {
        self.message(Message::user(content))
    }

    /// Add an assistant message.
    pub fn assistant(self, content: impl Into<String>) -> Self {
        self.message(Message::assistant(content))
    }

    /// Set the system prompt.
    pub fn system(mut self, prompt: impl Into<String>) -> Self {
        self.system = Some(prompt.into());
        self
    }

    /// Set max tokens.
    pub fn max_tokens(mut self, tokens: u32) -> Self {
        self.max_tokens = tokens;
        self
    }

    /// Enable recursive mode.
    pub fn recursive(mut self) -> Self {
        self.recursive = true;
        self
    }

    /// Add a tool definition.
    pub fn tool(mut self, tool: ToolDefinition) -> Self {
        self.tools.push(tool);
        self
    }

    /// Add multiple tools by name (creates simple tool definitions).
    pub fn with_tools(mut self, tool_names: &[&str]) -> Self {
        for name in tool_names {
            self.tools.push(ToolDefinition::new(
                *name,
                format!("Tool: {}", name),
                serde_json::json!({}),
            ));
        }
        self
    }

    /// Build the completion request.
    pub fn build(self) -> CompletionRequest {
        let mut request = CompletionRequest::new(self.model, self.messages, self.max_tokens);

        if let Some(system) = self.system {
            request = request.with_system(system);
        }

        if !self.tools.is_empty() {
            request = request.with_tools(self.tools);
        }

        if self.recursive {
            request = request.with_muninn(MuninnConfig::recursive());
        }

        request
    }
}

impl Default for RequestBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_request() {
        let req = simple_request();
        assert_eq!(req.model, "test-model");
        assert_eq!(req.messages.len(), 1);
    }

    #[test]
    fn test_recursive_request() {
        let req = recursive_request();
        assert!(req.muninn.is_some());
        assert!(req.muninn.unwrap().recursive);
    }

    #[test]
    fn test_text_response() {
        let resp = text_response("Hello!");
        assert_eq!(resp.text(), "Hello!");
        assert_eq!(resp.stop_reason, Some(StopReason::EndTurn));
    }

    #[test]
    fn test_tool_use_response() {
        let resp = tool_use_response("read_file", serde_json::json!({"path": "/test.rs"}));
        assert!(resp.has_tool_use());
        assert_eq!(resp.stop_reason, Some(StopReason::ToolUse));
        let tools = resp.tool_uses();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "read_file");
    }

    #[test]
    fn test_response_builder() {
        let resp = ResponseBuilder::new()
            .with_id("custom_id")
            .with_model("custom-model")
            .with_text("Hello")
            .with_usage(100, 50)
            .build();

        assert_eq!(resp.id, "custom_id");
        assert_eq!(resp.model, "custom-model");
        assert_eq!(resp.text(), "Hello");
        assert_eq!(resp.usage.input_tokens, 100);
        assert_eq!(resp.usage.output_tokens, 50);
    }

    #[test]
    fn test_response_builder_with_tool() {
        let resp = ResponseBuilder::new()
            .with_tool_use("search", serde_json::json!({"query": "test"}))
            .build();

        assert!(resp.has_tool_use());
        assert_eq!(resp.stop_reason, Some(StopReason::ToolUse));
    }

    #[test]
    fn test_multi_tool_response() {
        let resp = multi_tool_response(vec![
            ("read_file", serde_json::json!({"path": "/a.rs"})),
            ("read_file", serde_json::json!({"path": "/b.rs"})),
        ]);

        let tools = resp.tool_uses();
        assert_eq!(tools.len(), 2);
    }

    #[test]
    fn test_tool_then_answer_responses() {
        let responses = tool_then_answer_responses(
            "analyze",
            serde_json::json!({"target": "code"}),
            "Analysis complete",
        );

        assert_eq!(responses.len(), 2);
        assert!(responses[0].has_tool_use());
        assert_eq!(responses[1].text(), "Analysis complete");
    }

    #[test]
    fn test_user_message() {
        let msg = user_message("Hello!");
        assert_eq!(msg.content.as_text(), Some("Hello!"));
    }

    #[test]
    fn test_assistant_message() {
        let msg = assistant_message("Hi there!");
        assert_eq!(msg.content.as_text(), Some("Hi there!"));
    }

    #[test]
    fn test_tool_result_message() {
        let msg = tool_result_message("tool_123", "Result data");
        // Tool results are blocks, not simple text
        let blocks = msg.content.blocks();
        assert_eq!(blocks.len(), 1);
    }

    #[test]
    fn test_streaming_text_response() {
        let events = streaming_text_response("Hello!");
        assert_eq!(events.len(), 6);
        assert!(matches!(events[0], StreamEvent::MessageStart { .. }));
        assert!(matches!(events[5], StreamEvent::MessageStop));
    }

    #[test]
    fn test_streaming_text_chunked() {
        let events = streaming_text_chunked(&["Hello", " ", "World"]);
        // MessageStart, ContentBlockStart, 3 deltas, ContentBlockStop, MessageDelta, MessageStop
        assert_eq!(events.len(), 8);
    }

    #[test]
    fn test_request_builder_basic() {
        let req = RequestBuilder::new()
            .model("custom-model")
            .user("Hello!")
            .max_tokens(500)
            .build();

        assert_eq!(req.model, "custom-model");
        assert_eq!(req.messages.len(), 1);
        assert_eq!(req.max_tokens, 500);
    }

    #[test]
    fn test_request_builder_with_system() {
        let req = RequestBuilder::new()
            .system("Be helpful")
            .user("Hi")
            .build();

        assert!(req.system.is_some());
    }

    #[test]
    fn test_request_builder_recursive() {
        let req = RequestBuilder::new()
            .user("Analyze code")
            .recursive()
            .build();

        assert!(req.muninn.is_some());
        assert!(req.muninn.unwrap().recursive);
    }

    #[test]
    fn test_request_builder_with_tools() {
        let req = RequestBuilder::new()
            .user("Use tools")
            .with_tools(&["read_file", "search"])
            .build();

        assert_eq!(req.tools.len(), 2);
        assert_eq!(req.tools[0].name, "read_file");
        assert_eq!(req.tools[1].name, "search");
    }

    #[test]
    fn test_request_builder_conversation() {
        let req = RequestBuilder::new()
            .user("What is 2+2?")
            .assistant("4")
            .user("And 3+3?")
            .build();

        assert_eq!(req.messages.len(), 3);
    }
}
