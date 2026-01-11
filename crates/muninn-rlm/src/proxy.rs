//! HTTP proxy layer for the RLM gateway.
//!
//! This module implements a transparent proxy that accepts Anthropic Messages API
//! requests and routes them through either:
//! - Passthrough mode: Forward to upstream API (Anthropic) using original auth
//! - RLM mode: Use configured backend for recursive exploration

use axum::{
    Json, Router as AxumRouter,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::post,
};
use serde::Serialize;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;
use tokio::net::TcpListener;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

use crate::backend::LLMBackend;
use crate::engine::{EngineConfig, EngineDeps, RecursiveEngine};
use crate::error::RlmError;
use crate::passthrough::{Passthrough, PassthroughConfig};
use crate::router::{RouteDecision, Router as RlmRouter, RouterConfig};
use crate::token_manager::SharedTokenManager;
use crate::tools::ToolEnvironment;
use crate::types::{CompletionRequest, MuninnConfig};

// ============================================================================
// Proxy Trace Data
// ============================================================================

/// Trace data for incoming proxy requests.
#[derive(Debug, Clone, Serialize)]
pub struct ProxyRequestTraceData {
    /// Model requested.
    pub model: String,
    /// Whether streaming was requested.
    pub streaming: bool,
    /// Whether this is an explicit recursive request.
    pub explicit_recursive: bool,
    /// Number of messages in the request.
    pub message_count: usize,
}

/// Trace data for proxy request completion.
#[derive(Debug, Clone, Serialize)]
pub struct ProxyCompletionTraceData {
    /// How the request was handled.
    pub handling: String,
    /// Whether the request succeeded.
    pub success: bool,
    /// Error message if failed.
    pub error: Option<String>,
    /// Total request time (ms).
    pub total_time_ms: u64,
}

/// Configuration for the proxy server.
#[derive(Debug)]
pub struct ProxyConfig {
    /// Address to bind the server to.
    pub bind_addr: SocketAddr,
    /// Whether to enable CORS.
    pub enable_cors: bool,
    /// Whether to enable request tracing (HTTP layer).
    pub enable_tracing: bool,
    /// Passthrough configuration for upstream API forwarding.
    pub passthrough: PassthroughConfig,
    /// Optional token manager for OAuth authentication.
    pub token_manager: Option<SharedTokenManager>,
    /// Budget configuration for recursive exploration.
    pub budget: Option<crate::types::BudgetConfig>,
    /// Working directory for RLM context.
    pub work_dir: Option<std::path::PathBuf>,
    /// Configuration for agentic trace collection.
    pub trace_writer: Option<muninn_tracing::WriterConfig>,
    /// Session directory for logging (when set, uses session-based logging).
    pub session_dir: Option<std::path::PathBuf>,
}

impl Clone for ProxyConfig {
    fn clone(&self) -> Self {
        Self {
            bind_addr: self.bind_addr,
            enable_cors: self.enable_cors,
            enable_tracing: self.enable_tracing,
            passthrough: self.passthrough.clone(),
            token_manager: self.token_manager.clone(),
            budget: self.budget.clone(),
            work_dir: self.work_dir.clone(),
            trace_writer: self.trace_writer.clone(),
            session_dir: self.session_dir.clone(),
        }
    }
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            bind_addr: "127.0.0.1:8080".parse().unwrap(),
            enable_cors: true,
            enable_tracing: true,
            passthrough: PassthroughConfig::default(),
            token_manager: None,
            budget: None,
            work_dir: None,
            trace_writer: Some(muninn_tracing::WriterConfig::default()),
            session_dir: None,
        }
    }
}

impl ProxyConfig {
    /// Create a new proxy config with the given bind address.
    pub fn new(bind_addr: SocketAddr) -> Self {
        Self {
            bind_addr,
            ..Default::default()
        }
    }

    /// Set the bind address.
    pub fn with_bind_addr(mut self, addr: SocketAddr) -> Self {
        self.bind_addr = addr;
        self
    }

