# muninn-rlm Testing Guide

This document explains the testing infrastructure for the muninn-rlm crate and how to add new tests.

## Test Infrastructure

### MockBackend

The `MockBackend` provides deterministic LLM responses for testing:

```rust
use muninn_rlm::{MockBackend, CompletionResponse, ContentBlock, StopReason, Usage};

let response = CompletionResponse::new(
    "msg_123",
    "test-model",
    vec![ContentBlock::Text {
        text: "Test response".to_string(),
    }],
    StopReason::EndTurn,
    Usage::new(10, 5),
);

let backend = Arc::new(MockBackend::new(vec![response]));
```

**Features:**
- Returns pre-configured responses in sequence
- No network calls, fast execution
- Supports tool use responses for multi-turn tests

### Test Harness

Integration tests spawn a real proxy server on a random port:

```rust
// Get an available port
let port = get_test_port();
let addr = format!("127.0.0.1:{}", port).parse().unwrap();

// Create server
let config = ProxyConfig::new(addr);
let server = ProxyServer::new(config, backend, tools);

// Start with shutdown channel
let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
let server_handle = tokio::spawn(async move {
    server.run_with_shutdown(async {
        shutdown_rx.await.ok();
    }).await
});

// Give server time to start
tokio::time::sleep(Duration::from_millis(100)).await;

// ... make requests ...

// Shutdown
shutdown_tx.send(()).unwrap();
let _ = server_handle.await;
```

## Test Categories

### 1. Health & Basic Functionality

**test_proxy_health_check**: Verify proxy starts and responds to health endpoint
- Location: `tests/integration.rs:21`
- Coverage: Server startup, health endpoint

**test_proxy_completion_request**: Basic completion through proxy
- Location: `tests/integration.rs:71`
- Coverage: Request/response cycle, MockBackend integration

### 2. Routing Tests

**test_passthrough_mode**: AlwaysPassthrough strategy
- Location: `tests/integration.rs:237`
- Coverage: Passthrough to upstream (tests failure when upstream unavailable)

**test_text_trigger_forces_rlm**: Text pattern detection
- Location: `tests/integration.rs:296`
- Coverage: `{at}muninn explore` trigger bypasses router LLM (must be at start of line)

**test_router_llm_decision**: LLM-based routing
- Location: `tests/integration.rs:361`
- Coverage: Router LLM tool use, passthrough decision

### 3. RLM Engine Tests

**test_proxy_with_tools**: Tool execution in RLM
- Location: `tests/integration.rs:139`
- Coverage: FS tools, multi-turn execution, metadata

**test_graph_tools_integration**: Graph tool integration
- Location: `tests/integration.rs:437`
- Coverage: Graph store, symbol queries, tool execution

### 4. Budget Enforcement

**test_budget_depth_limit**: Depth budget enforcement
- Location: `tests/integration.rs:545`
- Coverage: max_depth limit, forced termination

**test_budget_tool_calls_limit**: Tool calls budget
- Location: `tests/integration.rs:636`
- Coverage: max_tool_calls limit

### 5. Error Handling

**test_malformed_request**: Invalid JSON handling
- Location: `tests/integration.rs:737`
- Coverage: 400 error on malformed input

## Adding New Tests

### Pattern: Basic Passthrough Test

```rust
#[tokio::test]
async fn test_my_feature() {
    let port = get_test_port();
    let addr = format!("127.0.0.1:{}", port).parse().unwrap();

    // Create mock response
    let response = CompletionResponse::new(
        "msg_test",
        "test-model",
        vec![ContentBlock::Text {
            text: "Expected output".to_string(),
        }],
        StopReason::EndTurn,
        Usage::new(10, 5),
    );
    let backend = Arc::new(MockBackend::new(vec![response]));
    let tools = Arc::new(ToolRegistry::new());

    // Start server
    let config = ProxyConfig::new(addr);
    let server = ProxyServer::new(config, backend, tools);

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let server_handle = tokio::spawn(async move {
        server.run_with_shutdown(async {
            shutdown_rx.await.ok();
        }).await
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Send request
    let request = CompletionRequest::new(
        "test-model",
        vec![Message::user("Test input")],
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

    // Assertions
    assert_eq!(response.status(), 200);
    let completion: CompletionResponse = response.json().await.unwrap();
    assert_eq!(completion.text(), "Expected output");

    // Cleanup
    shutdown_tx.send(()).unwrap();
    let _ = server_handle.await;
}
```

### Pattern: Multi-Turn RLM Test

