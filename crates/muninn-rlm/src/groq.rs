//! Groq API backend implementation.
//!
//! This module provides the `GroqBackend` which connects to Groq's
//! OpenAI-compatible API for fast LLM inference.

use async_trait::async_trait;
use bytes::Bytes;
use futures::{Stream, StreamExt};
use reqwest::{Client, Response, header};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use crate::backend::{ContentDelta, LLMBackend, ResponseStream, StreamEvent, with_retry};
use crate::error::{Result, RlmError};
use crate::types::{
    CompletionRequest, CompletionResponse, ContentBlock, Message, Role, StopReason, Usage,
};

/// Default Groq API base URL.
const DEFAULT_API_BASE: &str = "https://api.groq.com/openai";

/// Default timeout for requests.
const DEFAULT_TIMEOUT_SECS: u64 = 300;

/// Default model for Groq backend.
const DEFAULT_MODEL: &str = "llama-3.1-70b-versatile";

/// Configuration for the Groq backend.
#[derive(Debug, Clone)]
pub struct GroqConfig {
    /// API key for authentication.
    pub api_key: String,

    /// Base URL for the API.
    pub base_url: String,

    /// Model to use for completions (overrides request model).
    pub model: String,

    /// Request timeout.
    pub timeout: Duration,

    /// Maximum retries for transient errors.
    pub max_retries: u32,

    /// Initial backoff duration for retries.
    pub retry_backoff: Duration,
}

impl GroqConfig {
    /// Create a new config with the given API key.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: DEFAULT_API_BASE.to_string(),
            model: DEFAULT_MODEL.to_string(),
            timeout: Duration::from_secs(DEFAULT_TIMEOUT_SECS),
            max_retries: 3,
            retry_backoff: Duration::from_millis(500),
        }
    }

    /// Set the model to use.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    /// Create config from environment variable.
    pub fn from_env() -> Result<Self> {
        let api_key = std::env::var("GROQ_API_KEY").map_err(|_| {
            RlmError::Config("GROQ_API_KEY environment variable not set".to_string())
        })?;
        Ok(Self::new(api_key))
    }

    /// Set a custom base URL.
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    /// Set request timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Set max retries.
    pub fn with_max_retries(mut self, retries: u32) -> Self {
        self.max_retries = retries;
        self
    }
}

/// Groq API backend.
pub struct GroqBackend {
    client: Client,
    config: GroqConfig,
}

impl GroqBackend {
    /// Create a new Groq backend with the given configuration.
    pub fn new(config: GroqConfig) -> Result<Self> {
        let client = Client::builder()
            .timeout(config.timeout)
            .build()
            .map_err(|e| RlmError::Internal(format!("Failed to create HTTP client: {}", e)))?;

        Ok(Self { client, config })
    }

    /// Create a backend from environment configuration.
    pub fn from_env() -> Result<Self> {
        Self::new(GroqConfig::from_env()?)
    }

    /// Build the chat completions endpoint URL.
    fn completions_url(&self) -> String {
        format!("{}/v1/chat/completions", self.config.base_url)
    }

    /// Add authentication headers to a request.
    fn add_headers(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        builder
            .header(
                header::AUTHORIZATION,
                format!("Bearer {}", self.config.api_key),
            )
            .header(header::CONTENT_TYPE, "application/json")
    }

    /// Convert our CompletionRequest to Groq's OpenAI-compatible format.
    fn to_groq_request(&self, request: &CompletionRequest) -> GroqChatRequest {
        let mut messages: Vec<GroqMessage> = Vec::new();

        // Add system message if present
        if let Some(ref system) = request.system {
            messages.push(GroqMessage {
                role: "system".to_string(),
                content: Some(system.to_text()),
                tool_calls: None,
                tool_call_id: None,
            });
        }

        // Add conversation messages with proper tool handling
        for m in &request.messages {
            let blocks = m.content.blocks();

            // Check if this is an assistant message with tool calls
            let tool_calls: Vec<_> = blocks
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::ToolUse {
                        id,
                        name,
                        input,
                        cache_control: _,
                    } => Some(GroqToolCall {
                        id: id.clone(),
                        call_type: "function".to_string(),
                        function: GroqFunctionCall {
                            name: name.clone(),
                            arguments: serde_json::to_string(input).unwrap_or_default(),
                        },
                    }),
                    _ => None,
                })
                .collect();

