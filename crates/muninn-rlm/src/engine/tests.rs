//! Tests for the recursive exploration engine.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::json;

use crate::backend::MockBackend;
use crate::tools::MockToolEnvironment;
use crate::types::{
    BudgetConfig, CompletionRequest, CompletionResponse, ContentBlock, Message, MuninnConfig,
    StopReason, ToolDefinition, Usage,
};

use super::{EngineConfig, EngineDeps, RecursiveEngine};

fn create_engine(
    responses: Vec<CompletionResponse>,
    tools: Vec<ToolDefinition>,
) -> (RecursiveEngine, Arc<MockToolEnvironment>) {
    let backend = Arc::new(MockBackend::new(responses));
    let tool_env = Arc::new(MockToolEnvironment::new(tools));
    let deps = EngineDeps::new(backend, tool_env.clone());
    let engine = RecursiveEngine::new(deps, EngineConfig::default());
    (engine, tool_env)
}

#[tokio::test]
async fn test_simple_completion() {
    let responses = vec![CompletionResponse::new(
        "msg_1",
        "model",
        vec![ContentBlock::Text {
            text: "Hello!".to_string(),
            cache_control: None,
        }],
        StopReason::EndTurn,
        Usage::new(10, 5),
    )];

    let (engine, _) = create_engine(responses, vec![]);

    let request = CompletionRequest::new("test-model", vec![Message::user("Hi")], 100);

    let response = engine.complete(request).await.unwrap();
    assert_eq!(response.text(), "Hello!");
}

#[tokio::test]
async fn test_tool_use_loop() {
    let responses = vec![
        CompletionResponse::new(
            "msg_1",
            "model",
            vec![
                ContentBlock::Text {
                    text: "Let me check.".to_string(),
                    cache_control: None,
                },
                ContentBlock::ToolUse {
                    id: "tool_1".to_string(),
                    name: "read_file".to_string(),
                    input: json!({"path": "/foo.rs"}),
                    cache_control: None,
                },
            ],
            StopReason::ToolUse,
            Usage::new(20, 15),
        ),
        CompletionResponse::new(
            "msg_2",
            "model",
            vec![ContentBlock::Text {
                text: "The file contains: test content".to_string(),
                cache_control: None,
            }],
            StopReason::EndTurn,
            Usage::new(50, 30),
        ),
    ];

    let tools = vec![ToolDefinition::new(
        "read_file",
        "Read a file",
        json!({"type": "object", "properties": {"path": {"type": "string"}}}),
    )];

    let (engine, tool_env) = create_engine(responses, tools);
    tool_env.set_response("read_file", "test content");

    let request = CompletionRequest::new("test-model", vec![Message::user("Read /foo.rs")], 100);

    let response = engine.complete(request).await.unwrap();
    assert_eq!(response.text(), "The file contains: test content");
    assert_eq!(tool_env.execution_count(), 1);
}

#[tokio::test]
async fn test_multiple_tool_calls() {
    let responses = vec![
        CompletionResponse::new(
            "msg_1",
            "model",
            vec![
                ContentBlock::ToolUse {
                    id: "t1".to_string(),
                    name: "tool_a".to_string(),
                    input: json!({}),
                    cache_control: None,
                },
                ContentBlock::ToolUse {
                    id: "t2".to_string(),
                    name: "tool_b".to_string(),
                    input: json!({}),
                    cache_control: None,
                },
            ],
            StopReason::ToolUse,
            Usage::new(10, 10),
        ),
        CompletionResponse::new(
            "msg_2",
            "model",
            vec![ContentBlock::Text {
                text: "Done".to_string(),
                cache_control: None,
            }],
            StopReason::EndTurn,
            Usage::new(30, 10),
        ),
    ];

    let tools = vec![
        ToolDefinition::new("tool_a", "A", json!({})),
        ToolDefinition::new("tool_b", "B", json!({})),
    ];

    let (engine, tool_env) = create_engine(responses, tools);

    let request = CompletionRequest::new("test-model", vec![Message::user("Use both tools")], 100);

    let response = engine.complete(request).await.unwrap();
    assert_eq!(response.text(), "Done");
    assert_eq!(tool_env.execution_count(), 2);
}

#[tokio::test]
async fn test_exploration_metadata() {
    let responses = vec![
        CompletionResponse::new(
            "msg_1",
            "model",
            vec![ContentBlock::ToolUse {
                id: "tool_1".to_string(),
                name: "tool".to_string(),
                input: json!({}),
                cache_control: None,
            }],
            StopReason::ToolUse,
            Usage::new(100, 50),
        ),
        CompletionResponse::new(
            "msg_2",
            "model",
            vec![ContentBlock::Text {
                text: "Done".to_string(),
                cache_control: None,
            }],
            StopReason::EndTurn,
            Usage::new(200, 100),
        ),
    ];

    let tools = vec![ToolDefinition::new("tool", "A tool", json!({}))];
    let (engine, _) = create_engine(responses, tools);

    let request = CompletionRequest::new("test-model", vec![Message::user("Hi")], 100)
        .with_muninn(MuninnConfig::recursive());

    let response = engine.complete(request).await.unwrap();
    let metadata = response.muninn.unwrap();

    assert_eq!(metadata.depth_reached, 1);
    assert_eq!(metadata.tool_calls, 1);
    assert_eq!(metadata.tokens_used, 450);
}

