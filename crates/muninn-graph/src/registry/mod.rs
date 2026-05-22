//! Registry clients for fetching package metadata and source code.
//!
//! This module provides clients for interacting with package registries:
//! - crates.io for Rust crates
//! - PyPI for Python packages
//!
//! Documentation extraction utilities:
//! - rustdoc JSON extraction for Rust crates
//! - tree-sitter based Python docstring extraction (pure Rust, no Python required)
//! - llms.txt parsing for LLM-optimized documentation
//!
//! And indexing pipelines:
//! - RustDocIndexer for downloading, extracting, and storing Rust crate docs
//! - PyDocIndexer for downloading, extracting, and storing Python package docs
//! - LlmsTxtIndexer for fetching and storing llms.txt documentation

pub mod crates_io;
pub mod indexer;
pub mod llmstxt;
pub mod py_indexer;
pub mod pydoc;
pub mod pypi;
pub mod rustdoc;

pub use crates_io::{CrateVersion, CratesIoClient, CratesIoError};
pub use indexer::{
    IndexStats, IndexerConfig, IndexerError, RustDocIndexer, index_crate, index_local_crate,
};
pub use py_indexer::{
    PyDocIndexer, PyIndexStats, PyIndexerConfig, PyIndexerError, index_local_package, index_package,
};
pub use pydoc::{ExtractedPyItem, PyDocError, PyDocExtractor};
// Deprecated aliases for compatibility
pub use llmstxt::{
    LlmsTxt, LlmsTxtError, LlmsTxtFetcher, LlmsTxtIndexStats, LlmsTxtIndexer, LlmsTxtIndexerConfig,
    LlmsTxtIndexerError, LlmsTxtLink, LlmsTxtParser, index_llmstxt, index_llmstxt_fast,
};
#[allow(deprecated)]
pub use pydoc::{GriffeError, GriffeExtractor};
pub use pypi::{PackageInfo, PackageMetadata, PyPiClient, PyPiError, ReleaseFile};
pub use rustdoc::{
    ExtractedItem, ItemVisibility, RustdocError, RustdocExtractor, extract_docs_from_crate,
    extract_docs_from_json, items_to_chunks,
};