            // Check if this is a user message with tool results
            let tool_results: Vec<_> = blocks
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::ToolResult {
                        tool_use_id,
                        content,
                        ..
                    } => {
                        let text = match content {
                            Some(crate::types::ToolResultContent::Text(t)) => t.clone(),
                            Some(crate::types::ToolResultContent::Blocks(blocks)) => blocks
                                .iter()
                                .filter_map(|b| {
                                    if let serde_json::Value::Object(obj) = b {
                                        obj.get("text").and_then(|v| v.as_str()).map(String::from)
                                    } else {
                                        None
                                    }
                                })
                                .collect::<Vec<_>>()
                                .join("\n"),
                            None => String::new(),
                        };
                        Some((tool_use_id.clone(), text))
                    }
                    _ => None,
                })
                .collect();

            // Get text content
            let text_content: String = blocks
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::Text {
                        text,
                        cache_control: _,
                    } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("");

            if !tool_results.is_empty() {
                // Add tool results as separate "tool" role messages
                for (tool_id, result_text) in tool_results {
                    messages.push(GroqMessage {
                        role: "tool".to_string(),
                        content: Some(result_text),
                        tool_calls: None,
                        tool_call_id: Some(tool_id),
                    });
                }
            } else if !tool_calls.is_empty() {
                // Assistant message with tool calls
                messages.push(GroqMessage {
                    role: "assistant".to_string(),
                    content: if text_content.is_empty() {
                        None
                    } else {
                        Some(text_content)
                    },
                    tool_calls: Some(tool_calls),
                    tool_call_id: None,
                });
            } else {
                // Regular text message
                messages.push(GroqMessage {
                    role: match m.role {
                        Role::User => "user".to_string(),
                        Role::Assistant => "assistant".to_string(),
                    },
                    content: Some(text_content),
                    tool_calls: None,
                    tool_call_id: None,
                });
            }
        }

        let tools: Option<Vec<GroqTool>> = if request.tools.is_empty() {
            None
        } else {
            Some(
                request
                    .tools
                    .iter()
                    .map(|t| GroqTool {
                        tool_type: "function".to_string(),
                        function: GroqFunction {
                            name: t.name.clone(),
                            description: Some(t.description.clone()),
                            parameters: t.input_schema.clone(),
                        },
                    })
                    .collect(),
            )
        };

        let stop = if request.stop_sequences.is_empty() {
            None
        } else {
            Some(request.stop_sequences.clone())
        };

        // Disable thinking mode for Qwen3 to get direct responses
        let reasoning_effort = if self.config.model.contains("qwen") {
            Some("none".to_string())
        } else {
            None
        };

        GroqChatRequest {
            model: self.config.model.clone(),
            messages,
            max_completion_tokens: Some(request.max_tokens),
            temperature: request.temperature,
            top_p: request.top_p,
            stream: Some(request.stream),
            tools,
            stop,
            reasoning_effort,
        }
    }

    /// Handle a successful response.
    async fn handle_response(response: Response) -> Result<CompletionResponse> {
        if !response.status().is_success() {
            return Err(Self::handle_error_response(response).await);
        }

        let body = response.text().await?;
        let parsed: GroqChatResponse =
            serde_json::from_str(&body).map_err(|e| RlmError::Serialization(e.to_string()))?;

        Ok(parsed.into())
    }

    /// Handle an error response.
    async fn handle_error_response(response: Response) -> RlmError {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();

        if let Ok(error) = serde_json::from_str::<GroqErrorResponse>(&body) {
            match status.as_u16() {
                401 => RlmError::Config(format!("Authentication failed: {}", error.error.message)),
                429 => RlmError::Backend(format!("Rate limit exceeded: {}", error.error.message)),
                500..=599 => RlmError::Backend(format!("Server error: {}", error.error.message)),
                _ => RlmError::Backend(error.error.message),
            }
        } else {
            RlmError::Backend(format!("HTTP {}: {}", status, body))
        }
    }
}

#[async_trait]
impl LLMBackend for GroqBackend {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse> {
        let mut request = request;
        request.stream = false;

        let groq_request = self.to_groq_request(&request);

