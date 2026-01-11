//! File system tools for code exploration.
//!
//! This module provides tools for reading files, listing directories,
//! and searching code content.
//!
//! All tools use the `FileSystem` trait abstraction, enabling testing
//! with mock filesystems.

use async_trait::async_trait;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::error::{Result, RlmError};
use crate::fs::{RealFileSystem, SharedFileSystem};
use crate::tools::{Tool, ToolMetadata, ToolResult};

// ============================================================================
// ReadFileTool
// ============================================================================

/// Tool for reading file contents.
///
/// Supports optional line ranges and respects file size limits.
pub struct ReadFileTool {
    /// Filesystem abstraction for file operations.
    fs: SharedFileSystem,
    /// Root directory to resolve relative paths from.
    root: PathBuf,
    /// Maximum file size to read (bytes).
    max_size: usize,
    /// Maximum lines to return.
    max_lines: usize,
}

impl ReadFileTool {
    /// Create a new read_file tool rooted at the given directory.
    ///
    /// Uses the real filesystem by default.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            fs: Arc::new(RealFileSystem::new()),
            root: root.into(),
            max_size: 1024 * 1024, // 1MB default
            max_lines: 10000,
        }
    }

    /// Create a new read_file tool with a custom filesystem.
    pub fn with_fs(root: impl Into<PathBuf>, fs: SharedFileSystem) -> Self {
        Self {
            fs,
            root: root.into(),
            max_size: 1024 * 1024,
            max_lines: 10000,
        }
    }

    /// Set maximum file size.
    pub fn with_max_size(mut self, bytes: usize) -> Self {
        self.max_size = bytes;
        self
    }

    /// Set maximum lines to return.
    pub fn with_max_lines(mut self, lines: usize) -> Self {
        self.max_lines = lines;
        self
    }

    /// Resolve and validate a path.
    ///
    /// Returns the resolved path. For non-existent files, validates the parent directory
    /// is within root and returns the non-canonical path.
    async fn resolve_path(&self, path: &str) -> Result<PathBuf> {
        let requested = Path::new(path);

        // Build full path
        let full_path = if requested.is_absolute() {
            requested.to_path_buf()
        } else {
            self.root.join(requested)
        };

        // Get canonical root for security check
        let root_canonical = self
            .fs
            .canonicalize(&self.root)
            .await
            .map_err(|e| RlmError::ToolExecution(format!("Cannot resolve root: {}", e)))?;

        // Try to canonicalize - if file exists
        if let Ok(canonical) = self.fs.canonicalize(&full_path).await {
            // Security: ensure path is within root
            if !canonical.starts_with(&root_canonical) {
                return Err(RlmError::ToolExecution(format!(
                    "Path '{}' is outside allowed directory",
                    path
                )));
            }
            return Ok(canonical);
        }

        // File doesn't exist - check parent directory for security
        // and return non-canonical path (caller will handle not-found)
        if let Some(parent) = full_path.parent() {
            if let Ok(parent_canonical) = self.fs.canonicalize(parent).await {
                if !parent_canonical.starts_with(&root_canonical) {
                    return Err(RlmError::ToolExecution(format!(
                        "Path '{}' is outside allowed directory",
                        path
                    )));
                }
            }
        }

        // Check for path traversal attempts in the path itself
        let path_str = path.to_string();
        if path_str.contains("..") {
            // Double-check by normalizing
            let normalized = full_path.components().collect::<PathBuf>();
            if !normalized.starts_with(&self.root) {
                return Err(RlmError::ToolExecution(format!(
                    "Path '{}' contains invalid traversal",
                    path
                )));
            }
        }

        Ok(full_path)
    }

    /// Detect language from file extension.
    fn detect_language(path: &Path) -> Option<String> {
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| match ext {
                "rs" => "rust",
                "py" => "python",
                "js" => "javascript",
                "ts" => "typescript",
                "tsx" => "typescript",
                "jsx" => "javascript",
                "go" => "go",
                "java" => "java",
                "c" | "h" => "c",
                "cpp" | "cc" | "hpp" => "cpp",
                "rb" => "ruby",
                "php" => "php",
                "swift" => "swift",
                "kt" => "kotlin",
                "scala" => "scala",
                "cs" => "csharp",
                "md" => "markdown",
                "json" => "json",
                "yaml" | "yml" => "yaml",
                "toml" => "toml",
                "xml" => "xml",
                "html" => "html",
                "css" => "css",
                "sql" => "sql",
                "sh" | "bash" => "bash",
                _ => ext,
            })
            .map(String::from)
    }
}

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }

    fn description(&self) -> &str {
        "Read the contents of a file. Optionally specify line range with start_line and end_line (1-indexed, inclusive)."
    }

    fn is_internal(&self) -> bool {
        true // Don't expose via MCP - Claude Code has its own read tool
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file (relative to repository root or absolute)"
                },
                "start_line": {
                    "type": "integer",
                    "description": "First line to read (1-indexed). Omit to start from beginning."
                },
                "end_line": {
                    "type": "integer",
                    "description": "Last line to read (inclusive). Omit to read to end."
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<ToolResult> {
        let path = params.get("path").and_then(|v| v.as_str()).ok_or_else(|| {
            RlmError::ToolExecution("Missing required parameter 'path'".to_string())
        })?;

        let start_line = params
            .get("start_line")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize);

        let end_line = params
            .get("end_line")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize);

        // Resolve and validate path
        let full_path = self.resolve_path(path).await?;

        // Check file exists and is a file
        if !self.fs.exists(&full_path).await {
            return Ok(ToolResult::error(format!("File not found: {}", path), true));
        }

        if !self.fs.is_file(&full_path).await {
            return Ok(ToolResult::error(format!("Not a file: {}", path), true));
        }

        // Check file size
        let metadata =
            self.fs.metadata(&full_path).await.map_err(|e| {
                RlmError::ToolExecution(format!("Cannot read file metadata: {}", e))
            })?;

        if metadata.len > self.max_size as u64 {
            return Ok(ToolResult::error(
                format!(
                    "File too large ({} bytes, max {} bytes)",
                    metadata.len, self.max_size
                ),
                true,
            ));
        }

        // Read file content
        let content = self
            .fs
            .read_file(&full_path)
            .await
            .map_err(|e| RlmError::ToolExecution(format!("Cannot read file: {}", e)))?;

        // Apply line range if specified
        let lines: Vec<&str> = content.lines().collect();
        let total_lines = lines.len();

        let start = start_line.map(|n| n.saturating_sub(1)).unwrap_or(0);
        let end = end_line.unwrap_or(total_lines).min(total_lines);

        if start >= total_lines {
            return Ok(ToolResult::error(
                format!(
                    "start_line {} exceeds file length ({} lines)",
                    start + 1,
                    total_lines
                ),
                true,
            ));
        }

        let selected_lines: Vec<&str> = lines[start..end].to_vec();
        let truncated = selected_lines.len() > self.max_lines;
        let final_lines: Vec<&str> = if truncated {
            selected_lines.into_iter().take(self.max_lines).collect()
        } else {
            selected_lines
        };

        // Add line numbers
        let numbered_content: String = final_lines
            .iter()
            .enumerate()
            .map(|(i, line)| format!("{:>6} | {}", start + i + 1, line))
            .collect::<Vec<_>>()
            .join("\n");

        let language = Self::detect_language(&full_path);
        let display_path = full_path
            .strip_prefix(&self.root)
            .unwrap_or(&full_path)
            .display()
            .to_string();

        let mut result = ToolResult::file(&display_path, numbered_content, language);

        // Add metadata
        let token_estimate: usize = final_lines
            .iter()
            .map(|l: &&str| l.len() / 4)
            .sum::<usize>()
            + final_lines.len();
        result.metadata = ToolMetadata::with_source(&display_path)
            .with_tokens(token_estimate)
            .with_tag("file");

        if truncated {
            result.metadata.tags.push("truncated".to_string());
        }

        Ok(result)
    }
}

