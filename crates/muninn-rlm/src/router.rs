//! Request router for deciding RLM vs passthrough.
//!
//! The router analyzes incoming requests and decides whether they need:
//! - **Passthrough**: Direct forwarding to upstream API
//! - **RLM Processing**: Recursive exploration to build context
//!
//! # Routing Flow
//!
//! ```text
//! CompletionRequest
//!       │
//!       ▼
//! extract_routing_input() ─── None ──▶ passthrough
//!       │
//!       ▼ Some(input)
//! should_bypass()? ────────── true ──▶ passthrough (internal requests)
//!       │
//!       ▼ false
//! has_passthrough_trigger()? ─ true ─▶ passthrough ({at}muninn passthrough)
//!       │
//!       ▼ false
//! has_rlm_trigger()? ───────── true ─▶ rlm ({at}muninn explore)
//!       │
//!       ▼ false
//! strategy-based routing
//!   ├─ AlwaysPassthrough ──────────▶ passthrough
//!   ├─ AlwaysRlm ──────────────────▶ rlm
//!   └─ Llm ─▶ route_via_llm() ────▶ decision
//! ```
//!
//! Note: The JSON flag (`request.muninn.recursive`) is checked in proxy before routing.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::backend::LLMBackend;
use crate::types::{
    CompletionRequest, CompletionResponse, Message, Role, SystemPrompt, ToolChoice, ToolDefinition,
};

// ============================================================================
// Trace Data (for observability)
// ============================================================================

/// Trace data captured for router decisions.
#[derive(Debug, Clone, Serialize)]
pub struct RouterTraceData {
    /// The routing strategy used.
    pub strategy: String,
    /// How the decision was made: "disabled", "no_message", "internal_bypass",
    /// "passthrough_trigger", "rlm_trigger", "forced_passthrough", "forced_rlm", "llm".
    pub method: String,
    /// Model requested in the original request.
    pub model: String,
    /// System prompt (if any).
    pub system_prompt: Option<String>,
    /// The last user message analyzed (after stripping control tags).
    pub last_user_message: Option<String>,
    /// Number of messages in the conversation.
    pub message_count: usize,
    /// The decision made: "rlm" or "passthrough".
    pub decision: String,
    /// Reason for the decision (if RLM).
    pub reason: Option<String>,
    /// Time taken to make the decision (ms).
    pub decision_time_ms: u64,
}

/// Training data record for routing decisions.
/// This format is designed for fine-tuning a routing SLM.
#[derive(Debug, Clone, Serialize)]
pub struct RoutingTrainingRecord {
    /// Timestamp of the decision.
    pub timestamp: String,
    /// The user's request (last message).
    pub request: String,
    /// The routing decision: "rlm" or "passthrough".
    pub decision: String,
    /// Reason for the decision.
    pub reason: String,
    /// How the decision was made.
    pub method: String,
}

// ============================================================================
// Router Decision
// ============================================================================

/// The routing decision for a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteDecision {
    /// Pass request directly to upstream API.
    Passthrough,
    /// Process through RLM for context enrichment.
    Rlm {
        /// Reason for RLM routing (for logging/debugging).
        reason: String,
    },
}

impl RouteDecision {
    pub fn passthrough() -> Self {
        Self::Passthrough
    }

    pub fn rlm(reason: impl Into<String>) -> Self {
        Self::Rlm {
            reason: reason.into(),
        }
    }

    pub fn is_rlm(&self) -> bool {
        matches!(self, Self::Rlm { .. })
    }

    pub fn is_passthrough(&self) -> bool {
        matches!(self, Self::Passthrough)
    }
}

// ============================================================================
// Router Strategy & Configuration
// ============================================================================

/// Strategy for making routing decisions.
#[derive(Debug, Clone, Default)]
pub enum RouterStrategy {
    /// Use LLM to classify requests (default).
    #[default]
    Llm,
    /// Always use RLM (for testing/development).
    AlwaysRlm,
    /// Always passthrough (disable RLM).
    AlwaysPassthrough,
}

/// Configuration for the request router.
#[derive(Debug, Clone)]
pub struct RouterConfig {
    /// Routing strategy to use.
    pub strategy: RouterStrategy,
    /// Enable/disable the router (if disabled, always passthrough).
    pub enabled: bool,
    /// Model to use for LLM-based routing (if different from default).
    pub router_model: Option<String>,
}

