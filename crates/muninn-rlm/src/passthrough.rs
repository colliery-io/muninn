//! Passthrough client for forwarding requests to upstream APIs.
//!
//! This module provides direct forwarding to upstream API endpoints,
//! supporting both:
//! - API key authentication (x-api-key header passthrough)
//! - OAuth Bearer token authentication (for Claude MAX plan)
//!
//! For OAuth/MAX plan, the module automatically:
//! - Injects required system prompt for Claude Code
//! - Adds required anthropic-beta headers
//! - Uses Bearer token authentication

use reqwest::{Client, header};
use serde::Serialize;
use std::collections::HashMap;

use crate::error::{Result, RlmError};
use crate::token_manager::SharedTokenManager;
use crate::types::{CompletionRequest, CompletionResponse};

/// Known API providers with their default configurations.
#[derive(Debug, Clone, PartialEq)]
pub enum ApiProvider {
    /// Anthropic API (Claude)
    Anthropic,
    /// OpenAI-compatible API
    OpenAI,
    /// Custom provider with manual configuration
    Custom,
}

/// Default Anthropic API base URL.
pub const ANTHROPIC_API_URL: &str = "https://api.anthropic.com";
/// Default OpenAI API base URL.
pub const OPENAI_API_URL: &str = "https://api.openai.com";

/// Anthropic API version header value.
pub const ANTHROPIC_VERSION: &str = "2023-06-01";
/// Required anthropic-beta header for OAuth/MAX plan.
pub const ANTHROPIC_BETA: &str = "oauth-2025-04-20,claude-code-20250219,interleaved-thinking-2025-05-14,fine-grained-tool-streaming-2025-05-14";

/// Required system prompt text for Claude Code with MAX plan.
/// This MUST be the first element in the system array for OAuth requests.
pub const CLAUDE_CODE_SYSTEM_PROMPT: &str =
    "You are Claude Code, Anthropic's official CLI for Claude.";

/// Authentication mode for passthrough requests.
#[derive(Debug, Clone, PartialEq)]
pub enum AuthMode {
    /// Use API key from request headers (x-api-key or Authorization).
    ApiKey,
    /// Use OAuth Bearer token from token manager.
    OAuth,
    /// Try OAuth first, fall back to API key from headers.
    OAuthWithFallback,
}

/// Configuration for the passthrough client.
#[derive(Debug, Clone)]
pub struct PassthroughConfig {
    /// Base URL for the upstream API.
    pub base_url: String,
    /// API provider type (affects header handling).
    pub provider: ApiProvider,
    /// Messages endpoint path (default: /v1/messages for Anthropic, /v1/chat/completions for OpenAI).
    pub messages_path: String,
    /// Header name for API key (default: x-api-key for Anthropic, Authorization for OpenAI).
    pub auth_header: String,
    /// Additional headers to include in requests.
    pub extra_headers: HashMap<String, String>,
    /// Authentication mode.
    pub auth_mode: AuthMode,
    /// Whether to inject the required Claude Code system prompt (for OAuth/MAX).
    pub inject_system_prompt: bool,
}

impl PassthroughConfig {
    /// Create config for Anthropic API with API key auth.
    pub fn anthropic() -> Self {
        let mut extra_headers = HashMap::new();
        extra_headers.insert(
            "anthropic-version".to_string(),
            ANTHROPIC_VERSION.to_string(),
        );

        Self {
            base_url: ANTHROPIC_API_URL.to_string(),
            provider: ApiProvider::Anthropic,
            messages_path: "/v1/messages".to_string(),
            auth_header: "x-api-key".to_string(),
            extra_headers,
            auth_mode: AuthMode::ApiKey,
            inject_system_prompt: false,
        }
    }

    /// Create config for Anthropic API with OAuth (MAX plan).
    pub fn anthropic_oauth() -> Self {
        let mut extra_headers = HashMap::new();
        extra_headers.insert(
            "anthropic-version".to_string(),
            ANTHROPIC_VERSION.to_string(),
        );
        extra_headers.insert("anthropic-beta".to_string(), ANTHROPIC_BETA.to_string());

        Self {
            base_url: ANTHROPIC_API_URL.to_string(),
            provider: ApiProvider::Anthropic,
            messages_path: "/v1/messages".to_string(),
            auth_header: "Authorization".to_string(), // OAuth uses Bearer in Authorization
            extra_headers,
            auth_mode: AuthMode::OAuthWithFallback,
            inject_system_prompt: true,
        }
    }

