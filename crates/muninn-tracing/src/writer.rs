//! Trace writer for JSONL file persistence.
//!
//! Supports two modes:
//! - **Session mode**: Writes to a single file (e.g., `session_dir/traces.jsonl`)
//! - **Daily rotation**: Writes to dated files (e.g., `traces/2026-01-11.jsonl`)

use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::Utc;

use crate::types::Trace;

/// Error type for trace writing operations.
#[derive(Debug, thiserror::Error)]
pub enum WriteError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

/// Configuration for the trace writer.
#[derive(Debug, Clone)]
pub struct WriterConfig {
    /// Path for trace output.
    /// - Session mode: Full path to trace file (e.g., `session_dir/traces.jsonl`)
    /// - Daily rotation: Directory for dated files (e.g., `traces/`)
    pub trace_path: PathBuf,

    /// Whether tracing is enabled.
    pub enabled: bool,

    /// Session mode writes to a single file; daily rotation writes to dated files.
    pub session_mode: bool,
}

impl Default for WriterConfig {
    fn default() -> Self {
        Self::daily_rotation(".muninn/traces")
    }
}

impl WriterConfig {
    /// Create a config for session-based logging (single file).
    pub fn session(trace_file: impl Into<PathBuf>) -> Self {
        Self {
            trace_path: trace_file.into(),
            enabled: true,
            session_mode: true,
        }
    }

    /// Create a config for daily rotation (legacy mode).
    pub fn daily_rotation(trace_dir: impl Into<PathBuf>) -> Self {
        Self {
            trace_path: trace_dir.into(),
            enabled: true,
            session_mode: false,
        }
    }

    /// Create a new config with the given trace directory (legacy API).
    pub fn new(trace_dir: impl Into<PathBuf>) -> Self {
        Self::daily_rotation(trace_dir)
    }

    /// Disable tracing.
    pub fn disabled() -> Self {
        Self {
            trace_path: PathBuf::new(),
            enabled: false,
            session_mode: false,
        }
    }
}

/// Writes traces to JSONL files.
///
/// Thread-safe via internal mutex.
pub struct TraceWriter {
    config: WriterConfig,
    current_file: Mutex<Option<CurrentFile>>,
}

struct CurrentFile {
    /// For daily rotation: the date string. For session mode: "session".
    key: String,
    writer: BufWriter<File>,
}

impl TraceWriter {
    /// Create a new trace writer with the given configuration.
    pub fn new(config: WriterConfig) -> Result<Self, WriteError> {
        if config.enabled {
            if config.session_mode {
                // Session mode: create parent directory of trace file
                if let Some(parent) = config.trace_path.parent() {
                    fs::create_dir_all(parent)?;
                }
            } else {
                // Daily rotation: create trace directory
                fs::create_dir_all(&config.trace_path)?;
            }
        }

        Ok(Self {
            config,
            current_file: Mutex::new(None),
        })
    }

    /// Create a trace writer with default configuration.
    pub fn with_defaults() -> Result<Self, WriteError> {
        Self::new(WriterConfig::default())
    }

    /// Write a trace to the appropriate file.
    pub fn write(&self, trace: &Trace) -> Result<(), WriteError> {
        if !self.config.enabled {
            return Ok(());
        }

        let mut guard = self.current_file.lock().unwrap();

        if self.config.session_mode {
            // Session mode: write to single file
            self.write_session_mode(&mut guard, trace)?;
        } else {
            // Daily rotation mode
            self.write_daily_mode(&mut guard, trace)?;
        }

        Ok(())
    }

    fn write_session_mode(
        &self,
        guard: &mut Option<CurrentFile>,
        trace: &Trace,
    ) -> Result<(), WriteError> {
        // Open file if not already open
        if guard.is_none() {
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.config.trace_path)?;
            *guard = Some(CurrentFile {
                key: "session".to_string(),
                writer: BufWriter::new(file),
            });
        }

        if let Some(ref mut cf) = *guard {
            let line = serde_json::to_string(trace)?;
            writeln!(cf.writer, "{}", line)?;
            cf.writer.flush()?;
        }

