//! Ollama API backend implementation.
//!
//! This module provides the `OllamaBackend` which connects to Ollama's
//! OpenAI-compatible API for local LLM inference.

use async_trait::async_trait;
use reqwest::{Client, header};
use std::time::Duration;

use crate::backend::{LLMBackend, ResponseStream, StreamEvent, with_retry};
use crate::error::{Result, RlmError};
use crate::types::{
    CompletionRequest, CompletionResponse, ContentBlock, Role, StopReason, ToolResultContent, Usage,
};

/// Default Ollama API base URL.
const DEFAULT_API_BASE: &str = "http://localhost:11434/v1";

/// Default timeout for requests (longer for local inference).
const DEFAULT_TIMEOUT_SECS: u64 = 600;

/// Default model for Ollama backend.
const DEFAULT_MODEL: &str = "gpt-oss:20b";

/// Configuration for the Ollama backend.
#[derive(Debug, Clone)]
pub struct OllamaConfig {
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

impl Default for OllamaConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_API_BASE.to_string(),
            model: DEFAULT_MODEL.to_string(),
            timeout: Duration::from_secs(DEFAULT_TIMEOUT_SECS),
            max_retries: 3,
            retry_backoff: Duration::from_millis(500),
        }
    }
}

impl OllamaConfig {
    /// Create a new config with defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the model to use.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
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

/// Ollama API backend.
pub struct OllamaBackend {
    client: Client,
    config: OllamaConfig,
}

impl OllamaBackend {
    /// Create a new Ollama backend with the given configuration.
    pub fn new(config: OllamaConfig) -> Result<Self> {
        let client = Client::builder()
            .timeout(config.timeout)
            .build()
            .map_err(|e| RlmError::Internal(format!("Failed to create HTTP client: {}", e)))?;

        Ok(Self { client, config })
    }

    /// Build the chat completions endpoint URL.
    fn completions_url(&self) -> String {
        format!("{}/chat/completions", self.config.base_url)
    }

    /// Add headers to a request.
    fn add_headers(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        builder.header(header::CONTENT_TYPE, "application/json")
    }