impl Default for RouterConfig {
    fn default() -> Self {
        Self {
            strategy: RouterStrategy::Llm,
            enabled: true,
            router_model: None,
        }
    }
}

// ============================================================================
// Input Extraction & Preprocessing
// ============================================================================

/// Preprocessed input for routing decisions.
/// Contains the cleaned user message ready for analysis.
struct RoutingInput {
    /// The cleaned user message (after stripping control tags).
    text: String,
}

/// Control tag patterns to strip from router input.
/// These XML-like tags can be 35KB+ and would overwhelm the router LLM.
const CONTROL_TAG_PATTERNS: &[&str] = &[
    r"(?si)<system-reminder>.*?</system-reminder>",
    r"(?si)<context>.*?</context>",
    r"(?si)<metadata>.*?</metadata>",
    r"(?si)<internal>.*?</internal>",
];

/// Strip XML control tags from text.
fn strip_control_tags(text: &str) -> String {
    let original_len = text.len();
    let mut result = text.to_string();

    for pattern in CONTROL_TAG_PATTERNS {
        let re = Regex::new(pattern).expect("Invalid control tag pattern");
        result = re.replace_all(&result, "").to_string();
    }

    let result = result.trim().to_string();

    if result.len() != original_len {
        tracing::debug!(
            original_len,
            stripped_len = result.len(),
            bytes_stripped = original_len - result.len(),
            "Stripped control tags"
        );
    }

    result
}

/// Extract and clean the last user message for routing.
///
/// This function:
/// 1. Finds the last user message
/// 2. Strips control tags (system-reminder, context, etc.)
/// 3. Returns None if empty after stripping
/// 4. Logs AFTER transformation (key for debugging)
fn extract_routing_input(request: &CompletionRequest) -> Option<RoutingInput> {
    // Find last user message
    let last_msg = request
        .messages
        .iter()
        .rev()
        .find(|m| m.role == Role::User)?;

    // Strip control tags FIRST
    let text = strip_control_tags(&last_msg.content.to_text());

    // Empty after stripping? Return None
    if text.is_empty() {
        tracing::debug!("Message empty after stripping control tags");
        return None;
    }

    // Log AFTER transformation (this is the key fix - we log what we actually use)
    tracing::debug!(
        text_length = text.len(),
        text_preview = %text.chars().take(100).collect::<String>(),
        "Routing input extracted"
    );

    Some(RoutingInput { text })
}

// ============================================================================
// Fast Path Checks (no LLM needed)
// ============================================================================

/// Patterns that should always passthrough - internal Claude Code requests.
const PASSTHROUGH_BYPASS_PATTERNS: &[&str] = &[
    r"(?i)^please write a \d+-\d+ word title", // Title generation
    r"(?i)^you are now a prompt suggestion generator", // Autocomplete suggestions
];

/// Check if text matches internal bypass patterns (title gen, autocomplete).
fn should_bypass(text: &str) -> bool {
    for pattern in PASSTHROUGH_BYPASS_PATTERNS {
        if let Ok(re) = Regex::new(pattern) {
            if re.is_match(text) {
                tracing::debug!(
                    pattern,
                    "Bypassing to passthrough: matches internal request pattern"
                );
                return true;
            }
        }
    }
    false
}

/// Regex pattern for explicit RLM trigger ({at}muninn explore).
/// Must be at start of a line to avoid false positives from code/logs in context.
fn rlm_trigger_pattern() -> Regex {
    Regex::new(r"(?im)^@muninn\s+explore").expect("Invalid regex")
}

/// Regex pattern for explicit passthrough trigger ({at}muninn passthrough).
/// Allows user to bypass RLM and use upstream directly for expensive queries.
fn passthrough_trigger_pattern() -> Regex {
    Regex::new(r"(?im)^@muninn\s+passthrough").expect("Invalid regex")
}

/// Check if text contains the explicit RLM trigger.
fn has_rlm_trigger(text: &str) -> bool {
    rlm_trigger_pattern().is_match(text)
}

/// Check if text contains the explicit passthrough trigger.
fn has_passthrough_trigger(text: &str) -> bool {
    passthrough_trigger_pattern().is_match(text)
}

// ============================================================================
// LLM-Based Routing
// ============================================================================

/// Tool input for parsing the route_decision tool response.
#[derive(Debug, Clone, Deserialize)]
struct RouteDecisionInput {
    route: String,
    reason: String,
}