// ============================================================================
// ListDirectoryTool
// ============================================================================

/// Tool for listing directory contents.
///
/// Supports glob patterns and respects .gitignore.
pub struct ListDirectoryTool {
    /// Filesystem abstraction for file operations.
    fs: SharedFileSystem,
    /// Root directory.
    root: PathBuf,
    /// Maximum entries to return.
    max_entries: usize,
}

impl ListDirectoryTool {
    /// Create a new list_directory tool.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            fs: Arc::new(RealFileSystem::new()),
            root: root.into(),
            max_entries: 1000,
        }
    }

    /// Create a new list_directory tool with a custom filesystem.
    pub fn with_fs(root: impl Into<PathBuf>, fs: SharedFileSystem) -> Self {
        Self {
            fs,
            root: root.into(),
            max_entries: 1000,
        }
    }

    /// Set maximum entries to return.
    pub fn with_max_entries(mut self, entries: usize) -> Self {
        self.max_entries = entries;
        self
    }

    /// Resolve path safely.
    async fn resolve_path(&self, path: &str) -> Result<PathBuf> {
        let requested = Path::new(path);

        let full_path = if requested.is_absolute() {
            requested.to_path_buf()
        } else {
            self.root.join(requested)
        };

        let canonical = self.fs.canonicalize(&full_path).await.map_err(|e| {
            RlmError::ToolExecution(format!("Cannot resolve path '{}': {}", path, e))
        })?;

        let root_canonical = self
            .fs
            .canonicalize(&self.root)
            .await
            .map_err(|e| RlmError::ToolExecution(format!("Cannot resolve root: {}", e)))?;

        if !canonical.starts_with(&root_canonical) {
            return Err(RlmError::ToolExecution(format!(
                "Path '{}' is outside allowed directory",
                path
            )));
        }

        Ok(canonical)
    }
}

