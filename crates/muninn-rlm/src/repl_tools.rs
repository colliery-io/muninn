//! REPL sandbox for safe code execution during exploration.
//!
//! This module provides sandboxed code execution:
//! - Process isolation with resource limits
//! - Timeout enforcement
//! - Output capture (stdout/stderr)
//! - Support for multiple languages

use async_trait::async_trait;
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::time::timeout;

use crate::error::{Result, RlmError};
use crate::tools::{Tool, ToolMetadata, ToolResult};

// ============================================================================
// Configuration
// ============================================================================

/// Configuration for the sandbox environment.
#[derive(Debug, Clone)]
pub struct SandboxConfig {
    /// Maximum execution time in seconds.
    pub timeout_secs: u64,
    /// Maximum output size in bytes.
    pub max_output_bytes: usize,
    /// Working directory for execution.
    pub working_dir: Option<String>,
    /// Environment variables to set.
    pub env_vars: HashMap<String, String>,
    /// Whether to allow network access (not enforced in basic sandbox).
    pub allow_network: bool,
    /// Whether to allow filesystem writes (not enforced in basic sandbox).
    pub allow_writes: bool,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            timeout_secs: 30,
            max_output_bytes: 1024 * 1024, // 1MB
            working_dir: None,
            env_vars: HashMap::new(),
            allow_network: false,
            allow_writes: false,
        }
    }
}

impl SandboxConfig {
    /// Create a new sandbox config with default settings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the timeout.
    pub fn with_timeout(mut self, secs: u64) -> Self {
        self.timeout_secs = secs;
        self
    }

    /// Set the maximum output size.
    pub fn with_max_output(mut self, bytes: usize) -> Self {
        self.max_output_bytes = bytes;
        self
    }

    /// Set the working directory.
    pub fn with_working_dir(mut self, dir: impl Into<String>) -> Self {
        self.working_dir = Some(dir.into());
        self
    }

    /// Add an environment variable.
    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env_vars.insert(key.into(), value.into());
        self
    }
}

// ============================================================================
// Execution Result
// ============================================================================

/// Result of code execution.
#[derive(Debug, Clone)]
pub struct ExecutionResult {
    /// Exit code (0 = success).
    pub exit_code: i32,
    /// Standard output.
    pub stdout: String,
    /// Standard error.
    pub stderr: String,
    /// Whether execution timed out.
    pub timed_out: bool,
    /// Execution duration in milliseconds.
    pub duration_ms: u64,
    /// Whether output was truncated.
    pub truncated: bool,
}

impl ExecutionResult {
    /// Check if execution was successful.
    pub fn is_success(&self) -> bool {
        self.exit_code == 0 && !self.timed_out
    }

    /// Get combined output (stdout + stderr).
    pub fn combined_output(&self) -> String {
        if self.stderr.is_empty() {
            self.stdout.clone()
        } else if self.stdout.is_empty() {
            self.stderr.clone()
        } else {
            format!("{}\n--- stderr ---\n{}", self.stdout, self.stderr)
        }
    }
}

// ============================================================================
// Language Support
// ============================================================================

/// Supported languages for code execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Python,
    Shell,
}

impl Language {
    /// Parse language from string.
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "python" | "python3" | "py" => Some(Language::Python),
            "shell" | "bash" | "sh" => Some(Language::Shell),
            _ => None,
        }
    }

    /// Get the interpreter command for this language.
    pub fn interpreter(&self) -> &'static str {
        match self {
            Language::Python => "python3",
            Language::Shell => "bash",
        }
    }

    /// Get the flag for executing code from string.
    pub fn eval_flag(&self) -> &'static str {
        match self {
            Language::Python => "-c",
            Language::Shell => "-c",
        }
    }
}

// ============================================================================
// Sandbox Trait
// ============================================================================

/// Trait for sandbox implementations.
#[async_trait]
pub trait Sandbox: Send + Sync {
    /// Execute code in the sandbox.
    async fn execute(&self, language: Language, code: &str) -> Result<ExecutionResult>;

    /// Check if a language is available.
    async fn is_available(&self, language: Language) -> bool;
}

/// Thread-safe sandbox reference.
pub type SharedSandbox = Arc<dyn Sandbox>;

// ============================================================================
// Process Sandbox
// ============================================================================

/// Subprocess-based sandbox implementation.
///
/// This provides basic isolation through:
/// - Timeout enforcement
/// - Output size limits
/// - Separate process execution
///
/// Note: This does NOT provide security isolation like seccomp or sandbox-exec.
/// For production use, consider wrapping with platform-specific sandboxing.
pub struct ProcessSandbox {
    config: SandboxConfig,
}