    /// Enable or disable CORS.
    pub fn with_cors(mut self, enable: bool) -> Self {
        self.enable_cors = enable;
        self
    }

    /// Set the passthrough configuration.
    pub fn with_passthrough(mut self, config: PassthroughConfig) -> Self {
        self.passthrough = config;
        self
    }

    /// Set the token manager for OAuth authentication.
    pub fn with_token_manager(mut self, manager: SharedTokenManager) -> Self {
        self.token_manager = Some(manager);
        self
    }

    /// Set the budget configuration for recursive exploration.
    pub fn with_budget(mut self, budget: crate::types::BudgetConfig) -> Self {
        self.budget = Some(budget);
        self
    }

    /// Set the working directory for RLM context.
    pub fn with_work_dir(mut self, path: impl Into<std::path::PathBuf>) -> Self {
        self.work_dir = Some(path.into());
        self
    }

    /// Set the trace writer configuration.
    pub fn with_trace_writer(mut self, config: muninn_tracing::WriterConfig) -> Self {
        self.trace_writer = Some(config);
        self
    }

    /// Disable agentic tracing.
    pub fn without_agentic_tracing(mut self) -> Self {
        self.trace_writer = None;
        self
    }

    /// Set the session directory for session-based logging.
    pub fn with_session_dir(mut self, path: impl Into<std::path::PathBuf>) -> Self {
        self.session_dir = Some(path.into());
        self
    }
}

/// Shared state for the proxy server.
struct ProxyState {
    /// RLM engine for recursive context building (optional).
    engine: Option<RecursiveEngine>,
    /// Router for deciding passthrough vs RLM (optional).
    router: Option<RlmRouter>,
    /// Passthrough client for forwarding to upstream API.
    passthrough: Passthrough,
    /// Trace writer for agentic traces (optional).
    trace_writer: Option<muninn_tracing::TraceWriter>,
    /// Session directory for logging (optional).
    session_dir: Option<std::path::PathBuf>,
}

/// The RLM proxy server.
pub struct ProxyServer {
    config: ProxyConfig,
    state: Arc<ProxyState>,
}

impl ProxyServer {
    /// Create a trace writer from config.
    fn create_trace_writer(config: &ProxyConfig) -> Option<muninn_tracing::TraceWriter> {
        config.trace_writer.as_ref().and_then(
            |writer_config| match muninn_tracing::TraceWriter::new(writer_config.clone()) {
                Ok(writer) => Some(writer),
                Err(e) => {
                    tracing::warn!(error = %e, "Failed to create trace writer");
                    None
                }
            },
        )
    }

    /// Create a new proxy server with RLM backend.
    pub fn new(
        config: ProxyConfig,
        backend: Arc<dyn LLMBackend>,
        tools: Arc<dyn ToolEnvironment>,
    ) -> Self {
        let deps = EngineDeps::new(backend, tools);
        let mut engine_config = EngineConfig::default();
        if let Some(budget) = &config.budget {
            engine_config = engine_config.with_budget(budget.clone());
        }
        if let Some(ref work_dir) = config.work_dir {
            engine_config = engine_config.with_work_dir(work_dir.clone());
        }
        let engine = RecursiveEngine::new(deps, engine_config);
        let router = RlmRouter::new();
        let mut passthrough = Passthrough::with_config(config.passthrough.clone());
        if let Some(tm) = &config.token_manager {
            passthrough = passthrough.with_token_manager(tm.clone());
        }
        let trace_writer = Self::create_trace_writer(&config);
        Self {
            state: Arc::new(ProxyState {
                engine: Some(engine),
                router: Some(router),
                passthrough,
                trace_writer,
                session_dir: config.session_dir.clone(),
            }),
            config,
        }
    }

    /// Create a passthrough-only proxy (no RLM backend required).
    pub fn passthrough_only(config: ProxyConfig) -> Self {
        let mut passthrough = Passthrough::with_config(config.passthrough.clone());
        if let Some(tm) = &config.token_manager {
            passthrough = passthrough.with_token_manager(tm.clone());
        }
        let trace_writer = Self::create_trace_writer(&config);
        Self {
            state: Arc::new(ProxyState {
                engine: None,
                router: None,
                passthrough,
                trace_writer,
                session_dir: config.session_dir.clone(),
            }),
            config,
        }
    }

