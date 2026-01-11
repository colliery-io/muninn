//! Budget tracking and enforcement for RLM exploration.
//!
//! This module provides the `BudgetTracker` for monitoring resource usage
//! during recursive exploration: tokens, time, depth, and tool calls.

use std::time::{Duration, Instant};

use crate::error::{BudgetExceededError, BudgetType, Result, RlmError};
use crate::types::BudgetConfig;

/// Tracks resource usage against configured budget limits.
#[derive(Debug, Clone)]
pub struct BudgetTracker {
    config: BudgetConfig,
    started_at: Instant,
    tokens_used: u64,
    tool_calls: u32,
    current_depth: u32,
}

impl BudgetTracker {
    /// Create a new budget tracker with the given configuration.
    pub fn new(config: BudgetConfig) -> Self {
        Self {
            config,
            started_at: Instant::now(),
            tokens_used: 0,
            tool_calls: 0,
            current_depth: 0,
        }
    }

    /// Check if any budget limit has been exceeded.
    pub fn check_budget(&self) -> Result<()> {
        if let Some(max_tokens) = self.config.max_tokens {
            if self.tokens_used >= max_tokens {
                return Err(RlmError::BudgetExceeded(BudgetExceededError {
                    budget_type: BudgetType::Tokens,
                    limit: max_tokens,
                    actual: self.tokens_used,
                }));
            }
        }

        if let Some(max_secs) = self.config.max_duration_secs {
            let elapsed = self.started_at.elapsed().as_secs();
            if elapsed >= max_secs {
                return Err(RlmError::BudgetExceeded(BudgetExceededError {
                    budget_type: BudgetType::Duration,
                    limit: max_secs,
                    actual: elapsed,
                }));
            }
        }

        if let Some(max_depth) = self.config.max_depth {
            if self.current_depth >= max_depth {
                return Err(RlmError::BudgetExceeded(BudgetExceededError {
                    budget_type: BudgetType::Depth,
                    limit: max_depth as u64,
                    actual: self.current_depth as u64,
                }));
            }
        }

        if let Some(max_tool_calls) = self.config.max_tool_calls {
            if self.tool_calls >= max_tool_calls {
                return Err(RlmError::BudgetExceeded(BudgetExceededError {
                    budget_type: BudgetType::ToolCalls,
                    limit: max_tool_calls as u64,
                    actual: self.tool_calls as u64,
                }));
            }
        }

        Ok(())
    }

    pub fn record_tokens(&mut self, tokens: u64) {
        self.tokens_used += tokens;
    }

    pub fn record_tool_calls(&mut self, count: u32) {
        self.tool_calls += count;
    }

    pub fn increment_depth(&mut self) {
        self.current_depth += 1;
    }

    pub fn depth(&self) -> u32 {
        self.current_depth
    }

    pub fn tokens_used(&self) -> u64 {
        self.tokens_used
    }

    pub fn tool_calls(&self) -> u32 {
        self.tool_calls
    }

    pub fn elapsed(&self) -> Duration {
        self.started_at.elapsed()
    }

    pub fn is_last_turn(&self) -> bool {
        self.config
            .max_depth
            .is_some_and(|max| self.current_depth == max.saturating_sub(1))
    }

    pub fn would_exceed_depth(&self) -> bool {
        self.config
            .max_depth
            .is_some_and(|max| self.current_depth >= max.saturating_sub(1))
    }

    pub fn config(&self) -> &BudgetConfig {
        &self.config
    }

    pub fn summary(&self) -> BudgetSummary {
        BudgetSummary {
            tokens_used: self.tokens_used,
            token_limit: self.config.max_tokens,
            tool_calls: self.tool_calls,
            tool_call_limit: self.config.max_tool_calls,
            depth_reached: self.current_depth,
            depth_limit: self.config.max_depth,
            duration_ms: self.started_at.elapsed().as_millis() as u64,
            duration_limit_secs: self.config.max_duration_secs,
        }
    }
}

/// Summary of budget usage for reporting.
#[derive(Debug, Clone)]
pub struct BudgetSummary {
    pub tokens_used: u64,
    pub token_limit: Option<u64>,
    pub tool_calls: u32,
    pub tool_call_limit: Option<u32>,
    pub depth_reached: u32,
    pub depth_limit: Option<u32>,
    pub duration_ms: u64,
    pub duration_limit_secs: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_tracker() {
        let tracker = BudgetTracker::new(BudgetConfig::default());
        assert_eq!(tracker.tokens_used(), 0);
        assert_eq!(tracker.tool_calls(), 0);
        assert_eq!(tracker.depth(), 0);
    }

    #[test]
    fn test_record_tokens() {
        let mut tracker = BudgetTracker::new(BudgetConfig::default());
        tracker.record_tokens(100);
        tracker.record_tokens(50);
        assert_eq!(tracker.tokens_used(), 150);
    }

    #[test]
    fn test_check_budget_tokens_exceeded() {
        let config = BudgetConfig {
            max_tokens: Some(100),
            ..Default::default()
        };
        let mut tracker = BudgetTracker::new(config);
        tracker.record_tokens(150);
        assert!(matches!(
            tracker.check_budget(),
            Err(RlmError::BudgetExceeded(_))
        ));
    }

    #[test]
    fn test_is_last_turn() {
        let config = BudgetConfig {
            max_depth: Some(5),
            ..Default::default()
        };
        let mut tracker = BudgetTracker::new(config);
        for _ in 0..4 {
            tracker.increment_depth();
        }
        assert!(tracker.is_last_turn());
    }

    #[test]
    fn test_summary() {
        let config = BudgetConfig {
            max_tokens: Some(10000),
            max_depth: Some(10),
            ..Default::default()
        };
        let mut tracker = BudgetTracker::new(config);
        tracker.record_tokens(500);
        tracker.record_tool_calls(3);
        let summary = tracker.summary();
        assert_eq!(summary.tokens_used, 500);
        assert_eq!(summary.tool_calls, 3);
    }
}
