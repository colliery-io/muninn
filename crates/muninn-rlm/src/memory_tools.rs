//! Memory tools for storing and retrieving knowledge during exploration.
//!
//! This module provides tools for the RLM to maintain working memory:
//! - Store facts, observations, and intermediate results
//! - Query and search stored memories
//! - Retrieve relevant context for decision-making

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::error::{Result, RlmError};
use crate::tools::{Tool, ToolMetadata, ToolResult};

// ============================================================================
// Memory Store Abstraction
// ============================================================================

/// A memory entry with metadata.
#[derive(Debug, Clone)]
pub struct MemoryEntry {
    /// Unique identifier for this memory
    pub id: String,
    /// The content/value of the memory
    pub content: String,
    /// Category or type of memory (e.g., "fact", "observation", "result")
    pub category: String,
    /// Tags for filtering and search
    pub tags: Vec<String>,
    /// Relevance score (0.0 to 1.0)
    pub relevance: f32,
    /// Timestamp when the memory was created
    pub created_at: u64,
}

impl MemoryEntry {
    /// Create a new memory entry.
    pub fn new(
        id: impl Into<String>,
        content: impl Into<String>,
        category: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            content: content.into(),
            category: category.into(),
            tags: Vec::new(),
            relevance: 1.0,
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        }
    }

    /// Add tags to this memory entry.
    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }

    /// Set relevance score.
    pub fn with_relevance(mut self, relevance: f32) -> Self {
        self.relevance = relevance.clamp(0.0, 1.0);
        self
    }
}

/// Trait for memory storage backends.
pub trait MemoryStore: Send + Sync {
    /// Store a memory entry.
    fn store(&self, entry: MemoryEntry) -> Result<()>;

    /// Retrieve a memory by ID.
    fn get(&self, id: &str) -> Result<Option<MemoryEntry>>;

    /// Delete a memory by ID.
    fn delete(&self, id: &str) -> Result<bool>;

    /// List all memories, optionally filtered by category.
    fn list(&self, category: Option<&str>, limit: usize) -> Result<Vec<MemoryEntry>>;

    /// Search memories by content (simple substring match).
    fn search(&self, query: &str, limit: usize) -> Result<Vec<MemoryEntry>>;

    /// Search memories by tags.
    fn search_by_tags(&self, tags: &[String], limit: usize) -> Result<Vec<MemoryEntry>>;

    /// Clear all memories.
    fn clear(&self) -> Result<()>;

    /// Get memory count.
    fn count(&self) -> Result<usize>;
}

/// Thread-safe wrapper for memory stores.
pub type SharedMemoryStore = Arc<dyn MemoryStore>;

// ============================================================================
// In-Memory Store Implementation
// ============================================================================

/// Simple in-memory storage for memories.
#[derive(Debug, Default)]
pub struct InMemoryStore {
    memories: RwLock<HashMap<String, MemoryEntry>>,
}

impl InMemoryStore {
    /// Create a new empty in-memory store.
    pub fn new() -> Self {
        Self {
            memories: RwLock::new(HashMap::new()),
        }
    }

    /// Create a shared instance.
    pub fn shared() -> SharedMemoryStore {
        Arc::new(Self::new())
    }
}

impl MemoryStore for InMemoryStore {
    fn store(&self, entry: MemoryEntry) -> Result<()> {
        let mut memories = self
            .memories
            .write()
            .map_err(|e| RlmError::ToolExecution(format!("Failed to acquire write lock: {}", e)))?;
        memories.insert(entry.id.clone(), entry);
        Ok(())
    }

    fn get(&self, id: &str) -> Result<Option<MemoryEntry>> {
        let memories = self
            .memories
            .read()
            .map_err(|e| RlmError::ToolExecution(format!("Failed to acquire read lock: {}", e)))?;
        Ok(memories.get(id).cloned())
    }

    fn delete(&self, id: &str) -> Result<bool> {
        let mut memories = self
            .memories
            .write()
            .map_err(|e| RlmError::ToolExecution(format!("Failed to acquire write lock: {}", e)))?;
        Ok(memories.remove(id).is_some())
    }

    fn list(&self, category: Option<&str>, limit: usize) -> Result<Vec<MemoryEntry>> {
        let memories = self
            .memories
            .read()
            .map_err(|e| RlmError::ToolExecution(format!("Failed to acquire read lock: {}", e)))?;

        let mut entries: Vec<_> = memories
            .values()
            .filter(|e| category.is_none_or(|c| e.category == c))
            .cloned()
            .collect();

        // Sort by relevance descending, then by creation time descending
        entries.sort_by(|a, b| {
            b.relevance
                .partial_cmp(&a.relevance)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.created_at.cmp(&a.created_at))
        });