    /// Create config for OpenAI-compatible API.
    pub fn openai() -> Self {
        Self {
            base_url: OPENAI_API_URL.to_string(),
            provider: ApiProvider::OpenAI,
            messages_path: "/v1/chat/completions".to_string(),
            auth_header: "Authorization".to_string(),
            extra_headers: HashMap::new(),
            auth_mode: AuthMode::ApiKey,
            inject_system_prompt: false,
        }
    }

    /// Create config for a custom endpoint.
    pub fn custom(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            provider: ApiProvider::Custom,
            messages_path: "/v1/messages".to_string(),
            auth_header: "x-api-key".to_string(),
            extra_headers: HashMap::new(),
            auth_mode: AuthMode::ApiKey,
            inject_system_prompt: false,
        }
    }

    /// Set the base URL.
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    /// Set the messages endpoint path.
    pub fn with_messages_path(mut self, path: impl Into<String>) -> Self {
        self.messages_path = path.into();
        self
    }

    /// Set the auth header name.
    pub fn with_auth_header(mut self, header: impl Into<String>) -> Self {
        self.auth_header = header.into();
        self
    }

    /// Add an extra header.
    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.extra_headers.insert(key.into(), value.into());
        self
    }

    /// Set the authentication mode.
    pub fn with_auth_mode(mut self, mode: AuthMode) -> Self {
        self.auth_mode = mode;
        self
    }

    /// Enable or disable system prompt injection.
    pub fn with_system_prompt_injection(mut self, inject: bool) -> Self {
        self.inject_system_prompt = inject;
        self
    }
}

impl Default for PassthroughConfig {
    fn default() -> Self {
        // Default to OAuth mode for Claude MAX support
        Self::anthropic_oauth()
    }
}

/// Passthrough client for forwarding requests to upstream APIs.
#[derive(Debug)]
pub struct Passthrough {
    client: Client,
    config: PassthroughConfig,
    /// Token manager for OAuth authentication.
    token_manager: Option<SharedTokenManager>,
}

impl Clone for Passthrough {
    fn clone(&self) -> Self {
        Self {
            client: self.client.clone(),
            config: self.config.clone(),
            token_manager: self.token_manager.clone(),
        }
    }
}

impl Passthrough {
    /// Create a new passthrough client with default config (OAuth mode).
    pub fn new() -> Self {
        Self::with_config(PassthroughConfig::default())
    }

    /// Create a new passthrough client for Anthropic with API key auth.
    pub fn anthropic() -> Self {
        Self::with_config(PassthroughConfig::anthropic())
    }

    /// Create a new passthrough client for Anthropic with OAuth (MAX plan).
    pub fn anthropic_oauth() -> Self {
        Self::with_config(PassthroughConfig::anthropic_oauth())
    }

    /// Create a new passthrough client with custom config.
    pub fn with_config(config: PassthroughConfig) -> Self {
        Self {
            client: Client::new(),
            config,
            token_manager: None,
        }
    }

    /// Set the token manager for OAuth authentication.
    pub fn with_token_manager(mut self, manager: SharedTokenManager) -> Self {
        self.token_manager = Some(manager);
        self
    }

    /// Create with a custom base URL (convenience method).
    pub fn with_base_url(base_url: impl Into<String>) -> Self {
        Self::with_config(PassthroughConfig::anthropic().with_base_url(base_url))
    }

    /// Get the config.
    pub fn config(&self) -> &PassthroughConfig {
        &self.config
    }

    /// Get the token manager if set.
    pub fn token_manager(&self) -> Option<&SharedTokenManager> {
        self.token_manager.as_ref()
    }

