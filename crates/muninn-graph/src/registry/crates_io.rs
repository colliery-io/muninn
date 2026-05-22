//! crates.io API client for fetching Rust crate metadata and source code.
//!
//! API Reference: https://crates.io/api-reference
//!
//! This client supports:
//! - Fetching crate metadata (versions, dependencies, etc.)
//! - Downloading and extracting source tarballs
//!
//! # Example
//!
//! ```no_run
//! use muninn_graph::registry::CratesIoClient;
//!
//! let client = CratesIoClient::new();
//!
//! // Get crate metadata
//! let version = client.get_latest_version("tokio")?;
//! println!("Latest tokio version: {}", version.num);
//!
//! // Download and extract source
//! let extract_dir = client.download_source("tokio", &version.num, "/tmp/crates")?;
//! # Ok::<(), muninn_graph::registry::CratesIoError>(())
//! ```

use std::fs::{self, File};
use std::io::{self, BufReader};
use std::path::{Path, PathBuf};

use flate2::read::GzDecoder;
use reqwest::blocking::Client;
use serde::Deserialize;
use tar::Archive;

/// Error type for crates.io API operations.
#[derive(Debug, thiserror::Error)]
pub enum CratesIoError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Crate not found: {0}")]
    CrateNotFound(String),

    #[error("Version not found: {crate_name}@{version}")]
    VersionNotFound { crate_name: String, version: String },

    #[error("No versions available for crate: {0}")]
    NoVersions(String),

    #[error("API error: {status} - {message}")]
    ApiError { status: u16, message: String },
}

pub type Result<T> = std::result::Result<T, CratesIoError>;

/// Base URL for crates.io API.
const CRATES_IO_API: &str = "https://crates.io/api/v1";

/// Base URL for downloading crates.
const CRATES_IO_DOWNLOAD: &str = "https://static.crates.io/crates";

/// crates.io API client.
///
/// Provides methods for fetching crate metadata and downloading source code.
/// Uses blocking HTTP requests for simplicity.
pub struct CratesIoClient {
    client: Client,
}

impl CratesIoClient {
    /// Create a new crates.io client with default settings.
    pub fn new() -> Self {
        Self::with_user_agent("muninn/0.1.0 (https://github.com/colliery-io/muninn)")
    }

    /// Create a new client with a custom user agent.
    ///
    /// crates.io requires a user agent that identifies the application.
    pub fn with_user_agent(user_agent: &str) -> Self {
        let client = Client::builder()
            .user_agent(user_agent)
            .build()
            .expect("Failed to build HTTP client");

        Self { client }
    }

    /// Get metadata for a crate including all versions.
    pub fn get_crate(&self, crate_name: &str) -> Result<CrateResponse> {
        let url = format!("{}/crates/{}", CRATES_IO_API, crate_name);

        let response = self.client.get(&url).send()?;

        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(CratesIoError::CrateNotFound(crate_name.to_string()));
        }

        if !response.status().is_success() {
            return Err(CratesIoError::ApiError {
                status: response.status().as_u16(),
                message: response.text().unwrap_or_default(),
            });
        }

