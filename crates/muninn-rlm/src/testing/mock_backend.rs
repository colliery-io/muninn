//! Enhanced mock LLM backend for testing.
//!
//! Provides a configurable mock backend that captures requests and returns
//! pre-configured responses. More feature-rich than the basic MockBackend.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;

use crate::backend::{ContentDelta, LLMBackend, ResponseStream, StreamEvent};
use crate::error::{Result, RlmError};
use crate::types::{CompletionRequest, CompletionResponse, StopReason};

/// An enhanced mock LLM backend for testing.
///
/// Features:
/// - Queue multiple responses to be returned in order
/// - Capture all incoming requests for assertions
/// - Simulate streaming with configurable delays
/// - Optional tool support configuration
///
/// # Example
///
/// ```ignore
/// use muninn_rlm::testing::{MockLLMBackend, fixtures};
///
/// let backend = MockLLMBackend::new()
///     .with_response(fixtures::text_response("Hello!"))
///     .with_response(fixtures::text_response("Goodbye!"));
///
/// // First request gets "Hello!", second gets "Goodbye!"
/// ```
#[derive(Debug)]
pub struct MockLLMBackend {
    /// Queued responses to return.
    responses: Arc<Mutex<VecDeque<CompletionResponse>>>,
    /// Captured requests for assertions.
    requests: Arc<Mutex<Vec<CompletionRequest>>>,
    /// Whether to advertise native tool support.
    tool_support: bool,
    /// Simulated latency for responses.
    latency: Option<Duration>,
    /// Backend name.
    name: String,
}

impl MockLLMBackend {
    /// Create a new empty mock backend.
    pub fn new() -> Self {
        Self {
            responses: Arc::new(Mutex::new(VecDeque::new())),
            requests: Arc::new(Mutex::new(Vec::new())),
            tool_support: false,
            latency: None,
            name: "mock-llm".to_string(),
        }
    }

    /// Add a single response to the queue.
    pub fn with_response(self, response: CompletionResponse) -> Self {
        self.responses.lock().unwrap().push_back(response);
        self
    }

    /// Add multiple responses to the queue.
    pub fn with_responses(self, responses: Vec<CompletionResponse>) -> Self {
        let mut queue = self.responses.lock().unwrap();
        for response in responses {
            queue.push_back(response);
        }
        drop(queue);
        self
    }

    /// Enable or disable native tool support.
    pub fn with_tool_support(mut self, enabled: bool) -> Self {
        self.tool_support = enabled;
        self
    }

    /// Set simulated latency for responses.
    pub fn with_latency(mut self, latency: Duration) -> Self {
        self.latency = Some(latency);
        self
    }

    /// Set the backend name.
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// Get all captured requests.
    pub fn captured_requests(&self) -> Vec<CompletionRequest> {
        self.requests.lock().unwrap().clone()
    }

    /// Get the number of captured requests.
    pub fn request_count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }

    /// Assert that exactly N requests were made.
    ///
    /// # Panics
    ///
    /// Panics if the request count doesn't match.
    pub fn assert_request_count(&self, expected: usize) {
        let actual = self.request_count();
        assert_eq!(
            actual, expected,
            "Expected {} requests, but got {}",
            expected, actual
        );
    }

    /// Get the last captured request.
    pub fn last_request(&self) -> Option<CompletionRequest> {
        self.requests.lock().unwrap().last().cloned()
    }

    /// Clear all captured requests.
    pub fn clear_requests(&self) {
        self.requests.lock().unwrap().clear();
    }

    /// Queue a response for the next request.
    pub fn queue_response(&self, response: CompletionResponse) {
        self.responses.lock().unwrap().push_back(response);
    }

    /// Get the number of remaining queued responses.
    pub fn remaining_responses(&self) -> usize {
        self.responses.lock().unwrap().len()
    }

    /// Check if there are any remaining responses.
    pub fn has_responses(&self) -> bool {
        !self.responses.lock().unwrap().is_empty()
    }
}

impl Default for MockLLMBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for MockLLMBackend {
    fn clone(&self) -> Self {
        Self {
            responses: Arc::clone(&self.responses),
            requests: Arc::clone(&self.requests),
            tool_support: self.tool_support,
            latency: self.latency,
            name: self.name.clone(),
        }
    }
}

#[async_trait]
impl LLMBackend for MockLLMBackend {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse> {
        // Capture the request
        self.requests.lock().unwrap().push(request);

        // Simulate latency if configured
        if let Some(latency) = self.latency {
            tokio::time::sleep(latency).await;
        }

        // Return the next queued response
        let mut responses = self.responses.lock().unwrap();
        responses.pop_front().ok_or_else(|| {
            RlmError::Backend("MockLLMBackend: no more responses queued".to_string())
        })
    }

    async fn complete_stream(&self, request: CompletionRequest) -> Result<ResponseStream> {
        // Get the response first
        let response = self.complete(request).await?;

        // Convert to stream events
        let events = vec![
            Ok(StreamEvent::MessageStart {
                id: response.id.clone(),
                model: response.model.clone(),
            }),
            Ok(StreamEvent::ContentBlockStart {
                index: 0,
                content_type: "text".to_string(),
            }),
            Ok(StreamEvent::ContentBlockDelta {
                index: 0,
                delta: ContentDelta::TextDelta(response.text()),
            }),
            Ok(StreamEvent::ContentBlockStop { index: 0 }),
            Ok(StreamEvent::MessageDelta {
                stop_reason: response.stop_reason.unwrap_or(StopReason::EndTurn),
                usage: response.usage,
            }),
            Ok(StreamEvent::MessageStop),
        ];

        Ok(Box::pin(futures::stream::iter(events)))
    }