    /// Forward a completion request to the upstream API.
    ///
    /// # Arguments
    /// * `request` - The completion request to forward
    /// * `api_key` - Optional API key from request headers (used for fallback or ApiKey mode)
    pub async fn forward(
        &self,
        request: &CompletionRequest,
        api_key: Option<&str>,
    ) -> Result<CompletionResponse> {
        let url = format!("{}{}", self.config.base_url, self.config.messages_path);

        // Strip muninn-specific fields and optionally inject system prompt
        let forward_request = self.prepare_request(request);

        // Build the request
        let mut req = self
            .client
            .post(&url)
            .header(header::CONTENT_TYPE, "application/json");

        // Get auth token based on mode
        let auth_value = self.get_auth_value(api_key).await?;
        req = req.header(&self.config.auth_header, &auth_value);

        // Add extra headers
        for (key, value) in &self.config.extra_headers {
            req = req.header(key, value);
        }

        let response = req
            .json(&forward_request)
            .send()
            .await
            .map_err(|e| RlmError::Backend(format!("Failed to forward request: {}", e)))?;

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| RlmError::Backend(format!("Failed to read response: {}", e)))?;

        if !status.is_success() {
            return Err(RlmError::Backend(format!(
                "Upstream API error ({}): {}",
                status, body
            )));
        }

        let completion: CompletionResponse = serde_json::from_str(&body)
            .map_err(|e| RlmError::Backend(format!("Failed to parse response: {}", e)))?;

