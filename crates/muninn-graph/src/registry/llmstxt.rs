//! llms.txt parser and fetcher for LLM-friendly documentation.
//!
//! Implements the llms.txt standard for fetching and parsing
//! LLM-optimized documentation from websites.
//!
//! # llms.txt Format
//!
//! An llms.txt file is a markdown file with:
//! - H1 header: Project name (required)
//! - Blockquote: Project summary (optional)
//! - H2 sections: Categories of documentation links
//! - Markdown links: `[title](url): description`
//!
//! # Example
//!
//! ```text
//! # My Project
//!
//! > A brief summary of the project.
//!
//! ## Guides
//!
//! - [Getting Started](https://example.com/start.md): Introduction to the project.
//! - [API Reference](https://example.com/api.md): Full API documentation.
//! ```
//!
//! # Usage
//!
//! ```no_run
//! use muninn_graph::registry::llmstxt::{LlmsTxtFetcher, LlmsTxtParser};
//!
//! // Parse from string
//! let content = "# Project\n\n## Docs\n\n- [Guide](https://example.com/guide.md): A guide";
//! let parsed = LlmsTxtParser::parse(content)?;
//! println!("Project: {}", parsed.name);
//!
//! // Fetch from URL
//! let fetcher = LlmsTxtFetcher::new();
//! let parsed = fetcher.fetch("https://docs.example.com/llms.txt")?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use std::collections::HashMap;

/// Error type for llms.txt operations.
#[derive(Debug, thiserror::Error)]
pub enum LlmsTxtError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Parse error: {0}")]
    Parse(String),

    #[error("Missing required H1 header")]
    MissingHeader,

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, LlmsTxtError>;

/// A parsed llms.txt file.
#[derive(Debug, Clone)]
pub struct LlmsTxt {
    /// Project/site name from H1 header.
    pub name: String,
    /// Optional summary from blockquote.
    pub summary: Option<String>,
    /// Sections of documentation links, keyed by section name.
    pub sections: HashMap<String, Vec<LlmsTxtLink>>,
    /// All links in order of appearance.
    pub links: Vec<LlmsTxtLink>,
}

/// A documentation link from llms.txt.
#[derive(Debug, Clone)]
pub struct LlmsTxtLink {
    /// Link title/name.
    pub title: String,
    /// URL to the documentation.
    pub url: String,
    /// Optional description after the colon.
    pub description: Option<String>,
    /// Section this link belongs to.
    pub section: String,
}

/// Parser for llms.txt content.
pub struct LlmsTxtParser;