#[async_trait]
impl Tool for ListDirectoryTool {
    fn name(&self) -> &str {
        "list_directory"
    }

    fn description(&self) -> &str {
        "List files and directories in a path. Use pattern for glob filtering (e.g., '*.rs', '**/*.py')."
    }

    fn is_internal(&self) -> bool {
        true // Don't expose via MCP - Claude Code has its own glob/list tools
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Directory path to list (relative or absolute)"
                },
                "pattern": {
                    "type": "string",
                    "description": "Glob pattern to filter results (e.g., '*.rs', '**/*.py')"
                },
                "recursive": {
                    "type": "boolean",
                    "description": "List recursively (default: false)"
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<ToolResult> {
        let path = params.get("path").and_then(|v| v.as_str()).unwrap_or(".");

        let pattern = params.get("pattern").and_then(|v| v.as_str());

        let recursive = params
            .get("recursive")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // Resolve path
        let full_path = self.resolve_path(path).await?;

        if !self.fs.is_dir(&full_path).await {
            return Ok(ToolResult::error(
                format!("Not a directory: {}", path),
                true,
            ));
        }

        // Collect entries
        let mut entries: Vec<String> = Vec::new();

        if recursive {
            self.list_recursive(&full_path, pattern, &mut entries)
                .await?;
        } else {
            self.list_single(&full_path, pattern, &mut entries).await?;
        }

        // Sort entries
        entries.sort();

        // Truncate if needed
        let truncated = entries.len() > self.max_entries;
        if truncated {
            entries.truncate(self.max_entries);
        }

        // Format output
        let display_path = full_path
            .strip_prefix(&self.root)
            .unwrap_or(&full_path)
            .display()
            .to_string();

        let mut output = format!("Contents of {}:\n\n", display_path);
        for entry in &entries {
            output.push_str(entry);
            output.push('\n');
        }

        if truncated {
            output.push_str(&format!(
                "\n... ({} more entries truncated)",
                self.max_entries
            ));
        }

        let mut result = ToolResult::text(output);
        result.metadata = ToolMetadata::with_source(&display_path).with_tag("directory");

        Ok(result)
    }
}

impl ListDirectoryTool {
    async fn list_single(
        &self,
        dir: &Path,
        pattern: Option<&str>,
        entries: &mut Vec<String>,
    ) -> Result<()> {
        let dir_entries = self
            .fs
            .list_dir(dir)
            .await
            .map_err(|e| RlmError::ToolExecution(format!("Cannot read directory: {}", e)))?;

        for entry in dir_entries {
            let name = &entry.name;

            // Skip hidden files
            if name.starts_with('.') {
                continue;
            }

            // Apply pattern filter
            if let Some(pat) = pattern {
                if !Self::matches_pattern(name, pat) {
                    continue;
                }
            }

            let suffix = if entry.is_dir { "/" } else { "" };
            entries.push(format!("{}{}", name, suffix));

            if entries.len() >= self.max_entries {
                break;
            }
        }

        Ok(())
    }