impl ProcessSandbox {
    /// Create a new process sandbox with the given config.
    pub fn new(config: SandboxConfig) -> Self {
        Self { config }
    }

    /// Create with default config.
    pub fn default_sandbox() -> Self {
        Self::new(SandboxConfig::default())
    }

    /// Create a shared instance.
    pub fn shared(config: SandboxConfig) -> SharedSandbox {
        Arc::new(Self::new(config))
    }

    /// Truncate output if needed.
    fn truncate_output(&self, output: &[u8]) -> (String, bool) {
        let max = self.config.max_output_bytes;
        if output.len() > max {
            let truncated = String::from_utf8_lossy(&output[..max]).to_string();
            (truncated, true)
        } else {
            (String::from_utf8_lossy(output).to_string(), false)
        }
    }
}

#[async_trait]
impl Sandbox for ProcessSandbox {
    async fn execute(&self, language: Language, code: &str) -> Result<ExecutionResult> {
        let start = std::time::Instant::now();

        let mut cmd = Command::new(language.interpreter());
        cmd.arg(language.eval_flag());
        cmd.arg(code);

        // Set working directory if specified
        if let Some(ref dir) = self.config.working_dir {
            cmd.current_dir(dir);
        }

        // Set environment variables
        for (key, value) in &self.config.env_vars {
            cmd.env(key, value);
        }

        // Capture output
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.stdin(Stdio::null());

        // Spawn process
        let mut child = cmd
            .spawn()
            .map_err(|e| RlmError::ToolExecution(format!("Failed to spawn process: {}", e)))?;

        // Take stdout/stderr handles before waiting
        let mut stdout_handle = child.stdout.take();
        let mut stderr_handle = child.stderr.take();

        // Wait with timeout
        let timeout_duration = Duration::from_secs(self.config.timeout_secs);
        let wait_result = timeout(timeout_duration, child.wait()).await;

        let duration_ms = start.elapsed().as_millis() as u64;

        // Read output
        let mut stdout_buf = Vec::new();
        let mut stderr_buf = Vec::new();

        if let Some(ref mut stdout) = stdout_handle {
            let _ = stdout.read_to_end(&mut stdout_buf).await;
        }
        if let Some(ref mut stderr) = stderr_handle {
            let _ = stderr.read_to_end(&mut stderr_buf).await;
        }

        let (stdout, stdout_truncated) = self.truncate_output(&stdout_buf);
        let (stderr, stderr_truncated) = self.truncate_output(&stderr_buf);

        match wait_result {
            Ok(Ok(status)) => Ok(ExecutionResult {
                exit_code: status.code().unwrap_or(-1),
                stdout,
                stderr,
                timed_out: false,
                duration_ms,
                truncated: stdout_truncated || stderr_truncated,
            }),
            Ok(Err(e)) => Err(RlmError::ToolExecution(format!("Process error: {}", e))),
            Err(_) => {
                // Timeout - try to kill the process
                let _ = child.kill().await;

                Ok(ExecutionResult {
                    exit_code: -1,
                    stdout,
                    stderr,
                    timed_out: true,
                    duration_ms,
                    truncated: stdout_truncated || stderr_truncated,
                })
            }
        }
    }

    async fn is_available(&self, language: Language) -> bool {
        let result = Command::new(language.interpreter())
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;

        result.map(|s| s.success()).unwrap_or(false)
    }
}

// ============================================================================
// ExecuteCodeTool
// ============================================================================

/// Tool for executing code in a sandboxed environment.
pub struct ExecuteCodeTool {
    sandbox: SharedSandbox,
}

impl ExecuteCodeTool {
    /// Create a new execute code tool.
    pub fn new(sandbox: SharedSandbox) -> Self {
        Self { sandbox }
    }
}

#[async_trait]
impl Tool for ExecuteCodeTool {
    fn name(&self) -> &str {
        "execute_code"
    }

    fn description(&self) -> &str {
        "Execute code in a sandboxed environment. Supports Python and Shell (bash). \
         Use for testing hypotheses, running calculations, or validating code snippets. \
         Code runs with timeouts and output limits for safety."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "language": {
                    "type": "string",
                    "description": "Programming language: 'python' or 'shell'",
                    "enum": ["python", "shell"]
                },
                "code": {
                    "type": "string",
                    "description": "Code to execute"
                }
            },
            "required": ["language", "code"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<ToolResult> {
        let language_str = params
            .get("language")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                RlmError::ToolExecution("Missing required parameter 'language'".to_string())
            })?;

