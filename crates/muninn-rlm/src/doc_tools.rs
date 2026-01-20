//! Documentation search tools for the RLM engine.
//!
//! This module provides tools for searching library documentation and
//! on-demand indexing of Rust crates and Python packages.
//!
//! # Tools
//!
//! - `search_docs`: Search documentation in indexed libraries
//! - `index_crate`: Index a Rust crate from crates.io
//! - `index_package`: Index a Python package from PyPI
//! - `list_libraries`: List all indexed libraries

use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use muninn_graph::doc_store::{DocStore, Ecosystem};
use muninn_graph::registry::{
    PyDocIndexer, PyIndexerConfig, PyIndexerError, RustDocIndexer, IndexerConfig, IndexerError,
};

use crate::error::{Result, RlmError};
use crate::tools::{Tool, ToolMetadata, ToolResult};

/// Thread-safe wrapper around DocStore.
pub type SharedDocStore = Arc<Mutex<DocStore>>;

/// Create a shared doc store from a DocStore.
pub fn wrap_doc_store(store: DocStore) -> SharedDocStore {
    Arc::new(Mutex::new(store))
}

/// Acquire the doc store lock with consistent error handling.
fn lock_store(store: &SharedDocStore) -> Result<std::sync::MutexGuard<'_, DocStore>> {
    store
        .lock()
        .map_err(|e| RlmError::ToolExecution(format!("Failed to acquire doc store lock: {}", e)))
}

// ============================================================================
// SearchDocsTool
// ============================================================================

/// Tool for searching documentation in indexed libraries.
pub struct SearchDocsTool {
    store: SharedDocStore,
    max_results: usize,
}

impl SearchDocsTool {
    /// Create a new search docs tool.
    pub fn new(store: SharedDocStore) -> Self {
        Self {
            store,
            max_results: 20,
        }
    }

    /// Set maximum results to return.
    pub fn with_max_results(mut self, max: usize) -> Self {
        self.max_results = max;
        self
    }
}

#[async_trait]
impl Tool for SearchDocsTool {
    fn name(&self) -> &str {
        "search_docs"
    }

    fn description(&self) -> &str {
        "Search documentation for a library. Returns matching documentation chunks with \
         function signatures and descriptions. Use this to find information about APIs, \
         usage examples, and function behavior in external libraries."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "library": {
                    "type": "string",
                    "description": "Name of the library to search (e.g., 'tokio', 'requests')"
                },
                "query": {
                    "type": "string",
                    "description": "Search query (e.g., 'spawn async task', 'HTTP request')"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum results to return (default: 20)"
                }
            },
            "required": ["library", "query"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<ToolResult> {
        let library = params
            .get("library")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                RlmError::ToolExecution("Missing required parameter 'library'".to_string())
            })?;

        let query = params.get("query").and_then(|v| v.as_str()).ok_or_else(|| {
            RlmError::ToolExecution("Missing required parameter 'query'".to_string())
        })?;

        let limit = params
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(self.max_results);

        let store = lock_store(&self.store)?;

        // Check if library exists
        let lib = store.get_library(library).map_err(|e| {
            RlmError::ToolExecution(format!("Failed to get library: {}", e))
        })?;

        if lib.is_none() {
            return Ok(ToolResult::text(format!(
                "Library '{}' is not indexed. Use index_crate (for Rust) or index_package (for Python) to index it first.",
                library
            )));
        }

        let lib_info = lib.unwrap();

        // Search documentation
        let results = store.search(library, query, limit).map_err(|e| {
            RlmError::ToolExecution(format!("Search failed: {}", e))
        })?;

        if results.is_empty() {
            return Ok(ToolResult::text(format!(
                "No results found for '{}' in library '{}' ({})",
                query, library, lib_info.version
            )));
        }

        // Format results
        let formatted: Vec<serde_json::Value> = results
            .iter()
            .map(|r| {
                let mut obj = serde_json::Map::new();
                obj.insert("path".to_string(), serde_json::json!(r.chunk.item_path));
                obj.insert("type".to_string(), serde_json::json!(r.chunk.item_type.as_str()));
                obj.insert("doc".to_string(), serde_json::json!(r.chunk.doc_text));
                if let Some(ref sig) = r.chunk.signature {
                    obj.insert("signature".to_string(), serde_json::json!(sig));
                }
                obj.insert("score".to_string(), serde_json::json!(r.score));
                serde_json::Value::Object(obj)
            })
            .collect();

