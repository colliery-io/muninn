//! File system watcher for incremental graph updates.
//!
//! This module provides file watching capabilities with debouncing,
//! gitignore support, and integration with the graph builder.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use ignore::gitignore::{Gitignore, GitignoreBuilder};
use notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_mini::{DebouncedEvent, Debouncer, new_debouncer};

/// Error type for file watcher operations.
#[derive(Debug, thiserror::Error)]
pub enum WatchError {
    #[error("Notify error: {0}")]
    Notify(#[from] notify::Error),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Gitignore error: {0}")]
    Gitignore(#[from] ignore::Error),
}

pub type Result<T> = std::result::Result<T, WatchError>;

/// Events emitted by the file watcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileEvent {
    /// A new file was created.
    Created(PathBuf),
    /// A file was modified.
    Modified(PathBuf),
    /// A file was deleted.
    Deleted(PathBuf),
}

impl FileEvent {
    /// Get the path associated with this event.
    pub fn path(&self) -> &Path {
        match self {
            FileEvent::Created(p) | FileEvent::Modified(p) | FileEvent::Deleted(p) => p,
        }
    }
}

/// Configuration for the file watcher.
#[derive(Debug, Clone)]
pub struct WatcherConfig {
    /// Debounce duration for rapid events.
    pub debounce_duration: Duration,
    /// File extensions to watch (e.g., "rs", "py").
    pub extensions: Vec<String>,
    /// Whether to respect .gitignore files.
    pub use_gitignore: bool,
    /// Additional ignore patterns.
    pub ignore_patterns: Vec<String>,
}

impl Default for WatcherConfig {
    fn default() -> Self {
        Self {
            debounce_duration: Duration::from_millis(300),
            extensions: vec![
                "rs".to_string(),
                "py".to_string(),
                "c".to_string(),
                "cpp".to_string(),
                "h".to_string(),
                "hpp".to_string(),
            ],
            use_gitignore: true,
            ignore_patterns: vec![
                "target".to_string(),
                "node_modules".to_string(),
                ".git".to_string(),
                "__pycache__".to_string(),
                "*.pyc".to_string(),
            ],
        }
    }
}

/// File system watcher with debouncing and filtering.
pub struct FileWatcher {
    _debouncer: Debouncer<RecommendedWatcher>,
    rx: Receiver<std::result::Result<Vec<DebouncedEvent>, notify::Error>>,
    config: WatcherConfig,
    gitignore: Option<Gitignore>,
    root: PathBuf,
}

impl FileWatcher {
    /// Create a new file watcher for the given root directory.
    pub fn new(root: &Path) -> Result<Self> {
        Self::with_config(root, WatcherConfig::default())
    }

    /// Create a new file watcher with custom configuration.
    pub fn with_config(root: &Path, config: WatcherConfig) -> Result<Self> {
        let (tx, rx) = channel();

        let mut debouncer = new_debouncer(config.debounce_duration, tx)?;

        // Watch the root directory recursively
        debouncer.watcher().watch(root, RecursiveMode::Recursive)?;

        // Build gitignore matcher if enabled
        let gitignore = if config.use_gitignore {
            Self::build_gitignore(root, &config.ignore_patterns).ok()
        } else {
            None
        };

        Ok(Self {
            _debouncer: debouncer,
            rx,
            config,
            gitignore,
            root: root.to_path_buf(),
        })
    }

    /// Build a gitignore matcher from .gitignore and custom patterns.
    fn build_gitignore(root: &Path, extra_patterns: &[String]) -> Result<Gitignore> {
        let mut builder = GitignoreBuilder::new(root);

        // Add .gitignore if it exists
        let gitignore_path = root.join(".gitignore");
        if gitignore_path.exists() {
            builder.add(&gitignore_path);
        }

        // Add .muninnignore if it exists
        let muninnignore_path = root.join(".muninnignore");
        if muninnignore_path.exists() {
            builder.add(&muninnignore_path);
        }

        // Add extra patterns
        for pattern in extra_patterns {
            builder.add_line(None, pattern)?;
        }

        Ok(builder.build()?)
    }

    /// Check if a path should be ignored.
    fn should_ignore(&self, path: &Path) -> bool {
        // Check gitignore patterns
        if let Some(ref gi) = self.gitignore {
            if gi.matched(path, path.is_dir()).is_ignore() {
                return true;
            }
        }

        // Check if it's a supported extension
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if !self.config.extensions.contains(&ext.to_string()) {
                return true;
            }
        } else {
            // No extension - ignore unless it's a directory event
            return !path.is_dir();
        }

        false
    }

    /// Get the next file event, blocking until one is available.
    ///
    /// Returns `None` if the watcher has been stopped.
    pub fn next_event(&self) -> Option<FileEvent> {
        loop {
            match self.rx.recv() {
                Ok(Ok(events)) => {
                    for event in events {
                        let path = event.path;

                        // Skip ignored paths
                        if self.should_ignore(&path) {
                            continue;
                        }

                        // Determine event type based on file existence
                        // (debouncer doesn't distinguish create/modify/delete)
                        let file_event = if path.exists() {
                            FileEvent::Modified(path)
                        } else {
                            FileEvent::Deleted(path)
                        };

                        return Some(file_event);
                    }
                }
                Ok(Err(_error)) => {
                    // Log errors but continue
                    continue;
                }
                Err(_) => {
                    // Channel closed
                    return None;
                }
            }
        }
    }