    /// Create a new proxy server with custom router configuration.
    pub fn with_router(
        config: ProxyConfig,
        backend: Arc<dyn LLMBackend>,
        tools: Arc<dyn ToolEnvironment>,
        router_config: RouterConfig,
    ) -> Self {
        let deps = EngineDeps::new(backend.clone(), tools);
        let mut engine_config = EngineConfig::default();
        if let Some(budget) = &config.budget {
            engine_config = engine_config.with_budget(budget.clone());
        }
        if let Some(ref work_dir) = config.work_dir {
            engine_config = engine_config.with_work_dir(work_dir.clone());
        }
        let engine = RecursiveEngine::new(deps, engine_config);
        let router = RlmRouter::with_config(router_config).with_llm(backend);
        let mut passthrough = Passthrough::with_config(config.passthrough.clone());
        if let Some(tm) = &config.token_manager {
            passthrough = passthrough.with_token_manager(tm.clone());
        }
        let trace_writer = Self::create_trace_writer(&config);
        Self {
            state: Arc::new(ProxyState {
                engine: Some(engine),
                router: Some(router),
                passthrough,
                trace_writer,
                session_dir: config.session_dir.clone(),
            }),
            config,
        }
    }

    /// Create a new proxy server with separate backends for router and RLM.
    ///
    /// This allows using a fast, cheap model for routing decisions while using
    /// a more capable model for actual RLM exploration.
    pub fn with_separate_backends(
        config: ProxyConfig,
        router_backend: Arc<dyn LLMBackend>,
        rlm_backend: Arc<dyn LLMBackend>,
        tools: Arc<dyn ToolEnvironment>,
        router_config: RouterConfig,
    ) -> Self {
        // Use the RLM backend for the engine
        let deps = EngineDeps::new(rlm_backend, tools);
        let mut engine_config = EngineConfig::default();
        if let Some(budget) = &config.budget {
            engine_config = engine_config.with_budget(budget.clone());
        }
        if let Some(ref work_dir) = config.work_dir {
            engine_config = engine_config.with_work_dir(work_dir.clone());
        }
        let engine = RecursiveEngine::new(deps, engine_config);

        // Use the router backend for routing decisions
        let router = RlmRouter::with_config(router_config).with_llm(router_backend);
        let mut passthrough = Passthrough::with_config(config.passthrough.clone());
        if let Some(tm) = &config.token_manager {
            passthrough = passthrough.with_token_manager(tm.clone());
        }
        let trace_writer = Self::create_trace_writer(&config);
        Self {
            state: Arc::new(ProxyState {
                engine: Some(engine),
                router: Some(router),
                passthrough,
                trace_writer,
                session_dir: config.session_dir.clone(),
            }),
            config,
        }
    }

    /// Create a proxy with an existing engine.
    pub fn with_engine(config: ProxyConfig, engine: RecursiveEngine) -> Self {
        let router = RlmRouter::new();
        let mut passthrough = Passthrough::with_config(config.passthrough.clone());
        if let Some(tm) = &config.token_manager {
            passthrough = passthrough.with_token_manager(tm.clone());
        }
        let trace_writer = Self::create_trace_writer(&config);
        Self {
            state: Arc::new(ProxyState {
                engine: Some(engine),
                router: Some(router),
                passthrough,
                trace_writer,
                session_dir: config.session_dir.clone(),
            }),
            config,
        }
    }

    /// Build the axum router for the proxy.
    pub fn router(&self) -> AxumRouter {
        let mut router = AxumRouter::new()
            .route("/v1/messages", post(handle_messages))
            .route("/v1/chat/completions", post(handle_openai_chat))
            .route("/health", axum::routing::get(handle_health))
            .with_state(self.state.clone());

        if self.config.enable_cors {
            router = router.layer(
                CorsLayer::new()
                    .allow_origin(Any)
                    .allow_methods(Any)
                    .allow_headers(Any),
            );
        }

        if self.config.enable_tracing {
            router = router.layer(TraceLayer::new_for_http());
        }

        router
    }

