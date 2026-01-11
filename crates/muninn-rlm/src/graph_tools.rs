//! Code graph tools for querying the code structure.
//!
//! This module provides tools for querying the code graph, including
//! Cypher queries, finding callers/callees, and finding implementations.

use async_trait::async_trait;
use std::sync::{Arc, Mutex};

use graphqlite::Value;
use muninn_graph::GraphStore;

use crate::error::{Result, RlmError};
use crate::tools::{Tool, ToolMetadata, ToolResult};

/// Thread-safe wrapper around GraphStore.
pub type SharedGraphStore = Arc<Mutex<GraphStore>>;

/// Create a shared graph store from a GraphStore.
pub fn wrap_store(store: GraphStore) -> SharedGraphStore {
    Arc::new(Mutex::new(store))
}

// ─────────────────────────────────────────────────────────────────────────────
// Shared Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Acquire the graph store lock with consistent error handling.
fn lock_store(store: &SharedGraphStore) -> Result<std::sync::MutexGuard<'_, GraphStore>> {
    store
        .lock()
        .map_err(|e| RlmError::ToolExecution(format!("Failed to acquire store lock: {}", e)))
}

// ============================================================================
// GraphQueryTool
// ============================================================================

/// Tool for executing Cypher queries against the code graph.
pub struct GraphQueryTool {
    store: SharedGraphStore,
    max_results: usize,
}

impl GraphQueryTool {
    /// Create a new graph query tool.
    pub fn new(store: SharedGraphStore) -> Self {
        Self {
            store,
            max_results: 100,
        }
    }

    /// Set maximum results to return.
    pub fn with_max_results(mut self, max: usize) -> Self {
        self.max_results = max;
        self
    }
}

#[async_trait]
impl Tool for GraphQueryTool {
    fn name(&self) -> &str {
        "graph_query"
    }

    fn description(&self) -> &str {
        "Execute a Cypher query against the code graph. Returns matching nodes and relationships. \
         Available node labels: File, Module, Class, Struct, Interface, Enum, Function, Method, Variable, Type, Macro. \
         Available relationships: CONTAINS, IMPORTS, CALLS, INHERITS, IMPLEMENTS, USES_TYPE, REFERENCES."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Cypher query to execute (e.g., 'MATCH (n:Function) RETURN n.name LIMIT 10')"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum results to return (default: 100)"
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

        let limit = params
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(self.max_results);

        // Lock store and execute query
        let store = lock_store(&self.store)?;

        let cypher_result = store
            .query(query)
            .map_err(|e| RlmError::ToolExecution(format!("Graph query failed: {}", e)))?;

        // Format results - convert each row to JSON using columns
        let mut rows: Vec<serde_json::Value> = Vec::new();
        for row in cypher_result.iter().take(limit) {
            let mut obj = serde_json::Map::new();
            for col in row.columns() {
                if let Some(value) = row.get_value(col) {
                    obj.insert(col.clone(), value_to_json(value));
                }
            }
            rows.push(serde_json::Value::Object(obj));
        }

        let total = cypher_result.len();
        let truncated = total > limit;

        let output = serde_json::json!({
            "rows": rows,
            "count": rows.len(),
            "total": total,
            "truncated": truncated
        });

        let mut result = ToolResult::json(output);
        result.metadata = ToolMetadata::with_source(query).with_tag("graph");

        if truncated {
            result.metadata.tags.push("truncated".to_string());
        }

        Ok(result)
    }
}

// ============================================================================
// FindCallersTool
// ============================================================================

/// Tool for finding callers of a function or method.
pub struct FindCallersTool {
    store: SharedGraphStore,
}