    /// Try to get the next file event without blocking.
    ///
    /// Returns `None` if no event is immediately available.
    pub fn try_next_event(&self) -> Option<FileEvent> {
        match self.rx.try_recv() {
            Ok(Ok(events)) => {
                for event in events {
                    let path = event.path;

                    if self.should_ignore(&path) {
                        continue;
                    }

                    // Determine event type based on file existence
                    let file_event = if path.exists() {
                        FileEvent::Modified(path)
                    } else {
                        FileEvent::Deleted(path)
                    };

                    return Some(file_event);
                }
                None
            }
            _ => None,
        }
    }

    /// Get the root directory being watched.
    pub fn root(&self) -> &Path {
        &self.root
    }
}

/// Watch a directory and rebuild the graph on changes.
///
/// This is a convenience function that runs a watch loop, calling
/// the provided callback for each file event.
pub fn watch_and_rebuild<F>(watcher: &FileWatcher, mut on_event: F) -> Result<()>
where
    F: FnMut(FileEvent) -> std::result::Result<(), Box<dyn std::error::Error>>,
{
    while let Some(event) = watcher.next_event() {
        if let Err(e) = on_event(event) {
            eprintln!("Error handling file event: {}", e);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::thread;
    use tempfile::tempdir;

    #[test]
    fn test_watcher_config_default() {
        let config = WatcherConfig::default();
        assert_eq!(config.debounce_duration, Duration::from_millis(300));
        assert!(config.extensions.contains(&"rs".to_string()));
        assert!(config.use_gitignore);
    }

    #[test]
    fn test_file_event_path() {
        let path = PathBuf::from("/test/file.rs");

        let created = FileEvent::Created(path.clone());
        assert_eq!(created.path(), path.as_path());

        let modified = FileEvent::Modified(path.clone());
        assert_eq!(modified.path(), path.as_path());

        let deleted = FileEvent::Deleted(path.clone());
        assert_eq!(deleted.path(), path.as_path());
    }

    #[test]
    fn test_watcher_creation() {
        let temp_dir = tempdir().expect("Failed to create temp dir");
        let watcher = FileWatcher::new(temp_dir.path());
        assert!(watcher.is_ok(), "Should create watcher successfully");

        let watcher = watcher.unwrap();
        assert_eq!(watcher.root(), temp_dir.path());
    }

    #[test]
    fn test_watcher_detects_file_changes() {
        let temp_dir = tempdir().expect("Failed to create temp dir");
        let watcher = FileWatcher::new(temp_dir.path()).expect("Should create watcher");

        // Create a Rust file
        let file_path = temp_dir.path().join("test.rs");
        fs::write(&file_path, "fn main() {}").expect("Failed to write file");

        // Wait for debounce
        thread::sleep(Duration::from_millis(500));

        // Should detect the file creation/modification
        // May receive multiple events, look for the one matching our file
        let mut found = false;
        for _ in 0..10 {
            if let Some(event) = watcher.try_next_event() {
                // Canonicalize paths to handle symlink differences (macOS /var vs /private/var)
                let event_path = event.path().canonicalize().ok();
                let expected_path = file_path.canonicalize().ok();

                if event_path == expected_path {
                    found = true;
                    match event {
                        FileEvent::Modified(_) | FileEvent::Created(_) => {}
                        FileEvent::Deleted(_) => panic!("Expected Modified/Created, got Deleted"),
                    }
                    break;
                }
            }
            thread::sleep(Duration::from_millis(100));
        }

        assert!(found, "Should detect file creation");
    }

    #[test]
    fn test_watcher_ignores_non_source_files() {
        let temp_dir = tempdir().expect("Failed to create temp dir");
        let watcher = FileWatcher::new(temp_dir.path()).expect("Should create watcher");

        // Create a non-source file
        let file_path = temp_dir.path().join("readme.txt");
        fs::write(&file_path, "Hello").expect("Failed to write file");

        // Wait for debounce
        thread::sleep(Duration::from_millis(500));

        // Drain all events - none should be for the .txt file
        // (directory events may occur on some platforms when files are created)
        while let Some(event) = watcher.try_next_event() {
            let path = event.path();
            assert!(
                path.is_dir() || path.extension().map(|e| e != "txt").unwrap_or(true),
                "Should ignore .txt files, got event for: {:?}",
                path
            );
        }
    }

    #[test]
    fn test_should_ignore() {
        let temp_dir = tempdir().expect("Failed to create temp dir");

        // Create a custom config
        let config = WatcherConfig {
            extensions: vec!["rs".to_string()],
            ignore_patterns: vec!["ignored_dir".to_string()],
            ..Default::default()
        };

        let watcher =
            FileWatcher::with_config(temp_dir.path(), config).expect("Should create watcher");

        // .rs files should not be ignored
        let rs_file = temp_dir.path().join("test.rs");
        assert!(
            !watcher.should_ignore(&rs_file),
            "Should not ignore .rs files"
        );

        // .txt files should be ignored (not in extensions list)
        let txt_file = temp_dir.path().join("test.txt");
        assert!(watcher.should_ignore(&txt_file), "Should ignore .txt files");
    }
}