        let code = params.get("code").and_then(|v| v.as_str()).ok_or_else(|| {
            RlmError::ToolExecution("Missing required parameter 'code'".to_string())
        })?;

        let language = Language::parse(language_str).ok_or_else(|| {
            RlmError::ToolExecution(format!("Unsupported language: {}", language_str))
        })?;

        // Check if language is available
        if !self.sandbox.is_available(language).await {
            return Ok(ToolResult::error(
                format!("{} interpreter not available", language.interpreter()),
                true,
            ));
        }

        // Execute code
        let exec_result = self.sandbox.execute(language, code).await?;

        // Format output
        let output = serde_json::json!({
            "success": exec_result.is_success(),
            "exit_code": exec_result.exit_code,
            "stdout": exec_result.stdout,
            "stderr": exec_result.stderr,
            "timed_out": exec_result.timed_out,
            "duration_ms": exec_result.duration_ms,
            "truncated": exec_result.truncated
        });

        let mut result = ToolResult::json(output);
        result.metadata = ToolMetadata::with_source(language_str).with_tag("repl");

        if exec_result.timed_out {
            result.metadata.tags.push("timeout".to_string());
        }
        if !exec_result.is_success() {
            result.metadata.tags.push("error".to_string());
        }

        Ok(result)
    }
}

// ============================================================================
// CheckLanguageTool
// ============================================================================

/// Tool for checking which languages are available.
pub struct CheckLanguageTool {
    sandbox: SharedSandbox,
}

impl CheckLanguageTool {
    pub fn new(sandbox: SharedSandbox) -> Self {
        Self { sandbox }
    }
}

#[async_trait]
impl Tool for CheckLanguageTool {
    fn name(&self) -> &str {
        "check_language"
    }

    fn description(&self) -> &str {
        "Check which programming languages are available for code execution."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {}
        })
    }

    async fn execute(&self, _params: serde_json::Value) -> Result<ToolResult> {
        let languages = [Language::Python, Language::Shell];

        let mut available = Vec::new();
        for lang in languages {
            let is_available = self.sandbox.is_available(lang).await;
            available.push(serde_json::json!({
                "language": format!("{:?}", lang).to_lowercase(),
                "interpreter": lang.interpreter(),
                "available": is_available
            }));
        }

        let output = serde_json::json!({
            "languages": available
        });

        let mut result = ToolResult::json(output);
        result.metadata = ToolMetadata::with_source("check").with_tag("repl");

        Ok(result)
    }
}

// ============================================================================
// Factory Function
// ============================================================================

/// Create REPL tools with a given sandbox.
pub fn create_repl_tools(sandbox: SharedSandbox) -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(ExecuteCodeTool::new(Arc::clone(&sandbox))),
        Box::new(CheckLanguageTool::new(sandbox)),
    ]
}