    /// Run the proxy server.
    pub async fn run(self) -> std::io::Result<()> {
        let listener = TcpListener::bind(self.config.bind_addr).await?;
        tracing::info!(
            addr = %self.config.bind_addr,
            "Starting RLM proxy server"
        );
        axum::serve(listener, self.router()).await
    }

    /// Run the proxy server with graceful shutdown.
    pub async fn run_with_shutdown(
        self,
        shutdown: impl std::future::Future<Output = ()> + Send + 'static,
    ) -> std::io::Result<()> {
        let listener = TcpListener::bind(self.config.bind_addr).await?;
        tracing::info!(
            addr = %self.config.bind_addr,
            "Starting RLM proxy server"
        );
        axum::serve(listener, self.router())
            .with_graceful_shutdown(shutdown)
            .await
    }
}

/// Handle POST /v1/chat/completions (OpenAI-compatible endpoint)
///
/// This endpoint bypasses the router entirely and forwards requests directly
/// to Claude using OAuth tokens. Use this to "raw dog" the proxy with your
/// Claude MAX subscription credits as a simple OpenAI-compatible API.
async fn handle_openai_chat(
    State(state): State<Arc<ProxyState>>,
    headers: HeaderMap,
    body: String,
) -> Result<axum::response::Response, ProxyError> {
    // Parse body as raw JSON
    let raw_request: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| RlmError::InvalidRequest(format!("Invalid JSON: {}", e)))?;

    // Extract streaming flag
    let is_streaming = raw_request
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    // Extract API key from headers for fallback
    let api_key = extract_api_key(&headers, state.passthrough.config());

    tracing::debug!(
        streaming = is_streaming,
        model = %raw_request.get("model").and_then(|v| v.as_str()).unwrap_or("unknown"),
        "OpenAI-compatible direct passthrough"
    );

    // Forward directly via passthrough (bypass router entirely)
    forward_passthrough(
        &state.passthrough,
        raw_request,
        api_key.as_deref(),
        is_streaming,
    )
    .await
}