    /// Convert our CompletionRequest to Ollama's OpenAI-compatible format.
    fn to_ollama_request(&self, request: &CompletionRequest) -> OllamaChatRequest {
        let mut messages: Vec<OllamaMessage> = Vec::new();

        // Add system message if present
        if let Some(ref system) = request.system {
            messages.push(OllamaMessage {
                role: "system".to_string(),
                content: Some(system.to_text()),
                tool_calls: None,
                tool_call_id: None,
            });
        }

        // Add conversation messages with proper tool handling
        for m in &request.messages {
            let blocks = m.content.blocks();
            let role_str = match m.role {
                Role::User => "user",
                Role::Assistant => "assistant",
            };

            // Check if this message contains tool results
            let tool_results: Vec<_> = blocks
                .iter()
                .filter_map(|b| {
                    if let ContentBlock::ToolResult {
                        tool_use_id,
                        content,
                        ..
                    } = b
                    {
                        // Convert ToolResultContent to String
                        let content_str = content.as_ref().map(|c| match c {
                            ToolResultContent::Text(s) => s.clone(),
                            ToolResultContent::Blocks(blocks) => {
                                serde_json::to_string(blocks).unwrap_or_default()
                            }
                        });
                        Some((tool_use_id.clone(), content_str))
                    } else {
                        None
                    }
                })
                .collect();

            // If we have tool results, add them as separate tool messages
            if !tool_results.is_empty() {
                for (tool_use_id, content) in tool_results {
                    messages.push(OllamaMessage {
                        role: "tool".to_string(),
                        content,
                        tool_calls: None,
                        tool_call_id: Some(tool_use_id),
                    });
                }
                continue;
            }

            // Check for tool calls in assistant messages
            let tool_calls: Vec<_> = blocks
                .iter()
                .filter_map(|b| {
                    if let ContentBlock::ToolUse {
                        id, name, input, ..
                    } = b
                    {
                        Some(OllamaToolCall {
                            id: id.clone(),
                            call_type: "function".to_string(),
                            function: OllamaFunctionCall {
                                name: name.clone(),
                                arguments: serde_json::to_string(input).unwrap_or_default(),
                            },
                        })
                    } else {
                        None
                    }
                })
                .collect();

            // Extract text content
            let text_content: String = blocks
                .iter()
                .filter_map(|b| {
                    if let ContentBlock::Text { text, .. } = b {
                        Some(text.as_str())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("");

            messages.push(OllamaMessage {
                role: role_str.to_string(),
                content: if text_content.is_empty() {
                    None
                } else {
                    Some(text_content)
                },
                tool_calls: if tool_calls.is_empty() {
                    None
                } else {
                    Some(tool_calls)
                },
                tool_call_id: None,
            });
        }

        // Convert tools to OpenAI format
        let tools: Option<Vec<OllamaTool>> = if request.tools.is_empty() {
            None
        } else {
            Some(
                request
                    .tools
                    .iter()
                    .map(|t| OllamaTool {
                        tool_type: "function".to_string(),
                        function: OllamaFunction {
                            name: t.name.clone(),
                            description: Some(t.description.clone()),
                            parameters: t.input_schema.clone(),
                        },
                    })
                    .collect(),
            )
        };

        OllamaChatRequest {
            model: self.config.model.clone(),
            messages,
            max_tokens: Some(request.max_tokens),
            temperature: request.temperature,
            stream: Some(false),
            tools,
        }
    }

    /// Parse Ollama response into our format.
    fn parse_response(&self, response: OllamaChatResponse) -> CompletionResponse {
        let choice = response.choices.into_iter().next();

        let (content, stop_reason) = match choice {
            Some(c) => {
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

                let stop_reason = match c.finish_reason.as_deref() {
                    Some("stop") => StopReason::EndTurn,
                    Some("length") => StopReason::MaxTokens,
                    Some("tool_calls") => StopReason::ToolUse,
                    _ => StopReason::EndTurn,
                };

                (blocks, stop_reason)
            }
            None => (Vec::new(), StopReason::EndTurn),
        };

        let usage = response.usage.map(|u| Usage {
            input_tokens: u.prompt_tokens,
            output_tokens: u.completion_tokens,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
        });

        CompletionResponse {
            id: response.id,
            response_type: "message".to_string(),
            role: Role::Assistant,
            content,
            model: response.model,
            stop_reason: Some(stop_reason),
            usage: usage.unwrap_or_default(),
            muninn: None,
        }
    }

    /// Make a non-streaming request.
    async fn send_request(&self, request: &CompletionRequest) -> Result<CompletionResponse> {
        let ollama_request = self.to_ollama_request(request);
        let url = self.completions_url();

        tracing::debug!(
            model = %self.config.model,
            messages = ollama_request.messages.len(),
            "Ollama request"
        );

        let response = self
            .add_headers(self.client.post(&url))
            .json(&ollama_request)
            .send()
            .await
            .map_err(|e| RlmError::Network(format!("Ollama request failed: {}", e)))?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(RlmError::Backend(format!(
                "Ollama API error ({}): {}",
                status.as_u16(),
                body
            )));
        }

        let ollama_response: OllamaChatResponse = response
            .json()
            .await
            .map_err(|e| RlmError::Serialization(format!("Failed to parse response: {}", e)))?;

        Ok(self.parse_response(ollama_response))
    }
}

#[async_trait]
impl LLMBackend for OllamaBackend {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse> {
        with_retry(
            self.config.max_retries,
            self.config.retry_backoff,
            "ollama",
            || self.send_request(&request),
        )
        .await
    }

    async fn complete_stream(&self, request: CompletionRequest) -> Result<ResponseStream> {
        // For now, use non-streaming and emit as single event
        // TODO: Implement proper streaming
        let response = self.complete(request).await?;

        let events = vec![
            Ok(StreamEvent::MessageStart {
                id: response.id.clone(),
                model: response.model.clone(),
            }),
            Ok(StreamEvent::MessageDelta {
                stop_reason: response.stop_reason.unwrap_or(StopReason::EndTurn),
                usage: response.usage.clone(),
            }),
            Ok(StreamEvent::MessageStop),
        ];

        Ok(Box::pin(futures::stream::iter(events)))
    }

    fn name(&self) -> &str {
        "ollama"
    }

    async fn health_check(&self) -> Result<()> {
        // Try to hit the models endpoint to check if Ollama is running
        let url = format!("{}/models", self.config.base_url.trim_end_matches("/v1"));

        self.client
            .get(&url)
            .send()
            .await
            .map_err(|e| RlmError::Network(format!("Ollama health check failed: {}", e)))?;

        Ok(())
    }

    fn supports_native_tools(&self) -> bool {
        // Ollama supports native tool calling
        true
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Ollama API Types (OpenAI-compatible)
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, serde::Serialize)]
struct OllamaChatRequest {
    model: String,
    messages: Vec<OllamaMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<OllamaTool>>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct OllamaMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OllamaToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct OllamaToolCall {
    id: String,
    #[serde(rename = "type")]
    call_type: String,
    function: OllamaFunctionCall,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct OllamaFunctionCall {
    name: String,
    arguments: String,
}

#[derive(Debug, serde::Serialize)]
struct OllamaTool {
    #[serde(rename = "type")]
    tool_type: String,
    function: OllamaFunction,
}

#[derive(Debug, serde::Serialize)]
struct OllamaFunction {
    name: String,
    description: Option<String>,
    parameters: serde_json::Value,
}

#[derive(Debug, serde::Deserialize)]
struct OllamaChatResponse {
    id: String,
    model: String,
    choices: Vec<OllamaChoice>,
    usage: Option<OllamaUsage>,
}

#[derive(Debug, serde::Deserialize)]
struct OllamaChoice {
    message: OllamaMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct OllamaUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_defaults() {
        let config = OllamaConfig::new();
        assert_eq!(config.base_url, "http://localhost:11434/v1");
        assert_eq!(config.model, "gpt-oss:20b");
    }

    #[test]
    fn test_config_builder() {
        let config = OllamaConfig::new()
            .with_model("qwen2.5-coder:7b")
            .with_base_url("http://192.168.1.100:11434/v1");

        assert_eq!(config.model, "qwen2.5-coder:7b");
        assert_eq!(config.base_url, "http://192.168.1.100:11434/v1");
    }
}