    async fn list_recursive(
        &self,
        dir: &Path,
        pattern: Option<&str>,
        entries: &mut Vec<String>,
    ) -> Result<()> {
        Box::pin(self.walk_dir(dir, dir, pattern, entries)).await
    }

    async fn walk_dir(
        &self,
        base: &Path,
        current: &Path,
        pattern: Option<&str>,
        entries: &mut Vec<String>,
    ) -> Result<()> {
        if entries.len() >= self.max_entries {
            return Ok(());
        }

        let dir_entries = match self.fs.list_dir(current).await {
            Ok(entries) => entries,
            Err(_) => return Ok(()), // Skip unreadable directories
        };

        for entry in dir_entries {
            let path = &entry.path;
            let name = &entry.name;

            // Skip hidden files and directories
            if name.starts_with('.') {
                continue;
            }

            // Skip common non-code directories
            if entry.is_dir
                && matches!(
                    name.as_str(),
                    "node_modules" | "target" | "build" | "dist" | "__pycache__" | ".git"
                )
            {
                continue;
            }

            let relative = path
                .strip_prefix(base)
                .unwrap_or(path)
                .display()
                .to_string();

            if entry.is_dir {
                // Recurse into directory
                Box::pin(self.walk_dir(base, path, pattern, entries)).await?;
            } else {
                // Apply pattern filter
                if let Some(pat) = pattern {
                    if !Self::matches_pattern(name, pat) && !Self::matches_pattern(&relative, pat) {
                        continue;
                    }
                }

                entries.push(relative);
            }

            if entries.len() >= self.max_entries {
                break;
            }
        }

        Ok(())
    }

    /// Simple glob pattern matching.
    fn matches_pattern(name: &str, pattern: &str) -> bool {
        // Handle ** for recursive matching
        if pattern.contains("**") {
            let parts: Vec<&str> = pattern.split("**").collect();
            if parts.len() == 2 {
                let suffix = parts[1].trim_start_matches('/');
                return Self::matches_simple(name, suffix);
            }
        }

        Self::matches_simple(name, pattern)
    }

