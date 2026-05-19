//! Integration tests for documentation indexing pipelines.
//!
//! These tests hit real network endpoints (crates.io, PyPI, llms.txt URLs)
//! and are marked `#[ignore]` by default. Run with:
//!
//! ```bash
//! cargo test --package muninn-graph --test doc_indexing -- --ignored
//! ```
//!
//! Or run all tests including ignored:
//! ```bash
//! cargo test --package muninn-graph --test doc_indexing -- --include-ignored
//! ```

use muninn_graph::doc_store::{DocStore, Ecosystem};
use muninn_graph::registry::{
    IndexerConfig, LlmsTxtIndexer, LlmsTxtIndexerConfig, PyDocIndexer, PyIndexerConfig,
    RustDocIndexer,
};

// ============================================================================
// Rust Crate Indexing Tests
// ============================================================================

/// Index the `thiserror` crate and verify documentation is searchable.
///
/// thiserror is a good test case because:
/// - Small crate with focused functionality
/// - Stable API (unlikely to change drastically)
/// - Has well-documented derive macro
#[test]
#[ignore = "requires network access to crates.io"]
fn test_index_crate_thiserror() {
    let store = DocStore::open_in_memory().expect("Failed to create store");

    let config = IndexerConfig {
        keep_source: false,
        work_dir: None,
        rustdoc_flags: Vec::new(),
    };
    let indexer = RustDocIndexer::with_config(config);

    // Index thiserror
    let stats = indexer
        .index_crate(&store, "thiserror", None)
        .expect("Failed to index thiserror");

    println!(
        "Indexed {} v{}: {} items extracted, {} indexed",
        stats.crate_name, stats.version, stats.items_extracted, stats.items_indexed
    );

    // Verify library is in store
    let lib = store
        .get_library("thiserror")
        .expect("Failed to get library")
        .expect("Library should exist");

    assert_eq!(lib.library, "thiserror");
    assert_eq!(lib.ecosystem, Ecosystem::Rust);
    assert!(!lib.version.is_empty());

    // Verify we indexed some items
    assert!(
        stats.items_indexed > 0,
        "Should have indexed at least one item"
    );

    // Search for "Error" - thiserror's main export
    let results = store
        .search("thiserror", "Error derive", 10)
        .expect("Search failed");

    println!("Search results for 'Error derive':");
    for result in &results {
        println!(
            "  - {} ({}): {}",
            result.chunk.item_path,
            result.chunk.item_type.as_str(),
            &result.chunk.doc_text[..result.chunk.doc_text.len().min(100)]
        );
    }

    // Should find something related to Error
    assert!(
        !results.is_empty(),
        "Should find results for 'Error derive' in thiserror"
    );
}

/// Index the `once_cell` crate - another small, stable crate.
#[test]
#[ignore = "requires network access to crates.io"]
fn test_index_crate_once_cell() {
    let store = DocStore::open_in_memory().expect("Failed to create store");

    let config = IndexerConfig {
        keep_source: false,
        work_dir: None,
        rustdoc_flags: Vec::new(),
    };
    let indexer = RustDocIndexer::with_config(config);

    let stats = indexer
        .index_crate(&store, "once_cell", None)
        .expect("Failed to index once_cell");

    println!(
        "Indexed {} v{}: {} items extracted, {} indexed",
        stats.crate_name, stats.version, stats.items_extracted, stats.items_indexed
    );

    // Verify library exists
    let lib = store
        .get_library("once_cell")
        .expect("Failed to get library")
        .expect("Library should exist");

    assert_eq!(lib.ecosystem, Ecosystem::Rust);

    // Search for Lazy - one of once_cell's main types
    let results = store
        .search("once_cell", "Lazy initialization", 10)
        .expect("Search failed");

    println!("Search results for 'Lazy initialization':");
    for result in &results {
        println!(
            "  - {} ({})",
            result.chunk.item_path,
            result.chunk.item_type.as_str()
        );
    }

    assert!(
        !results.is_empty() || stats.items_indexed > 0,
        "Should either find search results or have indexed items"
    );
}

/// Index a specific version of a crate.
#[test]
#[ignore = "requires network access to crates.io"]
fn test_index_crate_specific_version() {
    let store = DocStore::open_in_memory().expect("Failed to create store");

    let config = IndexerConfig {
        keep_source: false,
        work_dir: None,
        rustdoc_flags: Vec::new(),
    };
    let indexer = RustDocIndexer::with_config(config);

    // Index a specific version of thiserror
    let stats = indexer
        .index_crate(&store, "thiserror", Some("1.0.50"))
        .expect("Failed to index thiserror 1.0.50");

    assert_eq!(stats.version, "1.0.50");

    let lib = store
        .get_library("thiserror")
        .expect("Failed to get library")
        .expect("Library should exist");

    assert_eq!(lib.version, "1.0.50");
}