/// Handle POST /v1/messages
///
/// This handler accepts raw JSON to support passthrough of all content types
/// (including thinking blocks, images, etc.) that may not be in our type definitions.
async fn handle_messages(
    State(state): State<Arc<ProxyState>>,
    headers: HeaderMap,
    body: String,
) -> Result<axum::response::Response, ProxyError> {
    let request_start = Instant::now();

    // Extract API key from request headers for passthrough
    let api_key = extract_api_key(&headers, state.passthrough.config());

    // Parse body as raw JSON first
    let raw_request: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| RlmError::InvalidRequest(format!("Invalid JSON: {}", e)))?;

    // Extract model and streaming flag for logging/routing
    let model = raw_request
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    let is_streaming = raw_request
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let message_count = raw_request
        .get("messages")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);

    // Log raw request to file for debugging
    {
        // Use session directory if available, otherwise fall back to legacy path
        let log_path = if let Some(ref session_dir) = state.session_dir {
            session_dir.join("raw_requests.jsonl")
        } else {
            let log_dir = std::path::Path::new(".muninn/debug");
            std::fs::create_dir_all(log_dir).ok();
            log_dir.join("raw_requests.jsonl")
        };

        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
        {
            use std::io::Write;
            let timestamp = chrono::Utc::now().to_rfc3339();
            let log_entry = serde_json::json!({
                "timestamp": timestamp,
                "model": model,
                "message_count": message_count,
                "request": raw_request
            });
            writeln!(file, "{}", serde_json::to_string(&log_entry).unwrap()).ok();
        }
    }

    // If no RLM engine available, always passthrough using raw JSON
    let (engine, router) = match (&state.engine, &state.router) {
        (Some(e), Some(r)) => (e, r),
        _ => {
            // Passthrough-only mode - use raw JSON forwarding
            tracing::debug!("Passthrough (no RLM backend)");
            return forward_passthrough(
                &state.passthrough,
                raw_request,
                api_key.as_deref(),
                is_streaming,
            )
            .await;
        }
    };

    // For RLM routing, try to parse into CompletionRequest
    // If parsing fails (unknown content types), fall back to passthrough
    let typed_request = match serde_json::from_str::<CompletionRequest>(&body) {
        Ok(r) => {
            // Log the parsed message content for debugging
            if let Some(last_msg) = r
                .messages
                .iter()
                .rev()
                .find(|m| m.role == crate::types::Role::User)
            {
                tracing::debug!(
                    content_debug = ?last_msg.content,
                    text_preview = %last_msg.content.to_text().chars().take(100).collect::<String>(),
                    "Parsed user message"
                );
            }
            r
        }
        Err(e) => {
            // Can't parse into our types - use passthrough
            tracing::debug!(error = %e, "Request parse failed, using passthrough");
            return forward_passthrough(
                &state.passthrough,
                raw_request,
                api_key.as_deref(),
                is_streaming,
            )
            .await;
        }
    };

    // First check for explicit muninn.recursive flag
    let explicit_recursive = RecursiveEngine::is_recursive(&typed_request);

    // Use with_tracing to collect trace data for RLM requests
    let (result, trace) = muninn_tracing::with_tracing(async {
        // Record request metadata
        let request_data = ProxyRequestTraceData {
            model: model.clone(),
            streaming: is_streaming,
            explicit_recursive,
            message_count,
        };
        muninn_tracing::start_span_with_data("proxy_request", &request_data);

        // If not explicitly set, use router to decide
        let trace_id = muninn_tracing::current_trace_id().unwrap_or_default();
        let should_use_rlm = if explicit_recursive {
            tracing::debug!(trace_id = %trace_id, "RLM request (explicit)");
            true
        } else {
            let decision = router.route(&typed_request).await;
            match &decision {
                RouteDecision::Passthrough => {
                    tracing::debug!(trace_id = %trace_id, "Passthrough request");
                    false
                }
                RouteDecision::Rlm { .. } => {
                    tracing::debug!(trace_id = %trace_id, "RLM request (routed)");
                    true
                }
            }
        };

        if should_use_rlm {
            // Use configured backend (Groq/local) for recursive exploration
            let mut request = typed_request;
            let muninn = request.muninn.get_or_insert_with(MuninnConfig::default);
            muninn.recursive = true;
            match engine.complete(request).await {
                Ok(response) => {
                    let completion_data = ProxyCompletionTraceData {
                        handling: "rlm".to_string(),
                        success: true,
                        error: None,
                        total_time_ms: request_start.elapsed().as_millis() as u64,
                    };
                    muninn_tracing::record_event("proxy_completion", Some(&completion_data));
                    muninn_tracing::end_span_ok();
                    Ok(Json(response).into_response())
                }
                Err(e) => {
                    let completion_data = ProxyCompletionTraceData {
                        handling: "rlm".to_string(),
                        success: false,
                        error: Some(e.to_string()),
                        total_time_ms: request_start.elapsed().as_millis() as u64,
                    };
                    muninn_tracing::record_event("proxy_completion", Some(&completion_data));
                    muninn_tracing::end_span_error(e.to_string());
                    Err(ProxyError::from(e))
                }
            }
        } else {
            // Passthrough - don't trace the actual passthrough request
            let completion_data = ProxyCompletionTraceData {
                handling: "passthrough".to_string(),
                success: true,
                error: None,
                total_time_ms: request_start.elapsed().as_millis() as u64,
            };
            muninn_tracing::record_event("proxy_completion", Some(&completion_data));
            muninn_tracing::end_span_ok();
            forward_passthrough(
                &state.passthrough,
                raw_request,
                api_key.as_deref(),
                is_streaming,
            )
            .await
        }
    })
    .await;

    // Write trace if we have a trace writer
    if let Some(ref writer) = state.trace_writer {
        if let Err(e) = writer.write(&trace) {
            tracing::warn!(trace_id = %trace.trace_id, error = %e, "Failed to write trace");
        }
    }

    result
}