        let crate_response: CrateResponse = response.json()?;
        Ok(crate_response)
    }

    /// Get the latest (non-yanked) version of a crate.
    pub fn get_latest_version(&self, crate_name: &str) -> Result<CrateVersion> {
        let crate_response = self.get_crate(crate_name)?;

        // Find the first non-yanked version (versions are sorted by newest first)
        crate_response
            .versions
            .into_iter()
            .find(|v| !v.yanked)
            .ok_or_else(|| CratesIoError::NoVersions(crate_name.to_string()))
    }

    /// Get a specific version of a crate.
    pub fn get_version(&self, crate_name: &str, version: &str) -> Result<CrateVersion> {
        let crate_response = self.get_crate(crate_name)?;

        crate_response
            .versions
            .into_iter()
            .find(|v| v.num == version)
            .ok_or_else(|| CratesIoError::VersionNotFound {
                crate_name: crate_name.to_string(),
                version: version.to_string(),
            })
    }

    /// Download and extract the source tarball for a crate version.
    ///
    /// # Arguments
    ///
    /// * `crate_name` - Name of the crate
    /// * `version` - Version string (e.g., "1.35.0")
    /// * `output_dir` - Directory to extract into
    ///
    /// # Returns
    ///
    /// Path to the extracted crate directory (e.g., `/tmp/crates/tokio-1.35.0`)
    pub fn download_source(
        &self,
        crate_name: &str,
        version: &str,
        output_dir: impl AsRef<Path>,
    ) -> Result<PathBuf> {
        let output_dir = output_dir.as_ref();

        // Download URL format: https://static.crates.io/crates/{name}/{name}-{version}.crate
        let download_url = format!(
            "{}/{}/{}-{}.crate",
            CRATES_IO_DOWNLOAD, crate_name, crate_name, version
        );

        // Download the tarball
        let response = self.client.get(&download_url).send()?;

        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(CratesIoError::VersionNotFound {
                crate_name: crate_name.to_string(),
                version: version.to_string(),
            });
        }

        if !response.status().is_success() {
            return Err(CratesIoError::ApiError {
                status: response.status().as_u16(),
                message: format!("Failed to download {}-{}", crate_name, version),
            });
        }

        // Create output directory if needed
        fs::create_dir_all(output_dir)?;

        // Write to temp file first
        let tarball_path = output_dir.join(format!("{}-{}.crate", crate_name, version));
        let mut tarball_file = File::create(&tarball_path)?;
        let content = response.bytes()?;
        io::copy(&mut content.as_ref(), &mut tarball_file)?;

        // Extract the tarball
        let extract_path = self.extract_tarball(&tarball_path, output_dir)?;

        // Clean up the tarball
        fs::remove_file(&tarball_path)?;

        Ok(extract_path)
    }

    /// Extract a .crate tarball (gzipped tar).
    fn extract_tarball(&self, tarball_path: &Path, output_dir: &Path) -> Result<PathBuf> {
        let file = File::open(tarball_path)?;
        let buf_reader = BufReader::new(file);
        let gz_decoder = GzDecoder::new(buf_reader);
        let mut archive = Archive::new(gz_decoder);

        // Get the name of the extracted directory (first entry in archive)
        let extracted_name = {
            let file = File::open(tarball_path)?;
            let buf_reader = BufReader::new(file);
            let gz_decoder = GzDecoder::new(buf_reader);
            let mut archive = Archive::new(gz_decoder);

            archive
                .entries()?
                .next()
                .and_then(|entry| entry.ok())
                .and_then(|entry| {
                    entry
                        .path()
                        .ok()
                        .and_then(|p| p.iter().next().map(|s| s.to_string_lossy().into_owned()))
                })
                .unwrap_or_else(|| "extracted".to_string())
        };

        // Extract all files
        archive.unpack(output_dir)?;

        Ok(output_dir.join(extracted_name))
    }

    /// Download and extract source, returning both the path and version info.
    ///
    /// This is a convenience method that combines `get_latest_version` and `download_source`.
    pub fn download_latest(
        &self,
        crate_name: &str,
        output_dir: impl AsRef<Path>,
    ) -> Result<(PathBuf, CrateVersion)> {
        let version = self.get_latest_version(crate_name)?;
        let path = self.download_source(crate_name, &version.num, output_dir)?;
        Ok((path, version))
    }
}

impl Default for CratesIoClient {
    fn default() -> Self {
        Self::new()
    }
}

/// Response from crates.io crate endpoint.
#[derive(Debug, Deserialize)]
pub struct CrateResponse {
    /// The crate metadata.
    #[serde(rename = "crate")]
    pub krate: CrateInfo,

    /// List of all versions (sorted newest first).
    pub versions: Vec<CrateVersion>,
}

