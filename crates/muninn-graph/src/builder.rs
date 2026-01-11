//! Graph builder for constructing code graphs from source files.
//!
//! This module coordinates parsing, symbol extraction, and graph storage
//! to build a queryable code graph from source files.

use std::path::Path;
use std::time::Instant;

use crate::edges::{Edge, EdgeKind};
use crate::lang::python::PythonExtractor;
use crate::lang::rust::RustExtractor;
use crate::parser::{Language, ParseError, Parser};
use crate::store::{GraphStore, StoreError};
use crate::symbols::Symbol;

/// Error type for graph building operations.
#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    #[error("Parse error: {0}")]
    Parse(#[from] ParseError),
    #[error("Store error: {0}")]
    Store(#[from] StoreError),
    #[error("Extraction error: {0}")]
    Extraction(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Unsupported language: {0:?}")]
    UnsupportedLanguage(Language),
}

pub type Result<T> = std::result::Result<T, BuildError>;

/// Statistics from a build operation.
#[derive(Debug, Clone, Default)]
pub struct BuildStats {
    /// Number of nodes (symbols) added to the graph.
    pub nodes_added: usize,
    /// Number of edges (relationships) added to the graph.
    pub edges_added: usize,
    /// Time spent parsing in milliseconds.
    pub parse_time_ms: u64,
    /// Time spent storing in milliseconds.
    pub store_time_ms: u64,
    /// Number of files processed.
    pub files_processed: usize,
}

impl BuildStats {
    /// Merge another BuildStats into this one.
    pub fn merge(&mut self, other: &BuildStats) {
        self.nodes_added += other.nodes_added;
        self.edges_added += other.edges_added;
        self.parse_time_ms += other.parse_time_ms;
        self.store_time_ms += other.store_time_ms;
        self.files_processed += other.files_processed;
    }
}

/// Coordinates parsing and graph construction.
///
/// The GraphBuilder takes source files, parses them, extracts symbols
/// and relationships, and stores everything in the graph database.
pub struct GraphBuilder {
    parser: Parser,
    store: GraphStore,
}

impl GraphBuilder {
    /// Create a new GraphBuilder with the given store.
    pub fn new(store: GraphStore) -> Self {
        Self {
            parser: Parser::new(),
            store,
        }
    }

    /// Parse and add a single file to the graph.
    ///
    /// Extracts symbols and relationships from the file and stores them.
    /// Does not remove existing data - use `rebuild_file` for that.
    pub fn build_file(&mut self, path: &Path) -> Result<BuildStats> {
        let mut stats = BuildStats::default();

        // Parse the file
        let parse_start = Instant::now();
        let parsed = self.parser.parse_file(path)?;
        stats.parse_time_ms = parse_start.elapsed().as_millis() as u64;

        // Get the relative file path for storage
        let file_path = path.to_string_lossy().to_string();

        // Extract symbols based on language
        let store_start = Instant::now();

        match parsed.language {
            Language::Rust => {
                self.build_rust_file(&parsed.tree, &parsed.source, &file_path, &mut stats)?;
            }
            Language::Python => {
                self.build_python_file(&parsed.tree, &parsed.source, &file_path, &mut stats)?;
            }
            Language::C | Language::Cpp => {
                // TODO: Implement extractors for C/C++
                return Err(BuildError::UnsupportedLanguage(parsed.language));
            }
        }

        stats.store_time_ms = store_start.elapsed().as_millis() as u64;
        stats.files_processed = 1;

        Ok(stats)
    }

    /// Remove old data and rebuild a file.
    ///
    /// First deletes all nodes and edges from the previous version of this file,
    /// then rebuilds it fresh.
    pub fn rebuild_file(&mut self, path: &Path) -> Result<BuildStats> {
        let file_path = path.to_string_lossy().to_string();

        // Delete existing data for this file
        // Note: This relies on graphqlite's delete_file working correctly
        let _ = self.store.delete_file(&file_path);

        // Rebuild the file
        self.build_file(path)
    }

    /// Build all supported files in a directory recursively.
    pub fn build_directory(&mut self, path: &Path) -> Result<BuildStats> {
        let mut stats = BuildStats::default();

        self.build_directory_recursive(path, &mut stats)?;

        Ok(stats)
    }

    /// Recursive helper for build_directory.
    fn build_directory_recursive(&mut self, path: &Path, stats: &mut BuildStats) -> Result<()> {
        if !path.is_dir() {
            return Ok(());
        }

        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            let entry_path = entry.path();

            if entry_path.is_dir() {
                // Skip hidden directories and common non-source directories
                let name = entry_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("");
                if name.starts_with('.') || name == "target" || name == "node_modules" {
                    continue;
                }
                self.build_directory_recursive(&entry_path, stats)?;
            } else if self.is_supported_file(&entry_path) {
                match self.build_file(&entry_path) {
                    Ok(file_stats) => stats.merge(&file_stats),
                    Err(BuildError::UnsupportedLanguage(_)) => {
                        // Skip unsupported languages silently
                    }
                    Err(e) => return Err(e),
                }
            }
        }

        Ok(())
    }

    /// Check if a file has a supported extension.
    fn is_supported_file(&self, path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|ext| matches!(ext, "rs" | "py" | "c" | "cpp" | "h" | "hpp"))
    }

    /// Build graph data for a Rust file.
    fn build_rust_file(
        &self,
        tree: &tree_sitter::Tree,
        source: &str,
        file_path: &str,
        stats: &mut BuildStats,
    ) -> Result<()> {
        // Extract symbols
        let symbols = RustExtractor::extract_symbols(tree, source, file_path)
            .map_err(|e| BuildError::Extraction(e.to_string()))?;

        // Extract imports
        let imports = RustExtractor::extract_imports(tree, source)
            .map_err(|e| BuildError::Extraction(e.to_string()))?;

        // Extract calls
        let calls = RustExtractor::extract_calls(tree, source)
            .map_err(|e| BuildError::Extraction(e.to_string()))?;

        // Extract trait implementations
        let impls = RustExtractor::extract_implementations(tree, source)
            .map_err(|e| BuildError::Extraction(e.to_string()))?;

        // Create a file node for CONTAINS edges
        let file_symbol = Symbol::new(
            file_path.rsplit('/').next().unwrap_or(file_path),
            crate::symbols::SymbolKind::File,
            file_path,
            1,
            1,
        );
        let file_node_id = file_symbol.id();

        // Batch insert all nodes (symbols + file) for optimal performance
        // Pre-allocate to avoid reallocation, take ownership to avoid clone
        let symbol_count = symbols.len();
        let mut all_symbols = Vec::with_capacity(symbol_count + 1);
        all_symbols.extend(symbols);
        all_symbols.push(file_symbol);
        let id_map = self.store.insert_nodes_batch(&all_symbols)?;
        stats.nodes_added += all_symbols.len();

        // Slice to get just the code symbols (excluding file node at end)
        let symbols = &all_symbols[..symbol_count];

        // Create CONTAINS edges from file to top-level symbols
        let mut edges = Vec::new();

        for symbol in symbols {
            // Only create CONTAINS for top-level symbols (rough heuristic: no :: in name)
            if !symbol.name.contains("::") {
                edges.push(Edge {
                    source_id: file_node_id.clone(),
                    target_id: symbol.id(),
                    kind: EdgeKind::Contains,
                });
            }
        }

        // Create IMPORTS edges
        for import in &imports {
            // Create an edge from the file to a synthetic import target
            // In a full implementation, we'd resolve this to the actual target node
            let import_target_id = format!("import__{}", import.path.replace("::", "__"));
            edges.push(Edge {
                source_id: file_node_id.clone(),
                target_id: import_target_id,
                kind: EdgeKind::Imports {
                    path: import.path.clone(),
                    alias: import.alias.clone(),
                },
            });
        }

        // Create CALLS edges
        // Build a map of function names to their node IDs for resolution
        let symbol_map: std::collections::HashMap<&str, &Symbol> = symbols
            .iter()
            .filter(|s| s.kind.is_callable())
            .map(|s| (s.name.as_str(), s))
            .collect();

        for call in &calls {
            // Try to find the caller (the function containing this call)
            let caller = symbols.iter().find(|s| {
                s.kind.is_callable() && s.start_line <= call.line && call.line <= s.end_line
            });

            if let Some(caller_symbol) = caller {
                // Try to resolve the callee
                let target_id = if let Some(target_symbol) = symbol_map.get(call.callee.as_str()) {
                    target_symbol.id()
                } else {
                    // Unresolved call - use a placeholder ID
                    format!("unresolved__{}", call.callee.replace("::", "__"))
                };

                // Determine call type based on is_method flag
                let call_type = if call.is_method {
                    crate::edges::CallType::Method
                } else {
                    crate::edges::CallType::Direct
                };

                edges.push(Edge {
                    source_id: caller_symbol.id(),
                    target_id,
                    kind: EdgeKind::Calls {
                        call_type,
                        line: call.line,
                    },
                });
            }
        }

        // Create IMPLEMENTS edges for trait implementations
        for impl_info in &impls {
            // Find the implementing type in symbols
            let impl_type = symbols.iter().find(|s| {
                s.name == impl_info.type_name
                    && matches!(
                        s.kind,
                        crate::symbols::SymbolKind::Struct | crate::symbols::SymbolKind::Enum
                    )
            });

            // Find the trait in symbols
            let trait_sym = symbols.iter().find(|s| {
                s.name == impl_info.trait_name && s.kind == crate::symbols::SymbolKind::Interface
            });

            if let Some(impl_type) = impl_type {
                // If trait is in this file, use its ID; otherwise use unresolved
                let trait_id = if let Some(t) = trait_sym {
                    t.id()
                } else {
                    format!("trait__{}", impl_info.trait_name)
                };

                edges.push(Edge {
                    source_id: impl_type.id(),
                    target_id: trait_id,
                    kind: EdgeKind::Implements,
                });
            }
        }

        // Partition edges: bulk insert those with both endpoints in id_map,
        // slow insert those with unresolved targets
        if !edges.is_empty() {
            let (bulk_edges, slow_edges): (Vec<_>, Vec<_>) = edges.into_iter().partition(|e| {
                id_map.contains_key(&e.source_id) && id_map.contains_key(&e.target_id)
            });

            if !bulk_edges.is_empty() {
                let inserted = self.store.insert_edges_batch(&bulk_edges, &id_map)?;
                stats.edges_added += inserted;
            }

            if !slow_edges.is_empty() {
                self.store.insert_edges_batch_slow(&slow_edges)?;
                stats.edges_added += slow_edges.len();
            }
        }

        Ok(())
    }

    /// Build graph data for a Python file.
    fn build_python_file(
        &self,
        tree: &tree_sitter::Tree,
        source: &str,
        file_path: &str,
        stats: &mut BuildStats,
    ) -> Result<()> {
        // Extract symbols
        let symbols = PythonExtractor::extract_symbols(tree, source, file_path)
            .map_err(BuildError::Extraction)?;

        // Extract imports
        let imports =
            PythonExtractor::extract_imports(tree, source).map_err(BuildError::Extraction)?;

        // Extract calls
        let calls = PythonExtractor::extract_calls(tree, source).map_err(BuildError::Extraction)?;

        // Create a file node for CONTAINS edges
        let file_symbol = Symbol::new(
            file_path.rsplit('/').next().unwrap_or(file_path),
            crate::symbols::SymbolKind::File,
            file_path,
            1,
            1,
        );
        let file_node_id = file_symbol.id();

        // Batch insert all nodes (symbols + file) for optimal performance
        // Pre-allocate to avoid reallocation, take ownership to avoid clone
        let symbol_count = symbols.len();
        let mut all_symbols = Vec::with_capacity(symbol_count + 1);
        all_symbols.extend(symbols);
        all_symbols.push(file_symbol);
        let id_map = self.store.insert_nodes_batch(&all_symbols)?;
        stats.nodes_added += all_symbols.len();

        // Slice to get just the code symbols (excluding file node at end)
        let symbols = &all_symbols[..symbol_count];

        // Create CONTAINS edges from file to top-level symbols
        let mut edges = Vec::new();

        for symbol in symbols {
            // Top-level symbols in Python (classes and functions not nested)
            if symbol.kind == crate::symbols::SymbolKind::Class
                || symbol.kind == crate::symbols::SymbolKind::Function
            {
                edges.push(Edge {
                    source_id: file_node_id.clone(),
                    target_id: symbol.id(),
                    kind: EdgeKind::Contains,
                });
            }
        }

        // Create IMPORTS edges
        for import in &imports {
            let import_path = if let Some(ref name) = import.name {
                format!("{}.{}", import.module, name)
            } else {
                import.module.clone()
            };
            let import_target_id = format!("import__{}", import_path.replace('.', "__"));
            edges.push(Edge {
                source_id: file_node_id.clone(),
                target_id: import_target_id,
                kind: EdgeKind::Imports {
                    path: import_path,
                    alias: import.alias.clone(),
                },
            });
        }

        // Create CALLS edges
        let symbol_map: std::collections::HashMap<&str, &Symbol> = symbols
            .iter()
            .filter(|s| s.kind.is_callable())
            .map(|s| (s.name.as_str(), s))
            .collect();

        for call in &calls {
            // Try to find the caller
            let caller = symbols.iter().find(|s| {
                s.kind.is_callable() && s.start_line <= call.line && call.line <= s.end_line
            });

            if let Some(caller_symbol) = caller {
                let target_id = if let Some(target_symbol) = symbol_map.get(call.callee.as_str()) {
                    target_symbol.id()
                } else {
                    format!("unresolved__{}", call.callee.replace('.', "__"))
                };

                let call_type = if call.is_method {
                    crate::edges::CallType::Method
                } else {
                    crate::edges::CallType::Direct
                };

                edges.push(Edge {
                    source_id: caller_symbol.id(),
                    target_id,
                    kind: EdgeKind::Calls {
                        call_type,
                        line: call.line,
                    },
                });
            }
        }

        // Partition edges: bulk insert those with both endpoints in id_map,
        // slow insert those with unresolved targets
        if !edges.is_empty() {
            let (bulk_edges, slow_edges): (Vec<_>, Vec<_>) = edges.into_iter().partition(|e| {
                id_map.contains_key(&e.source_id) && id_map.contains_key(&e.target_id)
            });

            if !bulk_edges.is_empty() {
                let inserted = self.store.insert_edges_batch(&bulk_edges, &id_map)?;
                stats.edges_added += inserted;
            }

            if !slow_edges.is_empty() {
                self.store.insert_edges_batch_slow(&slow_edges)?;
                stats.edges_added += slow_edges.len();
            }
        }

        Ok(())
    }

    /// Get a reference to the underlying store.
    pub fn store(&self) -> &GraphStore {
        &self.store
    }

    /// Get a mutable reference to the underlying store.
    pub fn store_mut(&mut self) -> &mut GraphStore {
        &mut self.store
    }

    /// Consume the builder and return the store.
    pub fn into_store(self) -> GraphStore {
        self.store
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::fs;
    use tempfile::tempdir;

    fn create_test_rust_file(dir: &Path, name: &str, content: &str) -> std::path::PathBuf {
        let file_path = dir.join(name);
        fs::write(&file_path, content).expect("Failed to write test file");
        file_path
    }

    #[test]
    #[serial]
    fn test_build_simple_rust_file() {
        let store = GraphStore::open_in_memory().expect("Failed to open store");
        let mut builder = GraphBuilder::new(store);

        let temp_dir = tempdir().expect("Failed to create temp dir");
        let file_path = create_test_rust_file(
            temp_dir.path(),
            "test.rs",
            r#"
pub struct Foo {
    value: i32,
}

impl Foo {
    pub fn new(value: i32) -> Self {
        Self { value }
    }

    pub fn get_value(&self) -> i32 {
        self.value
    }
}

pub fn helper() -> Foo {
    Foo::new(42)
}
"#,
        );

        let stats = builder
            .build_file(&file_path)
            .expect("Build should succeed");

        // Should have extracted several symbols
        assert!(
            stats.nodes_added >= 3,
            "Should have at least 3 nodes (struct, impl methods, function)"
        );
        assert!(stats.files_processed == 1, "Should process 1 file");

        // Check the store has data
        let store_stats = builder.store().stats().expect("Should get stats");
        assert!(
            store_stats.node_count >= 3,
            "Store should have at least 3 nodes"
        );
    }

    #[test]
    #[serial]
    fn test_build_directory() {
        let store = GraphStore::open_in_memory().expect("Failed to open store");
        let mut builder = GraphBuilder::new(store);

        let temp_dir = tempdir().expect("Failed to create temp dir");

        // Create a few Rust files
        create_test_rust_file(
            temp_dir.path(),
            "lib.rs",
            "pub mod utils;\npub fn main_func() {}",
        );
        create_test_rust_file(temp_dir.path(), "utils.rs", "pub fn helper() -> i32 { 42 }");

        let stats = builder
            .build_directory(temp_dir.path())
            .expect("Build directory should succeed");

        assert_eq!(stats.files_processed, 2, "Should process 2 files");
        assert!(stats.nodes_added >= 2, "Should have at least 2 nodes");
    }

    #[test]
    fn test_builder_stats() {
        let mut stats1 = BuildStats {
            nodes_added: 5,
            edges_added: 3,
            parse_time_ms: 10,
            store_time_ms: 20,
            files_processed: 1,
        };

        let stats2 = BuildStats {
            nodes_added: 3,
            edges_added: 2,
            parse_time_ms: 5,
            store_time_ms: 15,
            files_processed: 1,
        };

        stats1.merge(&stats2);

        assert_eq!(stats1.nodes_added, 8);
        assert_eq!(stats1.edges_added, 5);
        assert_eq!(stats1.parse_time_ms, 15);
        assert_eq!(stats1.store_time_ms, 35);
        assert_eq!(stats1.files_processed, 2);
    }

    #[test]
    #[serial]
    fn test_skip_hidden_directories() {
        let store = GraphStore::open_in_memory().expect("Failed to open store");
        let mut builder = GraphBuilder::new(store);

        let temp_dir = tempdir().expect("Failed to create temp dir");

        // Create a hidden directory with a Rust file
        let hidden_dir = temp_dir.path().join(".hidden");
        fs::create_dir(&hidden_dir).expect("Failed to create hidden dir");
        create_test_rust_file(&hidden_dir, "hidden.rs", "pub fn hidden() {}");

        // Create a normal Rust file
        create_test_rust_file(temp_dir.path(), "visible.rs", "pub fn visible() {}");

        let stats = builder
            .build_directory(temp_dir.path())
            .expect("Build directory should succeed");

        // Should only process the visible file
        assert_eq!(stats.files_processed, 1, "Should only process visible file");
    }
}
