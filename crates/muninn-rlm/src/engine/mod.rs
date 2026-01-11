//! Recursive exploration engine.
//!
//! This module implements the core recursive exploration loop that orchestrates
//! LLM completions with tool execution. It manages the exploration context,
//! tracks budget usage, and handles termination conditions.

mod budget;
mod context;
mod dir_tree;
mod tool_executor;
mod trace;

#[cfg(test)]
mod tests;

pub use budget::{BudgetSummary, BudgetTracker};
pub use context::ExplorationContext;
pub use tool_executor::ToolExecutor;
pub use trace::{
    RlmCompletionTraceData, RlmCycleTraceData, RlmIterationTraceData, ToolExecutionTraceData,
};

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use crate::backend::LLMBackend;
use crate::error::Result;
use crate::fs::{RealFileSystem, SharedFileSystem};
use crate::prompts::CORE_RLM_BEHAVIOR;
use crate::tools::ToolEnvironment;
use crate::types::{
    BudgetConfig, CompletionRequest, CompletionResponse, Message, Role, StopReason, SystemPrompt,
};

/// Dependencies for the recursive engine.
#[derive(Clone)]
pub struct EngineDeps {
    pub backend: Arc<dyn LLMBackend>,
    pub tools: Arc<dyn ToolEnvironment>,
    pub file_system: Option<SharedFileSystem>,
}

impl EngineDeps {
    pub fn new(backend: Arc<dyn LLMBackend>, tools: Arc<dyn ToolEnvironment>) -> Self {
        Self {
            backend,
            tools,
            file_system: None,
        }
    }

    pub fn with_file_system(mut self, fs: SharedFileSystem) -> Self {
        self.file_system = Some(fs);
        self
    }

    pub fn file_system(&self) -> SharedFileSystem {
        self.file_system
            .clone()
            .unwrap_or_else(|| Arc::new(RealFileSystem::new()))
    }
}

impl std::fmt::Debug for EngineDeps {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineDeps")
            .field("backend", &self.backend.name())
            .field("tools", &"<ToolEnvironment>")
            .field("file_system", &self.file_system.is_some())
            .finish()
    }
}

/// Configuration for the recursive engine.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub budget: BudgetConfig,
    pub work_dir: Option<PathBuf>,
    pub temperature: Option<f32>,
    pub inject_system_prompt: bool,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            budget: BudgetConfig::default(),
            work_dir: None,
            temperature: Some(0.1),
            inject_system_prompt: true,
        }
    }
}

impl EngineConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_budget(mut self, budget: BudgetConfig) -> Self {
        self.budget = budget;
        self
    }

    pub fn with_work_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.work_dir = Some(path.into());
        self
    }

    pub fn with_temperature(mut self, temp: f32) -> Self {
        self.temperature = Some(temp);
        self
    }

    pub fn without_temperature(mut self) -> Self {
        self.temperature = None;
        self
    }

    pub fn with_system_prompt_injection(mut self, inject: bool) -> Self {
        self.inject_system_prompt = inject;
        self
    }
}

/// Recursive exploration engine.
pub struct RecursiveEngine {
    backend: Arc<dyn LLMBackend>,
    tools: Arc<dyn ToolEnvironment>,
    tool_executor: ToolExecutor,
    #[allow(dead_code)]
    file_system: SharedFileSystem,
    default_budget: BudgetConfig,
    work_dir: Option<PathBuf>,
    #[allow(dead_code)]
    temperature: Option<f32>,
    #[allow(dead_code)]
    inject_system_prompt: bool,
}

impl RecursiveEngine {
    pub fn new(deps: EngineDeps, config: EngineConfig) -> Self {
        let file_system = deps.file_system();
        let tool_executor = ToolExecutor::new(deps.tools.clone());
        Self {
            backend: deps.backend,
            tools: deps.tools,
            tool_executor,
            file_system,
            default_budget: config.budget,
            work_dir: config.work_dir,
            temperature: config.temperature,
            inject_system_prompt: config.inject_system_prompt,
        }
    }