        // Log request details at debug level
        tracing::debug!(
            model = %groq_request.model,
            messages = %groq_request.messages.len(),
            tools = %groq_request.tools.as_ref().map(|t| t.len()).unwrap_or(0),
            temperature = ?groq_request.temperature,
            "Sending Groq request"
        );
        for (i, msg) in groq_request.messages.iter().enumerate() {
            let content_preview = serde_json::to_string(&msg.content)
                .map(|s| s.chars().take(300).collect::<String>())
                .unwrap_or_else(|_| "(serialization error)".to_string());
            tracing::debug!(
                msg_idx = i,
                role = %msg.role,
                content = %content_preview,
                "Message {}", i
            );
        }

        with_retry(
            self.config.max_retries,
            self.config.retry_backoff,
            "groq",
            || async {
                let response = self
                    .add_headers(self.client.post(self.completions_url()))
                    .json(&groq_request)
                    .send()
                    .await?;

                Self::handle_response(response).await
            },
        )
        .await
    }

    async fn complete_stream(&self, request: CompletionRequest) -> Result<ResponseStream> {
        let mut request = request;
        request.stream = true;

        let groq_request = self.to_groq_request(&request);

        let response = self
            .add_headers(self.client.post(self.completions_url()))
            .json(&groq_request)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(Self::handle_error_response(response).await);
        }

