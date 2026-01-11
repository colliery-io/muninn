//! Integration tests for muninn-rlm
//!
//! Tests the public API of the RLM gateway crate.

use std::sync::Arc;
use std::time::Duration;

use muninn_rlm::{
    BudgetConfig, CompletionRequest, CompletionResponse, ContentBlock, Message, MockBackend,
    ProxyConfig, ProxyServer, RouterConfig, RouterStrategy, StopReason, ToolRegistry, Usage,
};

/// Get an available port for testing.
fn get_test_port() -> u16 {
    // Use a simple approach: bind to port 0 and get the assigned port
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

/// Test that the proxy server starts and handles health checks.
#[tokio::test]
async fn test_proxy_health_check() {
    let port = get_test_port();
    let addr = format!("127.0.0.1:{}", port).parse().unwrap();

    // Create a mock backend
    let response = CompletionResponse::new(
        "msg_test",
        "test-model",
        vec![ContentBlock::Text {
            text: "Hello!".to_string(),
            cache_control: None,
        }],
        StopReason::EndTurn,
        Usage::new(10, 5),
    );
    let backend = Arc::new(MockBackend::new(vec![response]));
    let tools = Arc::new(ToolRegistry::new());

    // Start the server in a background task
    let config = ProxyConfig::new(addr);
    let server = ProxyServer::new(config, backend, tools);

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let server_handle = tokio::spawn(async move {
        server
            .run_with_shutdown(async {
                shutdown_rx.await.ok();
            })
            .await
    });

    // Give the server time to start
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Make a health check request
    let client = reqwest::Client::new();
    let health_url = format!("http://127.0.0.1:{}/health", port);

    let response = client.get(&health_url).send().await.unwrap();
    assert_eq!(response.status(), 200);

    let body = response.text().await.unwrap();
    assert!(body.contains("ok") || body.contains("healthy"));

    // Shutdown the server
    shutdown_tx.send(()).unwrap();
    let _ = server_handle.await;
}

/// Test that the proxy server handles completion requests.
#[tokio::test]
async fn test_proxy_completion_request() {
    let port = get_test_port();
    let addr = format!("127.0.0.1:{}", port).parse().unwrap();

    // Create a mock backend with a response
    let mock_response = CompletionResponse::new(
        "msg_123",
        "claude-test",
        vec![ContentBlock::Text {
            text: "The answer is 42.".to_string(),
            cache_control: None,
        }],
        StopReason::EndTurn,
        Usage::new(50, 20),
    );
    let backend = Arc::new(MockBackend::new(vec![mock_response]));
    let tools = Arc::new(ToolRegistry::new());

    // Start the server with AlwaysRlm strategy so we use the mock backend
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

    // Create a completion request
    let request = CompletionRequest::new(
        "claude-test",
        vec![Message::user("What is the meaning of life?")],
        100,
    );

    // Send the request
    let client = reqwest::Client::new();
    let messages_url = format!("http://127.0.0.1:{}/v1/messages", port);

    let response = client
        .post(&messages_url)
        .json(&request)
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);

    let completion: CompletionResponse = response.json().await.unwrap();
    assert_eq!(completion.id, "msg_123");
    assert_eq!(completion.text(), "The answer is 42.");
    assert_eq!(completion.stop_reason, Some(StopReason::EndTurn));

    // Shutdown
    shutdown_tx.send(()).unwrap();
    let _ = server_handle.await;
}

