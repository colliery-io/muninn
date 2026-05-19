//! Python documentation indexer for the DocStore.
//!
//! Provides a complete pipeline for indexing Python package documentation:
//! 1. Download package source from PyPI
//! 2. Extract documentation using tree-sitter (pure Rust, no Python required)
//! 3. Store in DocStore for search
//!
//! # Example
//!
//! ```no_run
//! use muninn_graph::registry::PyDocIndexer;
//! use muninn_graph::doc_store::DocStore;
//!
//! let store = DocStore::open_in_memory()?;
//! let indexer = PyDocIndexer::new();
//!
//! // Index a package by name (downloads latest version)
//! let stats = indexer.index_package(&store, "requests", None)?;
//! println!("Indexed {} items from {}", stats.items_indexed, stats.version);
//!
//! // Search the indexed documentation
//! let results = store.search("requests", "HTTP request", 10)?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use std::path::{Path, PathBuf};

use crate::doc_store::{DocStore, DocStoreError, Ecosystem};
use crate::registry::{
    pydoc::{PyDocError, PyDocExtractor, items_to_chunks},
    pypi::{PyPiClient, PyPiError},
};

/// Error type for Python indexer operations.
#[derive(Debug, thiserror::Error)]
pub enum PyIndexerError {
    #[error("PyPI error: {0}")]
    PyPi(#[from] PyPiError),

    #[error("Extraction error: {0}")]
    Extraction(#[from] PyDocError),

    #[error("Doc store error: {0}")]
    DocStore(#[from] DocStoreError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Indexing failed: {0}")]
    IndexingFailed(String),
}

pub type Result<T> = std::result::Result<T, PyIndexerError>;

/// Statistics from a Python indexing operation.
#[derive(Debug, Clone)]
pub struct PyIndexStats {
    /// Name of the package indexed
    pub package_name: String,
    /// Version that was indexed
    pub version: String,
    /// Number of documentation items extracted
    pub items_extracted: usize,
    /// Number of items actually indexed (with content)
    pub items_indexed: usize,
    /// Path where the package was extracted (if kept)
    pub extract_path: Option<PathBuf>,
}

/// Configuration for the Python indexer.
#[derive(Debug, Clone)]
pub struct PyIndexerConfig {
    /// Keep the downloaded package source after indexing
    pub keep_source: bool,
    /// Custom working directory for downloads (uses temp dir if None)
    pub work_dir: Option<PathBuf>,
    /// Python executable (deprecated, ignored - kept for API compatibility)
    #[deprecated(note = "No longer needed - tree-sitter is used for extraction")]
    pub python: String,
    /// Griffe flags (deprecated, ignored - kept for API compatibility)
    #[deprecated(note = "No longer needed - tree-sitter is used for extraction")]
    pub griffe_flags: Vec<String>,
}

impl Default for PyIndexerConfig {
    fn default() -> Self {
        #[allow(deprecated)]
        Self {
            keep_source: false,
            work_dir: None,
            python: "python3".to_string(),
            griffe_flags: Vec::new(),
        }
    }
}

/// Python documentation indexer.
///
/// Orchestrates the complete pipeline for downloading Python packages,
/// extracting their documentation with tree-sitter, and storing it in the DocStore.
///
/// Note: This indexer uses tree-sitter for Python parsing, requiring no external
/// Python runtime or dependencies.
pub struct PyDocIndexer {
    pypi_client: PyPiClient,
    config: PyIndexerConfig,
}

impl PyDocIndexer {
    /// Create a new indexer with default configuration.
    pub fn new() -> Self {
        Self {
            pypi_client: PyPiClient::new(),
            config: PyIndexerConfig::default(),
        }
    }

    /// Create an indexer with custom configuration.
    pub fn with_config(config: PyIndexerConfig) -> Self {
        Self {
            pypi_client: PyPiClient::new(),
            config,
        }
    }

    /// Index a Python package by name.
    ///
    /// Downloads the specified version (or latest if None), extracts documentation
    /// with tree-sitter, and stores it in the DocStore.
    ///
    /// # Arguments
    ///
    /// * `store` - The DocStore to index into
    /// * `package_name` - Name of the package to index
    /// * `version` - Specific version to index (None for latest)
    ///
    /// # Returns
    ///
    /// Statistics about the indexing operation.
    pub fn index_package(
        &self,
        store: &DocStore,
        package_name: &str,
        version: Option<&str>,
    ) -> Result<PyIndexStats> {
        // Create work directory
        let work_dir = self.get_work_dir()?;

        // Download the package
        let (package_path, pkg_version) =
            self.pypi_client
                .download_source(package_name, version, &work_dir)?;

        // Extract documentation with tree-sitter
        let mut extractor = PyDocExtractor::new();
        let items = extractor.extract_from_path(&package_path)?;
        let items_extracted = items.len();

        // Convert to chunks
        let chunks = items_to_chunks(items);
        let items_indexed = chunks.len();

        // Create library entry and insert chunks
        let source_url = format!("https://pypi.org/project/{}/{}/", package_name, pkg_version);
        let library_id = store.upsert_library(
            package_name,
            Ecosystem::Python,
            &pkg_version,
            Some(&source_url),
        )?;

        store.insert_chunks_batch(library_id, &chunks)?;

        // Clean up if not keeping source
        let extract_path = if self.config.keep_source {
            Some(package_path)
        } else {
            // Clean up the downloaded package
            if package_path.exists() {
                let _ = std::fs::remove_dir_all(&package_path);
            }
            None
        };

        Ok(PyIndexStats {
            package_name: package_name.to_string(),
            version: pkg_version,
            items_extracted,
            items_indexed,
            extract_path,
        })
    }

    /// Index a package from a local path.
    ///
    /// Useful for indexing local packages without downloading.
    ///
    /// # Arguments
    ///
    /// * `store` - The DocStore to index into
    /// * `package_path` - Path to the package directory
    /// * `package_name` - Name to use for the library entry
    /// * `version` - Version string for the library entry
    pub fn index_local(
        &self,
        store: &DocStore,
        package_path: impl AsRef<Path>,
        package_name: &str,
        version: &str,
    ) -> Result<PyIndexStats> {
        let package_path = package_path.as_ref();

        // Extract documentation with tree-sitter
        let mut extractor = PyDocExtractor::new();
        let items = extractor.extract_from_path(package_path)?;
        let items_extracted = items.len();

        // Convert to chunks
        let chunks = items_to_chunks(items);
        let items_indexed = chunks.len();

        // Create library entry and insert chunks
        let library_id = store.upsert_library(package_name, Ecosystem::Python, version, None)?;

        store.insert_chunks_batch(library_id, &chunks)?;

        Ok(PyIndexStats {
            package_name: package_name.to_string(),
            version: version.to_string(),
            items_extracted,
            items_indexed,
            extract_path: Some(package_path.to_path_buf()),
        })
    }

    /// Index multiple packages in batch.
    ///
    /// # Arguments
    ///
    /// * `store` - The DocStore to index into
    /// * `packages` - List of (package_name, optional_version) tuples
    ///
    /// # Returns
    ///
    /// Vector of results, one per package (including failures).
    pub fn index_batch(
        &self,
        store: &DocStore,
        packages: &[(&str, Option<&str>)],
    ) -> Vec<Result<PyIndexStats>> {
        packages
            .iter()
            .map(|(name, version)| self.index_package(store, name, *version))
            .collect()
    }

    /// Get the work directory for downloads.
    fn get_work_dir(&self) -> Result<PathBuf> {
        if let Some(ref dir) = self.config.work_dir {
            std::fs::create_dir_all(dir)?;
            Ok(dir.clone())
        } else {
            // Use system temp directory
            let temp = std::env::temp_dir().join("muninn-py-indexer");
            std::fs::create_dir_all(&temp)?;
            Ok(temp)
        }
    }
}

impl Default for PyDocIndexer {
    fn default() -> Self {
        Self::new()
    }
}

/// Convenience function to index a single Python package.
///
/// Creates a temporary indexer and indexes the specified package.
pub fn index_package(
    store: &DocStore,
    package_name: &str,
    version: Option<&str>,
) -> Result<PyIndexStats> {
    let indexer = PyDocIndexer::new();
    indexer.index_package(store, package_name, version)
}

/// Convenience function to index a local Python package.
pub fn index_local_package(
    store: &DocStore,
    package_path: impl AsRef<Path>,
    package_name: &str,
    version: &str,
) -> Result<PyIndexStats> {
    let indexer = PyDocIndexer::new();
    indexer.index_local(store, package_path, package_name, version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_indexer_creation() {
        let indexer = PyDocIndexer::new();
        drop(indexer);
    }

    #[test]
    #[allow(deprecated)]
    fn test_indexer_with_config() {
        let config = PyIndexerConfig {
            keep_source: true,
            work_dir: Some(PathBuf::from("/tmp/test")),
            python: "python".to_string(),
            griffe_flags: vec!["--resolve-aliases".to_string()],
        };
        let indexer = PyDocIndexer::with_config(config);
        drop(indexer);
    }

    #[test]
    #[ignore = "requires network access to PyPI"]
    fn test_index_package() {
        let store = DocStore::open_in_memory().expect("Failed to create store");
        let indexer = PyDocIndexer::new();

        // Index a small, well-documented package
        // Using 'six' because it's tiny and commonly available
        let result = indexer.index_package(&store, "six", Some("1.16.0"));

        match result {
            Ok(stats) => {
                println!(
                    "Indexed {} items from {} v{}",
                    stats.items_indexed, stats.package_name, stats.version
                );
                assert_eq!(stats.package_name, "six");
                assert_eq!(stats.version, "1.16.0");

                // Verify library was created
                let lib = store.get_library("six").unwrap();
                assert!(lib.is_some());
                assert_eq!(lib.unwrap().ecosystem, Ecosystem::Python);
            }
            Err(e) => panic!("Indexing failed: {}", e),
        }
    }

    #[test]
    #[ignore = "requires network access to PyPI"]
    fn test_index_batch() {
        let store = DocStore::open_in_memory().expect("Failed to create store");
        let indexer = PyDocIndexer::new();

        let packages = vec![("six", Some("1.16.0")), ("typing-extensions", None)];

        let results = indexer.index_batch(&store, &packages);

        let successful: Vec<_> = results.iter().filter(|r| r.is_ok()).collect();
        println!(
            "{} of {} packages indexed successfully",
            successful.len(),
            packages.len()
        );

        let libraries = store.list_libraries().unwrap();
        println!(
            "Libraries in store: {:?}",
            libraries.iter().map(|l| &l.library).collect::<Vec<_>>()
        );
    }
}