        Ok(parse_groq_sse_stream(response.bytes_stream()))
    }

    fn name(&self) -> &str {
        "groq"
    }

    async fn health_check(&self) -> Result<()> {
        let request =
            CompletionRequest::new("llama-3.1-8b-instant", vec![Message::user("ping")], 1);

        match self.complete(request).await {
            Ok(_) => Ok(()),
            Err(RlmError::Backend(msg)) if msg.contains("rate limit") => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Groq supports native tool calling via their OpenAI-compatible API.
    fn supports_native_tools(&self) -> bool {
        true
    }
}

// ============================================================================
// Request/Response types for Groq's OpenAI-compatible API
// ============================================================================

#[derive(Debug, serde::Serialize)]
struct GroqChatRequest {
    model: String,
    messages: Vec<GroqMessage>,
    /// For Qwen3 models, use max_completion_tokens instead of max_tokens.
    #[serde(skip_serializing_if = "Option::is_none")]
    max_completion_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<GroqTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop: Option<Vec<String>>,
    /// Controls Qwen3 reasoning/thinking mode. Set to "none" to disable thinking.
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<String>,
}

#[derive(Debug, serde::Serialize)]
struct GroqMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<GroqToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct GroqTool {
    #[serde(rename = "type")]
    tool_type: String,
    function: GroqFunction,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct GroqFunction {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    parameters: serde_json::Value,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct GroqToolCall {
    id: String,
    #[serde(rename = "type")]
    call_type: String,
    function: GroqFunctionCall,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct GroqFunctionCall {
    name: String,
    arguments: String,
}

#[derive(Debug, serde::Deserialize)]
struct GroqChatResponse {
    id: String,
    choices: Vec<GroqChoice>,
    model: String,
    usage: GroqUsage,
}

impl From<GroqChatResponse> for CompletionResponse {
    fn from(resp: GroqChatResponse) -> Self {
        let choice = resp.choices.into_iter().next();

        let (content, stop_reason) = if let Some(c) = choice {
            let mut blocks = Vec::new();

            // Add text content if present
            if let Some(text) = c.message.content {
                if !text.is_empty() {
                    blocks.push(ContentBlock::Text {
                        text,
                        cache_control: None,
                    });
                }
            }

            // Add tool calls if present
            if let Some(tool_calls) = c.message.tool_calls {
                for tc in tool_calls {
                    let input: serde_json::Value =
                        serde_json::from_str(&tc.function.arguments).unwrap_or_default();
                    blocks.push(ContentBlock::ToolUse {
                        id: tc.id,
                        name: tc.function.name,
                        input,
                        cache_control: None,
                    });
                }
            }

            let stop = match c.finish_reason.as_deref() {
                Some("stop") => Some(StopReason::EndTurn),
                Some("tool_calls") => Some(StopReason::ToolUse),
                Some("length") => Some(StopReason::MaxTokens),
                _ => Some(StopReason::EndTurn),
            };

            (blocks, stop)
        } else {
            (vec![], Some(StopReason::EndTurn))
        };

        CompletionResponse {
            id: resp.id,
            response_type: "message".to_string(),
            role: Role::Assistant,
            content,
            model: resp.model,
            stop_reason,
            usage: Usage {
                input_tokens: resp.usage.prompt_tokens,
                output_tokens: resp.usage.completion_tokens,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
            },
            muninn: None,
        }
    }
}

#[derive(Debug, serde::Deserialize)]
struct GroqChoice {
    message: GroqResponseMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct GroqResponseMessage {
    content: Option<String>,
    tool_calls: Option<Vec<GroqToolCall>>,
}

#[derive(Debug, serde::Deserialize)]
struct GroqUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
}

#[derive(Debug, serde::Deserialize)]
struct GroqErrorResponse {
    error: GroqError,
}

#[derive(Debug, serde::Deserialize)]
struct GroqError {
    message: String,
}

// ============================================================================
// SSE Streaming for Groq
// ============================================================================

fn parse_groq_sse_stream(
    byte_stream: impl Stream<Item = reqwest::Result<Bytes>> + Send + 'static,
) -> ResponseStream {
    Box::pin(futures::stream::unfold(
        GroqSseState {
            byte_stream: Box::pin(byte_stream),
            buffer: String::new(),
            done: false,
            message_id: None,
            model: None,
            current_index: 0,
            started: false,
        },
        |mut state| async move {
            if state.done {
                return None;
            }

            loop {
                // Process lines in buffer
                while let Some(line_end) = state.buffer.find('\n') {
                    let line = state.buffer[..line_end].trim().to_string();
                    state.buffer = state.buffer[line_end + 1..].to_string();

                    if line.is_empty() {
                        continue;
                    }

                    if let Some(data) = line.strip_prefix("data: ") {
                        if data == "[DONE]" {
                            return Some((Ok(StreamEvent::MessageStop), state));
                        }

                        if let Ok(chunk) = serde_json::from_str::<GroqStreamChunk>(data) {
                            // Emit MessageStart on first chunk
                            if !state.started {
                                state.started = true;
                                state.message_id = Some(chunk.id.clone());
                                state.model = Some(chunk.model.clone());
                                return Some((
                                    Ok(StreamEvent::MessageStart {
                                        id: chunk.id,
                                        model: chunk.model,
                                    }),
                                    state,
                                ));
                            }

                            // Process choices
                            if let Some(choice) = chunk.choices.into_iter().next() {
                                if let Some(delta) = choice.delta {
                                    // Text content
                                    if let Some(content) = delta.content {
                                        if !content.is_empty() {
                                            return Some((
                                                Ok(StreamEvent::ContentBlockDelta {
                                                    index: state.current_index,
                                                    delta: ContentDelta::TextDelta(content),
                                                }),
                                                state,
                                            ));
                                        }
                                    }
                                }

                                // Check for finish
                                if let Some(reason) = choice.finish_reason {
                                    let stop_reason = match reason.as_str() {
                                        "stop" => StopReason::EndTurn,
                                        "tool_calls" => StopReason::ToolUse,
                                        "length" => StopReason::MaxTokens,
                                        _ => StopReason::EndTurn,
                                    };
                                    return Some((
                                        Ok(StreamEvent::MessageDelta {
                                            stop_reason,
                                            usage: Usage::new(0, 0),
                                        }),
                                        state,
                                    ));
                                }
                            }
                        }
                    }
                }

                // Need more data
                match state.byte_stream.next().await {
                    Some(Ok(bytes)) => {
                        let text = String::from_utf8_lossy(&bytes);
                        state.buffer.push_str(&text);
                    }
                    Some(Err(e)) => {
                        return Some((Err(RlmError::Network(e.to_string())), state));
                    }
                    None => {
                        return None;
                    }
                }
            }
        },
    ))
}

struct GroqSseState {
    byte_stream: Pin<Box<dyn Stream<Item = reqwest::Result<Bytes>> + Send>>,
    buffer: String,
    done: bool,
    message_id: Option<String>,
    model: Option<String>,
    current_index: usize,
    started: bool,
}

#[derive(Debug, serde::Deserialize)]
struct GroqStreamChunk {
    id: String,
    model: String,
    choices: Vec<GroqStreamChoice>,
}

#[derive(Debug, serde::Deserialize)]
struct GroqStreamChoice {
    delta: Option<GroqStreamDelta>,
    finish_reason: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct GroqStreamDelta {
    content: Option<String>,
}

/// Create a shared Groq backend.
pub fn create_shared_backend(config: GroqConfig) -> Result<Arc<dyn LLMBackend>> {
    Ok(Arc::new(GroqBackend::new(config)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_new() {
        let config = GroqConfig::new("test-key");
        assert_eq!(config.api_key, "test-key");
        assert_eq!(config.base_url, DEFAULT_API_BASE);
    }

    #[test]
    fn test_config_with_base_url() {
        let config = GroqConfig::new("key").with_base_url("http://localhost:8080");
        assert_eq!(config.base_url, "http://localhost:8080");
    }

    #[test]
    fn test_config_with_timeout() {
        let config = GroqConfig::new("key").with_timeout(Duration::from_secs(60));
        assert_eq!(config.timeout, Duration::from_secs(60));
    }

    #[test]
    fn test_completions_url() {
        let config = GroqConfig::new("key");
        let backend = GroqBackend::new(config).unwrap();
        assert_eq!(
            backend.completions_url(),
            "https://api.groq.com/openai/v1/chat/completions"
        );
    }

    #[test]
    fn test_backend_name() {
        let config = GroqConfig::new("key");
        let backend = GroqBackend::new(config).unwrap();
        assert_eq!(backend.name(), "groq");
    }

    #[test]
    fn test_is_retryable() {
        use crate::backend::is_retryable;
        assert!(is_retryable(&RlmError::Network("timeout".to_string())));
        assert!(!is_retryable(&RlmError::Config("bad".to_string())));
    }

    #[test]
    fn test_groq_response_conversion() {
        let groq_resp = GroqChatResponse {
            id: "chatcmpl-123".to_string(),
            choices: vec![GroqChoice {
                message: GroqResponseMessage {
                    content: Some("Hello!".to_string()),
                    tool_calls: None,
                },
                finish_reason: Some("stop".to_string()),
            }],
            model: "llama-3.1-8b-instant".to_string(),
            usage: GroqUsage {
                prompt_tokens: 10,
                completion_tokens: 5,
            },
        };

        let response: CompletionResponse = groq_resp.into();
        assert_eq!(response.id, "chatcmpl-123");
        assert_eq!(response.text(), "Hello!");
        assert_eq!(response.stop_reason, Some(StopReason::EndTurn));
        assert_eq!(response.usage.input_tokens, 10);
        assert_eq!(response.usage.output_tokens, 5);
    }

    #[test]
    fn test_groq_response_with_tool_calls() {
        let groq_resp = GroqChatResponse {
            id: "chatcmpl-456".to_string(),
            choices: vec![GroqChoice {
                message: GroqResponseMessage {
                    content: Some("Let me check.".to_string()),
                    tool_calls: Some(vec![GroqToolCall {
                        id: "call_123".to_string(),
                        call_type: "function".to_string(),
                        function: GroqFunctionCall {
                            name: "read_file".to_string(),
                            arguments: r#"{"path": "/foo.rs"}"#.to_string(),
                        },
                    }]),
                },
                finish_reason: Some("tool_calls".to_string()),
            }],
            model: "llama-3.1-70b-versatile".to_string(),
            usage: GroqUsage {
                prompt_tokens: 50,
                completion_tokens: 30,
            },
        };

        let response: CompletionResponse = groq_resp.into();
        assert!(response.has_tool_use());
        assert_eq!(response.stop_reason, Some(StopReason::ToolUse));

        let tool_uses = response.tool_uses();
        assert_eq!(tool_uses.len(), 1);
        assert_eq!(tool_uses[0].name, "read_file");
    }

    #[test]
    fn test_to_groq_request() {
        let config = GroqConfig::new("key");
        let backend = GroqBackend::new(config).unwrap();

        let request =
            CompletionRequest::new("llama-3.1-8b-instant", vec![Message::user("Hello")], 100);

        let groq_req = backend.to_groq_request(&request);
        assert_eq!(groq_req.model, DEFAULT_MODEL);
        assert_eq!(groq_req.messages.len(), 1);
        assert_eq!(groq_req.messages[0].role, "user");
        assert_eq!(groq_req.max_completion_tokens, Some(100));
    }
}