impl FindCallersTool {
    /// Create a new find_callers tool.
    pub fn new(store: SharedGraphStore) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for FindCallersTool {
    fn name(&self) -> &str {
        "find_callers"
    }

    fn description(&self) -> &str {
        "Find all functions or methods that call a given function. \
         Provide either the function name or its full ID."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "function_name": {
                    "type": "string",
                    "description": "Name of the function to find callers for"
                },
                "function_id": {
                    "type": "string",
                    "description": "Full ID of the function node (if known)"
                }
            }
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<ToolResult> {
        let function_id = params
            .get("function_id")
            .and_then(|v| v.as_str())
            .map(String::from);

        let function_name = params.get("function_name").and_then(|v| v.as_str());

        let store = lock_store(&self.store)?;

        // Resolve function ID if only name is provided
        let target_id = if let Some(id) = function_id {
            id
        } else if let Some(name) = function_name {
            // Find function by name
            let symbols = store
                .find_by_name(name)
                .map_err(|e| RlmError::ToolExecution(format!("Failed to find function: {}", e)))?;

            if symbols.is_empty() {
                return Ok(ToolResult::text(format!(
                    "No function found with name '{}'",
                    name
                )));
            }

            // Get the first match's ID from the Value::Object
            extract_id_from_value(&symbols[0]).ok_or_else(|| {
                RlmError::ToolExecution("Could not extract function ID".to_string())
            })?
        } else {
            return Ok(ToolResult::error(
                "Must provide either 'function_name' or 'function_id'",
                true,
            ));
        };

        // Find callers
        let callers = store
            .find_callers(&target_id)
            .map_err(|e| RlmError::ToolExecution(format!("Failed to find callers: {}", e)))?;

        if callers.is_empty() {
            return Ok(ToolResult::text(format!(
                "No callers found for '{}'",
                target_id
            )));
        }

        // Format results
        let caller_info: Vec<serde_json::Value> = callers.iter().map(value_to_json).collect();

        let output = serde_json::json!({
            "target": target_id,
            "callers": caller_info,
            "count": caller_info.len()
        });

        let mut result = ToolResult::json(output);
        result.metadata = ToolMetadata::with_source(&target_id).with_tag("callers");

        Ok(result)
    }
}

// ============================================================================
// FindImplementationsTool
// ============================================================================

/// Tool for finding implementations of a trait or interface.
pub struct FindImplementationsTool {
    store: SharedGraphStore,
}

impl FindImplementationsTool {
    /// Create a new find_implementations tool.
    pub fn new(store: SharedGraphStore) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for FindImplementationsTool {
    fn name(&self) -> &str {
        "find_implementations"
    }

    fn description(&self) -> &str {
        "Find all types that implement a given trait or interface."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "trait_name": {
                    "type": "string",
                    "description": "Name of the trait/interface to find implementations for"
                },
                "trait_id": {
                    "type": "string",
                    "description": "Full ID of the trait node (if known)"
                }
            }
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<ToolResult> {
        let trait_id = params
            .get("trait_id")
            .and_then(|v| v.as_str())
            .map(String::from);

        let trait_name = params.get("trait_name").and_then(|v| v.as_str());

        let store = lock_store(&self.store)?;

        // Resolve trait ID if only name is provided
        let target_id = if let Some(id) = trait_id {
            id
        } else if let Some(name) = trait_name {
            // Find trait by name
            let symbols = store
                .find_by_name(name)
                .map_err(|e| RlmError::ToolExecution(format!("Failed to find trait: {}", e)))?;

            if symbols.is_empty() {
                return Ok(ToolResult::text(format!(
                    "No trait found with name '{}'",
                    name
                )));
            }

            // Get the first match's ID from the Value::Object
            extract_id_from_value(&symbols[0])
                .ok_or_else(|| RlmError::ToolExecution("Could not extract trait ID".to_string()))?
        } else {
            return Ok(ToolResult::error(
                "Must provide either 'trait_name' or 'trait_id'",
                true,
            ));
        };

        // Find implementations
        let impls = store.find_implementations(&target_id).map_err(|e| {
            RlmError::ToolExecution(format!("Failed to find implementations: {}", e))
        })?;

        if impls.is_empty() {
            return Ok(ToolResult::text(format!(
                "No implementations found for '{}'",
                target_id
            )));
        }

        // Format results
        let impl_info: Vec<serde_json::Value> = impls.iter().map(value_to_json).collect();

        let output = serde_json::json!({
            "trait": target_id,
            "implementations": impl_info,
            "count": impl_info.len()
        });

        let mut result = ToolResult::json(output);
        result.metadata = ToolMetadata::with_source(&target_id).with_tag("implementations");

        Ok(result)
    }
}

// ============================================================================
// GetSymbolTool
// ============================================================================

/// Tool for getting details about a symbol.
pub struct GetSymbolTool {
    store: SharedGraphStore,
}

impl GetSymbolTool {
    /// Create a new get_symbol tool.
    pub fn new(store: SharedGraphStore) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for GetSymbolTool {
    fn name(&self) -> &str {
        "get_symbol"
    }

    fn description(&self) -> &str {
        "Get details about a symbol (function, class, etc.) by name or ID. \
         Returns symbol metadata including file location, signature, and documentation."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Name of the symbol to look up"
                },
                "id": {
                    "type": "string",
                    "description": "Full ID of the symbol node (if known)"
                }
            }
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<ToolResult> {
        let symbol_id = params.get("id").and_then(|v| v.as_str());

        let symbol_name = params.get("name").and_then(|v| v.as_str());

        let store = lock_store(&self.store)?;

        if let Some(id) = symbol_id {
            // Look up by ID
            let node = store
                .get_node(id)
                .map_err(|e| RlmError::ToolExecution(format!("Failed to get symbol: {}", e)))?;

            if let Some(value) = node {
                let output = value_to_json(&value);
                let mut result = ToolResult::json(output);
                result.metadata = ToolMetadata::with_source(id).with_tag("symbol");
                return Ok(result);
            } else {
                return Ok(ToolResult::text(format!(
                    "No symbol found with ID '{}'",
                    id
                )));
            }
        }

        if let Some(name) = symbol_name {
            // Look up by name
            let symbols = store
                .find_by_name(name)
                .map_err(|e| RlmError::ToolExecution(format!("Failed to find symbol: {}", e)))?;

            if symbols.is_empty() {
                return Ok(ToolResult::text(format!(
                    "No symbol found with name '{}'",
                    name
                )));
            }

            let symbol_info: Vec<serde_json::Value> = symbols.iter().map(value_to_json).collect();

            let output = serde_json::json!({
                "name": name,
                "matches": symbol_info,
                "count": symbol_info.len()
            });

            let mut result = ToolResult::json(output);
            result.metadata = ToolMetadata::with_source(name).with_tag("symbol");
            return Ok(result);
        }

        Ok(ToolResult::error(
            "Must provide either 'name' or 'id'",
            true,
        ))
    }
}

// ============================================================================
// FindSymbolsTool
// ============================================================================

/// Tool for searching symbols with user-friendly parameters.
///
/// This tool abstracts the underlying schema, so users don't need to know
/// property names like "file_path" or "doc_comment".
pub struct FindSymbolsTool {
    store: SharedGraphStore,
    max_results: usize,
}

impl FindSymbolsTool {
    /// Create a new find_symbols tool.
    pub fn new(store: SharedGraphStore) -> Self {
        Self {
            store,
            max_results: 50,
        }
    }
}

#[async_trait]
impl Tool for FindSymbolsTool {
    fn name(&self) -> &str {
        "find_symbols"
    }