    pub fn with_deps(deps: EngineDeps) -> Self {
        Self::new(deps, EngineConfig::default())
    }

    pub fn from_components(backend: Arc<dyn LLMBackend>, tools: Arc<dyn ToolEnvironment>) -> Self {
        Self::new(EngineDeps::new(backend, tools), EngineConfig::default())
    }

    #[deprecated(note = "Use EngineConfig::with_budget() instead")]
    pub fn with_default_budget(mut self, budget: BudgetConfig) -> Self {
        self.default_budget = budget;
        self
    }

    #[deprecated(note = "Use EngineConfig::with_work_dir() instead")]
    pub fn with_work_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.work_dir = Some(path.into());
        self
    }

    pub async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse> {
        let cycle_data = RlmCycleTraceData {
            model: request.model.clone(),
            is_recursive: Self::is_recursive(&request),
            initial_message_count: request.messages.len(),
            system_prompt: request.system.as_ref().map(|s| s.to_text()),
        };
        muninn_tracing::start_span_with_data("rlm_cycle", &cycle_data);

        let request = if Self::is_recursive(&request) {
            self.prepare_recursive_request(request)
        } else {
            request
        };

        let mut context = ExplorationContext::new(request, self.default_budget.clone());
        self.run_exploration_loop(&mut context).await
    }

    fn prepare_recursive_request(&self, mut request: CompletionRequest) -> CompletionRequest {
        let tools = self.tools.available_tools();

        // Truncate to the last N user messages + intervening assistant/tool messages.
        // This gives conversational context without overwhelming Qwen's smaller context window
        // (Claude has 200K tokens, Qwen/Groq has much less).
        // TODO: Make this a tunable config parameter (rlm.context_user_messages or similar).
        const RLM_CONTEXT_USER_MESSAGES: usize = 3;
        let original_count = request.messages.len();
        request.messages =
            Self::truncate_to_last_n_user_messages(request.messages, RLM_CONTEXT_USER_MESSAGES);
        if request.messages.len() < original_count {
            tracing::debug!(
                original_count,
                truncated_to = request.messages.len(),
                "Truncated conversation for RLM"
            );
        }

        // Always replace the system prompt with RLM-specific prompt.
        // Claude Code's system prompt tells the model about Bash, Read, Edit, etc.
        // which confuses the RLM. We need our specialized exploration prompt.
        if self.backend.supports_native_tools() {
            let mut system = CORE_RLM_BEHAVIOR.to_string();
            if let Some(tree) = self
                .work_dir
                .as_ref()
                .and_then(|p| dir_tree::generate_dir_tree(p))
            {
                system.push_str("\n\n");
                system.push_str(&tree);
            }
            request.system = Some(SystemPrompt::Text(system));
        } else {
            let mut rlm_prompt = CORE_RLM_BEHAVIOR.to_string();
            let tool_defs = self.backend.format_tool_definitions(&tools);
            if !tool_defs.is_empty() {
                rlm_prompt.push_str("\n\n");
                rlm_prompt.push_str(&tool_defs);
            }
            if let Some(instructions) = self.backend.tool_calling_instructions() {
                rlm_prompt.push('\n');
                rlm_prompt.push_str(instructions);
            }
            request.system = Some(SystemPrompt::Text(rlm_prompt));
        }

        if self.backend.supports_native_tools() {
            request.tools = tools;
        }
        if request.temperature.is_none() {
            request.temperature = Some(0.1);
        }
        request
    }

    async fn run_exploration_loop(
        &self,
        context: &mut ExplorationContext,
    ) -> Result<CompletionResponse> {
        loop {
            if let Err(e) = context.check_budget() {
                self.end_rlm_span(context, "budget_exceeded", false);
                return Err(e);
            }

            if context.is_last_turn() {
                context.inject_last_turn_warning();
            }

            let iter_request = context.build_request();
            let llm_start = Instant::now();
            let response = match self.backend.complete(iter_request.clone()).await {
                Ok(r) => r,
                Err(e) => {
                    self.end_rlm_span(context, "llm_error", false);
                    return Err(e);
                }
            };

            let iteration_data = RlmIterationTraceData {
                depth: context.depth(),
                is_last_turn: context.is_last_turn(),
                message_count: iter_request.messages.len(),
                llm_latency_ms: llm_start.elapsed().as_millis() as u64,
                input_tokens: response.usage.input_tokens,
                output_tokens: response.usage.output_tokens,
                stop_reason: response.stop_reason.as_ref().map(|r| format!("{:?}", r)),
            };
            muninn_tracing::start_span_with_data("rlm_iteration", &iteration_data);
            muninn_tracing::end_span_ok();

            context.add_usage(&response.usage);

            if let Some(answer) = Self::extract_final_pattern(&response) {
                self.end_rlm_span(context, "final_pattern", true);
                return Ok(context.finalize_with_answer(response, answer));
            }

            match response.stop_reason {
                Some(StopReason::EndTurn) | None => {
                    self.end_rlm_span(context, "end_turn", false);
                    return Ok(context.finalize(response));
                }
                Some(StopReason::ToolUse) => {
                    if let Some(answer) = Self::extract_final_answer_tool(&response) {
                        self.end_rlm_span(context, "final_answer_tool", true);
                        return Ok(context.finalize_with_answer(response, answer));
                    }
                    if context.would_exceed_depth() {
                        let msg = format!(
                            "[Exploration limit reached]\nModel made {} tool calls across {} iterations.",
                            context.tool_call_count(),
                            context.depth()
                        );
                        self.end_rlm_span(context, "forced_termination", true);
                        return Ok(context.finalize_with_answer(response, msg));
                    }
                    let results = self.tool_executor.execute_tools(&response).await?;
                    context.add_tool_interaction(response, results);
                    context.increment_depth();
                }
                Some(StopReason::MaxTokens) => {
                    self.end_rlm_span(context, "max_tokens", false);
                    return Ok(context.finalize(response));
                }
                Some(StopReason::StopSequence) => {
                    self.end_rlm_span(context, "stop_sequence", false);
                    return Ok(context.finalize(response));
                }
            }
        }
    }

    fn end_rlm_span(&self, context: &ExplorationContext, reason: &str, has_final: bool) {
        let data = RlmCompletionTraceData {
            termination_reason: reason.to_string(),
            depth_reached: context.depth(),
            tool_calls: context.tool_call_count(),
            tokens_used: context.tokens_used(),
            duration_ms: context.elapsed().as_millis() as u64,
            has_final_answer: has_final,
        };
        muninn_tracing::record_event("rlm_completion", Some(&data));
        muninn_tracing::end_span_ok();
    }

    pub fn is_recursive(request: &CompletionRequest) -> bool {
        request.muninn.as_ref().is_some_and(|m| m.recursive)
    }

    fn extract_final_pattern(response: &CompletionResponse) -> Option<String> {
        let text = response.text();
        if text.is_empty() {
            return None;
        }
        let re = regex::Regex::new(r#"(?m)^FINAL\(["']?([\s\S]+?)["']?\)$"#).ok()?;
        re.captures(&text)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().trim().to_string())
            .filter(|s| !s.is_empty())
    }

    fn extract_final_answer_tool(response: &CompletionResponse) -> Option<String> {
        response
            .tool_uses()
            .iter()
            .find(|t| t.name == "final_answer")
            .and_then(|t| t.input.get("answer"))
            .and_then(|a| a.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from)
    }

    /// Truncate messages to the last N user messages plus intervening assistant/tool messages.
    /// This preserves conversational context while limiting total message count.
    fn truncate_to_last_n_user_messages(messages: Vec<Message>, n: usize) -> Vec<Message> {
        if n == 0 {
            return vec![];
        }

        // Find indices of user messages
        let user_indices: Vec<usize> = messages
            .iter()
            .enumerate()
            .filter(|(_, m)| m.role == Role::User)
            .map(|(i, _)| i)
            .collect();

        if user_indices.len() <= n {
            // Already within limit
            return messages;
        }

        // Find the start index: the (len - n)th user message
        let start_idx = user_indices[user_indices.len() - n];
        messages.into_iter().skip(start_idx).collect()
    }
}
