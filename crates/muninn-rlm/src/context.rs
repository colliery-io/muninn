//! Context aggregation for combining tool results.
//!
//! This module provides utilities for:
//! - Combining results from multiple tool calls
//! - Deduplicating overlapping information
//! - Ranking by relevance
//! - Truncating to fit context limits

use std::collections::HashSet;

// ============================================================================
// Context Item
// ============================================================================

/// A piece of context with metadata.
#[derive(Debug, Clone)]
pub struct ContextItem {
    /// The content of this context item.
    pub content: String,
    /// Source identifier (e.g., file path, tool name).
    pub source: String,
    /// Relevance score (0.0 to 1.0).
    pub relevance: f32,
    /// Category/type of context.
    pub category: String,
    /// Content hash for deduplication.
    hash: u64,
}

impl ContextItem {
    /// Create a new context item.
    pub fn new(content: impl Into<String>, source: impl Into<String>) -> Self {
        let content = content.into();
        let hash = Self::compute_hash(&content);
        Self {
            content,
            source: source.into(),
            relevance: 1.0,
            category: "general".to_string(),
            hash,
        }
    }

    /// Set the relevance score.
    pub fn with_relevance(mut self, relevance: f32) -> Self {
        self.relevance = relevance.clamp(0.0, 1.0);
        self
    }

    /// Set the category.
    pub fn with_category(mut self, category: impl Into<String>) -> Self {
        self.category = category.into();
        self
    }

    /// Compute a simple hash for deduplication.
    fn compute_hash(content: &str) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        // Normalize whitespace for hash
        content
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .hash(&mut hasher);
        hasher.finish()
    }

    /// Get content length in characters.
    pub fn len(&self) -> usize {
        self.content.len()
    }

    /// Check if content is empty.
    pub fn is_empty(&self) -> bool {
        self.content.is_empty()
    }
}

// ============================================================================
// Context Aggregator
// ============================================================================

/// Aggregates and manages context from multiple sources.
#[derive(Debug, Default)]
pub struct ContextAggregator {
    items: Vec<ContextItem>,
    seen_hashes: HashSet<u64>,
    max_total_chars: usize,
}