        Ok(completion)
    }

    /// Forward a raw JSON request to the upstream API.
    ///
    /// This method is preferred for passthrough mode as it doesn't require
    /// strict typing of all message content types (thinking blocks, images, etc).
    ///
    /// # Arguments
    /// * `request` - Raw JSON request body
    /// * `api_key` - Optional API key from request headers (used for fallback or ApiKey mode)
    pub async fn forward_raw(
        &self,
        request: serde_json::Value,
        api_key: Option<&str>,
    ) -> Result<serde_json::Value> {
        let url = format!("{}{}", self.config.base_url, self.config.messages_path);

        // Extract model for logging
        let model = request
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        // Check if this is a streaming request
        let is_streaming = request
            .get("stream")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if is_streaming {
            return Err(RlmError::InvalidRequest(
                "Streaming requests should use forward_raw_stream".to_string(),
            ));
        }

        tracing::debug!(
            url = %url,
            model = %model,
            auth_mode = ?self.config.auth_mode,
            "Forwarding raw request"
        );

        // Prepare the request - strip unknown fields, inject system prompt
        let forward_request = self.prepare_raw_request(request);

        // Build the request
        let mut req = self
            .client
            .post(&url)
            .header(header::CONTENT_TYPE, "application/json");

        // Get auth token based on mode
        let auth_value = match self.get_auth_value(api_key).await {
            Ok(v) => v,
            Err(e) => {
                tracing::error!(error = %e, "Failed to get auth value");
                return Err(e);
            }
        };
        req = req.header(&self.config.auth_header, &auth_value);

        // Add extra headers
        for (key, value) in &self.config.extra_headers {
            req = req.header(key, value);
        }

        let response = match req.json(&forward_request).send().await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!(error = %e, url = %url, "Failed to send request to upstream");
                return Err(RlmError::Backend(format!(
                    "Failed to forward request: {}",
                    e
                )));
            }
        };

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| RlmError::Backend(format!("Failed to read response: {}", e)))?;

        if !status.is_success() {
            tracing::error!(
                status = %status,
                body = %body,
                url = %url,
                model = %model,
                "Upstream API returned error"
            );
            return Err(RlmError::Backend(format!(
                "Upstream API error ({}): {}",
                status, body
            )));
        }

        let response_json: serde_json::Value = serde_json::from_str(&body)
            .map_err(|e| RlmError::Backend(format!("Failed to parse response: {}", e)))?;

        tracing::debug!(model = %model, "Successfully forwarded request");

        Ok(response_json)
    }

    /// Forward a raw JSON streaming request to the upstream API.
    ///
    /// Returns the raw reqwest::Response so the caller can stream it back.
    pub async fn forward_raw_stream(
        &self,
        request: serde_json::Value,
        api_key: Option<&str>,
    ) -> Result<reqwest::Response> {
        let url = format!("{}{}", self.config.base_url, self.config.messages_path);

        // Extract model for logging
        let model = request
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        tracing::debug!(
            url = %url,
            model = %model,
            auth_mode = ?self.config.auth_mode,
            "Forwarding streaming request"
        );

        // Prepare the request - strip unknown fields, inject system prompt
        let forward_request = self.prepare_raw_request(request);

        // Build the request
        let mut req = self
            .client
            .post(&url)
            .header(header::CONTENT_TYPE, "application/json");

        // Get auth token based on mode
        let auth_value = match self.get_auth_value(api_key).await {
            Ok(v) => v,
            Err(e) => {
                tracing::error!(error = %e, "Failed to get auth value");
                return Err(e);
            }
        };
        req = req.header(&self.config.auth_header, &auth_value);

        // Add extra headers
        for (key, value) in &self.config.extra_headers {
            req = req.header(key, value);
        }

        let response = match req.json(&forward_request).send().await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!(error = %e, url = %url, "Failed to send request to upstream");
                return Err(RlmError::Backend(format!(
                    "Failed to forward request: {}",
                    e
                )));
            }
        };

        let status = response.status();
        if !status.is_success() {
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "Failed to read error body".to_string());
            tracing::error!(
                status = %status,
                body = %body,
                url = %url,
                model = %model,
                "Upstream API returned error"
            );
            return Err(RlmError::Backend(format!(
                "Upstream API error ({}): {}",
                status, body
            )));
        }

        tracing::debug!(model = %model, "Streaming request started");

        Ok(response)
    }

    /// Get the authentication value based on the configured mode.
    async fn get_auth_value(&self, api_key: Option<&str>) -> Result<String> {
        match self.config.auth_mode {
            AuthMode::ApiKey => {
                let key = api_key.ok_or_else(|| {
                    RlmError::InvalidRequest("API key required but not provided".to_string())
                })?;
                Ok(self.format_auth_value(key))
            }
            AuthMode::OAuth => {
                let manager = self.token_manager.as_ref().ok_or_else(|| {
                    RlmError::Config("OAuth mode requires token manager".to_string())
                })?;
                let token = manager.get_valid_access_token().await?;
                Ok(format!("Bearer {}", token))
            }
            AuthMode::OAuthWithFallback => {
                // Try OAuth first if token manager is available
                if let Some(manager) = &self.token_manager {
                    if manager.has_tokens() {
                        match manager.get_valid_access_token().await {
                            Ok(token) => return Ok(format!("Bearer {}", token)),
                            Err(e) => {
                                tracing::warn!(
                                    "OAuth token refresh failed, trying API key fallback: {}",
                                    e
                                );
                            }
                        }
                    }
                }
                // Fall back to API key
                if let Some(key) = api_key {
                    Ok(self.format_auth_value(key))
                } else {
                    Err(RlmError::InvalidRequest(
                        "No OAuth tokens available and no API key provided. Run 'muninn oauth' to authenticate.".to_string(),
                    ))
                }
            }
        }
    }

    /// Format the auth value based on provider.
    fn format_auth_value(&self, key: &str) -> String {
        match self.config.provider {
            ApiProvider::OpenAI => format!("Bearer {}", key),
            ApiProvider::Anthropic if self.config.auth_mode != AuthMode::ApiKey => {
                format!("Bearer {}", key)
            }
            _ => key.to_string(),
        }
    }

    /// Prepare the request for forwarding.
    fn prepare_request(&self, request: &CompletionRequest) -> ForwardRequest {
        let mut forward = strip_muninn_fields(request);

        // Inject required system prompt for OAuth/MAX if enabled
        if self.config.inject_system_prompt {
            forward.system = Some(inject_claude_code_system_prompt(forward.system));
        }

        forward
    }

    /// Prepare a raw JSON request for forwarding.
    ///
    /// This strips unknown fields and optionally injects the required system prompt.
    fn prepare_raw_request(&self, request: serde_json::Value) -> serde_json::Value {
        // Strip unknown top-level fields
        let sanitized = strip_unknown_fields_raw(&request);

        let mut result = sanitized;

        // Inject required system prompt for OAuth/MAX if enabled
        if self.config.inject_system_prompt {
            inject_system_prompt_raw(&mut result);
        }

        result
    }
}

impl Default for Passthrough {
    fn default() -> Self {
        Self::new()
    }
}

// Keep the old name as an alias for backwards compatibility
pub type AnthropicPassthrough = Passthrough;

