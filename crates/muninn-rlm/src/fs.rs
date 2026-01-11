//! Virtual filesystem abstraction for testability.
//!
//! This module provides a `FileSystem` trait that abstracts filesystem operations,
//! allowing tools to be tested with mock filesystems instead of real files.

use async_trait::async_trait;
use std::collections::HashMap;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

// ============================================================================
// FileSystem Trait
// ============================================================================

/// Abstraction over filesystem operations for testability.
///
/// This trait allows filesystem operations to be mocked in tests,
/// enabling unit testing of file-based tools without real files.
#[async_trait]
pub trait FileSystem: Send + Sync {
    /// Read file contents as a string.
    async fn read_file(&self, path: &Path) -> io::Result<String>;

    /// Read file contents as bytes.
    async fn read_file_bytes(&self, path: &Path) -> io::Result<Vec<u8>>;

    /// Write content to a file.
    async fn write_file(&self, path: &Path, content: &str) -> io::Result<()>;

    /// List directory contents.
    async fn list_dir(&self, path: &Path) -> io::Result<Vec<DirEntry>>;

    /// Check if path exists.
    async fn exists(&self, path: &Path) -> bool;

    /// Check if path is a directory.
    async fn is_dir(&self, path: &Path) -> bool;

    /// Check if path is a file.
    async fn is_file(&self, path: &Path) -> bool;

    /// Get file metadata.
    async fn metadata(&self, path: &Path) -> io::Result<FileMetadata>;

    /// Canonicalize a path.
    async fn canonicalize(&self, path: &Path) -> io::Result<PathBuf>;
}

/// Directory entry information.
#[derive(Debug, Clone)]
pub struct DirEntry {
    /// The full path to the entry.
    pub path: PathBuf,
    /// The file name.
    pub name: String,
    /// Whether this is a directory.
    pub is_dir: bool,
}

/// File metadata.
#[derive(Debug, Clone)]
pub struct FileMetadata {
    /// File size in bytes.
    pub len: u64,
    /// Whether this is a directory.
    pub is_dir: bool,
    /// Whether this is a file.
    pub is_file: bool,
}

// ============================================================================
// RealFileSystem
// ============================================================================

/// Real filesystem implementation using tokio::fs.
#[derive(Debug, Clone, Default)]
pub struct RealFileSystem;

impl RealFileSystem {
    /// Create a new real filesystem.
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl FileSystem for RealFileSystem {
    async fn read_file(&self, path: &Path) -> io::Result<String> {
        tokio::fs::read_to_string(path).await
    }

    async fn read_file_bytes(&self, path: &Path) -> io::Result<Vec<u8>> {
        tokio::fs::read(path).await
    }

    async fn write_file(&self, path: &Path, content: &str) -> io::Result<()> {
        tokio::fs::write(path, content).await
    }

    async fn list_dir(&self, path: &Path) -> io::Result<Vec<DirEntry>> {
        let mut entries = Vec::new();
        let mut read_dir = tokio::fs::read_dir(path).await?;

        while let Some(entry) = read_dir.next_entry().await? {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            let file_type = entry.file_type().await?;

            entries.push(DirEntry {
                path,
                name,
                is_dir: file_type.is_dir(),
            });
        }

        Ok(entries)
    }

    async fn exists(&self, path: &Path) -> bool {
        tokio::fs::try_exists(path).await.unwrap_or(false)
    }

    async fn is_dir(&self, path: &Path) -> bool {
        tokio::fs::metadata(path)
            .await
            .map(|m| m.is_dir())
            .unwrap_or(false)
    }

    async fn is_file(&self, path: &Path) -> bool {
        tokio::fs::metadata(path)
            .await
            .map(|m| m.is_file())
            .unwrap_or(false)
    }

    async fn metadata(&self, path: &Path) -> io::Result<FileMetadata> {
        let meta = tokio::fs::metadata(path).await?;
        Ok(FileMetadata {
            len: meta.len(),
            is_dir: meta.is_dir(),
            is_file: meta.is_file(),
        })
    }

    async fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        tokio::fs::canonicalize(path).await
    }
}

// ============================================================================
// MockFileSystem
// ============================================================================