impl LlmsTxtParser {
    /// Parse llms.txt content from a string.
    pub fn parse(content: &str) -> Result<LlmsTxt> {
        let mut name: Option<String> = None;
        let mut summary: Option<String> = None;
        let mut sections: HashMap<String, Vec<LlmsTxtLink>> = HashMap::new();
        let mut links: Vec<LlmsTxtLink> = Vec::new();
        let mut current_section = String::from("General");
        let mut in_blockquote = false;
        let mut blockquote_lines: Vec<String> = Vec::new();

        for line in content.lines() {
            let trimmed = line.trim();

            // Skip empty lines
            if trimmed.is_empty() {
                if in_blockquote && !blockquote_lines.is_empty() {
                    summary = Some(blockquote_lines.join(" "));
                    blockquote_lines.clear();
                    in_blockquote = false;
                }
                continue;
            }

            // H1 header - project name
            if trimmed.starts_with("# ") && !trimmed.starts_with("## ") {
                name = Some(trimmed[2..].trim().to_string());
                continue;
            }

            // H2 header - section name
            if trimmed.starts_with("## ") {
                if in_blockquote && !blockquote_lines.is_empty() {
                    summary = Some(blockquote_lines.join(" "));
                    blockquote_lines.clear();
                    in_blockquote = false;
                }
                current_section = trimmed[3..].trim().to_string();
                if !sections.contains_key(&current_section) {
                    sections.insert(current_section.clone(), Vec::new());
                }
                continue;
            }

            // Blockquote - summary
            if trimmed.starts_with("> ") {
                in_blockquote = true;
                blockquote_lines.push(trimmed[2..].trim().to_string());
                continue;
            }

            // Continuation of blockquote
            if in_blockquote && !trimmed.starts_with("- ") && !trimmed.starts_with("* ") {
                blockquote_lines.push(trimmed.to_string());
                continue;
            }

            // End blockquote if we hit something else
            if in_blockquote && !blockquote_lines.is_empty() {
                summary = Some(blockquote_lines.join(" "));
                blockquote_lines.clear();
                in_blockquote = false;
            }

            // List item with link: - [title](url): description
            if (trimmed.starts_with("- ") || trimmed.starts_with("* ")) && trimmed.contains("](") {
                if let Some(link) = Self::parse_link(trimmed, &current_section) {
                    links.push(link.clone());
                    sections
                        .entry(current_section.clone())
                        .or_default()
                        .push(link);
                }
            }
        }

        // Handle trailing blockquote
        if in_blockquote && !blockquote_lines.is_empty() {
            summary = Some(blockquote_lines.join(" "));
        }

        let name = name.ok_or(LlmsTxtError::MissingHeader)?;

        Ok(LlmsTxt {
            name,
            summary,
            sections,
            links,
        })
    }

    /// Parse a single link line.
    fn parse_link(line: &str, section: &str) -> Option<LlmsTxtLink> {
        // Remove list marker
        let line = line
            .trim_start_matches("- ")
            .trim_start_matches("* ")
            .trim();

        // Find markdown link: [title](url)
        let open_bracket = line.find('[')?;
        let close_bracket = line.find(']')?;
        let open_paren = line.find("](")? + 1;
        let close_paren = line[open_paren..].find(')')? + open_paren;

        if close_bracket + 1 != open_paren {
            return None;
        }

        let title = line[open_bracket + 1..close_bracket].trim().to_string();
        let url = line[open_paren + 1..close_paren].trim().to_string();

        // Check for description after colon
        let description = if close_paren + 1 < line.len() {
            let rest = line[close_paren + 1..].trim();
            if rest.starts_with(':') {
                Some(rest[1..].trim().to_string())
            } else {
                None
            }
        } else {
            None
        };

        Some(LlmsTxtLink {
            title,
            url,
            description,
            section: section.to_string(),
        })
    }
}

/// Fetcher for llms.txt files from URLs.
pub struct LlmsTxtFetcher {
    client: reqwest::blocking::Client,
}

impl LlmsTxtFetcher {
    /// Create a new fetcher with default settings.
    pub fn new() -> Self {
        Self {
            client: reqwest::blocking::Client::builder()
                .user_agent("muninn-llmstxt/0.1")
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("Failed to create HTTP client"),
        }
    }

    /// Fetch and parse llms.txt from a URL.
    ///
    /// The URL can be:
    /// - Direct URL to llms.txt file
    /// - Base URL (will append /llms.txt)
    pub fn fetch(&self, url: &str) -> Result<LlmsTxt> {
        let llms_url = Self::normalize_url(url);
        let response = self.client.get(&llms_url).send()?;

        if !response.status().is_success() {
            return Err(LlmsTxtError::Parse(format!(
                "HTTP {} for {}",
                response.status(),
                llms_url
            )));
        }

        let content = response.text()?;
        LlmsTxtParser::parse(&content)
    }

    /// Fetch the content of a linked markdown file.
    pub fn fetch_linked_content(&self, url: &str) -> Result<String> {
        let response = self.client.get(url).send()?;

        if !response.status().is_success() {
            return Err(LlmsTxtError::Parse(format!(
                "HTTP {} for {}",
                response.status(),
                url
            )));
        }

        Ok(response.text()?)
    }