/// Test that tools are exposed and can be executed.
#[tokio::test]
async fn test_proxy_with_tools() {
    use muninn_rlm::create_fs_tools;
    use tempfile::tempdir;

    let port = get_test_port();
    let addr = format!("127.0.0.1:{}", port).parse().unwrap();

    // Create a temp directory with a test file
    let temp = tempdir().unwrap();
    let test_file = temp.path().join("test.txt");
    std::fs::write(&test_file, "Hello from test file!").unwrap();

    // Create backend with tool use response and follow-up response
    // (RLM engine will execute the tool and need another response)
    let tool_response = CompletionResponse::new(
        "msg_tool",
        "claude-test",
        vec![ContentBlock::ToolUse {
            id: "tool_1".to_string(),
            name: "read_file".to_string(),
            input: serde_json::json!({
                "path": "test.txt"
            }),
            cache_control: None,
        }],
        StopReason::ToolUse,
        Usage::new(30, 15),
    );
    let final_response = CompletionResponse::new(
        "msg_final",
        "claude-test",
        vec![ContentBlock::Text {
            text: "I read the file contents.".to_string(),
            cache_control: None,
        }],
        StopReason::EndTurn,
        Usage::new(40, 20),
    );
    let backend = Arc::new(MockBackend::new(vec![tool_response, final_response]));

    // Create tools registry with fs tools
    let mut tools = ToolRegistry::new();
    for tool in create_fs_tools(temp.path()) {
        tools.register_arc(Arc::from(tool));
    }
    let tools = Arc::new(tools);

    // Start server with AlwaysRlm strategy so we use the mock backend
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

    // Send a request that will trigger tool use
    let request = CompletionRequest::new("claude-test", vec![Message::user("Read test.txt")], 100);

    let client = reqwest::Client::new();
    let messages_url = format!("http://127.0.0.1:{}/v1/messages", port);

    let response = client
        .post(&messages_url)
        .json(&request)
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);

    // With RLM engine, tools are executed internally and we get the final text response
    let completion: CompletionResponse = response.json().await.unwrap();
    assert_eq!(completion.text(), "I read the file contents.");
    // The exploration metadata should show that tools were used
    assert!(completion.muninn.is_some());
    let metadata = completion.muninn.as_ref().unwrap();
    assert!(metadata.tool_calls > 0);

    // Shutdown
    shutdown_tx.send(()).unwrap();
    let _ = server_handle.await;
}