        let output = serde_json::json!({
            "library": library,
            "version": lib_info.version,
            "ecosystem": lib_info.ecosystem.as_str(),
            "query": query,
            "results": formatted,
            "count": formatted.len()
        });

        let mut result = ToolResult::json(output);
        result.metadata = ToolMetadata::with_source(library)
            .with_tag("docs")
            .with_tag(lib_info.ecosystem.as_str());

        Ok(result)
    }
}

// ============================================================================
// IndexCrateTool
// ============================================================================

/// Tool for on-demand indexing of Rust crates from crates.io.
pub struct IndexCrateTool {
    store: SharedDocStore,
    work_dir: Option<PathBuf>,
}

impl IndexCrateTool {
    /// Create a new index crate tool.
    pub fn new(store: SharedDocStore) -> Self {
        Self {
            store,
            work_dir: None,
        }
    }

    /// Set a custom work directory for downloads.
    pub fn with_work_dir(mut self, dir: PathBuf) -> Self {
        self.work_dir = Some(dir);
        self
    }
}

#[async_trait]
impl Tool for IndexCrateTool {
    fn name(&self) -> &str {
        "index_crate"
    }

    fn description(&self) -> &str {
        "Index a Rust crate from crates.io. Downloads the crate, generates rustdoc JSON, \
         and stores documentation in the search index. Requires cargo and nightly Rust. \
         After indexing, use search_docs to search the crate's documentation."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "crate_name": {
                    "type": "string",
                    "description": "Name of the crate to index (e.g., 'tokio', 'serde')"
                },
                "version": {
                    "type": "string",
                    "description": "Specific version to index (optional, defaults to latest)"
                }
            },
            "required": ["crate_name"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<ToolResult> {
        let crate_name = params
            .get("crate_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                RlmError::ToolExecution("Missing required parameter 'crate_name'".to_string())
            })?;

        let version = params.get("version").and_then(|v| v.as_str());

        // Check if already indexed
        {
            let store = lock_store(&self.store)?;
            if let Ok(Some(lib)) = store.get_library(crate_name) {
                if version.is_none() || version == Some(lib.version.as_str()) {
                    return Ok(ToolResult::text(format!(
                        "Crate '{}' v{} is already indexed. Use search_docs to search it.",
                        crate_name, lib.version
                    )));
                }
            }
        }

        // Configure indexer
        let config = IndexerConfig {
            keep_source: false,
            work_dir: self.work_dir.clone(),
            rustdoc_flags: Vec::new(),
        };

        let indexer = RustDocIndexer::with_config(config);

        // Get a reference to the store for indexing
        // Note: We need to run the indexer synchronously since it uses blocking I/O
        let store_clone = Arc::clone(&self.store);

        // Run indexer in blocking task to avoid blocking the async runtime
        let crate_name_owned = crate_name.to_string();
        let version_owned = version.map(|v| v.to_string());

        let result = tokio::task::spawn_blocking(move || {
            let store = store_clone
                .lock()
                .map_err(|e| IndexerError::IndexingFailed(format!("Lock error: {}", e)))?;

            let version_ref = version_owned.as_deref();
            indexer.index_crate(&store, &crate_name_owned, version_ref)
        })
        .await
        .map_err(|e| RlmError::ToolExecution(format!("Task join error: {}", e)))?;

        match result {
            Ok(stats) => {
                let output = serde_json::json!({
                    "status": "success",
                    "crate": stats.crate_name,
                    "version": stats.version,
                    "items_extracted": stats.items_extracted,
                    "items_indexed": stats.items_indexed,
                    "message": format!(
                        "Successfully indexed {} documentation items from {} v{}. Use search_docs to search.",
                        stats.items_indexed, stats.crate_name, stats.version
                    )
                });

                let mut result = ToolResult::json(output);
                result.metadata = ToolMetadata::with_source(&stats.crate_name)
                    .with_tag("indexed")
                    .with_tag("rust");

                Ok(result)
            }
            Err(e) => {
                let error_msg = format!("Failed to index crate '{}': {}", crate_name, e);
                Ok(ToolResult::error(error_msg, true))
            }
        }
    }
}