/// Create REPL tools with default configuration.
pub fn create_default_repl_tools() -> Vec<Box<dyn Tool>> {
    let sandbox = ProcessSandbox::shared(SandboxConfig::default());
    create_repl_tools(sandbox)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sandbox_config_builder() {
        let config = SandboxConfig::new()
            .with_timeout(60)
            .with_max_output(2048)
            .with_working_dir("/tmp")
            .with_env("FOO", "bar");

        assert_eq!(config.timeout_secs, 60);
        assert_eq!(config.max_output_bytes, 2048);
        assert_eq!(config.working_dir, Some("/tmp".to_string()));
        assert_eq!(config.env_vars.get("FOO"), Some(&"bar".to_string()));
    }

    #[test]
    fn test_language_parsing() {
        assert_eq!(Language::parse("python"), Some(Language::Python));
        assert_eq!(Language::parse("Python"), Some(Language::Python));
        assert_eq!(Language::parse("py"), Some(Language::Python));
        assert_eq!(Language::parse("shell"), Some(Language::Shell));
        assert_eq!(Language::parse("bash"), Some(Language::Shell));
        assert_eq!(Language::parse("sh"), Some(Language::Shell));
        assert_eq!(Language::parse("unknown"), None);
    }

    #[test]
    fn test_language_interpreter() {
        assert_eq!(Language::Python.interpreter(), "python3");
        assert_eq!(Language::Shell.interpreter(), "bash");
    }

    #[test]
    fn test_execution_result_success() {
        let result = ExecutionResult {
            exit_code: 0,
            stdout: "Hello".to_string(),
            stderr: String::new(),
            timed_out: false,
            duration_ms: 100,
            truncated: false,
        };

        assert!(result.is_success());
        assert_eq!(result.combined_output(), "Hello");
    }

    #[test]
    fn test_execution_result_with_stderr() {
        let result = ExecutionResult {
            exit_code: 0,
            stdout: "Output".to_string(),
            stderr: "Warning".to_string(),
            timed_out: false,
            duration_ms: 100,
            truncated: false,
        };

        assert!(result.combined_output().contains("Output"));
        assert!(result.combined_output().contains("Warning"));
        assert!(result.combined_output().contains("stderr"));
    }

    #[test]
    fn test_execution_result_timeout() {
        let result = ExecutionResult {
            exit_code: -1,
            stdout: String::new(),
            stderr: String::new(),
            timed_out: true,
            duration_ms: 30000,
            truncated: false,
        };

        assert!(!result.is_success());
    }

    #[tokio::test]
    async fn test_process_sandbox_python() {
        let sandbox = ProcessSandbox::default_sandbox();

        // Skip if Python not available
        if !sandbox.is_available(Language::Python).await {
            return;
        }

        let result = sandbox
            .execute(Language::Python, "print('Hello, World!')")
            .await
            .unwrap();

        assert!(result.is_success());
        assert!(result.stdout.contains("Hello, World!"));
    }

    #[tokio::test]
    async fn test_process_sandbox_shell() {
        let sandbox = ProcessSandbox::default_sandbox();

        // Skip if bash not available
        if !sandbox.is_available(Language::Shell).await {
            return;
        }

        let result = sandbox
            .execute(Language::Shell, "echo 'test'")
            .await
            .unwrap();

        assert!(result.is_success());
        assert!(result.stdout.contains("test"));
    }

    #[tokio::test]
    async fn test_process_sandbox_exit_code() {
        let sandbox = ProcessSandbox::default_sandbox();

        if !sandbox.is_available(Language::Shell).await {
            return;
        }

        let result = sandbox.execute(Language::Shell, "exit 42").await.unwrap();

        assert!(!result.is_success());
        assert_eq!(result.exit_code, 42);
    }

    #[tokio::test]
    async fn test_process_sandbox_timeout() {
        let config = SandboxConfig::new().with_timeout(1);
        let sandbox = ProcessSandbox::new(config);

        if !sandbox.is_available(Language::Shell).await {
            return;
        }

        let result = sandbox.execute(Language::Shell, "sleep 10").await.unwrap();

        assert!(result.timed_out);
        assert!(!result.is_success());
    }

    #[tokio::test]
    async fn test_process_sandbox_stderr() {
        let sandbox = ProcessSandbox::default_sandbox();

        if !sandbox.is_available(Language::Python).await {
            return;
        }

        let result = sandbox
            .execute(Language::Python, "import sys; sys.stderr.write('error\\n')")
            .await
            .unwrap();

        assert!(result.stderr.contains("error"));
    }

    #[test]
    fn test_create_repl_tools() {
        let tools = create_default_repl_tools();
        assert_eq!(tools.len(), 2);

        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert!(names.contains(&"execute_code"));
        assert!(names.contains(&"check_language"));
    }

    #[tokio::test]
    async fn test_execute_code_tool() {
        let sandbox = ProcessSandbox::shared(SandboxConfig::default());

        // Skip if Python not available
        if !sandbox.is_available(Language::Python).await {
            return;
        }

        let tool = ExecuteCodeTool::new(sandbox);

        let result = tool
            .execute(serde_json::json!({
                "language": "python",
                "code": "print(2 + 2)"
            }))
            .await
            .unwrap();

        assert!(!result.is_error());
        let content = result.to_string_content();
        assert!(content.contains("4"));
    }

    #[tokio::test]
    async fn test_execute_code_tool_invalid_language() {
        let sandbox = ProcessSandbox::shared(SandboxConfig::default());
        let tool = ExecuteCodeTool::new(sandbox);

        let result = tool
            .execute(serde_json::json!({
                "language": "unknown",
                "code": "test"
            }))
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_check_language_tool() {
        let sandbox = ProcessSandbox::shared(SandboxConfig::default());
        let tool = CheckLanguageTool::new(sandbox);

        let result = tool.execute(serde_json::json!({})).await.unwrap();

        assert!(!result.is_error());
        let content = result.to_string_content();
        assert!(content.contains("python"));
        assert!(content.contains("shell"));
    }
}