// ============================================================================
// Python Package Indexing Tests
// ============================================================================

/// Index the `six` package - a minimal, stable Python package.
///
/// six is a good test case because:
/// - Very small and simple
/// - Extremely stable (Python 2/3 compatibility layer)
/// - Has docstrings
#[test]
#[ignore = "requires network access to PyPI"]
fn test_index_package_six() {
    let store = DocStore::open_in_memory().expect("Failed to create store");

    let config = PyIndexerConfig {
        keep_source: false,
        work_dir: None,
        ..Default::default()
    };
    let indexer = PyDocIndexer::with_config(config);

    let stats = indexer
        .index_package(&store, "six", None)
        .expect("Failed to index six");

    println!(
        "Indexed {} v{}: {} items extracted, {} indexed",
        stats.package_name, stats.version, stats.items_extracted, stats.items_indexed
    );

    // Verify library exists
    let lib = store
        .get_library("six")
        .expect("Failed to get library")
        .expect("Library should exist");

    assert_eq!(lib.library, "six");
    assert_eq!(lib.ecosystem, Ecosystem::Python);

    // Search for Python version compatibility
    let results = store.search("six", "python", 10).expect("Search failed");

    println!("Search results for 'python':");
    for result in &results {
        println!(
            "  - {} ({})",
            result.chunk.item_path,
            result.chunk.item_type.as_str()
        );
    }
}

/// Index the `attrs` package - popular, well-documented package.
#[test]
#[ignore = "requires network access to PyPI"]
fn test_index_package_attrs() {
    let store = DocStore::open_in_memory().expect("Failed to create store");

    let config = PyIndexerConfig {
        keep_source: false,
        work_dir: None,
        ..Default::default()
    };
    let indexer = PyDocIndexer::with_config(config);

    let stats = indexer
        .index_package(&store, "attrs", None)
        .expect("Failed to index attrs");

    println!(
        "Indexed {} v{}: {} items extracted, {} indexed",
        stats.package_name, stats.version, stats.items_extracted, stats.items_indexed
    );

    // Verify library exists
    let lib = store
        .get_library("attrs")
        .expect("Failed to get library")
        .expect("Library should exist");

    assert_eq!(lib.ecosystem, Ecosystem::Python);

    // Search for define decorator
    let results = store
        .search("attrs", "define class", 10)
        .expect("Search failed");

    println!("Search results for 'define class':");
    for result in &results {
        println!(
            "  - {} ({})",
            result.chunk.item_path,
            result.chunk.item_type.as_str()
        );
    }

    // attrs should have indexed items
    assert!(
        stats.items_indexed > 0,
        "Should have indexed items from attrs"
    );
}

// ============================================================================
// llms.txt Indexing Tests
// ============================================================================

/// Index Mintlify's llms.txt in fast mode (descriptions only).
#[test]
#[ignore = "requires network access"]
fn test_index_llmstxt_mintlify_fast() {
    let store = DocStore::open_in_memory().expect("Failed to create store");

    let config = LlmsTxtIndexerConfig {
        fetch_linked_content: false, // Fast mode
        max_links: 50,
        ..Default::default()
    };
    let indexer = LlmsTxtIndexer::with_config(config);

    let stats = indexer
        .index_url(&store, "https://mintlify.com/docs/llms.txt")
        .expect("Failed to index Mintlify llms.txt");

    println!(
        "Indexed '{}': {} links found, {} indexed, {} failed",
        stats.name, stats.links_found, stats.links_indexed, stats.links_failed
    );

    // Verify library exists
    let lib = store
        .get_library(&stats.name)
        .expect("Failed to get library")
        .expect("Library should exist");

    assert_eq!(lib.ecosystem, Ecosystem::Web);

    // Search for components
    let results = store
        .search(&stats.name, "components", 10)
        .expect("Search failed");

    println!("Search results for 'components':");
    for result in &results {
        println!(
            "  - {} ({}): {}",
            result.chunk.item_path,
            result.chunk.item_type.as_str(),
            &result.chunk.doc_text[..result.chunk.doc_text.len().min(80)]
        );
    }

    assert!(stats.links_indexed > 0, "Should have indexed some links");
    assert!(!results.is_empty(), "Should find results for 'components'");
}

