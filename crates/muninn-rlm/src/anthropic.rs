//! Anthropic API backend implementation.
//!
//! This module provides the `AnthropicBackend` which connects to Anthropic's
//! Messages API for Claude completions.

use async_trait::async_trait;
use bytes::Bytes;
use futures::{Stream, StreamExt};
use reqwest::{Client, Response, header};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use crate::backend::{ContentDelta, LLMBackend, ResponseStream, StreamEvent, with_retry};
use crate::error::{Result, RlmError};
use crate::types::{CompletionRequest, CompletionResponse, ContentBlock, Role, StopReason, Usage};

/// Default API base URL.
const DEFAULT_API_BASE: &str = "https://api.anthropic.com";

/// Default API version.
const DEFAULT_API_VERSION: &str = "2023-06-01";

/// Default timeout for requests.
const DEFAULT_TIMEOUT_SECS: u64 = 300;

/// Configuration for the Anthropic backend.
#[derive(Debug, Clone)]
pub struct AnthropicConfig {
    /// API key for authentication.
    pub api_key: String,

    /// Base URL for the API.
    pub base_url: String,

    /// API version header.
    pub api_version: String,

    /// Request timeout.
    pub timeout: Duration,

    /// Maximum retries for transient errors.
    pub max_retries: u32,

    /// Initial backoff duration for retries.
    pub retry_backoff: Duration,
}