impl ContextAggregator {
    /// Create a new aggregator with default limits.
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            seen_hashes: HashSet::new(),
            max_total_chars: 100_000, // ~25k tokens
        }
    }

    /// Set the maximum total characters.
    pub fn with_max_chars(mut self, max: usize) -> Self {
        self.max_total_chars = max;
        self
    }

    /// Add a context item, deduplicating if already seen.
    pub fn add(&mut self, item: ContextItem) -> bool {
        if item.is_empty() {
            return false;
        }

        if self.seen_hashes.contains(&item.hash) {
            return false;
        }

        self.seen_hashes.insert(item.hash);
        self.items.push(item);
        true
    }

    /// Add content with source (convenience method).
    pub fn add_content(&mut self, content: impl Into<String>, source: impl Into<String>) -> bool {
        self.add(ContextItem::new(content, source))
    }

    /// Add content with source and relevance.
    pub fn add_with_relevance(
        &mut self,
        content: impl Into<String>,
        source: impl Into<String>,
        relevance: f32,
    ) -> bool {
        self.add(ContextItem::new(content, source).with_relevance(relevance))
    }

    /// Get the number of items.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Check if empty.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Get total content size in characters.
    pub fn total_chars(&self) -> usize {
        self.items.iter().map(|i| i.len()).sum()
    }

    /// Sort items by relevance (highest first).
    pub fn sort_by_relevance(&mut self) {
        self.items.sort_by(|a, b| {
            b.relevance
                .partial_cmp(&a.relevance)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    }

    /// Sort items by category, then relevance.
    pub fn sort_by_category(&mut self) {
        self.items.sort_by(|a, b| {
            a.category.cmp(&b.category).then_with(|| {
                b.relevance
                    .partial_cmp(&a.relevance)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
        });
    }

    /// Truncate to fit within max_total_chars, keeping highest relevance items.
    pub fn truncate_to_limit(&mut self) {
        self.sort_by_relevance();

        let mut total = 0;
        let mut keep = 0;

        for item in &self.items {
            if total + item.len() > self.max_total_chars {
                break;
            }
            total += item.len();
            keep += 1;
        }

        self.items.truncate(keep);
    }

    /// Get items as a slice.
    pub fn items(&self) -> &[ContextItem] {
        &self.items
    }

    /// Take all items, consuming the aggregator.
    pub fn into_items(self) -> Vec<ContextItem> {
        self.items
    }

    /// Clear all items.
    pub fn clear(&mut self) {
        self.items.clear();
        self.seen_hashes.clear();
    }

    /// Build formatted context string for LLM consumption.
    pub fn build(&self) -> String {
        if self.items.is_empty() {
            return String::new();
        }

        let mut output = String::new();

        for item in &self.items {
            if !output.is_empty() {
                output.push_str("\n\n---\n\n");
            }
            output.push_str(&format!(
                "[{}] (relevance: {:.2})\n",
                item.source, item.relevance
            ));
            output.push_str(&item.content);
        }

        output
    }

    /// Build as JSON structure.
    pub fn build_json(&self) -> serde_json::Value {
        let items: Vec<serde_json::Value> = self
            .items
            .iter()
            .map(|item| {
                serde_json::json!({
                    "source": item.source,
                    "category": item.category,
                    "relevance": item.relevance,
                    "content": item.content
                })
            })
            .collect();

        serde_json::json!({
            "context": items,
            "total_items": self.items.len(),
            "total_chars": self.total_chars()
        })
    }

    /// Build compact format (just content, no metadata).
    pub fn build_compact(&self) -> String {
        self.items
            .iter()
            .map(|i| i.content.as_str())
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

// ============================================================================
// Context Builder (Fluent API)
// ============================================================================

/// Builder for constructing context with a fluent API.
pub struct ContextBuilder {
    aggregator: ContextAggregator,
}

impl ContextBuilder {
    /// Create a new builder.
    pub fn new() -> Self {
        Self {
            aggregator: ContextAggregator::new(),
        }
    }

    /// Set max characters limit.
    pub fn max_chars(mut self, max: usize) -> Self {
        self.aggregator.max_total_chars = max;
        self
    }

    /// Add a context item.
    pub fn add(mut self, content: impl Into<String>, source: impl Into<String>) -> Self {
        self.aggregator.add_content(content, source);
        self
    }

    /// Add with relevance.
    pub fn add_with_relevance(
        mut self,
        content: impl Into<String>,
        source: impl Into<String>,
        relevance: f32,
    ) -> Self {
        self.aggregator
            .add_with_relevance(content, source, relevance);
        self
    }

    /// Add a pre-built item.
    pub fn add_item(mut self, item: ContextItem) -> Self {
        self.aggregator.add(item);
        self
    }

    /// Sort by relevance and truncate.
    pub fn finalize(mut self) -> ContextAggregator {
        self.aggregator.truncate_to_limit();
        self.aggregator
    }

    /// Build directly to string.
    pub fn build(self) -> String {
        self.finalize().build()
    }

    /// Build directly to JSON.
    pub fn build_json(self) -> serde_json::Value {
        self.finalize().build_json()
    }
}

impl Default for ContextBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_item_creation() {
        let item = ContextItem::new("test content", "test.rs")
            .with_relevance(0.8)
            .with_category("code");

        assert_eq!(item.content, "test content");
        assert_eq!(item.source, "test.rs");
        assert_eq!(item.relevance, 0.8);
        assert_eq!(item.category, "code");
    }

    #[test]
    fn test_relevance_clamping() {
        let item1 = ContextItem::new("test", "src").with_relevance(1.5);
        assert_eq!(item1.relevance, 1.0);

        let item2 = ContextItem::new("test", "src").with_relevance(-0.5);
        assert_eq!(item2.relevance, 0.0);
    }

    #[test]
    fn test_aggregator_add() {
        let mut agg = ContextAggregator::new();

        assert!(agg.add_content("first", "src1"));
        assert!(agg.add_content("second", "src2"));
        assert_eq!(agg.len(), 2);
    }

    #[test]
    fn test_aggregator_deduplication() {
        let mut agg = ContextAggregator::new();

        assert!(agg.add_content("same content", "src1"));
        assert!(!agg.add_content("same content", "src2")); // Duplicate
        assert_eq!(agg.len(), 1);
    }

    #[test]
    fn test_aggregator_whitespace_normalization() {
        let mut agg = ContextAggregator::new();

        assert!(agg.add_content("hello world", "src1"));
        assert!(!agg.add_content("hello  world", "src2")); // Same after normalization
        assert!(!agg.add_content("hello\nworld", "src3")); // Same after normalization
        assert_eq!(agg.len(), 1);
    }

    #[test]
    fn test_aggregator_empty_content() {
        let mut agg = ContextAggregator::new();

        assert!(!agg.add_content("", "src"));
        assert_eq!(agg.len(), 0);
    }

    #[test]
    fn test_aggregator_sort_by_relevance() {
        let mut agg = ContextAggregator::new();

        agg.add(ContextItem::new("low", "src1").with_relevance(0.2));
        agg.add(ContextItem::new("high", "src2").with_relevance(0.9));
        agg.add(ContextItem::new("med", "src3").with_relevance(0.5));

        agg.sort_by_relevance();

        let items = agg.items();
        assert_eq!(items[0].content, "high");
        assert_eq!(items[1].content, "med");
        assert_eq!(items[2].content, "low");
    }

    #[test]
    fn test_aggregator_truncate() {
        let mut agg = ContextAggregator::new().with_max_chars(20);

        agg.add(ContextItem::new("short", "src1").with_relevance(0.5)); // 5 chars
        agg.add(ContextItem::new("longer content here", "src2").with_relevance(0.9)); // 19 chars
        agg.add(ContextItem::new("medium len", "src3").with_relevance(0.7)); // 10 chars

        agg.truncate_to_limit();

        // Should keep highest relevance that fits
        assert_eq!(agg.len(), 1);
        assert_eq!(agg.items()[0].content, "longer content here");
    }

    #[test]
    fn test_aggregator_build() {
        let mut agg = ContextAggregator::new();
        agg.add_content("content one", "src1");
        agg.add_content("content two", "src2");

        let output = agg.build();
        assert!(output.contains("content one"));
        assert!(output.contains("content two"));
        assert!(output.contains("[src1]"));
        assert!(output.contains("[src2]"));
    }

    #[test]
    fn test_aggregator_build_compact() {
        let mut agg = ContextAggregator::new();
        agg.add_content("first", "src1");
        agg.add_content("second", "src2");

        let output = agg.build_compact();
        assert_eq!(output, "first\n\nsecond");
    }

    #[test]
    fn test_aggregator_build_json() {
        let mut agg = ContextAggregator::new();
        agg.add_content("test", "src");

        let json = agg.build_json();
        assert_eq!(json["total_items"], 1);
        assert!(json["context"].is_array());
    }

    #[test]
    fn test_context_builder() {
        let context = ContextBuilder::new()
            .max_chars(1000)
            .add("first content", "src1")
            .add_with_relevance("important", "src2", 0.9)
            .build();

        assert!(context.contains("first content"));
        assert!(context.contains("important"));
    }

    #[test]
    fn test_context_builder_finalize() {
        let agg = ContextBuilder::new().add("test", "src").finalize();

        assert_eq!(agg.len(), 1);
    }

    #[test]
    fn test_total_chars() {
        let mut agg = ContextAggregator::new();
        agg.add_content("hello", "src1"); // 5
        agg.add_content("world", "src2"); // 5

        assert_eq!(agg.total_chars(), 10);
    }

    #[test]
    fn test_clear() {
        let mut agg = ContextAggregator::new();
        agg.add_content("test", "src");
        assert_eq!(agg.len(), 1);

        agg.clear();
        assert_eq!(agg.len(), 0);
        assert!(agg.is_empty());

        // Can add same content again after clear
        assert!(agg.add_content("test", "src"));
    }
}