/// Basic crate information.
#[derive(Debug, Deserialize)]
pub struct CrateInfo {
    /// Crate name.
    pub name: String,

    /// Crate description.
    pub description: Option<String>,

    /// Homepage URL.
    pub homepage: Option<String>,

    /// Documentation URL.
    pub documentation: Option<String>,

    /// Repository URL.
    pub repository: Option<String>,

    /// Total downloads.
    pub downloads: i64,

    /// Maximum version (semver).
    pub max_version: String,

    /// Maximum stable version (if any).
    pub max_stable_version: Option<String>,
}

/// Version-specific information.
#[derive(Debug, Clone, Deserialize)]
pub struct CrateVersion {
    /// Version number (semver string).
    pub num: String,

    /// Whether this version is yanked.
    pub yanked: bool,

    /// Download count for this version.
    pub downloads: i64,

    /// Crate size in bytes.
    pub crate_size: Option<i64>,

    /// Checksum (sha256).
    pub checksum: String,

    /// When this version was published.
    pub created_at: String,

    /// Rust version requirement (MSRV).
    pub rust_version: Option<String>,

    /// License string.
    pub license: Option<String>,

    /// Link relationships (download URL, etc.)
    pub links: VersionLinks,
}

/// Links associated with a version.
#[derive(Debug, Clone, Deserialize)]
pub struct VersionLinks {
    /// Relative path to dependencies endpoint.
    pub dependencies: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    // Helper to check if we should skip network tests
    fn skip_network_tests() -> bool {
        env::var("SKIP_NETWORK_TESTS").is_ok()
    }

    #[test]
    fn test_client_creation() {
        let client = CratesIoClient::new();
        // Just verify it doesn't panic
        drop(client);
    }

    #[test]
    fn test_client_custom_user_agent() {
        let client = CratesIoClient::with_user_agent("test-agent/1.0");
        drop(client);
    }

    #[test]
    #[ignore] // Requires network - run with: cargo test -- --ignored
    fn test_get_crate() {
        if skip_network_tests() {
            return;
        }

        let client = CratesIoClient::new();
        let result = client.get_crate("serde");

        assert!(result.is_ok());
        let crate_response = result.unwrap();
        assert_eq!(crate_response.krate.name, "serde");
        assert!(!crate_response.versions.is_empty());
    }

    #[test]
    #[ignore] // Requires network
    fn test_get_latest_version() {
        if skip_network_tests() {
            return;
        }

        let client = CratesIoClient::new();
        let result = client.get_latest_version("serde");

        assert!(result.is_ok());
        let version = result.unwrap();
        assert!(!version.num.is_empty());
        assert!(!version.yanked);
    }

    #[test]
    #[ignore] // Requires network
    fn test_get_nonexistent_crate() {
        if skip_network_tests() {
            return;
        }

        let client = CratesIoClient::new();
        let result = client.get_crate("this-crate-definitely-does-not-exist-xyz123");

        assert!(matches!(result, Err(CratesIoError::CrateNotFound(_))));
    }

    #[test]
    #[ignore] // Requires network and disk
    fn test_download_source() {
        if skip_network_tests() {
            return;
        }

        let client = CratesIoClient::new();
        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");

        // Download a small crate
        let result = client.download_source("once_cell", "1.19.0", temp_dir.path());

        assert!(result.is_ok());
        let extract_path = result.unwrap();

        // Verify the directory exists and contains expected files
        assert!(extract_path.exists());
        assert!(extract_path.join("Cargo.toml").exists());
        assert!(extract_path.join("src").exists());
    }

    #[test]
    #[ignore] // Requires network and disk
    fn test_download_latest() {
        if skip_network_tests() {
            return;
        }

        let client = CratesIoClient::new();
        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");

        let result = client.download_latest("cfg-if", temp_dir.path());

        assert!(result.is_ok());
        let (path, version) = result.unwrap();

        assert!(path.exists());
        assert!(!version.num.is_empty());
    }
}