/// System prompt for the router LLM.
const ROUTER_SYSTEM_PROMPT: &str = "You route requests. Use 'rlm' for questions about code structure, implementation, architecture, or anything requiring reading source files. Use 'passthrough' for commands, log analysis, or tasks that don't need source code exploration.";

/// Build the user message for the router LLM.
fn build_router_user_message(user_request: &str) -> String {
    format!(
        r#"Analyze this user request and decide how it should be routed.

USER REQUEST:
{}

ROUTING RULES:

Use "rlm" for questions about SOURCE CODE, implementation, or architecture:
- "How does authentication work in this app?"
- "Explain the implementation of X"
- "Help me understand how information flows through Y"
- "Where is the router implemented?"
- "What does the Config struct look like?"
- "Find the function that handles X"
- "Show me the codebase structure"

Use "passthrough" for operational tasks that don't need code exploration:
- Running commands ("run tests", "build", "grep for X")
- Checking logs/output ("check the logs", "what errors occurred?")
- Writing/editing code when context is already provided
- Follow-up clarifying questions about previous answers
- General conversation ("ping", "what happened?")

If the request asks about "implementation", "architecture", "how X works", or "code structure", use rlm."#,
        user_request
    )
}

/// Create the route_decision tool definition.
fn route_decision_tool() -> ToolDefinition {
    ToolDefinition::new(
        "route_decision",
        "Make a routing decision for the user's request.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "route": {
                    "type": "string",
                    "enum": ["rlm", "passthrough"],
                    "description": "Use 'rlm' for SOURCE CODE exploration, 'passthrough' for everything else."
                },
                "reason": {
                    "type": "string",
                    "description": "Brief explanation (1-2 sentences)."
                }
            },
            "required": ["route", "reason"]
        }),
    )
}

/// Build the CompletionRequest for the router LLM.
fn build_router_request(user_message: &str, router_model: &Option<String>) -> CompletionRequest {
    let model = router_model.clone().unwrap_or_else(|| "router".to_string());

    CompletionRequest {
        model,
        messages: vec![Message::user(build_router_user_message(user_message))],
        system: Some(SystemPrompt::Text(ROUTER_SYSTEM_PROMPT.to_string())),
        max_tokens: 256,
        temperature: Some(0.0),
        tools: vec![route_decision_tool()],
        tool_choice: Some(ToolChoice::Tool {
            name: "route_decision".to_string(),
        }),
        stream: false,
        stop_sequences: Vec::new(),
        top_p: None,
        top_k: None,
        muninn: None,
        metadata: HashMap::new(),
        thinking: None,
    }
}