        entries.truncate(limit);
        Ok(entries)
    }

    fn search(&self, query: &str, limit: usize) -> Result<Vec<MemoryEntry>> {
        let memories = self
            .memories
            .read()
            .map_err(|e| RlmError::ToolExecution(format!("Failed to acquire read lock: {}", e)))?;

        let query_lower = query.to_lowercase();
        let mut matches: Vec<_> = memories
            .values()
            .filter(|e| e.content.to_lowercase().contains(&query_lower))
            .cloned()
            .collect();

        // Sort by relevance
        matches.sort_by(|a, b| {
            b.relevance
                .partial_cmp(&a.relevance)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        matches.truncate(limit);
        Ok(matches)
    }

    fn search_by_tags(&self, tags: &[String], limit: usize) -> Result<Vec<MemoryEntry>> {
        let memories = self
            .memories
            .read()
            .map_err(|e| RlmError::ToolExecution(format!("Failed to acquire read lock: {}", e)))?;

        let mut matches: Vec<_> = memories
            .values()
            .filter(|e| tags.iter().any(|t| e.tags.contains(t)))
            .cloned()
            .collect();

        // Sort by number of matching tags, then relevance
        matches.sort_by(|a, b| {
            let a_matches = tags.iter().filter(|t| a.tags.contains(t)).count();
            let b_matches = tags.iter().filter(|t| b.tags.contains(t)).count();
            b_matches.cmp(&a_matches).then_with(|| {
                b.relevance
                    .partial_cmp(&a.relevance)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
        });

        matches.truncate(limit);
        Ok(matches)
    }

    fn clear(&self) -> Result<()> {
        let mut memories = self
            .memories
            .write()
            .map_err(|e| RlmError::ToolExecution(format!("Failed to acquire write lock: {}", e)))?;
        memories.clear();
        Ok(())
    }

    fn count(&self) -> Result<usize> {
        let memories = self
            .memories
            .read()
            .map_err(|e| RlmError::ToolExecution(format!("Failed to acquire read lock: {}", e)))?;
        Ok(memories.len())
    }
}

// ============================================================================
// StoreMemoryTool
// ============================================================================

/// Tool for storing memories.
pub struct StoreMemoryTool {
    store: SharedMemoryStore,
}

impl StoreMemoryTool {
    pub fn new(store: SharedMemoryStore) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for StoreMemoryTool {
    fn name(&self) -> &str {
        "store_memory"
    }

    fn description(&self) -> &str {
        "Store a fact, observation, or intermediate result in working memory for later retrieval. \
         Use this to remember important information discovered during exploration."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "Unique identifier for this memory (use descriptive names)"
                },
                "content": {
                    "type": "string",
                    "description": "The content to store (fact, observation, result, etc.)"
                },
                "category": {
                    "type": "string",
                    "description": "Category: 'fact', 'observation', 'result', 'note', or custom",
                    "default": "note"
                },
                "tags": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Tags for organizing and searching memories"
                },
                "relevance": {
                    "type": "number",
                    "description": "Relevance score from 0.0 to 1.0 (default: 1.0)"
                }
            },
            "required": ["id", "content"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<ToolResult> {
        let id = params.get("id").and_then(|v| v.as_str()).ok_or_else(|| {
            RlmError::ToolExecution("Missing required parameter 'id'".to_string())
        })?;

        let content = params
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                RlmError::ToolExecution("Missing required parameter 'content'".to_string())
            })?;

        let category = params
            .get("category")
            .and_then(|v| v.as_str())
            .unwrap_or("note");

        let tags: Vec<String> = params
            .get("tags")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let relevance = params
            .get("relevance")
            .and_then(|v| v.as_f64())
            .map(|f| f as f32)
            .unwrap_or(1.0);

        let entry = MemoryEntry::new(id, content, category)
            .with_tags(tags)
            .with_relevance(relevance);

        self.store.store(entry)?;

        let output = serde_json::json!({
            "stored": true,
            "id": id
        });

        let mut result = ToolResult::json(output);
        result.metadata = ToolMetadata::with_source(id).with_tag("memory");

        Ok(result)
    }
}

// ============================================================================
// QueryMemoryTool
// ============================================================================

/// Tool for retrieving a specific memory by ID.
pub struct QueryMemoryTool {
    store: SharedMemoryStore,
}

impl QueryMemoryTool {
    pub fn new(store: SharedMemoryStore) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for QueryMemoryTool {
    fn name(&self) -> &str {
        "query_memory"
    }

    fn description(&self) -> &str {
        "Retrieve a specific memory by its ID. Returns the stored content and metadata."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "The ID of the memory to retrieve"
                }
            },
            "required": ["id"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<ToolResult> {
        let id = params.get("id").and_then(|v| v.as_str()).ok_or_else(|| {
            RlmError::ToolExecution("Missing required parameter 'id'".to_string())
        })?;

        match self.store.get(id)? {
            Some(entry) => {
                let output = serde_json::json!({
                    "found": true,
                    "id": entry.id,
                    "content": entry.content,
                    "category": entry.category,
                    "tags": entry.tags,
                    "relevance": entry.relevance,
                    "created_at": entry.created_at
                });

                let mut result = ToolResult::json(output);
                result.metadata = ToolMetadata::with_source(&entry.id).with_tag("memory");
                Ok(result)
            }
            None => Ok(ToolResult::text(format!(
                "No memory found with ID '{}'",
                id
            ))),
        }
    }
}

// ============================================================================
// SearchMemoryTool
// ============================================================================

/// Tool for searching memories by content or tags.
pub struct SearchMemoryTool {
    store: SharedMemoryStore,
}

impl SearchMemoryTool {
    pub fn new(store: SharedMemoryStore) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for SearchMemoryTool {
    fn name(&self) -> &str {
        "search_memory"
    }

    fn description(&self) -> &str {
        "Search stored memories by content text or tags. Returns matching memories sorted by relevance."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Text to search for in memory content"
                },
                "tags": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Tags to filter by (matches any)"
                },
                "category": {
                    "type": "string",
                    "description": "Filter by category"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum results to return (default: 20)"
                }
            }
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<ToolResult> {
        let query = params.get("query").and_then(|v| v.as_str());
        let tags: Option<Vec<String>> = params.get("tags").and_then(|v| v.as_array()).map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        });
        let category = params.get("category").and_then(|v| v.as_str());
        let limit = params
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(20);

        let results = if let Some(q) = query {
            self.store.search(q, limit)?
        } else if let Some(t) = tags {
            self.store.search_by_tags(&t, limit)?
        } else {
            self.store.list(category, limit)?
        };

        let entries: Vec<serde_json::Value> = results
            .iter()
            .map(|e| {
                serde_json::json!({
                    "id": e.id,
                    "content": e.content,
                    "category": e.category,
                    "tags": e.tags,
                    "relevance": e.relevance
                })
            })
            .collect();

        let output = serde_json::json!({
            "results": entries,
            "count": entries.len()
        });

        let mut result = ToolResult::json(output);
        result.metadata = ToolMetadata::with_source("search").with_tag("memory");

        Ok(result)
    }
}