impl AnthropicConfig {
    /// Create a new config with the given API key.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: DEFAULT_API_BASE.to_string(),
            api_version: DEFAULT_API_VERSION.to_string(),
            timeout: Duration::from_secs(DEFAULT_TIMEOUT_SECS),
            max_retries: 3,
            retry_backoff: Duration::from_millis(500),
        }
    }

    /// Create config from environment variable.
    pub fn from_env() -> Result<Self> {
        let api_key = std::env::var("ANTHROPIC_API_KEY").map_err(|_| {
            RlmError::Config("ANTHROPIC_API_KEY environment variable not set".to_string())
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

/// Anthropic API backend.
pub struct AnthropicBackend {
    client: Client,
    config: AnthropicConfig,
}

impl AnthropicBackend {
    /// Create a new Anthropic backend with the given configuration.
    pub fn new(config: AnthropicConfig) -> Result<Self> {
        let client = Client::builder()
            .timeout(config.timeout)
            .build()
            .map_err(|e| RlmError::Internal(format!("Failed to create HTTP client: {}", e)))?;

        Ok(Self { client, config })
    }

    /// Create a backend from environment configuration.
    pub fn from_env() -> Result<Self> {
        Self::new(AnthropicConfig::from_env()?)
    }

    /// Build the messages endpoint URL.
    fn messages_url(&self) -> String {
        format!("{}/v1/messages", self.config.base_url)
    }

    /// Add authentication and API headers to a request.
    fn add_headers(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        builder
            .header("x-api-key", &self.config.api_key)
            .header("anthropic-version", &self.config.api_version)
            .header(header::CONTENT_TYPE, "application/json")
    }
}

#[async_trait]
impl LLMBackend for AnthropicBackend {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse> {
        // Ensure streaming is off for this method
        let mut request = request;
        request.stream = false;

        with_retry(
            self.config.max_retries,
            self.config.retry_backoff,
            "anthropic",
            || async {
                let response = self
                    .add_headers(self.client.post(self.messages_url()))
                    .json(&request)
                    .send()
                    .await?;

                Self::handle_response(response).await
            },
        )
        .await
    }

    async fn complete_stream(&self, request: CompletionRequest) -> Result<ResponseStream> {
        // Ensure streaming is on
        let mut request = request;
        request.stream = true;

        let response = self
            .add_headers(self.client.post(self.messages_url()))
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(Self::handle_error_response(response).await);
        }

        Ok(parse_sse_stream(response.bytes_stream()))
    }

    fn name(&self) -> &str {
        "anthropic"
    }

    async fn health_check(&self) -> Result<()> {
        // Make a minimal request to check connectivity
        // We use a very short max_tokens to minimize cost
        let request = CompletionRequest::new(
            "claude-3-haiku-20240307",
            vec![crate::types::Message::user("ping")],
            1,
        );

        match self.complete(request).await {
            Ok(_) => Ok(()),
            Err(RlmError::Backend(msg)) if msg.contains("rate limit") => {
                // Rate limit means the API is reachable
                Ok(())
            }
            Err(e) => Err(e),
        }
    }
}

impl AnthropicBackend {
    /// Handle a successful response.
    async fn handle_response(response: Response) -> Result<CompletionResponse> {
        if !response.status().is_success() {
            return Err(Self::handle_error_response(response).await);
        }

        let body = response.text().await?;
        let parsed: ApiResponse =
            serde_json::from_str(&body).map_err(|e| RlmError::Serialization(e.to_string()))?;

        Ok(parsed.into())
    }

    /// Handle an error response.
    async fn handle_error_response(response: Response) -> RlmError {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();

        // Try to parse as API error
        if let Ok(error) = serde_json::from_str::<ApiError>(&body) {
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

/// Internal API response structure.
#[derive(Debug, serde::Deserialize)]
struct ApiResponse {
    id: String,
    #[serde(rename = "type")]
    response_type: String,
    #[allow(dead_code)]
    role: String,
    content: Vec<ApiContentBlock>,
    model: String,
    stop_reason: Option<String>,
    usage: ApiUsage,
}

impl From<ApiResponse> for CompletionResponse {
    fn from(api: ApiResponse) -> Self {
        let content = api
            .content
            .into_iter()
            .map(|block| match block {
                ApiContentBlock::Text { text } => ContentBlock::Text {
                    text,
                    cache_control: None,
                },
                ApiContentBlock::ToolUse { id, name, input } => ContentBlock::ToolUse {
                    id,
                    name,
                    input,
                    cache_control: None,
                },
            })
            .collect();

        let stop_reason = api.stop_reason.as_deref().map(|s| match s {
            "end_turn" => StopReason::EndTurn,
            "tool_use" => StopReason::ToolUse,
            "max_tokens" => StopReason::MaxTokens,
            "stop_sequence" => StopReason::StopSequence,
            _ => StopReason::EndTurn,
        });

        CompletionResponse {
            id: api.id,
            response_type: api.response_type,
            role: Role::Assistant,
            content,
            model: api.model,
            stop_reason,
            usage: Usage {
                input_tokens: api.usage.input_tokens,
                output_tokens: api.usage.output_tokens,
                cache_creation_input_tokens: api.usage.cache_creation_input_tokens.unwrap_or(0),
                cache_read_input_tokens: api.usage.cache_read_input_tokens.unwrap_or(0),
            },
            muninn: None,
        }
    }
}

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ApiContentBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
}

#[derive(Debug, serde::Deserialize)]
struct ApiUsage {
    input_tokens: u32,
    output_tokens: u32,
    cache_creation_input_tokens: Option<u32>,
    cache_read_input_tokens: Option<u32>,
}

#[derive(Debug, serde::Deserialize)]
struct ApiError {
    error: ApiErrorDetail,
}

#[derive(Debug, serde::Deserialize)]
struct ApiErrorDetail {
    #[serde(rename = "type")]
    #[allow(dead_code)]
    error_type: String,
    message: String,
}

/// Parse SSE events from a byte stream and convert to StreamEvents.
#[allow(unused_assignments)]
fn parse_sse_stream(
    byte_stream: impl Stream<Item = reqwest::Result<Bytes>> + Send + 'static,
) -> ResponseStream {
    Box::pin(futures::stream::unfold(
        SseState {
            byte_stream: Box::pin(byte_stream),
            buffer: String::new(),
            current_event: None,
            done: false,
        },
        |mut state| async move {
            if state.done {
                return None;
            }

            loop {
                // First, try to process any complete events in the buffer
                while let Some(line_end) = state.buffer.find('\n') {
                    let line = state.buffer[..line_end].trim().to_string();
                    state.buffer = state.buffer[line_end + 1..].to_string();

                    if line.is_empty() {
                        // Empty line marks end of event, clear current event
                        state.current_event = None;
                        continue;
                    }

                    if let Some((key, value)) = parse_sse_line(&line) {
                        match key {
                            "event" => {
                                state.current_event = Some(value.to_string());
                            }
                            "data" => {
                                if let Some(event_type) = &state.current_event {
                                    if let Some(event) = parse_stream_event(event_type, value) {
                                        if matches!(event, StreamEvent::MessageStop) {
                                            state.done = true;
                                        }
                                        return Some((Ok(event), state));
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }

                // Need more data from the byte stream
                match state.byte_stream.next().await {
                    Some(Ok(bytes)) => {
                        let text = String::from_utf8_lossy(&bytes);
                        state.buffer.push_str(&text);
                    }
                    Some(Err(e)) => {
                        state.done = true;
                        return Some((Err(RlmError::Network(e.to_string())), state));
                    }
                    None => {
                        state.done = true;
                        return None;
                    }
                }
            }
        },
    ))
}

struct SseState {
    byte_stream: Pin<Box<dyn Stream<Item = reqwest::Result<Bytes>> + Send>>,
    buffer: String,
    current_event: Option<String>,
    done: bool,
}

fn parse_sse_line(line: &str) -> Option<(&str, &str)> {
    if let Some(value) = line.strip_prefix("event: ") {
        Some(("event", value))
    } else if let Some(value) = line.strip_prefix("data: ") {
        Some(("data", value))
    } else {
        None
    }
}

fn parse_stream_event(event_type: &str, data: &str) -> Option<StreamEvent> {
    match event_type {
        "message_start" => {
            if let Ok(parsed) = serde_json::from_str::<MessageStartEvent>(data) {
                Some(StreamEvent::MessageStart {
                    id: parsed.message.id,
                    model: parsed.message.model,
                })
            } else {
                None
            }
        }
        "content_block_start" => {
            if let Ok(parsed) = serde_json::from_str::<ContentBlockStartEvent>(data) {
                Some(StreamEvent::ContentBlockStart {
                    index: parsed.index,
                    content_type: parsed.content_block.block_type,
                })
            } else {
                None
            }
        }
        "content_block_delta" => {
            if let Ok(parsed) = serde_json::from_str::<ContentBlockDeltaEvent>(data) {
                let delta = match parsed.delta {
                    DeltaContent::TextDelta { text } => ContentDelta::TextDelta(text),
                    DeltaContent::InputJsonDelta { partial_json } => {
                        ContentDelta::InputJsonDelta(partial_json)
                    }
                };
                Some(StreamEvent::ContentBlockDelta {
                    index: parsed.index,
                    delta,
                })
            } else {
                None
            }
        }
        "content_block_stop" => {
            if let Ok(parsed) = serde_json::from_str::<ContentBlockStopEvent>(data) {
                Some(StreamEvent::ContentBlockStop {
                    index: parsed.index,
                })
            } else {
                None
            }
        }
        "message_delta" => {
            if let Ok(parsed) = serde_json::from_str::<MessageDeltaEvent>(data) {
                let stop_reason = match parsed.delta.stop_reason.as_deref() {
                    Some("end_turn") => StopReason::EndTurn,
                    Some("tool_use") => StopReason::ToolUse,
                    Some("max_tokens") => StopReason::MaxTokens,
                    Some("stop_sequence") => StopReason::StopSequence,
                    _ => StopReason::EndTurn,
                };
                Some(StreamEvent::MessageDelta {
                    stop_reason,
                    usage: Usage::new(0, parsed.usage.output_tokens),
                })
            } else {
                None
            }
        }
        "message_stop" => Some(StreamEvent::MessageStop),
        "ping" => Some(StreamEvent::Ping),
        "error" => {
            if let Ok(parsed) = serde_json::from_str::<StreamErrorEvent>(data) {
                Some(StreamEvent::Error {
                    message: parsed.error.message,
                })
            } else {
                Some(StreamEvent::Error {
                    message: "Unknown streaming error".to_string(),
                })
            }
        }
        _ => None,
    }
}

// SSE event structures

#[derive(Debug, serde::Deserialize)]
struct MessageStartEvent {
    message: MessageStartMessage,
}

#[derive(Debug, serde::Deserialize)]
struct MessageStartMessage {
    id: String,
    model: String,
}

#[derive(Debug, serde::Deserialize)]
struct ContentBlockStartEvent {
    index: usize,
    content_block: ContentBlockType,
}

#[derive(Debug, serde::Deserialize)]
struct ContentBlockType {
    #[serde(rename = "type")]
    block_type: String,
}

#[derive(Debug, serde::Deserialize)]
struct ContentBlockDeltaEvent {
    index: usize,
    delta: DeltaContent,
}

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum DeltaContent {
    TextDelta { text: String },
    InputJsonDelta { partial_json: String },
}

#[derive(Debug, serde::Deserialize)]
struct ContentBlockStopEvent {
    index: usize,
}

#[derive(Debug, serde::Deserialize)]
struct MessageDeltaEvent {
    delta: MessageDelta,
    usage: MessageDeltaUsage,
}

#[derive(Debug, serde::Deserialize)]
struct MessageDelta {
    stop_reason: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct MessageDeltaUsage {
    output_tokens: u32,
}

#[derive(Debug, serde::Deserialize)]
struct StreamErrorEvent {
    error: StreamErrorDetail,
}

#[derive(Debug, serde::Deserialize)]
struct StreamErrorDetail {
    #[serde(rename = "type")]
    #[allow(dead_code)]
    error_type: String,
    message: String,
}

/// Create a shared Anthropic backend.
pub fn create_shared_backend(config: AnthropicConfig) -> Result<Arc<dyn LLMBackend>> {
    Ok(Arc::new(AnthropicBackend::new(config)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_new() {
        let config = AnthropicConfig::new("test-key");
        assert_eq!(config.api_key, "test-key");
        assert_eq!(config.base_url, DEFAULT_API_BASE);
        assert_eq!(config.api_version, DEFAULT_API_VERSION);
    }

    #[test]
    fn test_config_with_base_url() {
        let config = AnthropicConfig::new("key").with_base_url("http://localhost:8080");
        assert_eq!(config.base_url, "http://localhost:8080");
    }

    #[test]
    fn test_config_with_timeout() {
        let config = AnthropicConfig::new("key").with_timeout(Duration::from_secs(60));
        assert_eq!(config.timeout, Duration::from_secs(60));
    }

    #[test]
    fn test_parse_sse_line() {
        assert_eq!(
            super::parse_sse_line("event: message_start"),
            Some(("event", "message_start"))
        );
        assert_eq!(
            super::parse_sse_line("data: {\"foo\": 1}"),
            Some(("data", "{\"foo\": 1}"))
        );
        assert_eq!(super::parse_sse_line("invalid"), None);
    }

    #[test]
    fn test_api_response_conversion() {
        let api_response = ApiResponse {
            id: "msg_123".to_string(),
            response_type: "message".to_string(),
            role: "assistant".to_string(),
            content: vec![ApiContentBlock::Text {
                text: "Hello!".to_string(),
            }],
            model: "claude-3-sonnet-20240229".to_string(),
            stop_reason: Some("end_turn".to_string()),
            usage: ApiUsage {
                input_tokens: 10,
                output_tokens: 5,
                cache_creation_input_tokens: None,
                cache_read_input_tokens: None,
            },
        };

        let response: CompletionResponse = api_response.into();
        assert_eq!(response.id, "msg_123");
        assert_eq!(response.text(), "Hello!");
        assert_eq!(response.stop_reason, Some(StopReason::EndTurn));
        assert_eq!(response.usage.input_tokens, 10);
        assert_eq!(response.usage.output_tokens, 5);
    }

    #[test]
    fn test_api_response_with_tool_use() {
        let api_response = ApiResponse {
            id: "msg_456".to_string(),
            response_type: "message".to_string(),
            role: "assistant".to_string(),
            content: vec![
                ApiContentBlock::Text {
                    text: "Let me check.".to_string(),
                },
                ApiContentBlock::ToolUse {
                    id: "tool_1".to_string(),
                    name: "read_file".to_string(),
                    input: serde_json::json!({"path": "/foo.rs"}),
                },
            ],
            model: "claude-3-sonnet-20240229".to_string(),
            stop_reason: Some("tool_use".to_string()),
            usage: ApiUsage {
                input_tokens: 50,
                output_tokens: 30,
                cache_creation_input_tokens: Some(100),
                cache_read_input_tokens: Some(50),
            },
        };

        let response: CompletionResponse = api_response.into();
        assert!(response.has_tool_use());
        assert_eq!(response.stop_reason, Some(StopReason::ToolUse));
        assert_eq!(response.usage.cache_creation_input_tokens, 100);
        assert_eq!(response.usage.cache_read_input_tokens, 50);

        let tool_uses = response.tool_uses();
        assert_eq!(tool_uses.len(), 1);
        assert_eq!(tool_uses[0].name, "read_file");
    }

    #[test]
    fn test_messages_url() {
        let config = AnthropicConfig::new("key");
        let backend = AnthropicBackend::new(config).unwrap();
        assert_eq!(
            backend.messages_url(),
            "https://api.anthropic.com/v1/messages"
        );
    }

    #[test]
    fn test_messages_url_custom_base() {
        let config = AnthropicConfig::new("key").with_base_url("http://localhost:8080");
        let backend = AnthropicBackend::new(config).unwrap();
        assert_eq!(backend.messages_url(), "http://localhost:8080/v1/messages");
    }

    #[test]
    fn test_backend_name() {
        let config = AnthropicConfig::new("key");
        let backend = AnthropicBackend::new(config).unwrap();
        assert_eq!(backend.name(), "anthropic");
    }

    #[test]
    fn test_is_retryable() {
        use crate::backend::is_retryable;
        assert!(is_retryable(&RlmError::Network("timeout".to_string())));
        assert!(!is_retryable(&RlmError::Config("bad config".to_string())));
        assert!(!is_retryable(&RlmError::InvalidRequest(
            "bad request".to_string()
        )));
    }
}