// ============================================================================
// IndexPackageTool
// ============================================================================

/// Tool for on-demand indexing of Python packages from PyPI.
pub struct IndexPackageTool {
    store: SharedDocStore,
    work_dir: Option<PathBuf>,
    /// Deprecated: Python executable is no longer needed (tree-sitter is used for extraction)
    #[deprecated(note = "No longer needed - tree-sitter is used for extraction")]
    python: String,
}

impl IndexPackageTool {
    /// Create a new index package tool.
    #[allow(deprecated)]
    pub fn new(store: SharedDocStore) -> Self {
        Self {
            store,
            work_dir: None,
            python: "python3".to_string(),
        }
    }

    /// Set a custom work directory for downloads.
    pub fn with_work_dir(mut self, dir: PathBuf) -> Self {
        self.work_dir = Some(dir);
        self
    }

    /// Set the Python executable to use.
    ///
    /// Deprecated: No longer needed - tree-sitter is used for extraction.
    #[deprecated(note = "No longer needed - tree-sitter is used for extraction")]
    #[allow(deprecated)]
    pub fn with_python(mut self, python: impl Into<String>) -> Self {
        self.python = python.into();
        self
    }
}

#[async_trait]
impl Tool for IndexPackageTool {
    fn name(&self) -> &str {
        "index_package"
    }

    fn description(&self) -> &str {
        "Index a Python package from PyPI. Downloads the package, extracts documentation using griffe, \
         and stores it in the search index. Requires Python and griffe (pip install griffe). \
         After indexing, use search_docs to search the package's documentation."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "package_name": {
                    "type": "string",
                    "description": "Name of the package to index (e.g., 'requests', 'flask')"
                },
                "version": {
                    "type": "string",
                    "description": "Specific version to index (optional, defaults to latest)"
                }
            },
            "required": ["package_name"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<ToolResult> {
        let package_name = params
            .get("package_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                RlmError::ToolExecution("Missing required parameter 'package_name'".to_string())
            })?;

        let version = params.get("version").and_then(|v| v.as_str());

        // Check if already indexed
        {
            let store = lock_store(&self.store)?;
            if let Ok(Some(lib)) = store.get_library(package_name) {
                if version.is_none() || version == Some(lib.version.as_str()) {
                    return Ok(ToolResult::text(format!(
                        "Package '{}' v{} is already indexed. Use search_docs to search it.",
                        package_name, lib.version
                    )));
                }
            }
        }

        // Configure indexer
        let config = PyIndexerConfig {
            keep_source: false,
            work_dir: self.work_dir.clone(),
            ..Default::default()
        };

        let indexer = PyDocIndexer::with_config(config);

        // Run indexer in blocking task
        let store_clone = Arc::clone(&self.store);
        let package_name_owned = package_name.to_string();
        let version_owned = version.map(|v| v.to_string());

        let result = tokio::task::spawn_blocking(move || {
            let store = store_clone
                .lock()
                .map_err(|e| PyIndexerError::IndexingFailed(format!("Lock error: {}", e)))?;

            let version_ref = version_owned.as_deref();
            indexer.index_package(&store, &package_name_owned, version_ref)
        })
        .await
        .map_err(|e| RlmError::ToolExecution(format!("Task join error: {}", e)))?;

        match result {
            Ok(stats) => {
                let output = serde_json::json!({
                    "status": "success",
                    "package": stats.package_name,
                    "version": stats.version,
                    "items_extracted": stats.items_extracted,
                    "items_indexed": stats.items_indexed,
                    "message": format!(
                        "Successfully indexed {} documentation items from {} v{}. Use search_docs to search.",
                        stats.items_indexed, stats.package_name, stats.version
                    )
                });

                let mut result = ToolResult::json(output);
                result.metadata = ToolMetadata::with_source(&stats.package_name)
                    .with_tag("indexed")
                    .with_tag("python");

                Ok(result)
            }
            Err(e) => {
                let error_msg = format!("Failed to index package '{}': {}", package_name, e);
                Ok(ToolResult::error(error_msg, true))
            }
        }
    }
}