// ============================================================================
// ListMemoriesTool
// ============================================================================

/// Tool for listing all memories.
pub struct ListMemoriesTool {
    store: SharedMemoryStore,
}

impl ListMemoriesTool {
    pub fn new(store: SharedMemoryStore) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for ListMemoriesTool {
    fn name(&self) -> &str {
        "list_memories"
    }

    fn description(&self) -> &str {
        "List all stored memories, optionally filtered by category. Shows memory IDs and summaries."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "category": {
                    "type": "string",
                    "description": "Filter by category (e.g., 'fact', 'observation', 'result')"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum memories to list (default: 50)"
                }
            }
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<ToolResult> {
        let category = params.get("category").and_then(|v| v.as_str());
        let limit = params
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(50);

        let memories = self.store.list(category, limit)?;
        let total = self.store.count()?;

        let entries: Vec<serde_json::Value> = memories
            .iter()
            .map(|e| {
                // Truncate content for summary
                let summary = if e.content.len() > 100 {
                    format!("{}...", &e.content[..100])
                } else {
                    e.content.clone()
                };
                serde_json::json!({
                    "id": e.id,
                    "summary": summary,
                    "category": e.category,
                    "tags": e.tags
                })
            })
            .collect();

        let output = serde_json::json!({
            "memories": entries,
            "count": entries.len(),
            "total": total
        });

        let mut result = ToolResult::json(output);
        result.metadata = ToolMetadata::with_source("list").with_tag("memory");

        Ok(result)
    }
}

// ============================================================================
// DeleteMemoryTool
// ============================================================================

/// Tool for deleting memories.
pub struct DeleteMemoryTool {
    store: SharedMemoryStore,
}

impl DeleteMemoryTool {
    pub fn new(store: SharedMemoryStore) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for DeleteMemoryTool {
    fn name(&self) -> &str {
        "delete_memory"
    }

    fn description(&self) -> &str {
        "Delete a memory by ID. Use to remove outdated or incorrect information."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "The ID of the memory to delete"
                }
            },
            "required": ["id"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<ToolResult> {
        let id = params.get("id").and_then(|v| v.as_str()).ok_or_else(|| {
            RlmError::ToolExecution("Missing required parameter 'id'".to_string())
        })?;

        let deleted = self.store.delete(id)?;

        let output = serde_json::json!({
            "deleted": deleted,
            "id": id
        });

        let mut result = ToolResult::json(output);
        result.metadata = ToolMetadata::with_source(id).with_tag("memory");

        Ok(result)
    }
}

// ============================================================================
// Factory Function
// ============================================================================

/// Create all memory tools for a given store.
pub fn create_memory_tools(store: SharedMemoryStore) -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(StoreMemoryTool::new(Arc::clone(&store))),
        Box::new(QueryMemoryTool::new(Arc::clone(&store))),
        Box::new(SearchMemoryTool::new(Arc::clone(&store))),
        Box::new(ListMemoriesTool::new(Arc::clone(&store))),
        Box::new(DeleteMemoryTool::new(store)),
    ]
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_test_store() -> SharedMemoryStore {
        InMemoryStore::shared()
    }