/// Parse the router LLM response into a RouteDecision.
fn parse_route_response(response: &CompletionResponse) -> RouteDecision {
    // Try to extract tool call
    if let Some(tool_use) = response.tool_uses().first() {
        if tool_use.name == "route_decision" {
            match serde_json::from_value::<RouteDecisionInput>(tool_use.input.clone()) {
                Ok(decision) => {
                    let route = decision.route.to_lowercase();
                    if route == "rlm" || route == "explore" {
                        return RouteDecision::rlm(format!("Router LLM: {}", decision.reason));
                    } else {
                        return RouteDecision::passthrough();
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Failed to parse route_decision tool input");
                }
            }
        }
    }

    // Fallback: check text response for keywords
    let text = response.text().to_lowercase();
    if text.contains("rlm") || text.contains("explore") {
        RouteDecision::rlm("Router LLM fallback: text contained rlm/explore")
    } else {
        RouteDecision::passthrough()
    }
}

// ============================================================================
// Router
// ============================================================================

/// Request router that decides between RLM and passthrough.
pub struct Router {
    config: RouterConfig,
    llm: Option<Arc<dyn LLMBackend>>,
}

impl Router {
    /// Create a new router with default configuration.
    pub fn new() -> Self {
        Self {
            config: RouterConfig::default(),
            llm: None,
        }
    }

    /// Create with custom configuration.
    pub fn with_config(config: RouterConfig) -> Self {
        Self { config, llm: None }
    }

    /// Set the LLM backend for LLM-based routing.
    pub fn with_llm(mut self, llm: Arc<dyn LLMBackend>) -> Self {
        self.llm = Some(llm);
        self
    }

    /// Route a request to either passthrough or RLM.
    ///
    /// # Routing Phases
    ///
    /// 1. **Disabled check** - If router disabled, passthrough
    /// 2. **Extract & clean** - Get last user message, strip control tags
    /// 3. **Fast bypass** - Check for internal requests (title gen, autocomplete)
    /// 4. **Text triggers** - Check for explicit triggers:
    ///    - `{at}muninn passthrough` - Force passthrough to upstream
    ///    - `{at}muninn explore` - Force RLM processing
    /// 5. **Strategy** - Use configured strategy (LLM, AlwaysRlm, AlwaysPassthrough)
    pub async fn route(&self, request: &CompletionRequest) -> RouteDecision {
        let start = Instant::now();

        // Phase 1: Quick exit if disabled
        if !self.config.enabled {
            return self.finish(
                RouteDecision::passthrough(),
                "disabled",
                None,
                request,
                start,
            );
        }

        // Phase 2: Extract and clean input (logs AFTER stripping)
        let input = match extract_routing_input(request) {
            Some(i) => i,
            None => {
                return self.finish(
                    RouteDecision::passthrough(),
                    "no_message",
                    None,
                    request,
                    start,
                );
            }
        };

        // Phase 3: Fast bypass for internal requests
        if should_bypass(&input.text) {
            return self.finish(
                RouteDecision::passthrough(),
                "internal_bypass",
                Some(&input.text),
                request,
                start,
            );
        }

        // Phase 4: Check for text triggers
        if has_passthrough_trigger(&input.text) {
            return self.finish(
                RouteDecision::passthrough(),
                "passthrough_trigger",
                Some(&input.text),
                request,
                start,
            );
        }
        if has_rlm_trigger(&input.text) {
            return self.finish(
                RouteDecision::rlm("Text trigger: {at}muninn explore"),
                "rlm_trigger",
                Some(&input.text),
                request,
                start,
            );
        }

        // Phase 5: Strategy-based routing
        let (decision, method) = match &self.config.strategy {
            RouterStrategy::AlwaysPassthrough => {
                (RouteDecision::passthrough(), "forced_passthrough")
            }
            RouterStrategy::AlwaysRlm => (RouteDecision::rlm("Strategy: AlwaysRlm"), "forced_rlm"),
            RouterStrategy::Llm => (self.route_via_llm(&input.text).await, "llm"),
        };

        self.finish(decision, method, Some(&input.text), request, start)
    }

    /// Call the router LLM to make a routing decision.
    async fn route_via_llm(&self, user_message: &str) -> RouteDecision {
        let Some(llm) = &self.llm else {
            tracing::error!("Router LLM not configured");
            return RouteDecision::passthrough();
        };

        let request = build_router_request(user_message, &self.config.router_model);

        match llm.complete(request).await {
            Ok(response) => parse_route_response(&response),
            Err(e) => {
                tracing::warn!(error = %e, "Router LLM failed");
                RouteDecision::passthrough()
            }
        }
    }

    /// Emit trace data and return the decision.
    fn finish(
        &self,
        decision: RouteDecision,
        method: &str,
        cleaned_message: Option<&str>,
        request: &CompletionRequest,
        start: Instant,
    ) -> RouteDecision {
        let trace_data = RouterTraceData {
            strategy: format!("{:?}", self.config.strategy),
            method: method.to_string(),
            model: request.model.clone(),
            system_prompt: request.system.as_ref().map(|s| s.to_text()),
            last_user_message: cleaned_message.map(String::from),
            message_count: request.messages.len(),
            decision: if decision.is_rlm() {
                "rlm".to_string()
            } else {
                "passthrough".to_string()
            },
            reason: match &decision {
                RouteDecision::Rlm { reason } => Some(reason.clone()),
                RouteDecision::Passthrough => None,
            },
            decision_time_ms: start.elapsed().as_millis() as u64,
        };

        muninn_tracing::start_span_with_data("router_decision", &trace_data);
        muninn_tracing::end_span_ok();

        decision
    }
}

impl Default for Router {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::MockBackend;
    use crate::types::{ContentBlock, StopReason, Usage};

    fn make_request(messages: Vec<(&str, &str)>) -> CompletionRequest {
        CompletionRequest {
            model: "test".to_string(),
            messages: messages
                .into_iter()
                .map(|(role, content)| match role {
                    "user" => Message::user(content),
                    "assistant" => Message::assistant(content),
                    _ => Message::user(content),
                })
                .collect(),
            system: None,
            max_tokens: 1024,
            temperature: None,
            tools: Vec::new(),
            tool_choice: None,
            stream: false,
            stop_sequences: Vec::new(),
            top_p: None,
            top_k: None,
            muninn: None,
            metadata: HashMap::new(),
            thinking: None,
        }
    }

    fn mock_route_response(route: &str, reason: &str) -> CompletionResponse {
        CompletionResponse::new(
            "test-id",
            "test-model",
            vec![ContentBlock::ToolUse {
                id: "tool-1".to_string(),
                name: "route_decision".to_string(),
                input: serde_json::json!({
                    "route": route,
                    "reason": reason
                }),
                cache_control: None,
            }],
            StopReason::ToolUse,
            Usage {
                input_tokens: 10,
                output_tokens: 5,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
            },
        )
    }

    #[tokio::test]
    async fn test_llm_routes_passthrough() {
        let backend = Arc::new(MockBackend::new(vec![mock_route_response(
            "passthrough",
            "Simple math question",
        )]));
        let router = Router::new().with_llm(backend);
        let request = make_request(vec![("user", "What is 2+2?")]);

        let decision = router.route(&request).await;
        assert!(decision.is_passthrough());
    }

    #[tokio::test]
    async fn test_llm_routes_rlm() {
        let backend = Arc::new(MockBackend::new(vec![mock_route_response(
            "rlm",
            "Needs to explore codebase",
        )]));
        let router = Router::new().with_llm(backend);
        let request = make_request(vec![("user", "Find all functions that call parse()")]);

        let decision = router.route(&request).await;
        assert!(decision.is_rlm());
    }

    #[tokio::test]
    async fn test_strategy_always_passthrough() {
        let config = RouterConfig {
            strategy: RouterStrategy::AlwaysPassthrough,
            enabled: true,
            router_model: None,
        };
        let router = Router::with_config(config);
        let request = make_request(vec![("user", "Explain the entire codebase")]);

        let decision = router.route(&request).await;
        assert!(decision.is_passthrough());
    }

    #[tokio::test]
    async fn test_strategy_always_rlm() {
        let config = RouterConfig {
            strategy: RouterStrategy::AlwaysRlm,
            enabled: true,
            router_model: None,
        };
        let router = Router::with_config(config);
        let request = make_request(vec![("user", "Hello")]);

        let decision = router.route(&request).await;
        assert!(decision.is_rlm());
    }

    #[tokio::test]
    async fn test_router_disabled() {
        let config = RouterConfig {
            strategy: RouterStrategy::Llm,
            enabled: false,
            router_model: None,
        };
        let router = Router::with_config(config);
        let request = make_request(vec![("user", "Explain the entire codebase architecture")]);

        let decision = router.route(&request).await;
        assert!(decision.is_passthrough());
    }

    #[tokio::test]
    async fn test_rlm_trigger_forces_rlm() {
        let router = Router::new();
        let request = make_request(vec![("user", "@muninn explore the codebase")]);

        let decision = router.route(&request).await;
        assert!(decision.is_rlm());
    }

    #[tokio::test]
    async fn test_rlm_trigger_case_insensitive() {
        let router = Router::new();
        let request = make_request(vec![("user", "@MUNINN EXPLORE please help")]);

        let decision = router.route(&request).await;
        assert!(decision.is_rlm());
    }

    #[tokio::test]
    async fn test_rlm_trigger_requires_line_start() {
        let router = Router::new();
        // Trigger buried in text should NOT match (prevents false positives from logs/code)
        let request = make_request(vec![("user", "some text @muninn explore more text")]);

        let decision = router.route(&request).await;
        assert!(decision.is_passthrough());
    }

    #[tokio::test]
    async fn test_rlm_trigger_works_on_newline() {
        let router = Router::new();
        // Trigger on a new line should work
        let request = make_request(vec![("user", "some context\n@muninn explore")]);

        let decision = router.route(&request).await;
        assert!(decision.is_rlm());
    }

    #[tokio::test]
    async fn test_passthrough_trigger_forces_passthrough() {
        // Even with AlwaysRlm strategy, trigger should force passthrough
        let config = RouterConfig {
            strategy: RouterStrategy::AlwaysRlm,
            enabled: true,
            router_model: None,
        };
        let router = Router::with_config(config);
        let request = make_request(vec![("user", "@muninn passthrough explain the codebase")]);

        let decision = router.route(&request).await;
        assert!(decision.is_passthrough());
    }

    #[tokio::test]
    async fn test_passthrough_trigger_case_insensitive() {
        let router = Router::new();
        let request = make_request(vec![("user", "@MUNINN PASSTHROUGH do the thing")]);

        let decision = router.route(&request).await;
        assert!(decision.is_passthrough());
    }

    #[tokio::test]
    async fn test_passthrough_trigger_requires_line_start() {
        let router = Router::new();
        // Trigger buried in text should NOT match
        let request = make_request(vec![("user", "text @muninn passthrough more")]);

        // Without line-start, falls through to strategy (no LLM = passthrough anyway)
        let decision = router.route(&request).await;
        assert!(decision.is_passthrough());
    }

    #[tokio::test]
    async fn test_passthrough_trigger_works_on_newline() {
        let config = RouterConfig {
            strategy: RouterStrategy::AlwaysRlm,
            enabled: true,
            router_model: None,
        };
        let router = Router::with_config(config);
        let request = make_request(vec![("user", "context\n@muninn passthrough")]);

        let decision = router.route(&request).await;
        assert!(decision.is_passthrough());
    }

    #[tokio::test]
    async fn test_no_llm_configured_defaults_to_passthrough() {
        let router = Router::new(); // No LLM
        let request = make_request(vec![("user", "Some request without text trigger")]);

        let decision = router.route(&request).await;
        assert!(decision.is_passthrough());
    }

    #[tokio::test]
    async fn test_bypass_title_generation() {
        let router = Router::new();
        let request = make_request(vec![(
            "user",
            "please write a 3-5 word title for this conversation",
        )]);

        let decision = router.route(&request).await;
        assert!(decision.is_passthrough());
    }

    #[tokio::test]
    async fn test_bypass_prompt_suggestion() {
        let router = Router::new();
        let request = make_request(vec![(
            "user",
            "you are now a prompt suggestion generator for coding assistance",
        )]);

        let decision = router.route(&request).await;
        assert!(decision.is_passthrough());
    }

    #[tokio::test]
    async fn test_strip_control_tags() {
        let input = "<system-reminder>lots of stuff</system-reminder>actual user message";
        let result = strip_control_tags(input);
        assert_eq!(result, "actual user message");
    }

    #[tokio::test]
    async fn test_empty_after_stripping_passthrough() {
        let router = Router::new();
        let request = make_request(vec![(
            "user",
            "<system-reminder>only system reminder content</system-reminder>",
        )]);

        let decision = router.route(&request).await;
        assert!(decision.is_passthrough());
    }

    #[test]
    fn test_has_rlm_trigger() {
        // Valid triggers (at start of line)
        assert!(has_rlm_trigger("@muninn explore"));
        assert!(has_rlm_trigger("@MUNINN EXPLORE"));
        assert!(has_rlm_trigger("@muninn  explore with extra spaces"));
        assert!(has_rlm_trigger("some text\n@muninn explore")); // newline counts as line start

        // Invalid triggers
        assert!(!has_rlm_trigger("hello world"));
        assert!(!has_rlm_trigger("middle @muninn explore text")); // not at line start
        assert!(!has_rlm_trigger("@muninn")); // missing explore
        assert!(!has_rlm_trigger("muninn explore")); // missing @
    }

    #[test]
    fn test_has_passthrough_trigger() {
        // Valid triggers (at start of line)
        assert!(has_passthrough_trigger("@muninn passthrough"));
        assert!(has_passthrough_trigger("@MUNINN PASSTHROUGH"));
        assert!(has_passthrough_trigger(
            "@muninn  passthrough with extra text"
        ));
        assert!(has_passthrough_trigger("some context\n@muninn passthrough")); // newline counts

        // Invalid triggers
        assert!(!has_passthrough_trigger("hello world"));
        assert!(!has_passthrough_trigger("middle @muninn passthrough text")); // not at line start
        assert!(!has_passthrough_trigger("@muninn")); // missing passthrough
        assert!(!has_passthrough_trigger("muninn passthrough")); // missing @
    }

    #[test]
    fn test_should_bypass() {
        assert!(should_bypass("please write a 3-5 word title"));
        assert!(should_bypass(
            "Please write a 10-20 word title for this chat"
        ));
        assert!(should_bypass("you are now a prompt suggestion generator"));
        assert!(!should_bypass("how does the router work?"));
        assert!(!should_bypass("please write some code"));
    }
}