        Ok(())
    }

    fn write_daily_mode(
        &self,
        guard: &mut Option<CurrentFile>,
        trace: &Trace,
    ) -> Result<(), WriteError> {
        let today = Utc::now().format("%Y-%m-%d").to_string();

        // Check if we need to rotate to a new file
        let needs_new_file = match &*guard {
            None => true,
            Some(cf) => cf.key != today,
        };

        if needs_new_file {
            let file_path = self.config.trace_path.join(format!("{}.jsonl", today));
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&file_path)?;
            *guard = Some(CurrentFile {
                key: today,
                writer: BufWriter::new(file),
            });
        }

        if let Some(ref mut cf) = *guard {
            let line = serde_json::to_string(trace)?;
            writeln!(cf.writer, "{}", line)?;
            cf.writer.flush()?;
        }

        Ok(())
    }

    /// Get the path to the current trace file.
    pub fn current_file_path(&self) -> PathBuf {
        if self.config.session_mode {
            self.config.trace_path.clone()
        } else {
            let today = Utc::now().format("%Y-%m-%d").to_string();
            self.config.trace_path.join(format!("{}.jsonl", today))
        }
    }

    /// List all trace files in the trace directory.
    /// Note: Only works for daily rotation mode.
    pub fn list_trace_files(&self) -> Result<Vec<PathBuf>, WriteError> {
        if !self.config.enabled {
            return Ok(Vec::new());
        }

        if self.config.session_mode {
            // In session mode, return the single trace file if it exists
            if self.config.trace_path.exists() {
                return Ok(vec![self.config.trace_path.clone()]);
            }
            return Ok(Vec::new());
        }

        let mut files: Vec<PathBuf> = fs::read_dir(&self.config.trace_path)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|ext| ext == "jsonl"))
            .collect();

        files.sort();
        Ok(files)
    }

    /// Read traces from a specific file.
    pub fn read_traces(path: &Path) -> Result<Vec<Trace>, WriteError> {
        let content = fs::read_to_string(path)?;
        let traces: Result<Vec<Trace>, _> = content
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(serde_json::from_str)
            .collect();
        Ok(traces?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_write_and_read_trace_daily() {
        let dir = tempdir().unwrap();
        let config = WriterConfig::daily_rotation(dir.path().join("traces"));
        let writer = TraceWriter::new(config).unwrap();

        let mut trace = Trace::new("test-trace-1");
        trace.complete();

        writer.write(&trace).unwrap();

        let files = writer.list_trace_files().unwrap();
        assert_eq!(files.len(), 1);

        let traces = TraceWriter::read_traces(&files[0]).unwrap();
        assert_eq!(traces.len(), 1);
        assert_eq!(traces[0].trace_id, "test-trace-1");
    }

    #[test]
    fn test_write_and_read_trace_session() {
        let dir = tempdir().unwrap();
        let trace_file = dir.path().join("traces.jsonl");
        let config = WriterConfig::session(&trace_file);
        let writer = TraceWriter::new(config).unwrap();

        let mut trace = Trace::new("test-trace-session");
        trace.complete();

        writer.write(&trace).unwrap();

        // Verify file path
        assert_eq!(writer.current_file_path(), trace_file);

        // Verify file exists and has content
        let files = writer.list_trace_files().unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0], trace_file);

        let traces = TraceWriter::read_traces(&trace_file).unwrap();
        assert_eq!(traces.len(), 1);
        assert_eq!(traces[0].trace_id, "test-trace-session");
    }

    #[test]
    fn test_disabled_writer() {
        let config = WriterConfig::disabled();
        let writer = TraceWriter::new(config).unwrap();

        let mut trace = Trace::new("should-not-write");
        trace.complete();

        // Should not error
        writer.write(&trace).unwrap();

        // Should return empty list
        let files = writer.list_trace_files().unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn test_legacy_api_compatibility() {
        let dir = tempdir().unwrap();
        // Old API should still work
        let config = WriterConfig::new(dir.path().join("traces"));
        assert!(!config.session_mode);
        assert!(config.enabled);
    }
}