    #[test]
    fn test_memory_entry_creation() {
        let entry = MemoryEntry::new("test-1", "Test content", "fact")
            .with_tags(vec!["tag1".to_string(), "tag2".to_string()])
            .with_relevance(0.8);

        assert_eq!(entry.id, "test-1");
        assert_eq!(entry.content, "Test content");
        assert_eq!(entry.category, "fact");
        assert_eq!(entry.tags, vec!["tag1", "tag2"]);
        assert_eq!(entry.relevance, 0.8);
    }

    #[test]
    fn test_relevance_clamping() {
        let entry1 = MemoryEntry::new("1", "test", "note").with_relevance(1.5);
        assert_eq!(entry1.relevance, 1.0);

        let entry2 = MemoryEntry::new("2", "test", "note").with_relevance(-0.5);
        assert_eq!(entry2.relevance, 0.0);
    }

    #[test]
    fn test_in_memory_store_basic() {
        let store = InMemoryStore::new();

        // Store
        let entry = MemoryEntry::new("test-1", "Hello world", "fact");
        store.store(entry).unwrap();

        // Get
        let retrieved = store.get("test-1").unwrap().unwrap();
        assert_eq!(retrieved.content, "Hello world");

        // Count
        assert_eq!(store.count().unwrap(), 1);

        // Delete
        assert!(store.delete("test-1").unwrap());
        assert!(store.get("test-1").unwrap().is_none());
        assert_eq!(store.count().unwrap(), 0);
    }

    #[test]
    fn test_in_memory_store_search() {
        let store = InMemoryStore::new();

        store
            .store(MemoryEntry::new("1", "The quick brown fox", "fact"))
            .unwrap();
        store
            .store(MemoryEntry::new("2", "The lazy dog", "fact"))
            .unwrap();
        store
            .store(MemoryEntry::new("3", "Hello world", "note"))
            .unwrap();

        // Search by content
        let results = store.search("quick", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "1");

        // Search case insensitive
        let results = store.search("LAZY", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "2");
    }

    #[test]
    fn test_in_memory_store_search_by_tags() {
        let store = InMemoryStore::new();

        store
            .store(
                MemoryEntry::new("1", "Content 1", "fact")
                    .with_tags(vec!["rust".to_string(), "code".to_string()]),
            )
            .unwrap();
        store
            .store(
                MemoryEntry::new("2", "Content 2", "fact")
                    .with_tags(vec!["python".to_string(), "code".to_string()]),
            )
            .unwrap();
        store
            .store(MemoryEntry::new("3", "Content 3", "note").with_tags(vec!["rust".to_string()]))
            .unwrap();

        // Search by single tag
        let results = store.search_by_tags(&["rust".to_string()], 10).unwrap();
        assert_eq!(results.len(), 2);

        // Search by multiple tags (matches any)
        let results = store.search_by_tags(&["python".to_string()], 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "2");
    }