// ============================================================================
// ListLibrariesTool
// ============================================================================

/// Tool for listing all indexed libraries.
pub struct ListLibrariesTool {
    store: SharedDocStore,
}

impl ListLibrariesTool {
    /// Create a new list libraries tool.
    pub fn new(store: SharedDocStore) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for ListLibrariesTool {
    fn name(&self) -> &str {
        "list_libraries"
    }

    fn description(&self) -> &str {
        "List all indexed libraries available for documentation search. \
         Shows library names, versions, ecosystems (Rust/Python), and when they were indexed."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "ecosystem": {
                    "type": "string",
                    "enum": ["rust", "python"],
                    "description": "Filter by ecosystem (optional)"
                }
            }
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<ToolResult> {
        let ecosystem_filter = params
            .get("ecosystem")
            .and_then(|v| v.as_str())
            .and_then(Ecosystem::from_str);

        let store = lock_store(&self.store)?;

        let libraries = store.list_libraries().map_err(|e| {
            RlmError::ToolExecution(format!("Failed to list libraries: {}", e))
        })?;

        // Filter by ecosystem if specified
        let filtered: Vec<_> = libraries
            .into_iter()
            .filter(|lib| {
                ecosystem_filter
                    .map(|eco| lib.ecosystem == eco)
                    .unwrap_or(true)
            })
            .collect();

        if filtered.is_empty() {
            let msg = if let Some(eco) = ecosystem_filter {
                format!("No {} libraries are indexed.", eco.as_str())
            } else {
                "No libraries are indexed. Use index_crate or index_package to add libraries.".to_string()
            };
            return Ok(ToolResult::text(msg));
        }

        // Format results
        let formatted: Vec<serde_json::Value> = filtered
            .iter()
            .map(|lib| {
                serde_json::json!({
                    "name": lib.library,
                    "version": lib.version,
                    "ecosystem": lib.ecosystem.as_str(),
                    "indexed_at": lib.indexed_at,
                    "source_url": lib.source_url
                })
            })
            .collect();

        let output = serde_json::json!({
            "libraries": formatted,
            "count": formatted.len()
        });

        let mut result = ToolResult::json(output);
        result.metadata = ToolMetadata::default().with_tag("libraries");

        Ok(result)
    }
}

// ============================================================================
// Factory Function
// ============================================================================

/// Create all documentation tools for a given store.
pub fn create_doc_tools(store: SharedDocStore) -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(SearchDocsTool::new(store.clone())),
        Box::new(IndexCrateTool::new(store.clone())),
        Box::new(IndexPackageTool::new(store.clone())),
        Box::new(ListLibrariesTool::new(store)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    fn setup_test_store() -> SharedDocStore {
        let store = DocStore::open_in_memory().expect("Failed to create doc store");
        wrap_doc_store(store)
    }

    #[test]
    fn test_create_doc_tools() {
        let store = setup_test_store();
        let tools = create_doc_tools(store);
        assert_eq!(tools.len(), 4);

        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert!(names.contains(&"search_docs"));
        assert!(names.contains(&"index_crate"));
        assert!(names.contains(&"index_package"));
        assert!(names.contains(&"list_libraries"));
    }

    #[tokio::test]
    #[serial]
    async fn test_search_docs_library_not_found() {
        let store = setup_test_store();
        let tool = SearchDocsTool::new(store);

        let result = tool
            .execute(serde_json::json!({
                "library": "nonexistent",
                "query": "test"
            }))
            .await
            .unwrap();

        assert!(!result.is_error());
        assert!(result.to_string_content().contains("not indexed"));
    }

    #[tokio::test]
    #[serial]
    async fn test_search_docs_with_indexed_library() {
        let store = setup_test_store();

        // Insert a test library with chunks
        {
            let s = store.lock().unwrap();
            let lib_id = s
                .upsert_library("test-lib", Ecosystem::Rust, "1.0.0", None)
                .unwrap();

            use muninn_graph::doc_store::{DocChunkInput, ItemType};
            let chunk = DocChunkInput {
                item_path: "test_lib::foo".to_string(),
                item_type: ItemType::Function,
                doc_text: "A test function that does something useful.".to_string(),
                signature: Some("pub fn foo() -> i32".to_string()),
                embedding: None,
            };
            s.insert_chunk(lib_id, &chunk).unwrap();
        }

        let tool = SearchDocsTool::new(store);

        let result = tool
            .execute(serde_json::json!({
                "library": "test-lib",
                "query": "test function"
            }))
            .await
            .unwrap();

        assert!(!result.is_error());
        let content = result.to_string_content();
        assert!(content.contains("test_lib::foo") || content.contains("test function"));
    }

    #[tokio::test]
    #[serial]
    async fn test_list_libraries_empty() {
        let store = setup_test_store();
        let tool = ListLibrariesTool::new(store);

        let result = tool.execute(serde_json::json!({})).await.unwrap();

        assert!(!result.is_error());
        assert!(result.to_string_content().contains("No libraries"));
    }

    #[tokio::test]
    #[serial]
    async fn test_list_libraries_with_data() {
        let store = setup_test_store();

        // Insert test libraries
        {
            let s = store.lock().unwrap();
            s.upsert_library("tokio", Ecosystem::Rust, "1.35.0", None)
                .unwrap();
            s.upsert_library("requests", Ecosystem::Python, "2.31.0", None)
                .unwrap();
        }

        let tool = ListLibrariesTool::new(store);

        let result = tool.execute(serde_json::json!({})).await.unwrap();

        assert!(!result.is_error());
        let content = result.to_string_content();
        assert!(content.contains("tokio"));
        assert!(content.contains("requests"));
    }

    #[tokio::test]
    #[serial]
    async fn test_list_libraries_filter_by_ecosystem() {
        let store = setup_test_store();

        // Insert test libraries
        {
            let s = store.lock().unwrap();
            s.upsert_library("tokio", Ecosystem::Rust, "1.35.0", None)
                .unwrap();
            s.upsert_library("requests", Ecosystem::Python, "2.31.0", None)
                .unwrap();
        }

        let tool = ListLibrariesTool::new(store);

        // Filter for Python only
        let result = tool
            .execute(serde_json::json!({
                "ecosystem": "python"
            }))
            .await
            .unwrap();

        assert!(!result.is_error());
        let content = result.to_string_content();
        assert!(content.contains("requests"));
        assert!(!content.contains("tokio"));
    }

    #[tokio::test]
    #[serial]
    async fn test_index_crate_already_indexed() {
        let store = setup_test_store();

        // Pre-index a crate
        {
            let s = store.lock().unwrap();
            s.upsert_library("once_cell", Ecosystem::Rust, "1.19.0", None)
                .unwrap();
        }

        let tool = IndexCrateTool::new(store);

        let result = tool
            .execute(serde_json::json!({
                "crate_name": "once_cell"
            }))
            .await
            .unwrap();

        assert!(!result.is_error());
        assert!(result.to_string_content().contains("already indexed"));
    }

    #[tokio::test]
    #[serial]
    async fn test_index_package_already_indexed() {
        let store = setup_test_store();

        // Pre-index a package
        {
            let s = store.lock().unwrap();
            s.upsert_library("requests", Ecosystem::Python, "2.31.0", None)
                .unwrap();
        }

        let tool = IndexPackageTool::new(store);

        let result = tool
            .execute(serde_json::json!({
                "package_name": "requests"
            }))
            .await
            .unwrap();

        assert!(!result.is_error());
        assert!(result.to_string_content().contains("already indexed"));
    }

    #[test]
    fn test_tool_descriptions() {
        let store = setup_test_store();
        let tools = create_doc_tools(store);

        for tool in &tools {
            // Ensure descriptions are non-empty and helpful
            assert!(!tool.description().is_empty());
            assert!(tool.description().len() > 20); // Should have meaningful descriptions
        }
    }

    #[test]
    fn test_tool_schemas() {
        let store = setup_test_store();
        let tools = create_doc_tools(store);

        for tool in &tools {
            let schema = tool.parameters_schema();
            // Ensure schemas are valid JSON objects
            assert!(schema.is_object());
            assert!(schema.get("type").is_some());
        }
    }
}
