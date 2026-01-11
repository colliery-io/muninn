//! HTTP mock server for integration testing.
//!
//! Provides an HTTP server that mimics an LLM API endpoint for testing
//! the full request/response cycle without real API calls.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::{Json, Router, extract::State, http::StatusCode, response::IntoResponse, routing::post};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::backend::LLMBackend;
use crate::testing::MockLLMBackend;
use crate::types::{CompletionRequest, CompletionResponse};

/// An HTTP mock server for LLM API testing.
///
/// Starts a local HTTP server that accepts completion requests and returns
/// queued responses. Useful for integration tests that need to test the
/// full HTTP request cycle.
///
/// # Example
///
/// ```ignore
/// use muninn_rlm::testing::{MockLLMServer, fixtures};
///
/// let server = MockLLMServer::start().await;
/// server.queue_response(fixtures::text_response("Hello!"));
///
/// // Make HTTP request to server.url()
/// let response = reqwest::Client::new()
///     .post(format!("{}/v1/messages", server.url()))
///     .json(&request)
///     .send()
///     .await;
///
/// server.shutdown().await;
/// ```
pub struct MockLLMServer {
    /// Server address.
    addr: SocketAddr,
    /// The mock backend handling requests.
    backend: Arc<MockLLMBackend>,
    /// Shutdown signal sender.
    shutdown_tx: Option<oneshot::Sender<()>>,
    /// Server task handle.
    handle: Option<JoinHandle<()>>,
}

impl MockLLMServer {
    /// Start a new mock server on a random available port.
    pub async fn start() -> Self {
        Self::start_with_backend(MockLLMBackend::new()).await
    }

    /// Start a mock server with a pre-configured backend.
    pub async fn start_with_backend(backend: MockLLMBackend) -> Self {
        let backend = Arc::new(backend);
        let backend_clone = Arc::clone(&backend);

        // Build the router
        let app = Router::new()
            .route("/v1/messages", post(handle_messages))
            .route("/health", axum::routing::get(handle_health))
            .with_state(backend_clone);

        // Bind to a random port
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("Failed to bind mock server");
        let addr = listener.local_addr().expect("Failed to get local address");

        // Create shutdown channel
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

        // Spawn the server
        let handle = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    shutdown_rx.await.ok();
                })
                .await
                .ok();
        });

        // Give the server a moment to start
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        Self {
            addr,
            backend,
            shutdown_tx: Some(shutdown_tx),
            handle: Some(handle),
        }
    }

    /// Get the server's base URL.
    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Get the server's address.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Queue a response to be returned.
    pub fn queue_response(&self, response: CompletionResponse) {
        self.backend.queue_response(response);
    }

    /// Get captured requests.
    pub fn captured_requests(&self) -> Vec<CompletionRequest> {
        self.backend.captured_requests()
    }

    /// Get the number of requests made.
    pub fn request_count(&self) -> usize {
        self.backend.request_count()
    }

    /// Assert that exactly N requests were made.
    pub fn assert_request_count(&self, expected: usize) {
        self.backend.assert_request_count(expected);
    }

    /// Clear captured requests.
    pub fn clear_requests(&self) {
        self.backend.clear_requests();
    }

    /// Shutdown the server.
    pub async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.await;
        }
    }
}

/// Handle POST /v1/messages
async fn handle_messages(
    State(backend): State<Arc<MockLLMBackend>>,
    Json(request): Json<CompletionRequest>,
) -> impl IntoResponse {
    match backend.complete(request).await {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(e) => {
            let error_body = serde_json::json!({
                "error": {
                    "type": "server_error",
                    "message": e.to_string()
                }
            });
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error_body)).into_response()
        }
    }
}

/// Handle GET /health
async fn handle_health() -> impl IntoResponse {
    (StatusCode::OK, Json(serde_json::json!({"status": "ok"})))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::fixtures;

    #[tokio::test]
    async fn test_mock_server_start_and_shutdown() {
        let server = MockLLMServer::start().await;
        let url = server.url();
        assert!(url.starts_with("http://127.0.0.1:"));
        server.shutdown().await;
    }

    #[tokio::test]
    async fn test_mock_server_health_check() {
        let server = MockLLMServer::start().await;

        let client = reqwest::Client::new();
        let response = client
            .get(format!("{}/health", server.url()))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), 200);

        let body: serde_json::Value = response.json().await.unwrap();
        assert_eq!(body["status"], "ok");

        server.shutdown().await;
    }

    #[tokio::test]
    async fn test_mock_server_completion_request() {
        let server = MockLLMServer::start().await;
        server.queue_response(fixtures::text_response("Hello from mock!"));

        let client = reqwest::Client::new();
        let request = fixtures::simple_request();

        let response = client
            .post(format!("{}/v1/messages", server.url()))
            .json(&request)
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), 200);

        let completion: CompletionResponse = response.json().await.unwrap();
        assert_eq!(completion.text(), "Hello from mock!");

        server.assert_request_count(1);
        server.shutdown().await;
    }

    #[tokio::test]
    async fn test_mock_server_multiple_requests() {
        let server = MockLLMServer::start().await;
        server.queue_response(fixtures::text_response("First"));
        server.queue_response(fixtures::text_response("Second"));

        let client = reqwest::Client::new();
        let request = fixtures::simple_request();

        let r1: CompletionResponse = client
            .post(format!("{}/v1/messages", server.url()))
            .json(&request)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();

        let r2: CompletionResponse = client
            .post(format!("{}/v1/messages", server.url()))
            .json(&request)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();

        assert_eq!(r1.text(), "First");
        assert_eq!(r2.text(), "Second");
        server.assert_request_count(2);

        server.shutdown().await;
    }

    #[tokio::test]
    async fn test_mock_server_no_response_error() {
        let server = MockLLMServer::start().await;
        // No responses queued

        let client = reqwest::Client::new();
        let request = fixtures::simple_request();

        let response = client
            .post(format!("{}/v1/messages", server.url()))
            .json(&request)
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), 500);

        let body: serde_json::Value = response.json().await.unwrap();
        assert!(
            body["error"]["message"]
                .as_str()
                .unwrap()
                .contains("no more responses")
        );

        server.shutdown().await;
    }

    #[tokio::test]
    async fn test_mock_server_captures_requests() {
        let server = MockLLMServer::start().await;
        server.queue_response(fixtures::text_response("Ok"));

        let client = reqwest::Client::new();
        let request = crate::types::CompletionRequest::new(
            "custom-model",
            vec![crate::types::Message::user("Custom message")],
            200,
        );

        let _ = client
            .post(format!("{}/v1/messages", server.url()))
            .json(&request)
            .send()
            .await
            .unwrap();

        let captured = server.captured_requests();
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].model, "custom-model");
        assert_eq!(captured[0].max_tokens, 200);

        server.shutdown().await;
    }
}
