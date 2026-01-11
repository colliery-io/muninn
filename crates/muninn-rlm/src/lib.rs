//! muninn-rlm: RLM gateway and tool environment
//!
//! This crate provides the Recursive Language Model gateway for Muninn:
//! - Transparent proxy layer for LLM API calls (Anthropic-compatible)
//! - Recursive exploration engine with tool execution
//! - Budget management for tokens, time, and depth
//! - Backend abstraction for multiple LLM providers
//! - Sub-query spawning with context isolation

pub mod anthropic;
pub mod backend;
pub mod context;
pub mod engine;
pub mod error;
pub mod fs;
pub mod fs_tools;
pub mod graph_tools;
pub mod groq;
pub mod mcp;
pub mod memory_tools;
pub mod oauth;
pub mod ollama;
pub mod passthrough;
pub mod prompts;
pub mod proxy;
pub mod repl_tools;
pub mod router;
pub mod subquery;
pub mod token_manager;
pub mod tools;
pub mod types;

// Testing utilities - available in test builds
#[cfg(test)]
pub mod testing;

pub use anthropic::{AnthropicBackend, AnthropicConfig};
pub use backend::{
    LLMBackend, LoggingBackend, MockBackend, ParsedToolCall, ResponseStream, SharedBackend,
    StreamEvent, default_format_tool_definitions, default_format_tool_result,
};
pub use context::{ContextAggregator, ContextBuilder, ContextItem};
pub use engine::{EngineConfig, EngineDeps, ExplorationContext, RecursiveEngine};
pub use error::{BudgetExceededError, BudgetType, Result, RlmError};
pub use fs::{
    DirEntry, FileMetadata, FileSystem, MockFileSystem, RealFileSystem, SharedFileSystem,
};
pub use fs_tools::{
    FinalAnswerTool, ListDirectoryTool, ReadFileTool, SearchFilesTool, create_fs_tools,
    create_fs_tools_with_fs,
};
pub use graph_tools::{
    FindCallersTool, FindImplementationsTool, GetSymbolTool, GraphQueryTool, SharedGraphStore,
    create_graph_tools, wrap_store,
};
pub use groq::{GroqBackend, GroqConfig};
pub use mcp::{McpServerConfig, RlmServerHandler, run_mcp_server};
pub use memory_tools::{
    DeleteMemoryTool, InMemoryStore, ListMemoriesTool, MemoryEntry, MemoryStore, QueryMemoryTool,
    SearchMemoryTool, SharedMemoryStore, StoreMemoryTool, create_memory_tools,
};
pub use oauth::{
    OAuthConfig, OAuthTokens, PkceChallenge, build_authorization_url, exchange_code_for_tokens,
    generate_state, parse_code_state,
};
pub use ollama::{OllamaBackend, OllamaConfig};
pub use passthrough::{
    ANTHROPIC_API_URL, AnthropicPassthrough, ApiProvider, OPENAI_API_URL, Passthrough,
    PassthroughConfig,
};
pub use prompts::CORE_RLM_BEHAVIOR;
pub use proxy::{ProxyConfig, ProxyServer};
pub use repl_tools::{
    CheckLanguageTool, ExecuteCodeTool, ExecutionResult, Language, ProcessSandbox, Sandbox,
    SandboxConfig, SharedSandbox, create_default_repl_tools, create_repl_tools,
};
pub use router::{RouteDecision, Router, RouterConfig, RouterStrategy};
pub use subquery::{SubQuery, SubQueryExecutor, SubQueryResult, spawn_subquery_tool};
pub use token_manager::{
    FileTokenManager, InMemoryTokenManager, SharedTokenManager, TOKEN_FILE, TokenInfo,
    TokenManager, create_memory_token_manager, create_memory_token_manager_with_tokens,
    create_token_manager,
};
pub use tools::{
    CompositeToolEnvironment, EmptyToolEnvironment, MockToolEnvironment, SharedToolEnvironment,
    Tool, ToolContent, ToolEnvironment, ToolMetadata, ToolRegistry, ToolResult,
};
pub use types::{
    BudgetConfig, CompletionRequest, CompletionResponse, Content, ContentBlock,
    ExplorationMetadata, Message, MuninnConfig, Role, StopReason, ToolChoice, ToolDefinition,
    ToolResultBlock, ToolUseBlock, Usage,
};