#[test]
fn test_is_recursive() {
    let request = CompletionRequest::new("model", vec![Message::user("Hi")], 100);
    assert!(!RecursiveEngine::is_recursive(&request));

    let non_recursive = CompletionRequest::new("model", vec![Message::user("Hi")], 100)
        .with_muninn(MuninnConfig {
            recursive: false,
            ..Default::default()
        });
    assert!(!RecursiveEngine::is_recursive(&non_recursive));

    let recursive = CompletionRequest::new("model", vec![Message::user("Hi")], 100)
        .with_muninn(MuninnConfig::recursive());
    assert!(RecursiveEngine::is_recursive(&recursive));
}

#[test]
fn test_engine_deps_creation() {
    let backend = Arc::new(MockBackend::new(vec![]));
    let tools = Arc::new(MockToolEnvironment::new(vec![]));

    let deps = EngineDeps::new(backend, tools);
    assert!(deps.file_system.is_none());
}

#[test]
fn test_engine_deps_with_file_system() {
    use crate::fs::MockFileSystem;

    let backend = Arc::new(MockBackend::new(vec![]));
    let tools = Arc::new(MockToolEnvironment::new(vec![]));
    let mock_fs = Arc::new(MockFileSystem::new());

    let deps = EngineDeps::new(backend, tools).with_file_system(mock_fs);

    assert!(deps.file_system.is_some());
}

#[test]
fn test_engine_config_default() {
    let config = EngineConfig::default();

    assert_eq!(config.budget.max_depth, Some(10));
    assert_eq!(config.budget.max_tokens, Some(100_000));
    assert!(config.work_dir.is_none());
    assert_eq!(config.temperature, Some(0.1));
    assert!(config.inject_system_prompt);
}

#[test]
fn test_engine_config_builder() {
    let budget = BudgetConfig {
        max_depth: Some(5),
        ..Default::default()
    };

    let config = EngineConfig::new()
        .with_budget(budget)
        .with_work_dir("/test/path")
        .with_temperature(0.5)
        .with_system_prompt_injection(false);

    assert_eq!(config.budget.max_depth, Some(5));
    assert_eq!(config.work_dir, Some(PathBuf::from("/test/path")));
    assert_eq!(config.temperature, Some(0.5));
    assert!(!config.inject_system_prompt);
}

#[tokio::test]
async fn test_engine_with_mocked_deps_simple() {
    let response = CompletionResponse::new(
        "msg_1",
        "test-model",
        vec![ContentBlock::Text {
            text: "Hello from mocked backend!".to_string(),
            cache_control: None,
        }],
        StopReason::EndTurn,
        Usage::new(10, 20),
    );

    let backend = Arc::new(MockBackend::new(vec![response]));
    let tools: Arc<dyn crate::tools::ToolEnvironment> =
        Arc::new(crate::tools::EmptyToolEnvironment);

    let deps = EngineDeps::new(backend, tools);
    let config = EngineConfig::default();
    let engine = RecursiveEngine::new(deps, config);

    let request = CompletionRequest::new("test-model", vec![Message::user("Hi")], 100);

    let result = engine.complete(request).await.unwrap();
    assert_eq!(result.text(), "Hello from mocked backend!");
}

#[tokio::test]
async fn test_engine_with_custom_budget() {
    let response = CompletionResponse::new(
        "msg_1",
        "test-model",
        vec![ContentBlock::Text {
            text: "Response".to_string(),
            cache_control: None,
        }],
        StopReason::EndTurn,
        Usage::new(5, 10),
    );

    let backend = Arc::new(MockBackend::new(vec![response]));
    let tools: Arc<dyn crate::tools::ToolEnvironment> =
        Arc::new(crate::tools::EmptyToolEnvironment);

    let deps = EngineDeps::new(backend, tools);
    let config = EngineConfig::default().with_budget(BudgetConfig {
        max_depth: Some(10),
        max_tokens: Some(1000),
        ..Default::default()
    });

    let engine = RecursiveEngine::new(deps, config);

    let request = CompletionRequest::new("test-model", vec![Message::user("Test")], 50);

    let result = engine.complete(request).await.unwrap();
    assert_eq!(result.text(), "Response");
}

#[test]
fn test_from_components_convenience() {
    let backend = Arc::new(MockBackend::new(vec![]));
    let tools = Arc::new(MockToolEnvironment::new(vec![]));

    let _engine = RecursiveEngine::from_components(backend, tools);
    // If it compiles and doesn't panic, it works
}