/// Mock filesystem for testing.
///
/// Stores files in memory and tracks write operations for verification.
#[derive(Debug, Clone)]
pub struct MockFileSystem {
    /// In-memory file contents.
    files: Arc<RwLock<HashMap<PathBuf, Vec<u8>>>>,
    /// Directories (tracked separately).
    directories: Arc<RwLock<HashMap<PathBuf, ()>>>,
    /// Files that were written during test.
    written_files: Arc<RwLock<HashMap<PathBuf, Vec<u8>>>>,
}

impl Default for MockFileSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl MockFileSystem {
    /// Create a new empty mock filesystem.
    pub fn new() -> Self {
        Self {
            files: Arc::new(RwLock::new(HashMap::new())),
            directories: Arc::new(RwLock::new(HashMap::new())),
            written_files: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Add a file with string content.
    pub fn with_file(self, path: impl Into<PathBuf>, content: impl Into<String>) -> Self {
        let path = path.into();
        let content = content.into();

        // Add parent directories
        if let Some(parent) = path.parent() {
            self.ensure_parent_dirs(parent);
        }

        self.files
            .write()
            .unwrap()
            .insert(path, content.into_bytes());
        self
    }

    /// Add a file with binary content.
    pub fn with_file_bytes(self, path: impl Into<PathBuf>, content: Vec<u8>) -> Self {
        let path = path.into();

        // Add parent directories
        if let Some(parent) = path.parent() {
            self.ensure_parent_dirs(parent);
        }

        self.files.write().unwrap().insert(path, content);
        self
    }

    /// Add multiple files at once.
    pub fn with_files(
        mut self,
        files: impl IntoIterator<Item = (impl Into<PathBuf>, impl Into<String>)>,
    ) -> Self {
        for (path, content) in files {
            self = self.with_file(path, content);
        }
        self
    }

    /// Add a directory.
    pub fn with_directory(self, path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        self.ensure_parent_dirs(&path);
        self.directories.write().unwrap().insert(path, ());
        self
    }

    /// Get all files that were written during the test.
    pub fn get_written_files(&self) -> HashMap<PathBuf, String> {
        self.written_files
            .read()
            .unwrap()
            .iter()
            .filter_map(|(k, v)| String::from_utf8(v.clone()).ok().map(|s| (k.clone(), s)))
            .collect()
    }

    /// Get a specific file's content that was written.
    pub fn get_written_file(&self, path: &Path) -> Option<String> {
        self.written_files
            .read()
            .unwrap()
            .get(path)
            .and_then(|bytes| String::from_utf8(bytes.clone()).ok())
    }

    /// Check if a file was written.
    pub fn was_file_written(&self, path: &Path) -> bool {
        self.written_files.read().unwrap().contains_key(path)
    }

    /// Ensure parent directories exist.
    fn ensure_parent_dirs(&self, path: &Path) {
        let mut current = path.to_path_buf();
        let mut dirs_to_add = Vec::new();

        while current.parent().is_some() {
            dirs_to_add.push(current.clone());
            current = current.parent().unwrap().to_path_buf();
            if current.as_os_str().is_empty() {
                break;
            }
        }

        let mut directories = self.directories.write().unwrap();
        for dir in dirs_to_add {
            directories.insert(dir, ());
        }
    }

    /// Normalize a path for consistent lookups.
    fn normalize_path(&self, path: &Path) -> PathBuf {
        // Simple normalization - remove . and handle ..
        let mut components = Vec::new();
        for component in path.components() {
            match component {
                std::path::Component::CurDir => {}
                std::path::Component::ParentDir => {
                    components.pop();
                }
                _ => components.push(component),
            }
        }
        components.iter().collect()
    }
}

#[async_trait]
impl FileSystem for MockFileSystem {
    async fn read_file(&self, path: &Path) -> io::Result<String> {
        let normalized = self.normalize_path(path);
        self.files
            .read()
            .unwrap()
            .get(&normalized)
            .map(|bytes| String::from_utf8_lossy(bytes).to_string())
            .ok_or_else(|| {
                io::Error::new(ErrorKind::NotFound, format!("File not found: {:?}", path))
            })
    }

    async fn read_file_bytes(&self, path: &Path) -> io::Result<Vec<u8>> {
        let normalized = self.normalize_path(path);
        self.files
            .read()
            .unwrap()
            .get(&normalized)
            .cloned()
            .ok_or_else(|| {
                io::Error::new(ErrorKind::NotFound, format!("File not found: {:?}", path))
            })
    }

    async fn write_file(&self, path: &Path, content: &str) -> io::Result<()> {
        let normalized = self.normalize_path(path);
        let bytes = content.as_bytes().to_vec();

        self.files
            .write()
            .unwrap()
            .insert(normalized.clone(), bytes.clone());
        self.written_files
            .write()
            .unwrap()
            .insert(normalized, bytes);

        Ok(())
    }

    async fn list_dir(&self, path: &Path) -> io::Result<Vec<DirEntry>> {
        let normalized = self.normalize_path(path);

        // Check if directory exists
        if !self.is_dir(&normalized).await {
            return Err(io::Error::new(
                ErrorKind::NotFound,
                format!("Directory not found: {:?}", path),
            ));
        }

        let files = self.files.read().unwrap();
        let directories = self.directories.read().unwrap();

        let mut entries = Vec::new();
        let mut seen = std::collections::HashSet::new();

        // Find files in this directory
        for file_path in files.keys() {
            if let Some(parent) = file_path.parent() {
                if parent == normalized {
                    let name = file_path.file_name().unwrap().to_string_lossy().to_string();
                    if seen.insert(name.clone()) {
                        entries.push(DirEntry {
                            path: file_path.clone(),
                            name,
                            is_dir: false,
                        });
                    }
                }
            }
        }

        // Find subdirectories
        for dir_path in directories.keys() {
            if let Some(parent) = dir_path.parent() {
                if parent == normalized && dir_path != &normalized {
                    let name = dir_path.file_name().unwrap().to_string_lossy().to_string();
                    if seen.insert(name.clone()) {
                        entries.push(DirEntry {
                            path: dir_path.clone(),
                            name,
                            is_dir: true,
                        });
                    }
                }
            }
        }

        Ok(entries)
    }

    async fn exists(&self, path: &Path) -> bool {
        let normalized = self.normalize_path(path);
        self.files.read().unwrap().contains_key(&normalized)
            || self.directories.read().unwrap().contains_key(&normalized)
    }

    async fn is_dir(&self, path: &Path) -> bool {
        let normalized = self.normalize_path(path);
        self.directories.read().unwrap().contains_key(&normalized)
    }

    async fn is_file(&self, path: &Path) -> bool {
        let normalized = self.normalize_path(path);
        self.files.read().unwrap().contains_key(&normalized)
    }

    async fn metadata(&self, path: &Path) -> io::Result<FileMetadata> {
        let normalized = self.normalize_path(path);

        if let Some(bytes) = self.files.read().unwrap().get(&normalized) {
            return Ok(FileMetadata {
                len: bytes.len() as u64,
                is_dir: false,
                is_file: true,
            });
        }

        if self.directories.read().unwrap().contains_key(&normalized) {
            return Ok(FileMetadata {
                len: 0,
                is_dir: true,
                is_file: false,
            });
        }

        Err(io::Error::new(
            ErrorKind::NotFound,
            format!("Path not found: {:?}", path),
        ))
    }

    async fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        // In mock filesystem, canonicalize just normalizes the path
        // and checks that it exists
        let normalized = self.normalize_path(path);

        if self.exists(&normalized).await {
            Ok(normalized)
        } else {
            Err(io::Error::new(
                ErrorKind::NotFound,
                format!("Path not found: {:?}", path),
            ))
        }
    }
}

// ============================================================================
// Type Aliases
// ============================================================================

/// Shared filesystem reference.
pub type SharedFileSystem = Arc<dyn FileSystem>;

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_fs_read_file() {
        let fs = MockFileSystem::new().with_file("/test/hello.txt", "Hello, World!");

        let content = fs.read_file(Path::new("/test/hello.txt")).await.unwrap();
        assert_eq!(content, "Hello, World!");
    }

