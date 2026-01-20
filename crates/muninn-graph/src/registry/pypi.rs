//! PyPI API client for fetching Python package metadata and source code.
//!
//! API Reference: https://warehouse.pypa.io/api-reference/json.html
//!
//! This client supports:
//! - Fetching package metadata (versions, dependencies, etc.)
//! - Downloading and extracting sdist (source distribution) tarballs
//!
//! # Example
//!
//! ```no_run
//! use muninn_graph::registry::PyPiClient;
//!
//! let client = PyPiClient::new();
//!
//! // Get package metadata
//! let metadata = client.get_package("requests")?;
//! println!("Latest requests version: {}", metadata.info.version);
//!
//! // Download and extract source
//! let extract_dir = client.download_source("requests", None, "/tmp/packages")?;
//! # Ok::<(), muninn_graph::registry::PyPiError>(())
//! ```

use std::fs::{self, File};
use std::io::{self, BufReader};
use std::path::{Path, PathBuf};

use flate2::read::GzDecoder;
use reqwest::blocking::Client;
use serde::Deserialize;
use tar::Archive;

/// Error type for PyPI API operations.
#[derive(Debug, thiserror::Error)]
pub enum PyPiError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Package not found: {0}")]
    PackageNotFound(String),

    #[error("Version not found: {package}=={version}")]
    VersionNotFound { package: String, version: String },

    #[error("No sdist available for {package}=={version}")]
    NoSdist { package: String, version: String },

    #[error("No versions available for package: {0}")]
    NoVersions(String),

    #[error("API error: {status} - {message}")]
    ApiError { status: u16, message: String },

    #[error("Unsupported archive format: {0}")]
    UnsupportedFormat(String),

    #[error("Zip extraction error: {0}")]
    Zip(#[from] zip::result::ZipError),
}

pub type Result<T> = std::result::Result<T, PyPiError>;

/// Base URL for PyPI JSON API.
const PYPI_API: &str = "https://pypi.org/pypi";

/// PyPI API client.
///
/// Provides methods for fetching package metadata and downloading source code.
/// Uses blocking HTTP requests for simplicity.
pub struct PyPiClient {
    client: Client,
}

impl PyPiClient {
    /// Create a new PyPI client with default settings.
    pub fn new() -> Self {
        Self::with_user_agent("muninn/0.1.0 (https://github.com/colliery-io/muninn)")
    }

    /// Create a new client with a custom user agent.
    pub fn with_user_agent(user_agent: &str) -> Self {
        let client = Client::builder()
            .user_agent(user_agent)
            .build()
            .expect("Failed to build HTTP client");

        Self { client }
    }

    /// Get metadata for a package (all versions).
    pub fn get_package(&self, package_name: &str) -> Result<PackageMetadata> {
        let url = format!("{}/{}/json", PYPI_API, package_name);

        let response = self.client.get(&url).send()?;

        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(PyPiError::PackageNotFound(package_name.to_string()));
        }

        if !response.status().is_success() {
            return Err(PyPiError::ApiError {
                status: response.status().as_u16(),
                message: response.text().unwrap_or_default(),
            });
        }