/// Forward a request through passthrough, handling both streaming and non-streaming.
async fn forward_passthrough(
    passthrough: &Passthrough,
    request: serde_json::Value,
    api_key: Option<&str>,
    is_streaming: bool,
) -> Result<axum::response::Response, ProxyError> {
    use axum::body::Body;
    use futures::StreamExt;

    if is_streaming {
        // For streaming requests, get the raw response and stream it back
        let upstream_response = passthrough.forward_raw_stream(request, api_key).await?;

        // Get headers from upstream response
        let content_type = upstream_response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("text/event-stream")
            .to_string();

        // Convert reqwest body stream to axum body
        let stream = upstream_response
            .bytes_stream()
            .map(|result| result.map_err(std::io::Error::other));
        let body = Body::from_stream(stream);

        // Build response with SSE content type
        let response = axum::response::Response::builder()
            .status(StatusCode::OK)
            .header("content-type", content_type)
            .header("cache-control", "no-cache")
            .body(body)
            .map_err(|e| RlmError::Backend(format!("Failed to build response: {}", e)))?;

        Ok(response)
    } else {
        // Non-streaming: parse as JSON
        let response = passthrough.forward_raw(request, api_key).await?;
        Ok(Json(response).into_response())
    }
}

/// Extract API key from request headers based on passthrough config.
fn extract_api_key(headers: &HeaderMap, config: &PassthroughConfig) -> Option<String> {
    // Try the configured auth header first
    if let Some(value) = headers.get(&config.auth_header) {
        if let Ok(s) = value.to_str() {
            // Strip "Bearer " prefix if present (for OpenAI-style auth)
            let key = s.strip_prefix("Bearer ").unwrap_or(s);
            return Some(key.to_string());
        }
    }

    // For Anthropic, also try x-api-key
    if config.auth_header != "x-api-key" {
        if let Some(value) = headers.get("x-api-key") {
            if let Ok(s) = value.to_str() {
                return Some(s.to_string());
            }
        }
    }

    // For OpenAI-compatible, also try Authorization
    if config.auth_header != "Authorization" {
        if let Some(value) = headers.get("Authorization") {
            if let Ok(s) = value.to_str() {
                let key = s.strip_prefix("Bearer ").unwrap_or(s);
                return Some(key.to_string());
            }
        }
    }

    None
}

/// Handle GET /health
async fn handle_health() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "ok",
        "service": "muninn-rlm"
    }))
}

/// Error type for proxy responses.
#[derive(Debug)]
pub struct ProxyError(RlmError);

impl From<RlmError> for ProxyError {
    fn from(err: RlmError) -> Self {
        Self(err)
    }
}