/// Strip muninn-specific fields from request before forwarding.
fn strip_muninn_fields(request: &CompletionRequest) -> ForwardRequest {
    // Convert Vec to Option for fields that are empty by default
    let stop_sequences = if request.stop_sequences.is_empty() {
        None
    } else {
        Some(request.stop_sequences.clone())
    };

    let tools = if request.tools.is_empty() {
        None
    } else {
        Some(request.tools.clone())
    };

    // Convert stream: only include if true (default false means omit)
    let stream = if request.stream { Some(true) } else { None };

    // Convert system string to array format if present
    let system = request
        .system
        .as_ref()
        .map(|s| vec![SystemMessage::text(s.to_text())]);

    ForwardRequest {
        model: request.model.clone(),
        max_tokens: request.max_tokens,
        messages: request.messages.clone(),
        system,
        stop_sequences,
        temperature: request.temperature,
        top_p: request.top_p,
        top_k: request.top_k,
        tools,
        tool_choice: request.tool_choice.clone(),
        stream,
    }
}

/// Inject the required Claude Code system prompt at the beginning.
fn inject_claude_code_system_prompt(system: Option<Vec<SystemMessage>>) -> Vec<SystemMessage> {
    let required = SystemMessage::text(CLAUDE_CODE_SYSTEM_PROMPT);

    match system {
        Some(mut messages) => {
            // Check if already has the required prompt
            if messages.first().map(|m| m.text.as_str()) == Some(CLAUDE_CODE_SYSTEM_PROMPT) {
                messages
            } else {
                // Prepend required prompt
                messages.insert(0, required);
                messages
            }
        }
        None => vec![required],
    }
}

/// Valid top-level fields for Anthropic API requests.
/// Fields like 'context_management' from the Agent SDK are not supported.
const VALID_REQUEST_FIELDS: &[&str] = &[
    "model",
    "max_tokens",
    "system",
    "messages",
    "tools",
    "tool_choice",
    "stream",
    "temperature",
    "top_p",
    "top_k",
    "stop_sequences",
    "metadata",
    "thinking",
];

/// Strip unknown fields from a raw JSON request.
fn strip_unknown_fields_raw(request: &serde_json::Value) -> serde_json::Value {
    match request {
        serde_json::Value::Object(map) => {
            let mut sanitized = serde_json::Map::new();
            for (key, value) in map {
                if VALID_REQUEST_FIELDS.contains(&key.as_str()) {
                    sanitized.insert(key.clone(), value.clone());
                }
            }
            serde_json::Value::Object(sanitized)
        }
        // If not an object, return as-is (shouldn't happen for valid requests)
        _ => request.clone(),
    }
}

/// Inject the required system prompt into a raw JSON request.
fn inject_system_prompt_raw(request: &mut serde_json::Value) {
    let required_prompt = serde_json::json!({
        "type": "text",
        "text": CLAUDE_CODE_SYSTEM_PROMPT
    });

    if let serde_json::Value::Object(map) = request {
        let system = map
            .entry("system")
            .or_insert(serde_json::Value::Array(vec![]));

        // Normalize system to array format
        let system_array = match system {
            serde_json::Value::String(s) => {
                // Convert string to array format
                let text_msg = serde_json::json!({
                    "type": "text",
                    "text": s.clone()
                });
                vec![text_msg]
            }
            serde_json::Value::Array(arr) => arr.clone(),
            _ => vec![],
        };

        // Check if already has the required prompt
        let has_required = system_array.first().is_some_and(|first| {
            first.get("type").and_then(|t| t.as_str()) == Some("text")
                && first.get("text").and_then(|t| t.as_str()) == Some(CLAUDE_CODE_SYSTEM_PROMPT)
        });

        if !has_required {
            // Prepend required prompt
            let mut new_system = vec![required_prompt];
            new_system.extend(system_array);
            *system = serde_json::Value::Array(new_system);
        } else {
            // Already has it, just ensure it's in array format
            *system = serde_json::Value::Array(system_array);
        }
    }
}

/// System message structure for Anthropic API.
#[derive(Debug, Clone, Serialize)]
pub struct SystemMessage {
    /// Message type (always "text").
    #[serde(rename = "type")]
    pub msg_type: String,
    /// The text content.
    pub text: String,
    /// Optional cache control.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<CacheControl>,
}

impl SystemMessage {
    /// Create a text system message.
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            msg_type: "text".to_string(),
            text: content.into(),
            cache_control: None,
        }
    }

    /// Create a text system message with ephemeral cache control.
    pub fn text_with_cache(content: impl Into<String>) -> Self {
        Self {
            msg_type: "text".to_string(),
            text: content.into(),
            cache_control: Some(CacheControl::ephemeral()),
        }
    }
}