/// Index Anthropic's llms.txt (if available).
#[test]
#[ignore = "requires network access"]
fn test_index_llmstxt_anthropic() {
    let store = DocStore::open_in_memory().expect("Failed to create store");

    let config = LlmsTxtIndexerConfig {
        fetch_linked_content: false,
        max_links: 50,
        ..Default::default()
    };
    let indexer = LlmsTxtIndexer::with_config(config);

    // Try to index - may fail if Anthropic doesn't have llms.txt
    match indexer.index_url(&store, "https://docs.anthropic.com/llms.txt") {
        Ok(stats) => {
            println!(
                "Indexed '{}': {} links found, {} indexed",
                stats.name, stats.links_found, stats.links_indexed
            );

            let results = store
                .search(&stats.name, "Claude API", 10)
                .expect("Search failed");

            println!("Found {} results for 'Claude API'", results.len());
        }
        Err(e) => {
            println!("Anthropic llms.txt not available or failed: {}", e);
            // Not a failure - site may not have llms.txt
        }
    }
}

// ============================================================================
// Cross-Feature Tests
// ============================================================================

/// Index multiple libraries and verify listing works.
#[test]
#[ignore = "requires network access"]
fn test_list_multiple_libraries() {
    let store = DocStore::open_in_memory().expect("Failed to create store");

    // Index a Rust crate
    let rust_config = IndexerConfig {
        keep_source: false,
        work_dir: None,
        rustdoc_flags: Vec::new(),
    };
    let rust_indexer = RustDocIndexer::with_config(rust_config);

    rust_indexer
        .index_crate(&store, "thiserror", None)
        .expect("Failed to index thiserror");

    // Index llms.txt
    let llms_config = LlmsTxtIndexerConfig {
        fetch_linked_content: false,
        max_links: 20,
        ..Default::default()
    };
    let llms_indexer = LlmsTxtIndexer::with_config(llms_config);

    llms_indexer
        .index_url(&store, "https://mintlify.com/docs/llms.txt")
        .expect("Failed to index llms.txt");

    // List all libraries
    let libraries = store.list_libraries().expect("Failed to list libraries");

    println!("Indexed libraries:");
    for lib in &libraries {
        println!(
            "  - {} v{} ({})",
            lib.library,
            lib.version,
            lib.ecosystem.as_str()
        );
    }

    assert!(libraries.len() >= 2, "Should have at least 2 libraries");

    // Filter by ecosystem
    let rust_libs: Vec<_> = libraries
        .iter()
        .filter(|l| l.ecosystem == Ecosystem::Rust)
        .collect();
    let web_libs: Vec<_> = libraries
        .iter()
        .filter(|l| l.ecosystem == Ecosystem::Web)
        .collect();

    assert_eq!(rust_libs.len(), 1, "Should have 1 Rust library");
    assert_eq!(web_libs.len(), 1, "Should have 1 Web library");
}

/// Test removing and re-indexing a library.
#[test]
#[ignore = "requires network access"]
fn test_remove_and_reindex() {
    let store = DocStore::open_in_memory().expect("Failed to create store");

    let config = IndexerConfig {
        keep_source: false,
        work_dir: None,
        rustdoc_flags: Vec::new(),
    };
    let indexer = RustDocIndexer::with_config(config);

    // Index
    indexer
        .index_crate(&store, "thiserror", None)
        .expect("Failed to index");

    assert!(
        store.get_library("thiserror").unwrap().is_some(),
        "Library should exist"
    );

    // Remove
    let deleted = store.delete_library("thiserror").expect("Failed to delete");

    assert!(deleted, "Should have deleted library");
    assert!(
        store.get_library("thiserror").unwrap().is_none(),
        "Library should not exist after deletion"
    );

    // Re-index
    indexer
        .index_crate(&store, "thiserror", None)
        .expect("Failed to re-index");

    assert!(
        store.get_library("thiserror").unwrap().is_some(),
        "Library should exist after re-indexing"
    );
}

/// Test search across empty results gracefully handles edge cases.
#[test]
#[ignore = "requires network access"]
fn test_search_edge_cases() {
    let store = DocStore::open_in_memory().expect("Failed to create store");

    let config = IndexerConfig {
        keep_source: false,
        work_dir: None,
        rustdoc_flags: Vec::new(),
    };
    let indexer = RustDocIndexer::with_config(config);

    indexer
        .index_crate(&store, "thiserror", None)
        .expect("Failed to index");

    // Search for something that won't match
    let results = store
        .search("thiserror", "xyzzy_nonexistent_term_12345", 10)
        .expect("Search should not fail");

    assert!(
        results.is_empty(),
        "Should return empty results for non-matching query"
    );

    // Search with empty query
    let results = store
        .search("thiserror", "", 10)
        .expect("Search should not fail");

    // Empty query behavior depends on implementation
    println!("Empty query returned {} results", results.len());

    // Search non-existent library
    let result = store.search("nonexistent_library", "test", 10);
    // This might return empty or error depending on implementation
    println!("Non-existent library search result: {:?}", result.is_ok());
}