    /// Simple wildcard matching (* only).
    fn matches_simple(name: &str, pattern: &str) -> bool {
        if pattern == "*" {
            return true;
        }

        if let Some(ext) = pattern.strip_prefix("*.") {
            return name.ends_with(&format!(".{}", ext));
        }

        if let Some(prefix) = pattern.strip_suffix("*") {
            return name.starts_with(prefix);
        }

        name == pattern
    }
}

// ============================================================================
// SearchFilesTool
// ============================================================================

/// Tool for searching file contents.
///
/// Uses ripgrep-style searching for fast content search.
pub struct SearchFilesTool {
    /// Filesystem abstraction for file operations.
    fs: SharedFileSystem,
    /// Root directory.
    root: PathBuf,
    /// Maximum results to return.
    max_results: usize,
    /// Context lines before/after match.
    context_lines: usize,
}

impl SearchFilesTool {
    /// Create a new search_files tool.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            fs: Arc::new(RealFileSystem::new()),
            root: root.into(),
            max_results: 50,
            context_lines: 2,
        }
    }

    /// Create a new search_files tool with a custom filesystem.
    pub fn with_fs(root: impl Into<PathBuf>, fs: SharedFileSystem) -> Self {
        Self {
            fs,
            root: root.into(),
            max_results: 50,
            context_lines: 2,
        }
    }

    /// Set maximum results.
    pub fn with_max_results(mut self, results: usize) -> Self {
        self.max_results = results;
        self
    }

    /// Set context lines.
    pub fn with_context_lines(mut self, lines: usize) -> Self {
        self.context_lines = lines;
        self
    }

    /// Search a single file for matches.
    async fn search_file(
        &self,
        path: &Path,
        pattern: &regex::Regex,
        results: &mut Vec<SearchMatch>,
    ) -> Result<()> {
        let content = match self.fs.read_file(path).await {
            Ok(c) => c,
            Err(_) => return Ok(()), // Skip unreadable files
        };

        let lines: Vec<&str> = content.lines().collect();
        let total_lines = lines.len();

        for (i, line) in lines.iter().enumerate() {
            if results.len() >= self.max_results {
                break;
            }

            if pattern.is_match(line) {
                let start = i.saturating_sub(self.context_lines);
                let end = (i + self.context_lines + 1).min(total_lines);

                let context: Vec<ContextLine> = lines[start..end]
                    .iter()
                    .enumerate()
                    .map(|(j, l)| ContextLine {
                        line_number: start + j + 1,
                        content: l.to_string(),
                        is_match: start + j == i,
                    })
                    .collect();

                let relative_path = path
                    .strip_prefix(&self.root)
                    .unwrap_or(path)
                    .display()
                    .to_string();

                results.push(SearchMatch {
                    path: relative_path,
                    line_number: i + 1,
                    context,
                });
            }
        }

        Ok(())
    }

    /// Recursively search directory.
    async fn search_dir(
        &self,
        dir: &Path,
        pattern: &regex::Regex,
        file_pattern: Option<&str>,
        results: &mut Vec<SearchMatch>,
    ) -> Result<()> {
        if results.len() >= self.max_results {
            return Ok(());
        }

        let dir_entries = match self.fs.list_dir(dir).await {
            Ok(entries) => entries,
            Err(_) => return Ok(()),
        };

        for entry in dir_entries {
            let path = &entry.path;
            let name = &entry.name;

            // Skip hidden and common non-code directories
            if name.starts_with('.') {
                continue;
            }

            if entry.is_dir {
                if matches!(
                    name.as_str(),
                    "node_modules" | "target" | "build" | "dist" | "__pycache__" | ".git"
                ) {
                    continue;
                }
                Box::pin(self.search_dir(path, pattern, file_pattern, results)).await?;
            } else {
                // Apply file pattern filter
                if let Some(fp) = file_pattern {
                    if !ListDirectoryTool::matches_pattern(name, fp) {
                        continue;
                    }
                }

                // Skip binary files (simple heuristic)
                if Self::is_likely_binary(name) {
                    continue;
                }

                self.search_file(path, pattern, results).await?;
            }

            if results.len() >= self.max_results {
                break;
            }
        }

        Ok(())
    }

    /// Check if file is likely binary based on extension.
    fn is_likely_binary(name: &str) -> bool {
        let binary_extensions = [
            "exe", "dll", "so", "dylib", "a", "o", "obj", "png", "jpg", "jpeg", "gif", "bmp",
            "ico", "webp", "zip", "tar", "gz", "bz2", "xz", "7z", "rar", "pdf", "doc", "docx",
            "xls", "xlsx", "ppt", "pptx", "wasm", "pyc", "pyo", "class",
        ];

        if let Some(ext) = name.rsplit('.').next() {
            binary_extensions.contains(&ext.to_lowercase().as_str())
        } else {
            false
        }
    }
}