    #[test]
    fn test_in_memory_store_list_with_category() {
        let store = InMemoryStore::new();

        store
            .store(MemoryEntry::new("1", "Fact 1", "fact"))
            .unwrap();
        store
            .store(MemoryEntry::new("2", "Fact 2", "fact"))
            .unwrap();
        store
            .store(MemoryEntry::new("3", "Note 1", "note"))
            .unwrap();

        // List all
        let results = store.list(None, 10).unwrap();
        assert_eq!(results.len(), 3);

        // List by category
        let facts = store.list(Some("fact"), 10).unwrap();
        assert_eq!(facts.len(), 2);

        let notes = store.list(Some("note"), 10).unwrap();
        assert_eq!(notes.len(), 1);
    }

    #[test]
    fn test_create_memory_tools() {
        let store = setup_test_store();
        let tools = create_memory_tools(store);
        assert_eq!(tools.len(), 5);

        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert!(names.contains(&"store_memory"));
        assert!(names.contains(&"query_memory"));
        assert!(names.contains(&"search_memory"));
        assert!(names.contains(&"list_memories"));
        assert!(names.contains(&"delete_memory"));
    }

    #[tokio::test]
    async fn test_store_memory_tool() {
        let store = setup_test_store();
        let tool = StoreMemoryTool::new(Arc::clone(&store));

        let result = tool
            .execute(serde_json::json!({
                "id": "test-memory",
                "content": "This is a test fact",
                "category": "fact",
                "tags": ["test", "example"]
            }))
            .await
            .unwrap();

        assert!(!result.is_error());

        // Verify it was stored
        let entry = store.get("test-memory").unwrap().unwrap();
        assert_eq!(entry.content, "This is a test fact");
        assert_eq!(entry.category, "fact");
        assert_eq!(entry.tags, vec!["test", "example"]);
    }

    #[tokio::test]
    async fn test_query_memory_tool() {
        let store = setup_test_store();
        store
            .store(MemoryEntry::new("my-fact", "Important information", "fact"))
            .unwrap();

        let tool = QueryMemoryTool::new(store);

        let result = tool
            .execute(serde_json::json!({
                "id": "my-fact"
            }))
            .await
            .unwrap();

        assert!(!result.is_error());
        let content = result.to_string_content();
        assert!(content.contains("Important information"));
    }

    #[tokio::test]
    async fn test_query_memory_not_found() {
        let store = setup_test_store();
        let tool = QueryMemoryTool::new(store);

        let result = tool
            .execute(serde_json::json!({
                "id": "nonexistent"
            }))
            .await
            .unwrap();

        assert!(!result.is_error());
        assert!(result.to_string_content().contains("No memory found"));
    }

    #[tokio::test]
    async fn test_search_memory_tool() {
        let store = setup_test_store();
        store
            .store(MemoryEntry::new("1", "Rust is a systems language", "fact"))
            .unwrap();
        store
            .store(MemoryEntry::new("2", "Python is interpreted", "fact"))
            .unwrap();

        let tool = SearchMemoryTool::new(store);

        let result = tool
            .execute(serde_json::json!({
                "query": "Rust"
            }))
            .await
            .unwrap();

        assert!(!result.is_error());
        let content = result.to_string_content();
        assert!(content.contains("systems language"));
    }

    #[tokio::test]
    async fn test_list_memories_tool() {
        let store = setup_test_store();
        store
            .store(MemoryEntry::new("1", "Memory 1", "fact"))
            .unwrap();
        store
            .store(MemoryEntry::new("2", "Memory 2", "note"))
            .unwrap();

        let tool = ListMemoriesTool::new(store);

        let result = tool.execute(serde_json::json!({})).await.unwrap();

        assert!(!result.is_error());
        let content = result.to_string_content();
        // JSON may have spaces: "count": 2
        assert!(content.contains("\"count\"") && content.contains("2"));
    }

    #[tokio::test]
    async fn test_delete_memory_tool() {
        let store = setup_test_store();
        store
            .store(MemoryEntry::new("to-delete", "Content", "note"))
            .unwrap();

        let tool = DeleteMemoryTool::new(Arc::clone(&store));

        let result = tool
            .execute(serde_json::json!({
                "id": "to-delete"
            }))
            .await
            .unwrap();

        assert!(!result.is_error());
        let content = result.to_string_content();
        // JSON may have spaces: "deleted": true
        assert!(content.contains("\"deleted\"") && content.contains("true"));

        // Verify deletion
        assert!(store.get("to-delete").unwrap().is_none());
    }
}