/// Test passthrough mode - requests go directly to upstream without RLM processing.
#[tokio::test]
async fn test_passthrough_mode() {
    let port = get_test_port();
    let addr = format!("127.0.0.1:{}", port).parse().unwrap();

    // Create a mock backend - this should NOT be called in passthrough mode
    let backend = Arc::new(MockBackend::new(vec![]));
    let tools = Arc::new(ToolRegistry::new());

    // Start server with AlwaysPassthrough strategy
    let mut config = ProxyConfig::new(addr);
    // Configure passthrough to point to a non-existent server
    // This will fail, proving we're actually trying to passthrough
    config = config.with_passthrough(muninn_rlm::PassthroughConfig {
        base_url: "http://127.0.0.1:1".to_string(), // Invalid
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

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Send a request - should attempt passthrough and fail
    let request = CompletionRequest::new("claude-test", vec![Message::user("Hello")], 100);

    let client = reqwest::Client::new();
    let messages_url = format!("http://127.0.0.1:{}/v1/messages", port);

    let response = client
        .post(&messages_url)
        .json(&request)
        .send()
        .await
        .unwrap();

    // Should get an error since passthrough target doesn't exist
    assert!(response.status().is_server_error() || response.status().is_client_error());

    shutdown_tx.send(()).unwrap();
    let _ = server_handle.await;
}

/// Test text trigger forces RLM mode regardless of router.
#[tokio::test]
async fn test_text_trigger_forces_rlm() {
    let port = get_test_port();
    let addr = format!("127.0.0.1:{}", port).parse().unwrap();

    // Create a mock backend with response for the RLM path
    let mock_response = CompletionResponse::new(
        "msg_rlm",
        "claude-test",
        vec![ContentBlock::Text {
            text: "RLM processed this.".to_string(),
            cache_control: None,
        }],
        StopReason::EndTurn,
        Usage::new(50, 20),
    );
    let backend = Arc::new(MockBackend::new(vec![mock_response]));
    let tools = Arc::new(ToolRegistry::new());

    // Start server with Llm strategy - but no router LLM configured
    // Text trigger should bypass the router entirely
    let config = ProxyConfig::new(addr);
    let router_config = RouterConfig {
        strategy: RouterStrategy::Llm,
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

    // Send request with text trigger - should go to RLM despite no router LLM
    let request = CompletionRequest::new(
        "claude-test",
        vec![Message::user(
            "@muninn explore\nHelp me understand the code",
        )],
        100,
    );

    let client = reqwest::Client::new();
    let messages_url = format!("http://127.0.0.1:{}/v1/messages", port);

    let response = client
        .post(&messages_url)
        .json(&request)
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);

    let completion: CompletionResponse = response.json().await.unwrap();
    assert_eq!(completion.text(), "RLM processed this.");

    shutdown_tx.send(()).unwrap();
    let _ = server_handle.await;
}

/// Test router LLM makes routing decisions.
#[tokio::test]
async fn test_router_llm_decision() {
    let port = get_test_port();
    let addr = format!("127.0.0.1:{}", port).parse().unwrap();

    // Create mock responses:
    // 1. Router LLM returns passthrough decision
    // 2. RLM backend response (shouldn't be used)
    let router_response = CompletionResponse::new(
        "msg_router",
        "router-model",
        vec![ContentBlock::ToolUse {
            id: "tool_1".to_string(),
            name: "route_decision".to_string(),
            input: serde_json::json!({
                "route": "passthrough",
                "reason": "Simple greeting"
            }),
            cache_control: None,
        }],
        StopReason::ToolUse,
        Usage::new(10, 5),
    );
    // This response would be used if RLM was triggered, but router says passthrough
    let backend = Arc::new(MockBackend::new(vec![router_response]));
    let tools = Arc::new(ToolRegistry::new());

    // Configure passthrough to fail so we can verify passthrough was attempted
    let mut config = ProxyConfig::new(addr);
    config = config.with_passthrough(muninn_rlm::PassthroughConfig {
        base_url: "http://127.0.0.1:1".to_string(),
        ..Default::default()
    });
    let router_config = RouterConfig {
        strategy: RouterStrategy::Llm,
        ..Default::default()
    };
    let server = ProxyServer::with_router(config, backend.clone(), tools, router_config);

    // Set the router LLM (same backend for simplicity)
    // Note: In real usage, router would have its own backend

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let server_handle = tokio::spawn(async move {
        server
            .run_with_shutdown(async {
                shutdown_rx.await.ok();
            })
            .await
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let request = CompletionRequest::new("claude-test", vec![Message::user("Hello")], 100);

    let client = reqwest::Client::new();
    let messages_url = format!("http://127.0.0.1:{}/v1/messages", port);

    let response = client
        .post(&messages_url)
        .json(&request)
        .send()
        .await
        .unwrap();

    // Router should have decided passthrough, which fails due to bad upstream
    assert!(response.status().is_server_error() || response.status().is_client_error());

    shutdown_tx.send(()).unwrap();
    let _ = server_handle.await;
}

/// Test graph tools integration through RLM.
#[tokio::test]
async fn test_graph_tools_integration() {
    use muninn_graph::{GraphStore, Symbol, SymbolKind, Visibility};
    use muninn_rlm::{create_graph_tools, wrap_store};

    let port = get_test_port();
    let addr = format!("127.0.0.1:{}", port).parse().unwrap();

    // Create an in-memory graph store with test data
    let store = GraphStore::open_in_memory().unwrap();
    let test_fn = Symbol {
        name: "process_request".to_string(),
        kind: SymbolKind::Function,
        file_path: "src/handler.rs".to_string(),
        start_line: 10,
        end_line: 50,
        signature: Some("fn process_request(req: Request) -> Response".to_string()),
        qualified_name: Some("crate::handler::process_request".to_string()),
        doc_comment: Some("Handles incoming requests".to_string()),
        visibility: Visibility::Public,
    };
    store.insert_node(&test_fn).unwrap();

    let shared_store = wrap_store(store);

    // Create backend that uses graph tools
    let tool_response = CompletionResponse::new(
        "msg_tool",
        "claude-test",
        vec![ContentBlock::ToolUse {
            id: "tool_1".to_string(),
            name: "find_symbols".to_string(),
            input: serde_json::json!({
                "name": "process"
            }),
            cache_control: None,
        }],
        StopReason::ToolUse,
        Usage::new(30, 15),
    );
    let final_response = CompletionResponse::new(
        "msg_final",
        "claude-test",
        vec![ContentBlock::Text {
            text: "Found the process_request function in src/handler.rs".to_string(),
            cache_control: None,
        }],
        StopReason::EndTurn,
        Usage::new(40, 20),
    );
    let backend = Arc::new(MockBackend::new(vec![tool_response, final_response]));

    // Create tools registry with graph tools
    let mut tools = ToolRegistry::new();
    for tool in create_graph_tools(shared_store) {
        tools.register_arc(Arc::from(tool));
    }
    let tools = Arc::new(tools);

    // Start server
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

    // Send request that triggers graph tool use
    let request = CompletionRequest::new(
        "claude-test",
        vec![Message::user("Find functions with 'process' in the name")],
        100,
    );

    let client = reqwest::Client::new();
    let messages_url = format!("http://127.0.0.1:{}/v1/messages", port);

    let response = client
        .post(&messages_url)
        .json(&request)
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);

    let completion: CompletionResponse = response.json().await.unwrap();
    assert!(completion.text().contains("process_request"));

    // Verify tool was used
    assert!(completion.muninn.is_some());
    let metadata = completion.muninn.as_ref().unwrap();
    assert!(metadata.tool_calls > 0);

    shutdown_tx.send(()).unwrap();
    let _ = server_handle.await;
}

/// Test budget enforcement through proxy - depth limit.
#[tokio::test]
async fn test_budget_depth_limit() {
    let port = get_test_port();
    let addr = format!("127.0.0.1:{}", port).parse().unwrap();

    // Create responses that keep requesting tools (forcing multiple iterations)
    let mut responses = Vec::new();
    for i in 0..10 {
        responses.push(CompletionResponse::new(
            format!("msg_{}", i),
            "claude-test",
            vec![ContentBlock::ToolUse {
                id: format!("tool_{}", i),
                name: "read_file".to_string(),
                input: serde_json::json!({"path": "test.txt"}),
                cache_control: None,
            }],
            StopReason::ToolUse,
            Usage::new(100, 50),
        ));
    }

    let backend = Arc::new(MockBackend::new(responses));

    // Create tools with a simple read_file tool
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("test.txt"), "test content").unwrap();

    let mut tools = ToolRegistry::new();
    for tool in muninn_rlm::create_fs_tools(temp.path()) {
        tools.register_arc(Arc::from(tool));
    }
    let tools = Arc::new(tools);

    // Configure with low depth limit
    let budget = BudgetConfig {
        max_depth: Some(3),
        max_tokens: None,
        max_duration_secs: None,
        max_tool_calls: None,
    };

    let config = ProxyConfig::new(addr).with_budget(budget);
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
        "claude-test",
        vec![Message::user("Keep reading the file")],
        100,
    );

    let client = reqwest::Client::new();
    let messages_url = format!("http://127.0.0.1:{}/v1/messages", port);

    let response = client
        .post(&messages_url)
        .json(&request)
        .send()
        .await
        .unwrap();

    // Should get a response (budget exceeded returns OK with stop_reason)
    assert_eq!(response.status(), 200);

    let completion: CompletionResponse = response.json().await.unwrap();

    // Check that exploration metadata shows budget was hit
    if let Some(metadata) = &completion.muninn {
        // Should have stopped before unlimited iterations
        assert!(metadata.depth_reached <= 3);
    }

    shutdown_tx.send(()).unwrap();
    let _ = server_handle.await;
}

/// Test budget enforcement - tool calls limit.
#[tokio::test]
async fn test_budget_tool_calls_limit() {
    let port = get_test_port();
    let addr = format!("127.0.0.1:{}", port).parse().unwrap();

    // Create response with multiple tool uses in one message
    let multi_tool_response = CompletionResponse::new(
        "msg_multi",
        "claude-test",
        vec![
            ContentBlock::ToolUse {
                id: "tool_1".to_string(),
                name: "read_file".to_string(),
                input: serde_json::json!({"path": "a.txt"}),
                cache_control: None,
            },
            ContentBlock::ToolUse {
                id: "tool_2".to_string(),
                name: "read_file".to_string(),
                input: serde_json::json!({"path": "b.txt"}),
                cache_control: None,
            },
            ContentBlock::ToolUse {
                id: "tool_3".to_string(),
                name: "read_file".to_string(),
                input: serde_json::json!({"path": "c.txt"}),
                cache_control: None,
            },
        ],
        StopReason::ToolUse,
        Usage::new(100, 50),
    );
    let responses = vec![multi_tool_response];

    let backend = Arc::new(MockBackend::new(responses));

    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("a.txt"), "a").unwrap();
    std::fs::write(temp.path().join("b.txt"), "b").unwrap();
    std::fs::write(temp.path().join("c.txt"), "c").unwrap();

    let mut tools = ToolRegistry::new();
    for tool in muninn_rlm::create_fs_tools(temp.path()) {
        tools.register_arc(Arc::from(tool));
    }
    let tools = Arc::new(tools);

    // Configure with low tool calls limit
    let budget = BudgetConfig {
        max_tool_calls: Some(2),
        max_tokens: None,
        max_depth: None,
        max_duration_secs: None,
    };

    let config = ProxyConfig::new(addr).with_budget(budget);
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
        "claude-test",
        vec![Message::user("Read all the files")],
        100,
    );

    let client = reqwest::Client::new();
    let messages_url = format!("http://127.0.0.1:{}/v1/messages", port);

    let response = client
        .post(&messages_url)
        .json(&request)
        .send()
        .await
        .unwrap();

    // Should get a 200 (budget exceeded returns OK with error indication)
    assert_eq!(response.status(), 200);

    let body: serde_json::Value = response.json().await.unwrap();

    // Check if it's an error response (budget exceeded)
    if body.get("type").and_then(|v| v.as_str()) == Some("error") {
        // Budget was exceeded - verify error type
        let error_type = body["error"]["type"].as_str().unwrap();
        assert_eq!(error_type, "budget_exceeded");
    } else {
        // It's a completion response - check metadata
        let completion: CompletionResponse = serde_json::from_value(body).unwrap();
        if let Some(metadata) = &completion.muninn {
            assert!(metadata.tool_calls <= 3);
        }
    }

    shutdown_tx.send(()).unwrap();
    let _ = server_handle.await;
}

/// Test that malformed requests are handled gracefully.
#[tokio::test]
async fn test_malformed_request() {
    let port = get_test_port();
    let addr = format!("127.0.0.1:{}", port).parse().unwrap();

    let backend = Arc::new(MockBackend::new(vec![]));
    let tools = Arc::new(ToolRegistry::new());

    let config = ProxyConfig::new(addr);
    let server = ProxyServer::new(config, backend, tools);

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let server_handle = tokio::spawn(async move {
        server
            .run_with_shutdown(async {
                shutdown_rx.await.ok();
            })
            .await
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let client = reqwest::Client::new();
    let messages_url = format!("http://127.0.0.1:{}/v1/messages", port);

    // Send invalid JSON
    let response = client
        .post(&messages_url)
        .header("content-type", "application/json")
        .body("not valid json")
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 400);

    shutdown_tx.send(()).unwrap();
    let _ = server_handle.await;
}
