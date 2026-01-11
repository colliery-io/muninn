//! Exploration context for tracking state during recursive exploration.

use std::time::Duration;

use crate::types::{
    BudgetConfig, CompletionRequest, CompletionResponse, ContentBlock, ExplorationMetadata,
    Message, StopReason, ToolResultBlock, Usage,
};

use super::budget::BudgetTracker;

/// Context for tracking exploration state.
pub struct ExplorationContext {
    original_request: CompletionRequest,
    messages: Vec<Message>,
    budget: BudgetTracker,
}

impl ExplorationContext {
    pub fn new(request: CompletionRequest, budget: BudgetConfig) -> Self {
        Self {
            messages: request.messages.clone(),
            original_request: request,
            budget: BudgetTracker::new(budget),
        }
    }

    pub fn build_request(&self) -> CompletionRequest {
        CompletionRequest {
            model: self.original_request.model.clone(),
            messages: self.messages.clone(),
            max_tokens: self.original_request.max_tokens,
            system: self.original_request.system.clone(),
            tools: self.original_request.tools.clone(),
            tool_choice: self.original_request.tool_choice.clone(),
            stream: false,
            temperature: self.original_request.temperature,
            top_p: self.original_request.top_p,
            top_k: self.original_request.top_k,
            stop_sequences: self.original_request.stop_sequences.clone(),
            muninn: None,
            metadata: self.original_request.metadata.clone(),
            thinking: None,
        }
    }

    pub fn check_budget(&self) -> crate::error::Result<()> {
        self.budget.check_budget()
    }

    pub fn add_usage(&mut self, usage: &Usage) {
        self.budget.record_tokens(usage.total() as u64);
    }

    pub fn add_tool_interaction(
        &mut self,
        response: CompletionResponse,
        results: Vec<ToolResultBlock>,
    ) {
        self.messages
            .push(Message::assistant_blocks(response.content));
        self.messages.push(Message::tool_results(results.clone()));
        self.budget.record_tool_calls(results.len() as u32);
    }

    pub fn increment_depth(&mut self) {
        self.budget.increment_depth();
    }

    pub fn depth(&self) -> u32 {
        self.budget.depth()
    }

    pub fn tool_call_count(&self) -> u32 {
        self.budget.tool_calls()
    }

    pub fn tokens_used(&self) -> u64 {
        self.budget.tokens_used()
    }

    pub fn is_last_turn(&self) -> bool {
        self.budget.is_last_turn()
    }

    pub fn would_exceed_depth(&self) -> bool {
        self.budget.would_exceed_depth()
    }

    pub fn inject_last_turn_warning(&mut self) {
        let warning = Message::user(
            "This is your FINAL turn - you have reached the exploration limit.\n\n\
             You MUST call `final_answer` NOW with whatever information you have gathered.\n\n\
             DO NOT call any other tools. If you call any tool other than `final_answer`, \
             the request will fail.\n\n\
             Synthesize your findings and provide your best answer based on what you've learned.",
        );
        self.messages.push(warning);
    }

    pub fn elapsed(&self) -> Duration {
        self.budget.elapsed()
    }

    pub fn budget_config(&self) -> &BudgetConfig {
        self.budget.config()
    }

    pub fn build_metadata(&self) -> ExplorationMetadata {
        ExplorationMetadata {
            depth_reached: self.budget.depth(),
            tokens_used: self.budget.tokens_used(),
            tool_calls: self.budget.tool_calls(),
            duration_ms: self.budget.elapsed().as_millis() as u64,
        }
    }

    pub fn finalize(&self, mut response: CompletionResponse) -> CompletionResponse {
        let include_metadata = self
            .original_request
            .muninn
            .as_ref()
            .is_none_or(|m| m.include_metadata);
        if include_metadata {
            response.muninn = Some(self.build_metadata());
        }
        response
    }

    pub fn finalize_with_answer(
        &self,
        mut response: CompletionResponse,
        answer: String,
    ) -> CompletionResponse {
        response.content = vec![ContentBlock::Text {
            text: answer,
            cache_control: None,
        }];
        response.stop_reason = Some(StopReason::EndTurn);
        let include_metadata = self
            .original_request
            .muninn
            .as_ref()
            .is_none_or(|m| m.include_metadata);
        if include_metadata {
            response.muninn = Some(self.build_metadata());
        }
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::MuninnConfig;

    fn make_request() -> CompletionRequest {
        CompletionRequest::new("test-model", vec![Message::user("Hello")], 100)
    }

    #[test]
    fn test_context_creation() {
        let context = ExplorationContext::new(make_request(), BudgetConfig::default());
        assert_eq!(context.depth(), 0);
        assert_eq!(context.tool_call_count(), 0);
        assert_eq!(context.tokens_used(), 0);
    }

    #[test]
    fn test_build_request() {
        let request = CompletionRequest::new("test-model", vec![Message::user("Hello")], 100)
            .with_system("Be helpful");
        let context = ExplorationContext::new(request, BudgetConfig::default());
        let built = context.build_request();
        assert_eq!(built.model, "test-model");
        assert!(built.system.is_some());
        assert!(!built.stream);
        assert!(built.muninn.is_none());
    }

    #[test]
    fn test_add_usage() {
        let mut context = ExplorationContext::new(make_request(), BudgetConfig::default());
        context.add_usage(&Usage::new(100, 50));
        context.add_usage(&Usage::new(50, 25));
        assert_eq!(context.tokens_used(), 225);
    }

    #[test]
    fn test_finalize_with_metadata() {
        let request = CompletionRequest::new("model", vec![Message::user("Hi")], 100)
            .with_muninn(MuninnConfig::recursive());
        let context = ExplorationContext::new(request, BudgetConfig::default());
        let response = CompletionResponse::new(
            "msg_1",
            "model",
            vec![ContentBlock::Text {
                text: "Answer".to_string(),
                cache_control: None,
            }],
            StopReason::EndTurn,
            Usage::new(10, 10),
        );
        let finalized = context.finalize(response);
        assert!(finalized.muninn.is_some());
    }

    #[test]
    fn test_finalize_with_answer() {
        let context = ExplorationContext::new(make_request(), BudgetConfig::default());
        let response = CompletionResponse::new(
            "msg_1",
            "model",
            vec![ContentBlock::Text {
                text: "Intermediate".to_string(),
                cache_control: None,
            }],
            StopReason::ToolUse,
            Usage::new(10, 10),
        );
        let finalized = context.finalize_with_answer(response, "Final answer".to_string());
        assert_eq!(finalized.text(), "Final answer");
        assert_eq!(finalized.stop_reason, Some(StopReason::EndTurn));
    }
}