/// Cache control for system messages.
#[derive(Debug, Clone, Serialize)]
pub struct CacheControl {
    /// Cache type.
    #[serde(rename = "type")]
    pub cache_type: String,
}

impl CacheControl {
    /// Create ephemeral cache control.
    pub fn ephemeral() -> Self {
        Self {
            cache_type: "ephemeral".to_string(),
        }
    }
}

/// Request structure for forwarding (without muninn fields).
#[derive(Debug, Serialize)]
struct ForwardRequest {
    model: String,
    max_tokens: u32,
    messages: Vec<crate::types::Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<Vec<SystemMessage>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop_sequences: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_k: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<crate::types::ToolDefinition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<crate::types::ToolChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_passthrough_default() {
        let pt = Passthrough::new();
        assert_eq!(pt.config.base_url, ANTHROPIC_API_URL);
        assert_eq!(pt.config.provider, ApiProvider::Anthropic);
        // Default is OAuth mode
        assert_eq!(pt.config.auth_mode, AuthMode::OAuthWithFallback);
        assert!(pt.config.inject_system_prompt);
    }

    #[test]
    fn test_passthrough_anthropic() {
        let pt = Passthrough::anthropic();
        assert_eq!(pt.config.base_url, ANTHROPIC_API_URL);
        assert_eq!(pt.config.messages_path, "/v1/messages");
        assert_eq!(pt.config.auth_header, "x-api-key");
        assert_eq!(pt.config.auth_mode, AuthMode::ApiKey);
    }

    #[test]
    fn test_passthrough_anthropic_oauth() {
        let pt = Passthrough::anthropic_oauth();
        assert_eq!(pt.config.base_url, ANTHROPIC_API_URL);
        assert_eq!(pt.config.auth_header, "Authorization");
        assert_eq!(pt.config.auth_mode, AuthMode::OAuthWithFallback);
        assert!(pt.config.inject_system_prompt);
        assert!(pt.config.extra_headers.contains_key("anthropic-beta"));
    }

    #[test]
    fn test_passthrough_custom_url() {
        let pt = Passthrough::with_base_url("http://localhost:8080");
        assert_eq!(pt.config.base_url, "http://localhost:8080");
    }

    #[test]
    fn test_config_builder() {
        let config = PassthroughConfig::custom("http://example.com")
            .with_messages_path("/api/chat")
            .with_auth_header("Authorization")
            .with_header("X-Custom", "value")
            .with_auth_mode(AuthMode::OAuth);

        assert_eq!(config.base_url, "http://example.com");
        assert_eq!(config.messages_path, "/api/chat");
        assert_eq!(config.auth_header, "Authorization");
        assert_eq!(
            config.extra_headers.get("X-Custom"),
            Some(&"value".to_string())
        );
        assert_eq!(config.auth_mode, AuthMode::OAuth);
    }

    #[test]
    fn test_openai_config() {
        let config = PassthroughConfig::openai();
        assert_eq!(config.base_url, OPENAI_API_URL);
        assert_eq!(config.messages_path, "/v1/chat/completions");
        assert_eq!(config.auth_header, "Authorization");
        assert_eq!(config.auth_mode, AuthMode::ApiKey);
    }

    #[test]
    fn test_system_message() {
        let msg = SystemMessage::text("Hello");
        assert_eq!(msg.msg_type, "text");
        assert_eq!(msg.text, "Hello");
        assert!(msg.cache_control.is_none());
    }

    #[test]
    fn test_inject_system_prompt_empty() {
        let result = inject_claude_code_system_prompt(None);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].text, CLAUDE_CODE_SYSTEM_PROMPT);
    }

    #[test]
    fn test_inject_system_prompt_prepend() {
        let existing = vec![SystemMessage::text("Custom prompt")];
        let result = inject_claude_code_system_prompt(Some(existing));
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].text, CLAUDE_CODE_SYSTEM_PROMPT);
        assert_eq!(result[1].text, "Custom prompt");
    }

    #[test]
    fn test_inject_system_prompt_already_present() {
        let existing = vec![
            SystemMessage::text(CLAUDE_CODE_SYSTEM_PROMPT),
            SystemMessage::text("Custom prompt"),
        ];
        let result = inject_claude_code_system_prompt(Some(existing));
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].text, CLAUDE_CODE_SYSTEM_PROMPT);
    }
}