    /// Normalize URL to point to llms.txt.
    fn normalize_url(url: &str) -> String {
        if url.ends_with("/llms.txt") || url.ends_with("/llms.txt/") {
            url.trim_end_matches('/').to_string()
        } else if url.ends_with('/') {
            format!("{}llms.txt", url)
        } else {
            format!("{}/llms.txt", url)
        }
    }
}

impl Default for LlmsTxtFetcher {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Indexer
// ============================================================================

use crate::doc_store::{DocChunkInput, DocStore, DocStoreError, Ecosystem, ItemType};

/// Error type for llms.txt indexer operations.
#[derive(Debug, thiserror::Error)]
pub enum LlmsTxtIndexerError {
    #[error("llms.txt error: {0}")]
    LlmsTxt(#[from] LlmsTxtError),

    #[error("Doc store error: {0}")]
    DocStore(#[from] DocStoreError),

    #[error("Indexing failed: {0}")]
    IndexingFailed(String),
}

/// Statistics from an llms.txt indexing operation.
#[derive(Debug, Clone)]
pub struct LlmsTxtIndexStats {
    /// Name of the project indexed.
    pub name: String,
    /// Source URL of the llms.txt file.
    pub source_url: String,
    /// Number of links found in llms.txt.
    pub links_found: usize,
    /// Number of links successfully fetched and indexed.
    pub links_indexed: usize,
    /// Number of links that failed to fetch.
    pub links_failed: usize,
}

/// Configuration for the llms.txt indexer.
#[derive(Debug, Clone)]
pub struct LlmsTxtIndexerConfig {
    /// Whether to fetch and index linked content (slower but more complete).
    pub fetch_linked_content: bool,
    /// Maximum number of links to fetch (0 = unlimited).
    pub max_links: usize,
    /// Timeout for fetching individual links in seconds.
    pub link_timeout_secs: u64,
}

impl Default for LlmsTxtIndexerConfig {
    fn default() -> Self {
        Self {
            fetch_linked_content: true,
            max_links: 100,
            link_timeout_secs: 30,
        }
    }
}

/// Indexer for llms.txt documentation.
///
/// Fetches llms.txt files and optionally their linked content,
/// then stores the documentation in the DocStore.
pub struct LlmsTxtIndexer {
    fetcher: LlmsTxtFetcher,
    config: LlmsTxtIndexerConfig,
}

impl LlmsTxtIndexer {
    /// Create a new indexer with default configuration.
    pub fn new() -> Self {
        Self {
            fetcher: LlmsTxtFetcher::new(),
            config: LlmsTxtIndexerConfig::default(),
        }
    }

    /// Create an indexer with custom configuration.
    pub fn with_config(config: LlmsTxtIndexerConfig) -> Self {
        Self {
            fetcher: LlmsTxtFetcher::new(),
            config,
        }
    }

    /// Index llms.txt from a URL.
    ///
    /// # Arguments
    ///
    /// * `store` - The DocStore to index into
    /// * `url` - URL to fetch llms.txt from (can be base URL or direct llms.txt URL)
    ///
    /// # Returns
    ///
    /// Statistics about the indexing operation.
    pub fn index_url(
        &self,
        store: &DocStore,
        url: &str,
    ) -> std::result::Result<LlmsTxtIndexStats, LlmsTxtIndexerError> {
        // Fetch and parse llms.txt
        let llms_txt = self.fetcher.fetch(url)?;
        let source_url = LlmsTxtFetcher::normalize_url(url);

        self.index_llmstxt(store, &llms_txt, &source_url)
    }

    /// Index a parsed llms.txt.
    pub fn index_llmstxt(
        &self,
        store: &DocStore,
        llms_txt: &LlmsTxt,
        source_url: &str,
    ) -> std::result::Result<LlmsTxtIndexStats, LlmsTxtIndexerError> {
        let links_found = llms_txt.links.len();
        let mut links_indexed = 0;
        let mut links_failed = 0;

        // Create library entry
        let library_id =
            store.upsert_library(&llms_txt.name, Ecosystem::Web, "llms.txt", Some(source_url))?;

        // Prepare chunks
        let mut chunks: Vec<DocChunkInput> = Vec::new();

        // Add summary as a chunk if present
        if let Some(ref summary) = llms_txt.summary {
            chunks.push(DocChunkInput {
                item_path: format!("{}::summary", llms_txt.name),
                item_type: ItemType::Page,
                doc_text: summary.clone(),
                signature: None,
                embedding: None,
            });
        }

        // Process links
        let max_links = if self.config.max_links == 0 {
            llms_txt.links.len()
        } else {
            self.config.max_links.min(llms_txt.links.len())
        };

        for link in llms_txt.links.iter().take(max_links) {
            let item_path = format!("{}::{}", llms_txt.name, link.title);

            if self.config.fetch_linked_content {
                // Try to fetch linked content
                match self.fetcher.fetch_linked_content(&link.url) {
                    Ok(content) => {
                        // Use description + fetched content
                        let doc_text = if let Some(ref desc) = link.description {
                            format!("{}\n\n{}", desc, content)
                        } else {
                            content
                        };

                        chunks.push(DocChunkInput {
                            item_path,
                            item_type: Self::section_to_item_type(&link.section),
                            doc_text,
                            signature: Some(link.url.clone()),
                            embedding: None,
                        });
                        links_indexed += 1;
                    }
                    Err(_) => {
                        // Fall back to description only
                        if let Some(ref desc) = link.description {
                            chunks.push(DocChunkInput {
                                item_path,
                                item_type: Self::section_to_item_type(&link.section),
                                doc_text: desc.clone(),
                                signature: Some(link.url.clone()),
                                embedding: None,
                            });
                            links_indexed += 1;
                        } else {
                            links_failed += 1;
                        }
                    }
                }
            } else {
                // Just use the description from llms.txt
                if let Some(ref desc) = link.description {
                    chunks.push(DocChunkInput {
                        item_path,
                        item_type: Self::section_to_item_type(&link.section),
                        doc_text: desc.clone(),
                        signature: Some(link.url.clone()),
                        embedding: None,
                    });
                    links_indexed += 1;
                }
            }
        }

        // Insert all chunks
        store.insert_chunks_batch(library_id, &chunks)?;

        Ok(LlmsTxtIndexStats {
            name: llms_txt.name.clone(),
            source_url: source_url.to_string(),
            links_found,
            links_indexed,
            links_failed,
        })
    }

    /// Map section names to item types.
    fn section_to_item_type(section: &str) -> ItemType {
        let lower = section.to_lowercase();
        if lower.contains("guide")
            || lower.contains("tutorial")
            || lower.contains("getting started")
        {
            ItemType::Guide
        } else {
            ItemType::Page
        }
    }
}

impl Default for LlmsTxtIndexer {
    fn default() -> Self {
        Self::new()
    }
}

/// Convenience function to index llms.txt from a URL.
pub fn index_llmstxt(
    store: &DocStore,
    url: &str,
) -> std::result::Result<LlmsTxtIndexStats, LlmsTxtIndexerError> {
    let indexer = LlmsTxtIndexer::new();
    indexer.index_url(store, url)
}

/// Convenience function to index llms.txt without fetching linked content (fast mode).
pub fn index_llmstxt_fast(
    store: &DocStore,
    url: &str,
) -> std::result::Result<LlmsTxtIndexStats, LlmsTxtIndexerError> {
    let config = LlmsTxtIndexerConfig {
        fetch_linked_content: false,
        ..Default::default()
    };
    let indexer = LlmsTxtIndexer::with_config(config);
    indexer.index_url(store, url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_llmstxt() {
        let content = r#"# My Project

> A brief summary of the project.

## Guides

- [Getting Started](https://example.com/start.md): Introduction to the project.
- [API Reference](https://example.com/api.md): Full API documentation.

## Optional

- [Advanced Topics](https://example.com/advanced.md): For power users.
"#;

        let parsed = LlmsTxtParser::parse(content).unwrap();

        assert_eq!(parsed.name, "My Project");
        assert_eq!(
            parsed.summary,
            Some("A brief summary of the project.".to_string())
        );
        assert_eq!(parsed.links.len(), 3);
        assert_eq!(parsed.sections.len(), 2);

        let guides = &parsed.sections["Guides"];
        assert_eq!(guides.len(), 2);
        assert_eq!(guides[0].title, "Getting Started");
        assert_eq!(guides[0].url, "https://example.com/start.md");
        assert_eq!(
            guides[0].description,
            Some("Introduction to the project.".to_string())
        );
    }

    #[test]
    fn test_parse_minimal_llmstxt() {
        let content = "# Project Name\n\n- [Docs](https://example.com/docs.md)";

        let parsed = LlmsTxtParser::parse(content).unwrap();

        assert_eq!(parsed.name, "Project Name");
        assert!(parsed.summary.is_none());
        assert_eq!(parsed.links.len(), 1);
        assert_eq!(parsed.links[0].title, "Docs");
        assert_eq!(parsed.links[0].section, "General");
    }

    #[test]
    fn test_parse_no_header_fails() {
        let content = "## Section\n\n- [Link](https://example.com)";

        let result = LlmsTxtParser::parse(content);
        assert!(matches!(result, Err(LlmsTxtError::MissingHeader)));
    }

    #[test]
    fn test_parse_link() {
        let line = "- [Title](https://example.com/page.md): A description";
        let link = LlmsTxtParser::parse_link(line, "Test").unwrap();

        assert_eq!(link.title, "Title");
        assert_eq!(link.url, "https://example.com/page.md");
        assert_eq!(link.description, Some("A description".to_string()));
        assert_eq!(link.section, "Test");
    }

    #[test]
    fn test_parse_link_no_description() {
        let line = "- [Title](https://example.com/page.md)";
        let link = LlmsTxtParser::parse_link(line, "Test").unwrap();

        assert_eq!(link.title, "Title");
        assert_eq!(link.url, "https://example.com/page.md");
        assert!(link.description.is_none());
    }

    #[test]
    fn test_normalize_url() {
        assert_eq!(
            LlmsTxtFetcher::normalize_url("https://example.com"),
            "https://example.com/llms.txt"
        );
        assert_eq!(
            LlmsTxtFetcher::normalize_url("https://example.com/"),
            "https://example.com/llms.txt"
        );
        assert_eq!(
            LlmsTxtFetcher::normalize_url("https://example.com/llms.txt"),
            "https://example.com/llms.txt"
        );
        assert_eq!(
            LlmsTxtFetcher::normalize_url("https://example.com/docs/llms.txt"),
            "https://example.com/docs/llms.txt"
        );
    }

    #[test]
    fn test_parse_real_world_example() {
        // Example from Mintlify format
        let content = r#"# Mintlify

## Docs

- [Customize agent behavior](https://www.mintlify.com/docs/agent/customize.md): Configure how the agent handles documentation tasks with AGENTS.md.
- [Write effective prompts](https://www.mintlify.com/docs/agent/effective-prompts.md): Get better results from the agent with clear, focused prompts.
"#;

        let parsed = LlmsTxtParser::parse(content).unwrap();

        assert_eq!(parsed.name, "Mintlify");
        assert_eq!(parsed.links.len(), 2);
        assert_eq!(parsed.links[0].title, "Customize agent behavior");
        assert!(
            parsed.links[0]
                .description
                .as_ref()
                .unwrap()
                .contains("AGENTS.md")
        );
    }
}