    fn name(&self) -> &str {
        &self.name
    }

    async fn health_check(&self) -> Result<()> {
        Ok(())
    }

    fn supports_native_tools(&self) -> bool {
        self.tool_support
    }
}

/// A request matcher for conditional responses.
pub struct RequestMatcher {
    model_pattern: Option<String>,
    message_contains: Option<String>,
}

impl RequestMatcher {
    /// Match any request.
    pub fn any() -> Self {
        Self {
            model_pattern: None,
            message_contains: None,
        }
    }

    /// Match requests to a specific model.
    pub fn model(model: impl Into<String>) -> Self {
        Self {
            model_pattern: Some(model.into()),
            message_contains: None,
        }
    }

    /// Match requests containing specific text.
    pub fn contains(text: impl Into<String>) -> Self {
        Self {
            model_pattern: None,
            message_contains: Some(text.into()),
        }
    }

    /// Check if a request matches.
    pub fn matches(&self, request: &CompletionRequest) -> bool {
        if let Some(ref model) = self.model_pattern {
            if !request.model.contains(model) {
                return false;
            }
        }

        if let Some(ref text) = self.message_contains {
            let all_text: String = request
                .messages
                .iter()
                .filter_map(|m| m.content.as_text())
                .collect::<Vec<_>>()
                .join(" ");
            if !all_text.contains(text) {
                return false;
            }
        }

        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::fixtures;
    use crate::types::Message;

    #[tokio::test]
    async fn test_mock_backend_single_response() {
        let backend = MockLLMBackend::new().with_response(fixtures::text_response("Hello!"));

        let request = fixtures::simple_request();
        let response = backend.complete(request).await.unwrap();

        assert_eq!(response.text(), "Hello!");
        backend.assert_request_count(1);
    }

    #[tokio::test]
    async fn test_mock_backend_multiple_responses() {
        let backend = MockLLMBackend::new()
            .with_response(fixtures::text_response("First"))
            .with_response(fixtures::text_response("Second"));

        let r1 = backend.complete(fixtures::simple_request()).await.unwrap();
        let r2 = backend.complete(fixtures::simple_request()).await.unwrap();

        assert_eq!(r1.text(), "First");
        assert_eq!(r2.text(), "Second");
        backend.assert_request_count(2);
    }

    #[tokio::test]
    async fn test_mock_backend_exhausted() {
        let backend = MockLLMBackend::new();

        let result = backend.complete(fixtures::simple_request()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_mock_backend_captured_requests() {
        let backend = MockLLMBackend::new().with_response(fixtures::text_response("Ok"));

        let request =
            CompletionRequest::new("special-model", vec![Message::user("Special request")], 100);
        let _ = backend.complete(request).await;

        let captured = backend.captured_requests();
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].model, "special-model");
    }

    #[tokio::test]
    async fn test_mock_backend_queue_response() {
        let backend = MockLLMBackend::new();

        backend.queue_response(fixtures::text_response("Queued!"));
        assert!(backend.has_responses());

        let response = backend.complete(fixtures::simple_request()).await.unwrap();
        assert_eq!(response.text(), "Queued!");
        assert!(!backend.has_responses());
    }

    #[tokio::test]
    async fn test_mock_backend_with_tool_support() {
        let backend = MockLLMBackend::new().with_tool_support(true);
        assert!(backend.supports_native_tools());

        let backend = MockLLMBackend::new().with_tool_support(false);
        assert!(!backend.supports_native_tools());
    }

    #[tokio::test]
    async fn test_mock_backend_streaming() {
        use futures::StreamExt;

        let backend = MockLLMBackend::new().with_response(fixtures::text_response("Streamed!"));

        let mut stream = backend
            .complete_stream(fixtures::simple_request())
            .await
            .unwrap();

        let mut events = Vec::new();
        while let Some(event) = stream.next().await {
            events.push(event.unwrap());
        }

        assert_eq!(events.len(), 6);
        assert!(matches!(events[0], StreamEvent::MessageStart { .. }));
        assert!(matches!(events[5], StreamEvent::MessageStop));
    }

    #[test]
    fn test_request_matcher_any() {
        let matcher = RequestMatcher::any();
        let request = fixtures::simple_request();
        assert!(matcher.matches(&request));
    }

    #[test]
    fn test_request_matcher_model() {
        let matcher = RequestMatcher::model("test");
        let request = fixtures::simple_request();
        assert!(matcher.matches(&request));

        let matcher = RequestMatcher::model("other");
        assert!(!matcher.matches(&request));
    }

    #[test]
    fn test_request_matcher_contains() {
        let request =
            CompletionRequest::new("model", vec![Message::user("Find the bug in auth")], 100);

        let matcher = RequestMatcher::contains("bug");
        assert!(matcher.matches(&request));

        let matcher = RequestMatcher::contains("feature");
        assert!(!matcher.matches(&request));
    }
}
