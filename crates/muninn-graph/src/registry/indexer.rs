//! Rust documentation indexer for the DocStore.
//!
//! Provides a complete pipeline for indexing Rust crate documentation:
//! 1. Download crate source from crates.io
//! 2. Generate rustdoc JSON via `cargo +nightly rustdoc`
//! 3. Extract documentation items
//! 4. Store in DocStore for search
//!
//! # Example
//!
//! ```no_run
//! use muninn_graph::registry::RustDocIndexer;
//! use muninn_graph::doc_store::DocStore;
//!
//! let store = DocStore::open_in_memory()?;
//! let indexer = RustDocIndexer::new();
//!
//! // Index a crate by name (downloads latest version)
//! let stats = indexer.index_crate(&store, "once_cell", None)?;
//! println!("Indexed {} items from {}", stats.items_indexed, stats.version);
//!
//! // Search the indexed documentation
//! let results = store.search("once_cell", "lazy initialization", 10)?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use std::path::{Path, PathBuf};

use crate::doc_store::{DocStore, DocStoreError, Ecosystem};
use crate::registry::{
    crates_io::{CratesIoClient, CratesIoError},
    rustdoc::{RustdocError, RustdocExtractor, extract_docs_from_json, items_to_chunks},
};

/// Error type for indexer operations.
#[derive(Debug, thiserror::Error)]
pub enum IndexerError {
    #[error("crates.io error: {0}")]
    CratesIo(#[from] CratesIoError),

    #[error("Rustdoc error: {0}")]
    Rustdoc(#[from] RustdocError),

    #[error("Doc store error: {0}")]
    DocStore(#[from] DocStoreError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Indexing failed: {0}")]
    IndexingFailed(String),
}

pub type Result<T> = std::result::Result<T, IndexerError>;

/// Statistics from an indexing operation.
#[derive(Debug, Clone)]
pub struct IndexStats {
    /// Name of the crate indexed
    pub crate_name: String,
    /// Version that was indexed
    pub version: String,
    /// Number of documentation items extracted
    pub items_extracted: usize,
    /// Number of items actually indexed (with content)
    pub items_indexed: usize,
    /// Path where the crate was extracted (if kept)
    pub extract_path: Option<PathBuf>,
}

/// Configuration for the indexer.
#[derive(Debug, Clone, Default)]
pub struct IndexerConfig {
    /// Keep the downloaded crate source after indexing
    pub keep_source: bool,
    /// Custom working directory for downloads (uses temp dir if None)
    pub work_dir: Option<PathBuf>,
    /// Custom rustdoc flags
    pub rustdoc_flags: Vec<String>,
}

/// Rust documentation indexer.
///
/// Orchestrates the complete pipeline for downloading Rust crates,
/// extracting their documentation, and storing it in the DocStore.
pub struct RustDocIndexer {
    crates_client: CratesIoClient,
    config: IndexerConfig,
}

impl RustDocIndexer {
    /// Create a new indexer with default configuration.
    pub fn new() -> Self {
        Self {
            crates_client: CratesIoClient::new(),
            config: IndexerConfig::default(),
        }
    }

    /// Create an indexer with custom configuration.
    pub fn with_config(config: IndexerConfig) -> Self {
        Self {
            crates_client: CratesIoClient::new(),
            config,
        }
    }

    /// Index a crate by name.
    ///
    /// Downloads the specified version (or latest if None), generates rustdoc JSON,
    /// extracts documentation, and stores it in the DocStore.
    ///
    /// # Arguments
    ///
    /// * `store` - The DocStore to index into
    /// * `crate_name` - Name of the crate to index
    /// * `version` - Specific version to index (None for latest)
    ///
    /// # Returns
    ///
    /// Statistics about the indexing operation.
    pub fn index_crate(
        &self,
        store: &DocStore,
        crate_name: &str,
        version: Option<&str>,
    ) -> Result<IndexStats> {
        // Create work directory
        let work_dir = self.get_work_dir()?;

        // Download the crate
        let (crate_path, crate_version) = if let Some(v) = version {
            let path = self
                .crates_client
                .download_source(crate_name, v, &work_dir)?;
            let version_info = self.crates_client.get_version(crate_name, v)?;
            (path, version_info)
        } else {
            self.crates_client.download_latest(crate_name, &work_dir)?
        };

        // Generate rustdoc JSON
        let extractor = RustdocExtractor::new().with_flags(self.config.rustdoc_flags.clone());
        let json_path = extractor.generate_json(&crate_path)?;

        // Extract documentation items
        let items = extract_docs_from_json(&json_path)?;
        let items_extracted = items.len();

        // Convert to chunks
        let chunks = items_to_chunks(items);
        let items_indexed = chunks.len();

        // Create library entry and insert chunks
        let source_url = format!(
            "https://crates.io/crates/{}/{}",
            crate_name, crate_version.num
        );
        let library_id = store.upsert_library(
            crate_name,
            Ecosystem::Rust,
            &crate_version.num,
            Some(&source_url),
        )?;

        store.insert_chunks_batch(library_id, &chunks)?;

        // Clean up if not keeping source
        let extract_path = if self.config.keep_source {
            Some(crate_path)
        } else {
            // Clean up the downloaded crate
            if crate_path.exists() {
                let _ = std::fs::remove_dir_all(&crate_path);
            }
            None
        };

        Ok(IndexStats {
            crate_name: crate_name.to_string(),
            version: crate_version.num,
            items_extracted,
            items_indexed,
            extract_path,
        })
    }

    /// Index a crate from a local path.
    ///
    /// Useful for indexing local crates or workspace members without downloading.
    ///
    /// # Arguments
    ///
    /// * `store` - The DocStore to index into
    /// * `crate_path` - Path to the crate directory (containing Cargo.toml)
    /// * `crate_name` - Name to use for the library entry
    /// * `version` - Version string for the library entry
    pub fn index_local(
        &self,
        store: &DocStore,
        crate_path: impl AsRef<Path>,
        crate_name: &str,
        version: &str,
    ) -> Result<IndexStats> {
        let crate_path = crate_path.as_ref();

        // Generate rustdoc JSON
        let extractor = RustdocExtractor::new().with_flags(self.config.rustdoc_flags.clone());
        let json_path = extractor.generate_json(crate_path)?;

        // Extract documentation items
        let items = extract_docs_from_json(&json_path)?;
        let items_extracted = items.len();

        // Convert to chunks
        let chunks = items_to_chunks(items);
        let items_indexed = chunks.len();

        // Create library entry and insert chunks
        let library_id = store.upsert_library(crate_name, Ecosystem::Rust, version, None)?;

        store.insert_chunks_batch(library_id, &chunks)?;

        Ok(IndexStats {
            crate_name: crate_name.to_string(),
            version: version.to_string(),
            items_extracted,
            items_indexed,
            extract_path: Some(crate_path.to_path_buf()),
        })
    }

    /// Index multiple crates in batch.
    ///
    /// # Arguments
    ///
    /// * `store` - The DocStore to index into
    /// * `crates` - List of (crate_name, optional_version) tuples
    ///
    /// # Returns
    ///
    /// Vector of results, one per crate (including failures).
    pub fn index_batch(
        &self,
        store: &DocStore,
        crates: &[(&str, Option<&str>)],
    ) -> Vec<Result<IndexStats>> {
        crates
            .iter()
            .map(|(name, version)| self.index_crate(store, name, *version))
            .collect()
    }

    /// Index a pre-downloaded crate from an extracted directory.
    ///
    /// This is useful when you've already downloaded and extracted a crate
    /// and just want to index it.
    ///
    /// # Arguments
    ///
    /// * `store` - The DocStore to index into
    /// * `crate_path` - Path to the extracted crate directory
    /// * `crate_name` - Name of the crate
    /// * `version` - Version of the crate
    pub fn index_extracted(
        &self,
        store: &DocStore,
        crate_path: impl AsRef<Path>,
        crate_name: &str,
        version: &str,
    ) -> Result<IndexStats> {
        self.index_local(store, crate_path, crate_name, version)
    }

    /// Get the work directory for downloads.
    fn get_work_dir(&self) -> Result<PathBuf> {
        if let Some(ref dir) = self.config.work_dir {
            std::fs::create_dir_all(dir)?;
            Ok(dir.clone())
        } else {
            // Use system temp directory
            let temp = std::env::temp_dir().join("muninn-indexer");
            std::fs::create_dir_all(&temp)?;
            Ok(temp)
        }
    }
}

impl Default for RustDocIndexer {
    fn default() -> Self {
        Self::new()
    }
}

/// Convenience function to index a single crate.
///
/// Creates a temporary indexer and indexes the specified crate.
pub fn index_crate(
    store: &DocStore,
    crate_name: &str,
    version: Option<&str>,
) -> Result<IndexStats> {
    let indexer = RustDocIndexer::new();
    indexer.index_crate(store, crate_name, version)
}

/// Convenience function to index a local crate.
pub fn index_local_crate(
    store: &DocStore,
    crate_path: impl AsRef<Path>,
    crate_name: &str,
    version: &str,
) -> Result<IndexStats> {
    let indexer = RustDocIndexer::new();
    indexer.index_local(store, crate_path, crate_name, version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_indexer_creation() {
        let indexer = RustDocIndexer::new();
        drop(indexer);
    }

    #[test]
    fn test_indexer_with_config() {
        let config = IndexerConfig {
            keep_source: true,
            work_dir: Some(PathBuf::from("/tmp/test")),
            rustdoc_flags: vec!["--document-private-items".to_string()],
        };
        let indexer = RustDocIndexer::with_config(config);
        drop(indexer);
    }

    #[test]
    #[ignore] // Requires network, cargo, and nightly Rust
    fn test_index_crate() {
        use std::process::Command;

        // Check if nightly is available
        let nightly_check = Command::new("cargo")
            .args(["+nightly", "--version"])
            .output();

        if nightly_check.is_err() || !nightly_check.unwrap().status.success() {
            eprintln!("Skipping test: nightly Rust not available");
            return;
        }

        let store = DocStore::open_in_memory().expect("Failed to create store");
        let indexer = RustDocIndexer::new();

        // Index a small, well-documented crate
        let result = indexer.index_crate(&store, "cfg-if", None);

        match result {
            Ok(stats) => {
                println!(
                    "Indexed {} items from {} v{}",
                    stats.items_indexed, stats.crate_name, stats.version
                );
                assert_eq!(stats.crate_name, "cfg-if");
                assert!(stats.items_extracted > 0);

                // Verify we can search
                let search_results = store.search("cfg-if", "macro", 10).unwrap();
                // cfg-if may not have "macro" in its docs, so just verify search works
                println!("Search returned {} results", search_results.len());
            }
            Err(IndexerError::Rustdoc(RustdocError::CargoFailed(msg)))
                if msg.contains("nightly") =>
            {
                eprintln!("Skipping test: {}", msg);
            }
            Err(e) => panic!("Indexing failed: {}", e),
        }
    }

    #[test]
    #[ignore] // Requires network, cargo, and nightly Rust
    fn test_index_crate_specific_version() {
        use std::process::Command;

        // Check if nightly is available
        let nightly_check = Command::new("cargo")
            .args(["+nightly", "--version"])
            .output();

        if nightly_check.is_err() || !nightly_check.unwrap().status.success() {
            eprintln!("Skipping test: nightly Rust not available");
            return;
        }

        let store = DocStore::open_in_memory().expect("Failed to create store");
        let indexer = RustDocIndexer::new();

        // Index a specific version
        let result = indexer.index_crate(&store, "once_cell", Some("1.19.0"));

        match result {
            Ok(stats) => {
                assert_eq!(stats.crate_name, "once_cell");
                assert_eq!(stats.version, "1.19.0");
                assert!(stats.items_indexed > 0);
                println!("Indexed {} items", stats.items_indexed);

                // Verify library was created
                let lib = store.get_library("once_cell").unwrap();
                assert!(lib.is_some());
                assert_eq!(lib.unwrap().version, "1.19.0");

                // Verify we can search
                let results = store.search("once_cell", "lazy", 10).unwrap();
                assert!(!results.is_empty(), "Should find results for 'lazy'");
            }
            Err(IndexerError::Rustdoc(RustdocError::CargoFailed(msg)))
                if msg.contains("nightly") =>
            {
                eprintln!("Skipping test: {}", msg);
            }
            Err(e) => panic!("Indexing failed: {}", e),
        }
    }

    #[test]
    #[ignore] // Requires network, cargo, and nightly Rust
    fn test_index_batch() {
        use std::process::Command;

        // Check if nightly is available
        let nightly_check = Command::new("cargo")
            .args(["+nightly", "--version"])
            .output();

        if nightly_check.is_err() || !nightly_check.unwrap().status.success() {
            eprintln!("Skipping test: nightly Rust not available");
            return;
        }

        let store = DocStore::open_in_memory().expect("Failed to create store");
        let indexer = RustDocIndexer::new();

        let crates = vec![("cfg-if", None), ("once_cell", Some("1.19.0"))];

        let results = indexer.index_batch(&store, &crates);

        let successful: Vec<_> = results.iter().filter(|r| r.is_ok()).collect();
        println!(
            "{} of {} crates indexed successfully",
            successful.len(),
            crates.len()
        );

        // At least one should succeed (or be skipped due to nightly)
        let libraries = store.list_libraries().unwrap();
        println!(
            "Libraries in store: {:?}",
            libraries.iter().map(|l| &l.library).collect::<Vec<_>>()
        );
    }
}