```rust
#[tokio::test]
async fn test_multi_turn_exploration() {
    let port = get_test_port();
    let addr = format!("127.0.0.1:{}", port).parse().unwrap();

    // First response: request tool use
    let tool_response = CompletionResponse::new(
        "msg_tool",
        "test-model",
        vec![ContentBlock::ToolUse {
            id: "tool_1".to_string(),
            name: "read_file".to_string(),
            input: serde_json::json!({"path": "test.txt"}),
        }],
        StopReason::ToolUse,
        Usage::new(10, 5),
    );

    // Second response: final answer after tool execution
    let final_response = CompletionResponse::new(
        "msg_final",
        "test-model",
        vec![ContentBlock::Text {
            text: "Processed result".to_string(),
        }],
        StopReason::EndTurn,
        Usage::new(10, 5),
    );

    let backend = Arc::new(MockBackend::new(vec![tool_response, final_response]));

    // Create tools
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("test.txt"), "content").unwrap();

    let mut tools = ToolRegistry::new();
    for tool in muninn_rlm::create_fs_tools(temp.path()) {
        tools.register_arc(Arc::from(tool));
    }
    let tools = Arc::new(tools);

    // Use AlwaysRlm to force RLM processing
    let config = ProxyConfig::new(addr);
    let router_config = RouterConfig {
        strategy: RouterStrategy::AlwaysRlm,
        ..Default::default()
    };
    let server = ProxyServer::with_router(config, backend, tools, router_config);

    // ... rest of test setup and assertions ...
}
```

### Pattern: Budget Test

```rust
#[tokio::test]
async fn test_my_budget_limit() {
    // Configure specific budget
    let budget = BudgetConfig {
        max_depth: Some(3),
        max_tokens: Some(10000),
        max_tool_calls: Some(5),
        max_duration_secs: Some(30),
    };

    let config = ProxyConfig::new(addr).with_budget(budget);
    let router_config = RouterConfig {
        strategy: RouterStrategy::AlwaysRlm,
        ..Default::default()
    };
    let server = ProxyServer::with_router(config, backend, tools, router_config);

    // ... test that budget is enforced ...

    // Budget exceeded returns 200 with error JSON
    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();

    if body.get("type").and_then(|v| v.as_str()) == Some("error") {
        assert_eq!(body["error"]["type"].as_str().unwrap(), "budget_exceeded");
    }
}
```

## Running Tests

```bash
# Run all integration tests
cargo test -p muninn-rlm --test integration

# Run specific test
cargo test -p muninn-rlm --test integration test_proxy_health_check

# Run with output
cargo test -p muninn-rlm --test integration -- --nocapture

# Run in release mode (faster for slow tests)
cargo test -p muninn-rlm --test integration --release
```

## Test Organization

```
crates/muninn-rlm/
├── src/
│   ├── backend.rs          # MockBackend implementation
│   ├── proxy.rs            # ProxyServer
│   ├── router.rs           # Routing logic
│   └── engine.rs           # RLM engine
└── tests/
    └── integration.rs      # All integration tests
```

## Best Practices

1. **Use MockBackend for deterministic tests**: Avoid real API calls
2. **Test one thing per test**: Keep tests focused and clear
3. **Clean up resources**: Always shutdown servers in tests
4. **Use descriptive names**: Test names should explain what they verify
5. **Add comments for complex setups**: Explain multi-response sequences
6. **Check metadata**: Verify `completion.muninn` for RLM-specific data
7. **Test error cases**: Don't just test happy paths

## Common Patterns

### Testing Router Strategies

```rust
// Force specific strategy
let router_config = RouterConfig {
    strategy: RouterStrategy::AlwaysRlm,
    ..Default::default()
};
```

### Testing Tool Execution

```rust
// Create temporary directory with test files
let temp = tempfile::tempdir().unwrap();
std::fs::write(temp.path().join("test.txt"), "content").unwrap();

// Register tools scoped to temp directory
let mut tools = ToolRegistry::new();
for tool in muninn_rlm::create_fs_tools(temp.path()) {
    tools.register_arc(Arc::from(tool));
}
```

### Testing Graph Tools

```rust
// Create in-memory graph store
let store = GraphStore::open_in_memory().unwrap();
let symbol = Symbol {
    name: "my_function".to_string(),
    kind: SymbolKind::Function,
    // ... other fields ...
};
store.insert_node(&symbol).unwrap();

let shared_store = wrap_store(store);
for tool in create_graph_tools(shared_store) {
    tools.register_arc(Arc::from(tool));
}
```

## Debugging Tests

```rust
// Add logging to tests
env_logger::init();

// Check trace data
let completion: CompletionResponse = response.json().await.unwrap();
eprintln!("Metadata: {:?}", completion.muninn);

// Print full response
eprintln!("Response: {}", serde_json::to_string_pretty(&completion).unwrap());
```

## Future Work

- [ ] Add CI integration (GitHub Actions)
- [ ] Add timeout tests for backend failures
- [ ] Add streaming response validation tests
- [ ] Add trace data validation tests
- [ ] Performance benchmarks for RLM cycles
