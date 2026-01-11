//! Cross-crate integration and E2E tests
//!
//! These tests verify that the crates work together correctly
//! and test full request flows through the system.

use std::sync::Arc;
use std::time::Duration;

use muninn_rlm::{
    CompletionRequest, CompletionResponse, ContentBlock, Message, MockBackend, ProxyConfig,
    ProxyServer, RouterConfig, RouterStrategy, StopReason, ToolRegistry, Usage,
};

/// Get an available port for testing.
fn get_test_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

/// E2E Test: Full passthrough flow through the proxy
///
/// This test verifies the complete request flow:
/// 1. Client sends request to proxy
/// 2. Proxy routes to passthrough (mock fails as expected)
/// 3. Error is properly propagated back to client
#[tokio::test]
async fn test_e2e_passthrough_flow() {
    let port = get_test_port();
    let addr = format!("127.0.0.1:{}", port).parse().unwrap();

    // Create a mock backend - won't be used in passthrough mode
    let backend = Arc::new(MockBackend::new(vec![]));
    let tools = Arc::new(ToolRegistry::new());

    // Configure for passthrough to a non-existent server
    // This proves we're actually attempting passthrough
    let mut config = ProxyConfig::new(addr);
    config = config.with_passthrough(muninn_rlm::PassthroughConfig {
        base_url: "http://127.0.0.1:1".to_string(), // Invalid - will fail
        ..Default::default()
    });

    let router_config = RouterConfig {
        strategy: RouterStrategy::AlwaysPassthrough,
        ..Default::default()
    };

    let server = ProxyServer::with_router(config, backend, tools, router_config);

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let server_handle = tokio::spawn(async move {
        server
            .run_with_shutdown(async {
                shutdown_rx.await.ok();
            })
            .await
    });

    // Wait for server to start
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Send a request through the proxy
    let request = CompletionRequest::new(
        "claude-sonnet-4-20250514",
        vec![Message::user("Hello, Claude!")],
        100,
    );

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://127.0.0.1:{}/v1/messages", port))
        .json(&request)
        .send()
        .await
        .unwrap();

    // Should get an error since passthrough target doesn't exist
    // This proves the request went through the passthrough path
    assert!(
        response.status().is_server_error() || response.status().is_client_error(),
        "Expected error status for passthrough to non-existent server"
    );

    // Cleanup
    shutdown_tx.send(()).unwrap();
    let _ = server_handle.await;
}

/// E2E Test: Full RLM recursive flow through the proxy
///
/// This test verifies the complete RLM flow:
/// 1. Client sends recursive request to proxy
/// 2. Proxy routes to RLM engine
/// 3. Engine processes with mock backend
/// 4. Tool is executed
/// 5. Final response is returned with metadata
#[tokio::test]
async fn test_e2e_rlm_recursive_flow() {
    let port = get_test_port();
    let addr = format!("127.0.0.1:{}", port).parse().unwrap();

    // Create temp directory with test file
    let temp = tempfile::tempdir().unwrap();
    let test_file = temp.path().join("example.rs");
    std::fs::write(&test_file, "fn main() { println!(\"Hello!\"); }").unwrap();

    // Create backend with tool use followed by final response
    let tool_response = CompletionResponse::new(
        "msg_tool",
        "test-model",
        vec![ContentBlock::ToolUse {
            id: "tool_1".to_string(),
            name: "read_file".to_string(),
            input: serde_json::json!({
                "path": "example.rs"
            }),
            cache_control: None,
        }],
        StopReason::ToolUse,
        Usage::new(50, 30),
    );

    let final_response = CompletionResponse::new(
        "msg_final",
        "test-model",
        vec![ContentBlock::Text {
            text: "I found a main function that prints Hello!".to_string(),
            cache_control: None,
        }],
        StopReason::EndTurn,
        Usage::new(80, 50),
    );

    let backend = Arc::new(MockBackend::new(vec![tool_response, final_response]));

    // Create tools with fs tools
    let mut tools = ToolRegistry::new();
    for tool in muninn_rlm::create_fs_tools(temp.path()) {
        tools.register_arc(Arc::from(tool));
    }
    let tools = Arc::new(tools);

    // Configure for always RLM mode
    let config = ProxyConfig::new(addr);
    let router_config = RouterConfig {
        strategy: RouterStrategy::AlwaysRlm,
        ..Default::default()
    };

    let server = ProxyServer::with_router(config, backend, tools, router_config);

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let server_handle = tokio::spawn(async move {
        server
            .run_with_shutdown(async {
                shutdown_rx.await.ok();
            })
            .await
    });

    // Wait for server to start
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Send a recursive request
    let request = CompletionRequest::new(
        "test-model",
        vec![Message::user("What does example.rs contain?")],
        2048,
    );

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://127.0.0.1:{}/v1/messages", port))
        .json(&request)
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);

    let completion: CompletionResponse = response.json().await.unwrap();

    // Verify the response contains expected content
    assert!(
        completion.text().contains("main") || completion.text().contains("Hello"),
        "Expected response to mention the file contents"
    );

    // Verify exploration metadata is present
    assert!(
        completion.muninn.is_some(),
        "Expected exploration metadata in response"
    );

    let metadata = completion.muninn.as_ref().unwrap();
    assert!(metadata.tool_calls > 0, "Expected at least one tool call");
    assert!(
        metadata.tokens_used > 0,
        "Expected token usage to be tracked"
    );

    // Cleanup
    shutdown_tx.send(()).unwrap();
    let _ = server_handle.await;
}