        let metadata: PackageMetadata = response.json()?;
        Ok(metadata)
    }

    /// Get metadata for a specific version.
    pub fn get_package_version(&self, package_name: &str, version: &str) -> Result<PackageMetadata> {
        let url = format!("{}/{}/{}/json", PYPI_API, package_name, version);

        let response = self.client.get(&url).send()?;

        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(PyPiError::VersionNotFound {
                package: package_name.to_string(),
                version: version.to_string(),
            });
        }

        if !response.status().is_success() {
            return Err(PyPiError::ApiError {
                status: response.status().as_u16(),
                message: response.text().unwrap_or_default(),
            });
        }

        let metadata: PackageMetadata = response.json()?;
        Ok(metadata)
    }

    /// Get the latest version string for a package.
    pub fn get_latest_version(&self, package_name: &str) -> Result<String> {
        let metadata = self.get_package(package_name)?;
        Ok(metadata.info.version)
    }

    /// List all available versions for a package.
    pub fn list_versions(&self, package_name: &str) -> Result<Vec<String>> {
        let metadata = self.get_package(package_name)?;
        let mut versions: Vec<String> = metadata.releases.keys().cloned().collect();
        // Sort versions (simple string sort - not semver aware)
        versions.sort();
        versions.reverse();
        Ok(versions)
    }

    /// Download and extract the source distribution for a package.
    ///
    /// # Arguments
    ///
    /// * `package_name` - Name of the package
    /// * `version` - Specific version (None for latest)
    /// * `output_dir` - Directory to extract into
    ///
    /// # Returns
    ///
    /// Tuple of (path to extracted directory, version string)
    pub fn download_source(
        &self,
        package_name: &str,
        version: Option<&str>,
        output_dir: impl AsRef<Path>,
    ) -> Result<(PathBuf, String)> {
        let output_dir = output_dir.as_ref();

        // Get metadata for the specific version or latest
        let metadata = if let Some(v) = version {
            self.get_package_version(package_name, v)?
        } else {
            self.get_package(package_name)?
        };

        let version = metadata.info.version.clone();

        // Find sdist URL from urls array
        let sdist_url = metadata
            .urls
            .iter()
            .find(|u| u.packagetype == "sdist")
            .ok_or_else(|| PyPiError::NoSdist {
                package: package_name.to_string(),
                version: version.clone(),
            })?;

        // Download the sdist
        let response = self.client.get(&sdist_url.url).send()?;

        if !response.status().is_success() {
            return Err(PyPiError::ApiError {
                status: response.status().as_u16(),
                message: format!("Failed to download {}", sdist_url.url),
            });
        }

        // Create output directory if needed
        fs::create_dir_all(output_dir)?;

        // Determine file extension and save
        let filename = &sdist_url.filename;
        let archive_path = output_dir.join(filename);
        let content = response.bytes()?;
        let mut archive_file = File::create(&archive_path)?;
        io::copy(&mut content.as_ref(), &mut archive_file)?;

        // Extract based on file type
        let extract_path = if filename.ends_with(".tar.gz") || filename.ends_with(".tgz") {
            self.extract_tar_gz(&archive_path, output_dir)?
        } else if filename.ends_with(".zip") {
            self.extract_zip(&archive_path, output_dir)?
        } else {
            // Clean up downloaded file
            let _ = fs::remove_file(&archive_path);
            return Err(PyPiError::UnsupportedFormat(filename.clone()));
        };

        // Clean up the archive
        let _ = fs::remove_file(&archive_path);

        Ok((extract_path, version))
    }

    /// Download and extract source, returning both path and full metadata.
    pub fn download_latest(
        &self,
        package_name: &str,
        output_dir: impl AsRef<Path>,
    ) -> Result<(PathBuf, PackageInfo)> {
        let (path, _version) = self.download_source(package_name, None, &output_dir)?;
        let metadata = self.get_package(package_name)?;
        Ok((path, metadata.info))
    }

    /// Extract a .tar.gz archive.
    fn extract_tar_gz(&self, archive_path: &Path, output_dir: &Path) -> Result<PathBuf> {
        let file = File::open(archive_path)?;
        let buf_reader = BufReader::new(file);
        let gz_decoder = GzDecoder::new(buf_reader);
        let mut archive = Archive::new(gz_decoder);

        // Get the name of the top-level directory
        let extracted_name = {
            let file = File::open(archive_path)?;
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

    /// Extract a .zip archive.
    fn extract_zip(&self, archive_path: &Path, output_dir: &Path) -> Result<PathBuf> {
        let file = File::open(archive_path)?;
        let mut archive = zip::ZipArchive::new(file)?;

        // Get the name of the top-level directory
        let extracted_name = archive
            .file_names()
            .next()
            .and_then(|name| name.split('/').next())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "extracted".to_string());

        // Extract all files
        archive.extract(output_dir)?;

        Ok(output_dir.join(extracted_name))
    }
}

impl Default for PyPiClient {
    fn default() -> Self {
        Self::new()
    }
}

/// Package metadata from PyPI JSON API.
#[derive(Debug, Deserialize)]
pub struct PackageMetadata {
    /// Package information (latest version).
    pub info: PackageInfo,

    /// Release files for the requested version.
    pub urls: Vec<ReleaseFile>,

    /// All releases keyed by version string.
    /// Note: This field is only present when fetching package metadata without a version,
    /// not when fetching a specific version.
    #[serde(default)]
    pub releases: std::collections::HashMap<String, Vec<ReleaseFile>>,
}

/// Package information.
#[derive(Debug, Clone, Deserialize)]
pub struct PackageInfo {
    /// Package name.
    pub name: String,

    /// Version string.
    pub version: String,

    /// Package summary/description.
    pub summary: Option<String>,

    /// Homepage URL.
    pub home_page: Option<String>,

