//! Session management for proxy runs.
//!
//! Each proxy run gets a unique session ID and directory for isolated logging.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Unique identifier for a proxy session.
///
/// Format: `YYYY-MM-DDTHH-MM-SS_XXXX` where XXXX is a short UUID suffix.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionId(String);

impl SessionId {
    /// Generate a new session ID with current timestamp and random suffix.
    pub fn generate() -> Self {
        let now = Utc::now();
        let short_uuid = &uuid::Uuid::new_v4().to_string()[..4];
        Self(format!(
            "{}_{}",
            now.format("%Y-%m-%dT%H-%M-%S"),
            short_uuid
        ))
    }

    /// Create a session ID from a string (for testing or restoration).
    #[allow(dead_code)]
    pub fn from_string(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    /// Get the session ID as a string.
    #[allow(dead_code)]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Get the session directory path for a given session ID.
pub fn session_dir(muninn_dir: &Path, session_id: &SessionId) -> PathBuf {
    muninn_dir.join("sessions").join(&session_id.0)
}

/// Metadata about a proxy session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMetadata {
    /// The session ID.
    pub session_id: String,

    /// When the session started.
    pub started_at: DateTime<Utc>,

    /// Working directory for the session.
    pub work_dir: PathBuf,

    /// Router strategy being used.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub router_strategy: Option<String>,

    /// RLM model being used.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rlm_model: Option<String>,
}

impl SessionMetadata {
    /// Create new session metadata.
    pub fn new(session_id: &SessionId, work_dir: PathBuf) -> Self {
        Self {
            session_id: session_id.to_string(),
            started_at: Utc::now(),
            work_dir,
            router_strategy: None,
            rlm_model: None,
        }
    }

    /// Set the router strategy.
    pub fn with_router_strategy(mut self, strategy: impl Into<String>) -> Self {
        self.router_strategy = Some(strategy.into());
        self
    }

    /// Set the RLM model.
    pub fn with_rlm_model(mut self, model: impl Into<String>) -> Self {
        self.rlm_model = Some(model.into());
        self
    }
}

/// Write session metadata to the session directory.
pub fn write_metadata(session_dir: &Path, metadata: &SessionMetadata) -> anyhow::Result<()> {
    let path = session_dir.join("session.json");
    let json = serde_json::to_string_pretty(metadata)?;
    fs::write(&path, json)?;
    Ok(())
}

/// Read session metadata from a session directory.
#[allow(dead_code)]
pub fn read_metadata(session_dir: &Path) -> anyhow::Result<SessionMetadata> {
    let path = session_dir.join("session.json");
    let json = fs::read_to_string(&path)?;
    let metadata: SessionMetadata = serde_json::from_str(&json)?;
    Ok(metadata)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_session_id_format() {
        let id = SessionId::generate();
        let s = id.to_string();

        // Format: YYYY-MM-DDTHH-MM-SS_XXXX
        assert!(s.len() >= 24, "Session ID too short: {}", s);
        assert!(s.contains('T'), "Missing T separator: {}", s);
        assert!(s.contains('_'), "Missing UUID separator: {}", s);
    }

    #[test]
    fn test_session_dir_path() {
        let muninn_dir = Path::new("/tmp/.muninn");
        let session_id = SessionId::from_string("2026-01-11T17-34-52_a3f2");

        let dir = session_dir(muninn_dir, &session_id);
        assert_eq!(
            dir,
            PathBuf::from("/tmp/.muninn/sessions/2026-01-11T17-34-52_a3f2")
        );
    }

    #[test]
    fn test_metadata_roundtrip() {
        let dir = tempdir().unwrap();
        let session_id = SessionId::generate();
        let metadata = SessionMetadata::new(&session_id, PathBuf::from("/test/project"))
            .with_router_strategy("llm")
            .with_rlm_model("claude-sonnet");

        write_metadata(dir.path(), &metadata).unwrap();
        let loaded = read_metadata(dir.path()).unwrap();

        assert_eq!(loaded.session_id, metadata.session_id);
        assert_eq!(loaded.work_dir, metadata.work_dir);
        assert_eq!(loaded.router_strategy, Some("llm".to_string()));
        assert_eq!(loaded.rlm_model, Some("claude-sonnet".to_string()));
    }
}