/// E2E Test: Graph tools integration across crates
///
/// Tests that muninn-graph and muninn-rlm work together correctly
/// through the proxy layer.
#[tokio::test]
async fn test_e2e_graph_tools_integration() {
    use muninn_graph::{GraphStore, Symbol, SymbolKind, Visibility};
    use muninn_rlm::{create_graph_tools, wrap_store};

    let port = get_test_port();
    let addr = format!("127.0.0.1:{}", port).parse().unwrap();

    // Create in-memory graph store with test data
    let store = GraphStore::open_in_memory().unwrap();
    let test_symbol = Symbol {
        name: "calculate_sum".to_string(),
        kind: SymbolKind::Function,
        file_path: "src/math.rs".to_string(),
        start_line: 1,
        end_line: 5,
        signature: Some("fn calculate_sum(a: i32, b: i32) -> i32".to_string()),
        qualified_name: Some("crate::math::calculate_sum".to_string()),
        doc_comment: Some("Adds two numbers together".to_string()),
        visibility: Visibility::Public,
    };
    store.insert_node(&test_symbol).unwrap();

    let shared_store = wrap_store(store);

    // Create backend responses for graph tool usage
    let tool_response = CompletionResponse::new(
        "msg_tool",
        "test-model",
        vec![ContentBlock::ToolUse {
            id: "tool_1".to_string(),
            name: "find_symbols".to_string(),
            input: serde_json::json!({
                "name": "calculate"
            }),
            cache_control: None,
        }],
        StopReason::ToolUse,
        Usage::new(30, 20),
    );

    let final_response = CompletionResponse::new(
        "msg_final",
        "test-model",
        vec![ContentBlock::Text {
            text: "Found calculate_sum function in src/math.rs".to_string(),
            cache_control: None,
        }],
        StopReason::EndTurn,
        Usage::new(50, 30),
    );

    let backend = Arc::new(MockBackend::new(vec![tool_response, final_response]));

    // Create tools with graph tools
    let mut tools = ToolRegistry::new();
    for tool in create_graph_tools(shared_store) {
        tools.register_arc(Arc::from(tool));
    }
    let tools = Arc::new(tools);

    let config = ProxyConfig::new(addr);
    let router_config = RouterConfig {
        strategy: RouterStrategy::AlwaysRlm,
        ..Default::default()
    };

    let server = ProxyServer::with_router(config, backend, tools, router_config);

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let server_handle = tokio::spawn(async move {
        server
            .run_with_shutdown(async {
                shutdown_rx.await.ok();
            })
            .await
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let request = CompletionRequest::new(
        "test-model",
        vec![Message::user("Find functions with 'calculate' in the name")],
        2048,
    );

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://127.0.0.1:{}/v1/messages", port))
        .json(&request)
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);

    let completion: CompletionResponse = response.json().await.unwrap();
    assert!(
        completion.text().contains("calculate_sum"),
        "Expected response to mention the found symbol"
    );

    shutdown_tx.send(()).unwrap();
    let _ = server_handle.await;
}