    /// Documentation URL.
    pub docs_url: Option<String>,

    /// Project URL (from PyPI).
    pub project_url: Option<String>,

    /// Package license.
    pub license: Option<String>,

    /// Package author.
    pub author: Option<String>,

    /// Author email.
    pub author_email: Option<String>,

    /// Required Python version.
    pub requires_python: Option<String>,

    /// Package keywords.
    pub keywords: Option<String>,

    /// Classifiers.
    #[serde(default)]
    pub classifiers: Vec<String>,

    /// Project URLs (dict).
    #[serde(default)]
    pub project_urls: Option<std::collections::HashMap<String, String>>,

    /// Package dependencies.
    #[serde(default)]
    pub requires_dist: Option<Vec<String>>,
}

/// A release file (sdist or wheel).
#[derive(Debug, Clone, Deserialize)]
pub struct ReleaseFile {
    /// Filename.
    pub filename: String,

    /// Download URL.
    pub url: String,

    /// Package type: "sdist" or "bdist_wheel".
    pub packagetype: String,

    /// Python version requirement.
    pub python_version: Option<String>,

    /// File size in bytes.
    pub size: i64,

    /// MD5 digest.
    pub md5_digest: Option<String>,

    /// SHA256 digest.
    #[serde(default)]
    pub digests: FileDigests,

    /// Upload time.
    pub upload_time: Option<String>,
}

/// File digests.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct FileDigests {
    pub md5: Option<String>,
    pub sha256: Option<String>,
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
        let client = PyPiClient::new();
        drop(client);
    }

    #[test]
    fn test_client_custom_user_agent() {
        let client = PyPiClient::with_user_agent("test-agent/1.0");
        drop(client);
    }

    #[test]
    #[ignore] // Requires network
    fn test_get_package() {
        if skip_network_tests() {
            return;
        }

        let client = PyPiClient::new();
        let result = client.get_package("requests");

        assert!(result.is_ok());
        let metadata = result.unwrap();
        assert_eq!(metadata.info.name.to_lowercase(), "requests");
        assert!(!metadata.releases.is_empty());
    }

    #[test]
    #[ignore] // Requires network
    fn test_get_package_version() {
        if skip_network_tests() {
            return;
        }

        let client = PyPiClient::new();
        let result = client.get_package_version("requests", "2.31.0");

        assert!(result.is_ok());
        let metadata = result.unwrap();
        assert_eq!(metadata.info.version, "2.31.0");
    }

    #[test]
    #[ignore] // Requires network
    fn test_get_latest_version() {
        if skip_network_tests() {
            return;
        }

        let client = PyPiClient::new();
        let result = client.get_latest_version("requests");

        assert!(result.is_ok());
        let version = result.unwrap();
        assert!(!version.is_empty());
    }

    #[test]
    #[ignore] // Requires network
    fn test_list_versions() {
        if skip_network_tests() {
            return;
        }

        let client = PyPiClient::new();
        let result = client.list_versions("requests");

        assert!(result.is_ok());
        let versions = result.unwrap();
        assert!(!versions.is_empty());
        assert!(versions.iter().any(|v| v == "2.31.0"));
    }

    #[test]
    #[ignore] // Requires network
    fn test_get_nonexistent_package() {
        if skip_network_tests() {
            return;
        }

        let client = PyPiClient::new();
        let result = client.get_package("this-package-definitely-does-not-exist-xyz123");

        assert!(matches!(result, Err(PyPiError::PackageNotFound(_))));
    }

    #[test]
    #[ignore] // Requires network and disk
    fn test_download_source() {
        if skip_network_tests() {
            return;
        }

        let client = PyPiClient::new();
        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");

        // Download a small package
        let result = client.download_source("six", Some("1.16.0"), temp_dir.path());

        assert!(result.is_ok());
        let (extract_path, version) = result.unwrap();

        assert_eq!(version, "1.16.0");
        assert!(extract_path.exists());
        // six has setup.py
        assert!(extract_path.join("setup.py").exists() || extract_path.join("setup.cfg").exists());
    }

    #[test]
    #[ignore] // Requires network and disk
    fn test_download_latest() {
        if skip_network_tests() {
            return;
        }

        let client = PyPiClient::new();
        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");

        let result = client.download_latest("six", temp_dir.path());

        assert!(result.is_ok());
        let (path, info) = result.unwrap();

        assert!(path.exists());
        assert_eq!(info.name.to_lowercase(), "six");
    }
}