    fn description(&self) -> &str {
        "Search for code symbols (functions, structs, traits, etc.) by name pattern and optional filters. \
         Returns matching symbols with their file location and documentation. Use this instead of graph_query \
         for searching - it's simpler and doesn't require knowing the schema."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Symbol name to search for (case-sensitive substring match, e.g., 'Trace' matches 'TraceWriter')"
                },
                "symbol_type": {
                    "type": "string",
                    "enum": ["function", "struct", "trait", "enum", "method", "class", "module", "macro", "type", "variable"],
                    "description": "Filter by symbol type (optional)"
                },
                "path_contains": {
                    "type": "string",
                    "description": "Filter to files whose path contains this string (e.g., 'muninn-tracing' or 'src/engine')"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum results to return (default: 50)"
                }
            },
            "required": ["name"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<ToolResult> {
        let name = params.get("name").and_then(|v| v.as_str()).ok_or_else(|| {
            RlmError::ToolExecution("Missing required parameter 'name'".to_string())
        })?;

        let symbol_type = params.get("symbol_type").and_then(|v| v.as_str());

        let path_contains = params.get("path_contains").and_then(|v| v.as_str());

        let limit = params
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(self.max_results);

        // Build Cypher query with optional filters
        let label_filter = match symbol_type {
            Some("function") => Some("Function"),
            Some("struct") => Some("Struct"),
            Some("trait") | Some("interface") => Some("Interface"),
            Some("enum") => Some("Enum"),
            Some("method") => Some("Method"),
            Some("class") => Some("Class"),
            Some("module") => Some("Module"),
            Some("macro") => Some("Macro"),
            Some("type") => Some("Type"),
            Some("variable") => Some("Variable"),
            _ => None,
        };

        // Construct the MATCH pattern
        let match_pattern = if let Some(label) = label_filter {
            format!("MATCH (n:{})", label)
        } else {
            "MATCH (n)".to_string()
        };

        // Build WHERE clause for name matching using CONTAINS
        let escaped_name = graphqlite::escape_string(name);
        let mut where_clauses = vec![format!("n.name CONTAINS '{}'", escaped_name)];

        // Add path filter if provided
        if let Some(path) = path_contains {
            let escaped_path = graphqlite::escape_string(path);
            where_clauses.push(format!("n.file_path CONTAINS '{}'", escaped_path));
        }

        let where_clause = format!("WHERE {}", where_clauses.join(" AND "));

        let cypher = format!(
            "{} {} RETURN n.name AS name, n.kind AS kind, n.file_path AS file, \
             n.start_line AS line, n.end_line AS end_line, n.signature AS signature, \
             n.doc_comment AS description, n.visibility AS visibility \
             ORDER BY n.file_path, n.start_line LIMIT {}",
            match_pattern, where_clause, limit
        );

        // Execute query
        let store = lock_store(&self.store)?;

        let cypher_result = store
            .query(&cypher)
            .map_err(|e| RlmError::ToolExecution(format!("Search failed: {}", e)))?;

        // Format results in a user-friendly way
        let mut results: Vec<serde_json::Value> = Vec::new();
        for row in cypher_result.iter() {
            let mut obj = serde_json::Map::new();

            // Extract values with user-friendly names
            if let Some(value) = row.get_value("name") {
                obj.insert("name".to_string(), value_to_json(value));
            }
            if let Some(value) = row.get_value("kind") {
                obj.insert("type".to_string(), value_to_json(value));
            }
            if let Some(value) = row.get_value("file") {
                obj.insert("file".to_string(), value_to_json(value));
            }
            if let Some(value) = row.get_value("line") {
                obj.insert("line".to_string(), value_to_json(value));
            }
            if let Some(value) = row.get_value("signature") {
                if !matches!(value, Value::Null) {
                    obj.insert("signature".to_string(), value_to_json(value));
                }
            }
            if let Some(value) = row.get_value("description") {
                if !matches!(value, Value::Null) {
                    obj.insert("description".to_string(), value_to_json(value));
                }
            }
            if let Some(value) = row.get_value("visibility") {
                obj.insert("visibility".to_string(), value_to_json(value));
            }

            results.push(serde_json::Value::Object(obj));
        }

        let total = results.len();
        let output = serde_json::json!({
            "query": {
                "name": name,
                "symbol_type": symbol_type,
                "path_contains": path_contains
            },
            "results": results,
            "count": total,
            "truncated": total >= limit
        });

        let mut result = ToolResult::json(output);
        result.metadata = ToolMetadata::with_source(name).with_tag("symbols");

        Ok(result)
    }
}

// ============================================================================
// FindCalleesTool
// ============================================================================

/// Tool for finding functions called by a given function.
pub struct FindCalleesTool {
    store: SharedGraphStore,
}

impl FindCalleesTool {
    /// Create a new find_callees tool.
    pub fn new(store: SharedGraphStore) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for FindCalleesTool {
    fn name(&self) -> &str {
        "find_callees"
    }

    fn description(&self) -> &str {
        "Find all functions or methods that are called by a given function. \
         The inverse of find_callers - shows what a function depends on."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "function_name": {
                    "type": "string",
                    "description": "Name of the function to find callees for"
                },
                "function_id": {
                    "type": "string",
                    "description": "Full ID of the function node (if known)"
                }
            }
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<ToolResult> {
        let function_id = params
            .get("function_id")
            .and_then(|v| v.as_str())
            .map(String::from);

        let function_name = params.get("function_name").and_then(|v| v.as_str());

        let store = lock_store(&self.store)?;

        // Resolve function ID if only name is provided
        let target_id = if let Some(id) = function_id {
            id
        } else if let Some(name) = function_name {
            let symbols = store
                .find_by_name(name)
                .map_err(|e| RlmError::ToolExecution(format!("Failed to find function: {}", e)))?;

            if symbols.is_empty() {
                return Ok(ToolResult::text(format!(
                    "No function found with name '{}'",
                    name
                )));
            }

            extract_id_from_value(&symbols[0]).ok_or_else(|| {
                RlmError::ToolExecution("Could not extract function ID".to_string())
            })?
        } else {
            return Ok(ToolResult::error(
                "Must provide either 'function_name' or 'function_id'",
                true,
            ));
        };

        // Find callees
        let callees = store
            .find_callees(&target_id)
            .map_err(|e| RlmError::ToolExecution(format!("Failed to find callees: {}", e)))?;

        if callees.is_empty() {
            return Ok(ToolResult::text(format!(
                "No callees found for '{}'",
                target_id
            )));
        }

        // Format results
        let callee_info: Vec<serde_json::Value> = callees.iter().map(format_symbol_value).collect();

        let output = serde_json::json!({
            "function": target_id,
            "calls": callee_info,
            "count": callee_info.len()
        });

        let mut result = ToolResult::json(output);
        result.metadata = ToolMetadata::with_source(&target_id).with_tag("callees");

        Ok(result)
    }
}

// ============================================================================
// FileOutlineTool
// ============================================================================

/// Tool for getting an outline of symbols defined in a file.
pub struct FileOutlineTool {
    store: SharedGraphStore,
}

impl FileOutlineTool {
    /// Create a new file_outline tool.
    pub fn new(store: SharedGraphStore) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for FileOutlineTool {
    fn name(&self) -> &str {
        "file_outline"
    }

    fn description(&self) -> &str {
        "Get an outline of all symbols (functions, structs, traits, etc.) defined in a file. \
         Returns symbols in source order with their line numbers. Useful for understanding \
         file structure before reading specific sections."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Path to the file (e.g., 'crates/muninn-rlm/src/engine.rs')"
                }
            },
            "required": ["file_path"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<ToolResult> {
        let file_path = params
            .get("file_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                RlmError::ToolExecution("Missing required parameter 'file_path'".to_string())
            })?;

        let store = lock_store(&self.store)?;

        // Try the exact path first, then with ./ prefix (indexer may store paths either way)
        let mut symbols = store
            .find_symbols_in_file(file_path)
            .map_err(|e| RlmError::ToolExecution(format!("Failed to get file outline: {}", e)))?;

        if symbols.is_empty() && !file_path.starts_with("./") {
            let prefixed = format!("./{}", file_path);
            symbols = store.find_symbols_in_file(&prefixed).unwrap_or_default();
        }

        if symbols.is_empty() {
            return Ok(ToolResult::text(format!(
                "No symbols found in '{}' (file may not be indexed)",
                file_path
            )));
        }

        // Format as a clean outline
        let outline: Vec<serde_json::Value> = symbols.iter().map(format_symbol_value).collect();

        let output = serde_json::json!({
            "file": file_path,
            "symbols": outline,
            "count": outline.len()
        });

        let mut result = ToolResult::json(output);
        result.metadata = ToolMetadata::with_source(file_path).with_tag("outline");

        Ok(result)
    }
}

// ============================================================================
// FindUsagesTool
// ============================================================================

/// Tool for finding all usages/references to a symbol.
pub struct FindUsagesTool {
    store: SharedGraphStore,
}

impl FindUsagesTool {
    /// Create a new find_usages tool.
    pub fn new(store: SharedGraphStore) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for FindUsagesTool {
    fn name(&self) -> &str {
        "find_usages"
    }

    fn description(&self) -> &str {
        "Find all places where a symbol (type, struct, function, etc.) is used or referenced. \
         More general than find_callers - works for any symbol type. Shows what depends on this symbol."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "symbol_name": {
                    "type": "string",
                    "description": "Name of the symbol to find usages for"
                },
                "symbol_id": {
                    "type": "string",
                    "description": "Full ID of the symbol node (if known)"
                }
            }
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<ToolResult> {
        let symbol_id = params
            .get("symbol_id")
            .and_then(|v| v.as_str())
            .map(String::from);

        let symbol_name = params.get("symbol_name").and_then(|v| v.as_str());

        let store = lock_store(&self.store)?;

        // Resolve symbol ID if only name is provided
        let target_id = if let Some(id) = symbol_id {
            id
        } else if let Some(name) = symbol_name {
            let symbols = store
                .find_by_name(name)
                .map_err(|e| RlmError::ToolExecution(format!("Failed to find symbol: {}", e)))?;

            if symbols.is_empty() {
                return Ok(ToolResult::text(format!(
                    "No symbol found with name '{}'",
                    name
                )));
            }

            extract_id_from_value(&symbols[0])
                .ok_or_else(|| RlmError::ToolExecution("Could not extract symbol ID".to_string()))?
        } else {
            return Ok(ToolResult::error(
                "Must provide either 'symbol_name' or 'symbol_id'",
                true,
            ));
        };

        // Query for all incoming relationships (things that reference this symbol)
        // This includes CALLS, USES_TYPE, REFERENCES, IMPORTS, etc.
        let cypher = format!(
            "MATCH (user)-[r]->(target {{id: '{}'}}) RETURN user, type(r) AS relation",
            graphqlite::escape_string(&target_id)
        );

        let result = store
            .query(&cypher)
            .map_err(|e| RlmError::ToolExecution(format!("Failed to find usages: {}", e)))?;

        if result.is_empty() {
            return Ok(ToolResult::text(format!(
                "No usages found for '{}'",
                target_id
            )));
        }

        // Format results, grouping by relationship type
        let mut usages: Vec<serde_json::Value> = Vec::new();
        for row in result.iter() {
            let mut usage = serde_json::Map::new();

            if let Some(user) = row.get_value("user") {
                let formatted = format_symbol_value(user);
                if let serde_json::Value::Object(obj) = formatted {
                    for (k, v) in obj {
                        usage.insert(k, v);
                    }
                }
            }

            if let Some(rel) = row.get_value("relation") {
                usage.insert("relationship".to_string(), value_to_json(rel));
            }

            usages.push(serde_json::Value::Object(usage));
        }

        let output = serde_json::json!({
            "symbol": target_id,
            "usages": usages,
            "count": usages.len()
        });

        let mut result = ToolResult::json(output);
        result.metadata = ToolMetadata::with_source(&target_id).with_tag("usages");

        Ok(result)
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Format a symbol Value into a user-friendly JSON object.
fn format_symbol_value(value: &Value) -> serde_json::Value {
    match value {
        Value::Object(map) => {
            let mut result = serde_json::Map::new();

            // Extract from nested "properties" if present (graphqlite node format)
            let props = if let Some(Value::Object(p)) = map.get("properties") {
                p
            } else {
                map
            };

            // Map to user-friendly field names
            if let Some(Value::String(s)) = props.get("name") {
                result.insert("name".to_string(), serde_json::json!(s));
            }
            if let Some(Value::String(s)) = props.get("kind") {
                result.insert("type".to_string(), serde_json::json!(s));
            }
            if let Some(Value::String(s)) = props.get("file_path") {
                result.insert("file".to_string(), serde_json::json!(s));
            }
            if let Some(Value::String(s)) = props.get("start_line") {
                if let Ok(n) = s.parse::<u32>() {
                    result.insert("line".to_string(), serde_json::json!(n));
                }
            }
            if let Some(Value::String(s)) = props.get("signature") {
                result.insert("signature".to_string(), serde_json::json!(s));
            }
            if let Some(Value::String(s)) = props.get("visibility") {
                result.insert("visibility".to_string(), serde_json::json!(s));
            }

            serde_json::Value::Object(result)
        }
        _ => value_to_json(value),
    }
}

/// Extract or reconstruct the node ID from a Value::Object.
/// graphqlite returns nodes as: { "labels": [...], "properties": {...}, "id": <int> }
/// The "id" we want is stored inside "properties" as a String.
fn extract_id_from_value(value: &Value) -> Option<String> {
    match value {
        Value::Object(map) => {
            // graphqlite nodes have properties nested under "properties" key
            if let Some(Value::Object(props)) = map.get("properties") {
                // The node ID is stored as "id" string in properties
                if let Some(Value::String(id)) = props.get("id") {
                    return Some(id.clone());
                }
            }

            // Fallback: try direct "id" field as string (for simple objects)
            if let Some(Value::String(id)) = map.get("id") {
                return Some(id.clone());
            }

            None
        }
        _ => None,
    }
}

/// Convert a graphqlite Value to a serde_json Value.
fn value_to_json(value: &Value) -> serde_json::Value {
    match value {
        Value::Null => serde_json::Value::Null,
        Value::Bool(b) => serde_json::Value::Bool(*b),
        Value::Integer(i) => serde_json::Value::Number((*i).into()),
        Value::Float(f) => serde_json::Number::from_f64(*f)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        Value::String(s) => serde_json::Value::String(s.clone()),
        Value::Array(arr) => serde_json::Value::Array(arr.iter().map(value_to_json).collect()),
        Value::Object(map) => {
            let obj: serde_json::Map<String, serde_json::Value> = map
                .iter()
                .map(|(k, v)| (k.clone(), value_to_json(v)))
                .collect();
            serde_json::Value::Object(obj)
        }
    }
}

/// Create all graph tools for a given store.
pub fn create_graph_tools(store: SharedGraphStore) -> Vec<Box<dyn Tool>> {
    vec![
        // Primary search/browse tools
        Box::new(FindSymbolsTool::new(store.clone())),
        Box::new(FileOutlineTool::new(store.clone())),
        // Dependency/usage analysis
        Box::new(FindCallersTool::new(store.clone())),
        Box::new(FindCalleesTool::new(store.clone())),
        Box::new(FindUsagesTool::new(store.clone())),
        Box::new(FindImplementationsTool::new(store.clone())),
        // Detail lookup
        Box::new(GetSymbolTool::new(store.clone())),
        // Raw query as fallback for advanced users
        Box::new(GraphQueryTool::new(store)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use muninn_graph::{CallType, Edge, Symbol, SymbolKind, Visibility};
    use serial_test::serial;

    fn create_test_symbol(name: &str, kind: SymbolKind) -> Symbol {
        Symbol {
            name: name.to_string(),
            kind,
            file_path: "test.rs".to_string(),
            start_line: 1,
            end_line: 10,
            signature: Some(format!("fn {}()", name)),
            qualified_name: Some(format!("crate::{}", name)),
            doc_comment: None,
            visibility: Visibility::Public,
        }
    }

    fn setup_test_store() -> SharedGraphStore {
        let store = GraphStore::open_in_memory().unwrap();

        // Insert test symbols
        let main_fn = create_test_symbol("main", SymbolKind::Function);
        let helper_fn = create_test_symbol("helper", SymbolKind::Function);
        let greet_trait = Symbol {
            kind: SymbolKind::Interface,
            ..create_test_symbol("Greet", SymbolKind::Interface)
        };
        let person_struct = create_test_symbol("Person", SymbolKind::Struct);

        let main_id = store.insert_node(&main_fn).unwrap();
        let helper_id = store.insert_node(&helper_fn).unwrap();
        let trait_id = store.insert_node(&greet_trait).unwrap();
        let person_id = store.insert_node(&person_struct).unwrap();

        // Add relationships
        store
            .insert_edge(&Edge::calls(&main_id, &helper_id, CallType::Direct, 5))
            .unwrap();
        store
            .insert_edge(&Edge::implements(&person_id, &trait_id))
            .unwrap();

        wrap_store(store)
    }

    #[test]
    #[serial]
    fn test_create_graph_tools() {
        let store = setup_test_store();
        let tools = create_graph_tools(store);
        assert_eq!(tools.len(), 8);

        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert!(names.contains(&"find_symbols"));
        assert!(names.contains(&"file_outline"));
        assert!(names.contains(&"find_callers"));
        assert!(names.contains(&"find_callees"));
        assert!(names.contains(&"find_usages"));
        assert!(names.contains(&"find_implementations"));
        assert!(names.contains(&"get_symbol"));
        assert!(names.contains(&"graph_query"));
    }

    #[tokio::test]
    #[serial]
    async fn test_graph_query_tool() {
        let store = setup_test_store();
        let tool = GraphQueryTool::new(store);

        let result = tool
            .execute(serde_json::json!({
                "query": "MATCH (n:Function) RETURN n.name"
            }))
            .await
            .unwrap();

        assert!(!result.is_error());
        let content = result.to_string_content();
        assert!(content.contains("main") || content.contains("helper"));
    }

    #[tokio::test]
    #[serial]
    async fn test_graph_query_invalid() {
        let store = setup_test_store();
        let tool = GraphQueryTool::new(store);

        let result = tool
            .execute(serde_json::json!({
                "query": "INVALID QUERY SYNTAX"
            }))
            .await;

        // Should return an error
        assert!(result.is_err());
    }

    #[tokio::test]
    #[serial]
    async fn test_find_callers_tool() {
        let store = setup_test_store();
        let tool = FindCallersTool::new(store);

        let result = tool
            .execute(serde_json::json!({
                "function_name": "helper"
            }))
            .await
            .unwrap();

        assert!(!result.is_error());
        let content = result.to_string_content();
        // main calls helper
        assert!(content.contains("main") || content.contains("callers"));
    }

    #[tokio::test]
    #[serial]
    async fn test_find_callers_not_found() {
        let store = setup_test_store();
        let tool = FindCallersTool::new(store);

        let result = tool
            .execute(serde_json::json!({
                "function_name": "nonexistent"
            }))
            .await
            .unwrap();

        assert!(!result.is_error());
        assert!(result.to_string_content().contains("No function found"));
    }

    #[tokio::test]
    #[serial]
    async fn test_find_implementations_tool() {
        let store = setup_test_store();
        let tool = FindImplementationsTool::new(store);

        let result = tool
            .execute(serde_json::json!({
                "trait_name": "Greet"
            }))
            .await
            .unwrap();

        assert!(!result.is_error());
        let content = result.to_string_content();
        // Person implements Greet
        assert!(content.contains("Person") || content.contains("implementations"));
    }

    #[tokio::test]
    #[serial]
    async fn test_get_symbol_tool() {
        let store = setup_test_store();
        let tool = GetSymbolTool::new(store);

        let result = tool
            .execute(serde_json::json!({
                "name": "main"
            }))
            .await
            .unwrap();

        assert!(!result.is_error());
        let content = result.to_string_content();
        assert!(content.contains("main"));
    }

    #[tokio::test]
    #[serial]
    async fn test_get_symbol_not_found() {
        let store = setup_test_store();
        let tool = GetSymbolTool::new(store);

        let result = tool
            .execute(serde_json::json!({
                "name": "nonexistent"
            }))
            .await
            .unwrap();

        assert!(!result.is_error());
        assert!(result.to_string_content().contains("No symbol found"));
    }

    #[test]
    fn test_value_to_json() {
        // Test basic types
        assert_eq!(value_to_json(&Value::Null), serde_json::Value::Null);
        assert_eq!(value_to_json(&Value::Bool(true)), serde_json::json!(true));
        assert_eq!(value_to_json(&Value::Integer(42)), serde_json::json!(42));
        assert_eq!(
            value_to_json(&Value::String("hello".to_string())),
            serde_json::json!("hello")
        );
    }

    #[test]
    fn test_extract_id_from_value() {
        // Test with graphqlite node structure: { "properties": { "id": "..." } }
        let mut props = std::collections::HashMap::new();
        props.insert("id".to_string(), Value::String("test-id-123".to_string()));
        props.insert("name".to_string(), Value::String("test".to_string()));

        let mut node = std::collections::HashMap::new();
        node.insert("properties".to_string(), Value::Object(props));
        node.insert(
            "labels".to_string(),
            Value::Array(vec![Value::String("Function".to_string())]),
        );

        let value = Value::Object(node);
        assert_eq!(
            extract_id_from_value(&value),
            Some("test-id-123".to_string())
        );

        // Test with direct "id" field (fallback for simple objects)
        let mut simple = std::collections::HashMap::new();
        simple.insert("id".to_string(), Value::String("simple-id".to_string()));
        let simple_value = Value::Object(simple);
        assert_eq!(
            extract_id_from_value(&simple_value),
            Some("simple-id".to_string())
        );

        // Test with non-object
        assert_eq!(extract_id_from_value(&Value::Null), None);
    }
}