#[derive(Debug)]
struct SearchMatch {
    path: String,
    line_number: usize,
    context: Vec<ContextLine>,
}

#[derive(Debug)]
struct ContextLine {
    line_number: usize,
    content: String,
    is_match: bool,
}

#[async_trait]
impl Tool for SearchFilesTool {
    fn name(&self) -> &str {
        "search_files"
    }

    fn description(&self) -> &str {
        "Search for content in files using regex patterns. Returns matching lines with context."
    }

    fn is_internal(&self) -> bool {
        true // Don't expose via MCP - Claude Code has its own grep tool
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search pattern (regex supported)"
                },
                "path": {
                    "type": "string",
                    "description": "Directory to search in (default: repository root)"
                },
                "file_pattern": {
                    "type": "string",
                    "description": "Filter files by pattern (e.g., '*.rs', '*.py')"
                },
                "case_sensitive": {
                    "type": "boolean",
                    "description": "Case-sensitive search (default: false)"
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<ToolResult> {
        let query = params
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                RlmError::ToolExecution("Missing required parameter 'query'".to_string())
            })?;

        let path = params.get("path").and_then(|v| v.as_str()).unwrap_or(".");

        let file_pattern = params.get("file_pattern").and_then(|v| v.as_str());

        let case_sensitive = params
            .get("case_sensitive")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // Build regex
        let pattern = if case_sensitive {
            regex::Regex::new(query)
        } else {
            regex::RegexBuilder::new(query)
                .case_insensitive(true)
                .build()
        };

        let pattern = pattern
            .map_err(|e| RlmError::ToolExecution(format!("Invalid regex pattern: {}", e)))?;

        // Resolve search path
        let search_path = if path == "." {
            self.root.clone()
        } else {
            let requested = Path::new(path);
            if requested.is_absolute() {
                requested.to_path_buf()
            } else {
                self.root.join(requested)
            }
        };

        if !self.fs.exists(&search_path).await {
            return Ok(ToolResult::error(format!("Path not found: {}", path), true));
        }

        // Search
        let mut results: Vec<SearchMatch> = Vec::new();

        if self.fs.is_file(&search_path).await {
            self.search_file(&search_path, &pattern, &mut results)
                .await?;
        } else {
            self.search_dir(&search_path, &pattern, file_pattern, &mut results)
                .await?;
        }

        // Format output
        if results.is_empty() {
            return Ok(ToolResult::text(format!("No matches found for: {}", query)));
        }

        let mut output = format!("Found {} matches for '{}':\n\n", results.len(), query);

        for m in &results {
            output.push_str(&format!("{}:{}\n", m.path, m.line_number));
            for ctx in &m.context {
                let marker = if ctx.is_match { ">" } else { " " };
                output.push_str(&format!(
                    "{} {:>4} | {}\n",
                    marker, ctx.line_number, ctx.content
                ));
            }
            output.push('\n');
        }

        let truncated = results.len() >= self.max_results;
        if truncated {
            output.push_str(&format!("(showing first {} results)\n", self.max_results));
        }

        let mut result = ToolResult::text(output);
        result.metadata = ToolMetadata::with_source(query).with_tag("search");

        if truncated {
            result.metadata.tags.push("truncated".to_string());
        }

        Ok(result)
    }
}

// ============================================================================
// FinalAnswerTool
// ============================================================================

/// Tool for signaling completion with a final answer.
///
/// When the RLM has gathered enough context and is ready to respond,
/// it calls this tool to signal termination. The engine intercepts
/// this call and returns the answer without further exploration.
pub struct FinalAnswerTool;

impl FinalAnswerTool {
    /// Create a new final_answer tool.
    pub fn new() -> Self {
        Self
    }
}

impl Default for FinalAnswerTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for FinalAnswerTool {
    fn name(&self) -> &str {
        "final_answer"
    }

