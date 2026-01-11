//! MCP (Model Context Protocol) server integration.
//!
//! This module provides an MCP server that exposes tools via the Model Context Protocol,
//! allowing external LLM clients to discover and execute tools.
//!
//! Uses `rust-mcp-sdk` for protocol handling.

use std::sync::Arc;

use async_trait::async_trait;
use rust_mcp_sdk::{
    McpServer, StdioTransport, ToMcpServerHandler, TransportOptions,
    mcp_server::{McpServerOptions, ServerHandler, server_runtime},
    schema::{
        CallToolRequestParams, CallToolResult, Implementation, InitializeResult,
        LATEST_PROTOCOL_VERSION, ListToolsResult, PaginatedRequestParams, RpcError,
        ServerCapabilities, ServerCapabilitiesTools, TextContent, Tool as McpTool, ToolInputSchema,
    },
};
use tracing::info;

use crate::error::{Result, RlmError};
use crate::tools::ToolEnvironment;
use crate::types::ToolUseBlock;

// ============================================================================
// MCP Server Configuration
// ============================================================================

/// Configuration for the MCP server.
#[derive(Debug, Clone)]
pub struct McpServerConfig {
    /// Server name for identification.
    pub name: String,
    /// Server version.
    pub version: String,
    /// Optional instructions for the LLM.
    pub instructions: Option<String>,
}

impl Default for McpServerConfig {
    fn default() -> Self {
        Self {
            name: "muninn-rlm".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            instructions: None,
        }
    }
}

impl McpServerConfig {
    /// Create a new configuration with the given name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Default::default()
        }
    }

    /// Set the server version.
    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = version.into();
        self
    }

    /// Set instructions for the LLM.
    pub fn with_instructions(mut self, instructions: impl Into<String>) -> Self {
        self.instructions = Some(instructions.into());
        self
    }
}

// ============================================================================
// MCP Server Handler
// ============================================================================

/// MCP server handler that bridges `ToolEnvironment` to MCP protocol.
pub struct RlmServerHandler {
    tools: Arc<dyn ToolEnvironment>,
}

impl RlmServerHandler {
    /// Create a new handler with the given tool environment.
    pub fn new(tools: Arc<dyn ToolEnvironment>) -> Self {
        info!("Initializing RLM MCP Server Handler");
        Self { tools }
    }
}

#[async_trait]
impl ServerHandler for RlmServerHandler {
    async fn handle_list_tools_request(
        &self,
        _params: Option<PaginatedRequestParams>,
        _runtime: Arc<dyn McpServer>,
    ) -> std::result::Result<ListToolsResult, RpcError> {
        // Only expose external tools (not internal fs_tools that would collide with Claude Code)
        let tools: Vec<McpTool> = self
            .tools
            .available_tools_external()
            .into_iter()
            .map(|t| {
                // Convert our JSON schema to ToolInputSchema
                let input_schema: ToolInputSchema = serde_json::from_value(t.input_schema)
                    .unwrap_or_else(|_| {
                        // Fallback: create empty object schema
                        ToolInputSchema::new(vec![], None, None)
                    });

                McpTool {
                    name: t.name,
                    description: Some(t.description),
                    input_schema,
                    annotations: None,
                    execution: None,
                    icons: vec![],
                    meta: None,
                    output_schema: None,
                    title: None,
                }
            })
            .collect();

        Ok(ListToolsResult {
            tools,
            meta: None,
            next_cursor: None,
        })
    }

    async fn handle_call_tool_request(
        &self,
        params: CallToolRequestParams,
        _runtime: Arc<dyn McpServer>,
    ) -> std::result::Result<CallToolResult, rust_mcp_sdk::schema::schema_utils::CallToolError>
    {
        let args = serde_json::Value::Object(params.arguments.unwrap_or_default());

        // Create a ToolUseBlock for our tool environment
        let tool_use = ToolUseBlock {
            id: uuid::Uuid::new_v4().to_string(),
            name: params.name.clone(),
            input: args,
        };

        match self.tools.execute_tool(&tool_use).await {
            Ok(result) => {
                // Extract text content from the ToolResultContent
                let text_content = match result.content {
                    Some(crate::types::ToolResultContent::Text(s)) => s,
                    Some(crate::types::ToolResultContent::Blocks(blocks)) => {
                        // Serialize blocks back to JSON string as fallback
                        serde_json::to_string_pretty(&blocks).unwrap_or_default()
                    }
                    None => String::new(),
                };
                Ok(CallToolResult {
                    content: vec![TextContent::new(text_content, None, None).into()],
                    is_error: if result.is_error { Some(true) } else { None },
                    meta: None,
                    structured_content: None,
                })
            }
            Err(e) => Ok(CallToolResult {
                content: vec![TextContent::new(e.to_string(), None, None).into()],
                is_error: Some(true),
                meta: None,
                structured_content: None,
            }),
        }
    }
}

// ============================================================================
// MCP Server Runner
// ============================================================================

/// Run an MCP server on stdio transport with the given tool environment.
pub async fn run_mcp_server(
    tools: Arc<dyn ToolEnvironment>,
    config: McpServerConfig,
) -> Result<()> {
    info!("Starting MCP Server: {}", config.name);

    let server_details = InitializeResult {
        server_info: Implementation {
            name: config.name.clone(),
            version: config.version.clone(),
            title: Some(format!("{} MCP Server", config.name)),
            description: Some("RLM tool environment exposed via MCP".to_string()),
            icons: vec![],
            website_url: None,
        },
        capabilities: ServerCapabilities {
            tools: Some(ServerCapabilitiesTools { list_changed: None }),
            ..Default::default()
        },
        meta: None,
        instructions: config.instructions,
        protocol_version: LATEST_PROTOCOL_VERSION.to_string(),
    };

    let transport = StdioTransport::new(TransportOptions::default())
        .map_err(|e| RlmError::Protocol(format!("Failed to create transport: {}", e)))?;

    let handler = RlmServerHandler::new(tools).to_mcp_server_handler();

    let server = server_runtime::create_server(McpServerOptions {
        server_details,
        transport,
        handler,
        task_store: None,
        client_task_store: None,
    });

    info!("MCP Server starting on stdio transport");
    server
        .start()
        .await
        .map_err(|e| RlmError::Protocol(format!("MCP server failed: {}", e)))?;

    Ok(())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::MockToolEnvironment;
    use crate::types::ToolDefinition;
    use serde_json::json;

    fn mock_env() -> Arc<dyn ToolEnvironment> {
        Arc::new(MockToolEnvironment::new(vec![ToolDefinition::new(
            "test_tool",
            "A test tool",
            json!({
                "type": "object",
                "properties": {
                    "message": { "type": "string" }
                },
                "required": ["message"]
            }),
        )]))
    }

    #[test]
    fn test_config_default() {
        let config = McpServerConfig::default();
        assert_eq!(config.name, "muninn-rlm");
        assert!(config.instructions.is_none());
    }

    #[test]
    fn test_config_builder() {
        let config = McpServerConfig::new("test-server")
            .with_version("1.0.0")
            .with_instructions("Test instructions");

        assert_eq!(config.name, "test-server");
        assert_eq!(config.version, "1.0.0");
        assert_eq!(config.instructions, Some("Test instructions".to_string()));
    }

    #[tokio::test]
    async fn test_handler_creation() {
        let _handler = RlmServerHandler::new(mock_env());
        // Handler created successfully
    }
}