    #[tokio::test]
    async fn test_mock_fs_read_file_not_found() {
        let fs = MockFileSystem::new();

        let result = fs.read_file(Path::new("/nonexistent.txt")).await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), ErrorKind::NotFound);
    }

    #[tokio::test]
    async fn test_mock_fs_write_file() {
        let fs = MockFileSystem::new().with_directory("/test");

        fs.write_file(Path::new("/test/output.txt"), "Written content")
            .await
            .unwrap();

        // Verify file was written
        assert!(fs.was_file_written(Path::new("/test/output.txt")));
        assert_eq!(
            fs.get_written_file(Path::new("/test/output.txt")),
            Some("Written content".to_string())
        );

        // Verify we can read it back
        let content = fs.read_file(Path::new("/test/output.txt")).await.unwrap();
        assert_eq!(content, "Written content");
    }

    #[tokio::test]
    async fn test_mock_fs_exists() {
        let fs = MockFileSystem::new()
            .with_file("/test/file.txt", "content")
            .with_directory("/test/subdir");

        assert!(fs.exists(Path::new("/test/file.txt")).await);
        assert!(fs.exists(Path::new("/test/subdir")).await);
        assert!(!fs.exists(Path::new("/nonexistent")).await);
    }

    #[tokio::test]
    async fn test_mock_fs_is_file_is_dir() {
        let fs = MockFileSystem::new()
            .with_file("/test/file.txt", "content")
            .with_directory("/test/subdir");

        assert!(fs.is_file(Path::new("/test/file.txt")).await);
        assert!(!fs.is_dir(Path::new("/test/file.txt")).await);

        assert!(fs.is_dir(Path::new("/test/subdir")).await);
        assert!(!fs.is_file(Path::new("/test/subdir")).await);
    }

    #[tokio::test]
    async fn test_mock_fs_metadata() {
        let fs = MockFileSystem::new().with_file("/test/file.txt", "Hello!");

        let meta = fs.metadata(Path::new("/test/file.txt")).await.unwrap();
        assert_eq!(meta.len, 6); // "Hello!" is 6 bytes
        assert!(meta.is_file);
        assert!(!meta.is_dir);
    }

    #[tokio::test]
    async fn test_mock_fs_list_dir() {
        let fs = MockFileSystem::new()
            .with_directory("/root")
            .with_file("/root/a.txt", "a")
            .with_file("/root/b.txt", "b")
            .with_directory("/root/subdir");

        let entries = fs.list_dir(Path::new("/root")).await.unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();

        assert!(names.contains(&"a.txt"));
        assert!(names.contains(&"b.txt"));
        assert!(names.contains(&"subdir"));
    }

    #[tokio::test]
    async fn test_mock_fs_with_files() {
        let fs = MockFileSystem::new().with_files([
            ("/a.txt", "content a"),
            ("/b.txt", "content b"),
            ("/c.txt", "content c"),
        ]);

        assert!(fs.exists(Path::new("/a.txt")).await);
        assert!(fs.exists(Path::new("/b.txt")).await);
        assert!(fs.exists(Path::new("/c.txt")).await);
    }

    #[tokio::test]
    async fn test_mock_fs_canonicalize() {
        let fs = MockFileSystem::new().with_file("/test/file.txt", "content");

        // Existing file can be canonicalized
        let canonical = fs.canonicalize(Path::new("/test/file.txt")).await.unwrap();
        assert_eq!(canonical, PathBuf::from("/test/file.txt"));

        // Nonexistent file cannot
        let result = fs.canonicalize(Path::new("/nonexistent.txt")).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_mock_fs_get_written_files() {
        let fs = MockFileSystem::new().with_directory("/output");

        fs.write_file(Path::new("/output/one.txt"), "first")
            .await
            .unwrap();
        fs.write_file(Path::new("/output/two.txt"), "second")
            .await
            .unwrap();

        let written = fs.get_written_files();
        assert_eq!(written.len(), 2);
        assert_eq!(
            written.get(&PathBuf::from("/output/one.txt")),
            Some(&"first".to_string())
        );
        assert_eq!(
            written.get(&PathBuf::from("/output/two.txt")),
            Some(&"second".to_string())
        );
    }

    #[test]
    fn test_mock_fs_builder_pattern() {
        let fs = MockFileSystem::new()
            .with_directory("/project")
            .with_file("/project/main.rs", "fn main() {}")
            .with_file("/project/lib.rs", "pub mod utils;")
            .with_directory("/project/src")
            .with_file("/project/src/utils.rs", "pub fn helper() {}");

        // Verify all files exist synchronously by checking internal state
        assert!(
            fs.files
                .read()
                .unwrap()
                .contains_key(&PathBuf::from("/project/main.rs"))
        );
        assert!(
            fs.files
                .read()
                .unwrap()
                .contains_key(&PathBuf::from("/project/lib.rs"))
        );
        assert!(
            fs.files
                .read()
                .unwrap()
                .contains_key(&PathBuf::from("/project/src/utils.rs"))
        );
    }
}