    fn description(&self) -> &str {
        "Signal completion and provide the final answer to the user's query. Call this when you have gathered sufficient context and are ready to respond. The answer should be comprehensive and directly address the user's question."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "answer": {
                    "type": "string",
                    "description": "The complete answer to the user's query, incorporating all gathered context."
                }
            },
            "required": ["answer"]
        })
    }

    async fn execute(&self, input: serde_json::Value) -> Result<ToolResult> {
        // This tool's execution is intercepted by the engine before reaching here.
        // If we do get here, just return the answer as the result.
        let answer = input
            .get("answer")
            .and_then(|v| v.as_str())
            .unwrap_or("No answer provided");

        Ok(ToolResult::text(answer.to_string()))
    }
}

// ============================================================================
// Builder for filesystem tools
// ============================================================================

/// Create all file system tools for a given root directory.
///
/// Uses the real filesystem by default.
pub fn create_fs_tools(root: impl Into<PathBuf>) -> Vec<Box<dyn Tool>> {
    let root = root.into();
    vec![
        Box::new(ReadFileTool::new(root.clone())),
        Box::new(ListDirectoryTool::new(root.clone())),
        Box::new(SearchFilesTool::new(root)),
        Box::new(FinalAnswerTool::new()),
    ]
}

/// Create all file system tools with a custom filesystem.
///
/// Useful for testing with mock filesystems.
pub fn create_fs_tools_with_fs(
    root: impl Into<PathBuf>,
    fs: SharedFileSystem,
) -> Vec<Box<dyn Tool>> {
    let root = root.into();
    vec![
        Box::new(ReadFileTool::with_fs(root.clone(), fs.clone())),
        Box::new(ListDirectoryTool::with_fs(root.clone(), fs.clone())),
        Box::new(SearchFilesTool::with_fs(root, fs)),
        Box::new(FinalAnswerTool::new()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup_test_dir() -> TempDir {
        let dir = TempDir::new().unwrap();

        // Create test files
        fs::write(
            dir.path().join("hello.rs"),
            "fn main() {\n    println!(\"Hello\");\n}\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("lib.rs"),
            "pub mod utils;\npub fn greet() {}\n",
        )
        .unwrap();
        fs::create_dir(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/utils.rs"), "pub fn helper() {}\n").unwrap();
        fs::write(dir.path().join("README.md"), "# Test Project\n").unwrap();

        dir
    }

    #[tokio::test]
    async fn test_read_file_tool() {
        let dir = setup_test_dir();
        let tool = ReadFileTool::new(dir.path());

        let result = tool
            .execute(serde_json::json!({
                "path": "hello.rs"
            }))
            .await
            .unwrap();

        assert!(!result.is_error());
        let content = result.to_string_content();
        assert!(content.contains("fn main()"));
        assert!(content.contains("println!"));
    }

    #[tokio::test]
    async fn test_read_file_with_line_range() {
        let dir = setup_test_dir();
        let tool = ReadFileTool::new(dir.path());

        let result = tool
            .execute(serde_json::json!({
                "path": "hello.rs",
                "start_line": 2,
                "end_line": 2
            }))
            .await
            .unwrap();

        assert!(!result.is_error());
        let content = result.to_string_content();
        assert!(content.contains("println!"));
        assert!(!content.contains("fn main()"));
    }

    #[tokio::test]
    async fn test_read_file_not_found() {
        let dir = setup_test_dir();
        let tool = ReadFileTool::new(dir.path());

        let result = tool
            .execute(serde_json::json!({
                "path": "nonexistent.rs"
            }))
            .await
            .unwrap();

        assert!(result.is_error());
    }

    #[tokio::test]
    async fn test_read_file_path_traversal() {
        let dir = setup_test_dir();
        let tool = ReadFileTool::new(dir.path());

        let result = tool
            .execute(serde_json::json!({
                "path": "../../../etc/passwd"
            }))
            .await;

        // Should either error or return path-outside-directory error
        assert!(result.is_err() || result.unwrap().is_error());
    }

    #[tokio::test]
    async fn test_list_directory_tool() {
        let dir = setup_test_dir();
        let tool = ListDirectoryTool::new(dir.path());

        let result = tool
            .execute(serde_json::json!({
                "path": "."
            }))
            .await
            .unwrap();

        assert!(!result.is_error());
        let content = result.to_string_content();
        assert!(content.contains("hello.rs"));
        assert!(content.contains("src/"));
    }

    #[tokio::test]
    async fn test_list_directory_with_pattern() {
        let dir = setup_test_dir();
        let tool = ListDirectoryTool::new(dir.path());

        let result = tool
            .execute(serde_json::json!({
                "path": ".",
                "pattern": "*.rs"
            }))
            .await
            .unwrap();

        assert!(!result.is_error());
        let content = result.to_string_content();
        assert!(content.contains("hello.rs"));
        assert!(!content.contains("README.md"));
    }

    #[tokio::test]
    async fn test_list_directory_recursive() {
        let dir = setup_test_dir();
        let tool = ListDirectoryTool::new(dir.path());

        let result = tool
            .execute(serde_json::json!({
                "path": ".",
                "recursive": true,
                "pattern": "*.rs"
            }))
            .await
            .unwrap();

        assert!(!result.is_error());
        let content = result.to_string_content();
        assert!(content.contains("hello.rs"));
        assert!(content.contains("src/utils.rs") || content.contains("src\\utils.rs"));
    }

    #[tokio::test]
    async fn test_search_files_tool() {
        let dir = setup_test_dir();
        let tool = SearchFilesTool::new(dir.path());

        let result = tool
            .execute(serde_json::json!({
                "query": "println"
            }))
            .await
            .unwrap();

        assert!(!result.is_error());
        let content = result.to_string_content();
        assert!(content.contains("hello.rs"));
        assert!(content.contains("println"));
    }

    #[tokio::test]
    async fn test_search_files_with_file_pattern() {
        let dir = setup_test_dir();
        let tool = SearchFilesTool::new(dir.path());

        let result = tool
            .execute(serde_json::json!({
                "query": "pub",
                "file_pattern": "*.rs"
            }))
            .await
            .unwrap();

        assert!(!result.is_error());
        let content = result.to_string_content();
        assert!(content.contains("lib.rs") || content.contains("utils.rs"));
    }

    #[tokio::test]
    async fn test_search_files_no_matches() {
        let dir = setup_test_dir();
        let tool = SearchFilesTool::new(dir.path());

        let result = tool
            .execute(serde_json::json!({
                "query": "xyznonexistent123"
            }))
            .await
            .unwrap();

        assert!(!result.is_error());
        assert!(result.to_string_content().contains("No matches found"));
    }

    #[test]
    fn test_create_fs_tools() {
        let tools = create_fs_tools("/tmp");
        assert_eq!(tools.len(), 4);

        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert!(names.contains(&"read_file"));
        assert!(names.contains(&"list_directory"));
        assert!(names.contains(&"search_files"));
        assert!(names.contains(&"final_answer"));
    }

    #[test]
    fn test_language_detection() {
        assert_eq!(
            ReadFileTool::detect_language(Path::new("foo.rs")),
            Some("rust".to_string())
        );
        assert_eq!(
            ReadFileTool::detect_language(Path::new("bar.py")),
            Some("python".to_string())
        );
        assert_eq!(
            ReadFileTool::detect_language(Path::new("baz.ts")),
            Some("typescript".to_string())
        );
        assert_eq!(ReadFileTool::detect_language(Path::new("no_ext")), None);
    }

    #[test]
    fn test_pattern_matching() {
        assert!(ListDirectoryTool::matches_pattern("foo.rs", "*.rs"));
        assert!(!ListDirectoryTool::matches_pattern("foo.py", "*.rs"));
        assert!(ListDirectoryTool::matches_pattern("src/lib.rs", "**/*.rs"));
        assert!(ListDirectoryTool::matches_pattern("anything", "*"));
    }
}