impl IntoResponse for ProxyError {
    fn into_response(self) -> axum::response::Response {
        let (status, error_type, message) = match &self.0 {
            RlmError::Backend(msg) => (StatusCode::BAD_GATEWAY, "backend_error", msg.clone()),
            RlmError::ToolExecution(msg) => {
                (StatusCode::INTERNAL_SERVER_ERROR, "tool_error", msg.clone())
            }
            RlmError::BudgetExceeded(err) => (
                StatusCode::OK, // Return OK with stop_reason, not an error
                "budget_exceeded",
                err.to_string(),
            ),
            RlmError::InvalidRequest(msg) => {
                (StatusCode::BAD_REQUEST, "invalid_request", msg.clone())
            }
            RlmError::Network(msg) => (StatusCode::BAD_GATEWAY, "network_error", msg.clone()),
            RlmError::Serialization(msg) => {
                (StatusCode::BAD_REQUEST, "serialization_error", msg.clone())
            }
            RlmError::Config(msg) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "config_error",
                msg.clone(),
            ),
            RlmError::Internal(msg) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                msg.clone(),
            ),
            RlmError::Protocol(msg) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "protocol_error",
                msg.clone(),
            ),
        };

        let body = serde_json::json!({
            "type": "error",
            "error": {
                "type": error_type,
                "message": message
            }
        });

        (status, Json(body)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::MockBackend;
    use crate::router::RouterStrategy;
    use crate::tools::EmptyToolEnvironment;
    use crate::types::{CompletionResponse, ContentBlock, StopReason, Usage};
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use serde_json::json;
    use tower::ServiceExt;

    fn create_test_server(responses: Vec<CompletionResponse>) -> ProxyServer {
        let backend = Arc::new(MockBackend::new(responses));
        let tools = Arc::new(EmptyToolEnvironment);
        // Use always-rlm strategy for tests so we use the mock backend, not passthrough
        let router_config = RouterConfig {
            strategy: RouterStrategy::AlwaysRlm,
            ..Default::default()
        };
        ProxyServer::with_router(ProxyConfig::default(), backend, tools, router_config)
    }

    #[tokio::test]
    async fn test_health_endpoint() {
        let server = create_test_server(vec![]);
        let router = server.router();

        let response = router
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_messages_endpoint_simple() {
        let responses = vec![CompletionResponse::new(
            "msg_1",
            "test-model",
            vec![ContentBlock::Text {
                text: "Hello!".to_string(),
                cache_control: None,
            }],
            StopReason::EndTurn,
            Usage::new(10, 5),
        )];

        let server = create_test_server(responses);
        let router = server.router();

        let request_body = json!({
            "model": "test-model",
            "max_tokens": 100,
            "messages": [{"role": "user", "content": "Hi"}]
        });

        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/messages")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_string(&request_body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let parsed: CompletionResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed.text(), "Hello!");
    }

    #[tokio::test]
    async fn test_messages_endpoint_with_muninn() {
        let responses = vec![CompletionResponse::new(
            "msg_1",
            "test-model",
            vec![ContentBlock::Text {
                text: "Explored!".to_string(),
                cache_control: None,
            }],
            StopReason::EndTurn,
            Usage::new(50, 30),
        )];

        let server = create_test_server(responses);
        let router = server.router();

        let request_body = json!({
            "model": "test-model",
            "max_tokens": 100,
            "messages": [{"role": "user", "content": "Explore this"}],
            "muninn": {
                "recursive": true,
                "budget": {
                    "max_tokens": 10000,
                    "max_depth": 5
                }
            }
        });

        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/messages")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_string(&request_body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let parsed: CompletionResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed.text(), "Explored!");
        // Should have exploration metadata
        assert!(parsed.muninn.is_some());
    }

    #[tokio::test]
    async fn test_messages_endpoint_invalid_json() {
        let server = create_test_server(vec![]);
        let router = server.router();

        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/messages")
                    .header("content-type", "application/json")
                    .body(Body::from("not valid json"))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Axum returns 400 Bad Request for JSON parsing errors
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_proxy_error_response() {
        // Create a server with no responses (will fail)
        let server = create_test_server(vec![]);
        let router = server.router();

        let request_body = json!({
            "model": "test-model",
            "max_tokens": 100,
            "messages": [{"role": "user", "content": "Hi"}]
        });

        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/messages")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_string(&request_body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Should get a backend error since MockBackend has no responses
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed["type"], "error");
        assert_eq!(parsed["error"]["type"], "backend_error");
    }

    #[test]
    fn test_proxy_config_default() {
        let config = ProxyConfig::default();
        assert!(config.enable_cors);
        assert!(config.enable_tracing);
        assert_eq!(config.bind_addr.port(), 8080);
    }

    #[test]
    fn test_proxy_config_builder() {
        let config = ProxyConfig::new("0.0.0.0:3000".parse().unwrap()).with_cors(false);
        assert!(!config.enable_cors);
        assert_eq!(config.bind_addr.port(), 3000);
    }
}
