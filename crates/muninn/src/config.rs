//! Configuration file support for Muninn.
//!
//! All muninn data is stored in a `.muninn/` directory:
//! - `.muninn/config.toml` - Configuration file
//! - `.muninn/graph.db` - Code graph database
//! - `.muninn/logs/` - Log files (future)
//!
//! Config discovery searches for `.muninn/config.toml` starting from the current
//! directory and walking up to parent directories.

use std::path::{Path, PathBuf};

/// The muninn data directory name.
pub const MUNINN_DIR: &str = ".muninn";
/// The config file name within the muninn directory.
pub const CONFIG_FILE: &str = "config.toml";

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Main configuration structure.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
#[derive(Default)]
pub struct Config {
    /// Project settings.
    pub project: ProjectConfig,
    /// Graph/index settings.
    pub graph: GraphConfig,
    /// LLM backend settings (deprecated - use router/rlm sections).
    #[serde(default)]
    pub backend: BackendConfig,
    /// Groq-specific settings.
    pub groq: GroqProviderConfig,
    /// Anthropic-specific settings.
    pub anthropic: AnthropicProviderConfig,
    /// Router settings.
    pub router: RouterConfig,
    /// RLM (Recursive Language Model) settings.
    pub rlm: RlmConfig,
    /// Budget settings for recursive exploration.
    pub budget: BudgetConfig,
}

/// Project configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct ProjectConfig {
    /// Root directory of the project.
    pub root: PathBuf,
}

impl Default for ProjectConfig {
    fn default() -> Self {
        Self {
            root: PathBuf::from("."),
        }
    }
}

/// Graph/index configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct GraphConfig {
    /// Path to the graph database.
    pub path: PathBuf,
    /// File extensions to index.
    pub extensions: Vec<String>,
}

impl Default for GraphConfig {
    fn default() -> Self {
        Self {
            // Relative to .muninn/ directory
            path: PathBuf::from("graph.db"),
            extensions: vec![
                "rs".to_string(),
                "py".to_string(),
                "ts".to_string(),
                "js".to_string(),
                "go".to_string(),
                "c".to_string(),
                "cpp".to_string(),
                "h".to_string(),
            ],
        }
    }
}

/// Backend configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct BackendConfig {
    /// Backend type: "groq", "anthropic", or "local".
    #[serde(rename = "type")]
    pub backend_type: String,
    /// Model to use (optional, backend-specific default).
    pub model: Option<String>,
    /// API base URL override.
    pub base_url: Option<String>,
    /// Path to local model file (for "local" backend).
    pub model_path: Option<PathBuf>,
}

impl Default for BackendConfig {
    fn default() -> Self {
        Self {
            backend_type: "groq".to_string(),
            model: None,
            base_url: None,
            model_path: None,
        }
    }
}

/// Router configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct RouterConfig {
    /// Routing strategy: "llm", "always-rlm", "always-passthrough".
    pub strategy: String,
    /// Enable/disable routing.
    pub enabled: bool,
    /// Provider for LLM-based routing: "groq", "anthropic", "local".
    pub provider: String,
    /// Model to use for LLM-based routing.
    pub model: String,
}

impl Default for RouterConfig {
    fn default() -> Self {
        Self {
            strategy: "llm".to_string(),
            enabled: true,
            provider: "groq".to_string(),
            model: "llama-3.1-8b-instant".to_string(),
        }
    }
}

/// RLM (Recursive Language Model) configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct RlmConfig {
    /// Provider for RLM exploration: "groq", "anthropic", "local".
    pub provider: String,
    /// Model to use for recursive exploration.
    pub model: String,
}

impl Default for RlmConfig {
    fn default() -> Self {
        Self {
            provider: "groq".to_string(),
            model: "qwen/qwen3-32b".to_string(),
        }
    }
}

/// Budget configuration for recursive exploration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct BudgetConfig {
    /// Maximum tokens across all recursive calls.
    pub max_tokens: u32,
    /// Maximum recursion depth.
    pub max_depth: u32,
    /// Maximum tool calls per exploration.
    pub max_tool_calls: u32,
    /// Maximum duration in seconds.
    pub max_duration_secs: u64,
}

impl Default for BudgetConfig {
    fn default() -> Self {
        Self {
            max_tokens: 100_000,
            max_depth: 5,
            max_tool_calls: 50,
            max_duration_secs: 300,
        }
    }
}

/// Groq provider configuration.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct GroqProviderConfig {
    /// Groq API key.
    pub api_key: Option<String>,
    /// API base URL override.
    pub base_url: Option<String>,
}

/// Anthropic provider configuration.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct AnthropicProviderConfig {
    /// Anthropic API key.
    pub api_key: Option<String>,
    /// API base URL override.
    pub base_url: Option<String>,
}

impl Config {
    /// Load configuration from a file.
    pub fn from_file(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file: {}", path.display()))?;
        let config: Config = toml::from_str(&content)
            .with_context(|| format!("Failed to parse config file: {}", path.display()))?;
        Ok(config)
    }

    /// Find and load configuration from current or parent directories.
    ///
    /// Searches for `.muninn/config.toml` starting from the current directory
    /// and walking up to parent directories.
    pub fn find_and_load() -> Result<Option<(Self, PathBuf)>> {
        let current = std::env::current_dir()?;
        Self::find_and_load_from(&current)
    }

    /// Find and load configuration starting from a specific directory.
    ///
    /// Looks for `.muninn/config.toml` in the directory and its parents.
    pub fn find_and_load_from(start: &Path) -> Result<Option<(Self, PathBuf)>> {
        let mut dir = start.to_path_buf();

        loop {
            // Look for .muninn/config.toml
            let muninn_dir = dir.join(MUNINN_DIR);
            let config_path = muninn_dir.join(CONFIG_FILE);
            if config_path.exists() {
                let config = Self::from_file(&config_path)?;
                // Return the .muninn directory, not the config file
                return Ok(Some((config, muninn_dir)));
            }

            if !dir.pop() {
                break;
            }
        }

        Ok(None)
    }

    /// Load configuration or use defaults.
    #[allow(dead_code)]
    pub fn load_or_default() -> Self {
        match Self::find_and_load() {
            Ok(Some((config, path))) => {
                tracing::info!("Loaded config from {}", path.display());
                config
            }
            Ok(None) => {
                tracing::debug!("No .muninn/config.toml found, using defaults");
                Self::default()
            }
            Err(e) => {
                tracing::warn!("Failed to load config: {}, using defaults", e);
                Self::default()
            }
        }
    }

    /// Resolve the graph path relative to the .muninn directory.
    pub fn resolve_graph_path(&self, muninn_dir: Option<&Path>) -> PathBuf {
        if self.graph.path.is_absolute() {
            self.graph.path.clone()
        } else if let Some(dir) = muninn_dir {
            dir.join(&self.graph.path)
        } else {
            // Fall back to .muninn in current directory
            PathBuf::from(MUNINN_DIR).join(&self.graph.path)
        }
    }

    /// Get the path to the .muninn directory for a given base path.
    #[allow(dead_code)]
    pub fn muninn_dir(base: &Path) -> PathBuf {
        base.join(MUNINN_DIR)
    }

    /// Get the config file path for a given .muninn directory.
    #[allow(dead_code)]
    pub fn config_path(muninn_dir: &Path) -> PathBuf {
        muninn_dir.join(CONFIG_FILE)
    }
}

/// Configuration validation error.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ConfigValidationError {
    pub field: String,
    pub message: String,
}

impl std::fmt::Display for ConfigValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.field, self.message)
    }
}

impl std::error::Error for ConfigValidationError {}

impl Config {
    /// Validate the configuration.
    ///
    /// Returns a list of validation errors if any are found.
    #[allow(dead_code)]
    pub fn validate(&self) -> Vec<ConfigValidationError> {
        let mut errors = Vec::new();

        // Validate router provider
        if !["groq", "anthropic", "local"].contains(&self.router.provider.as_str()) {
            errors.push(ConfigValidationError {
                field: "router.provider".to_string(),
                message: format!(
                    "Invalid provider '{}'. Expected 'groq', 'anthropic', or 'local'.",
                    self.router.provider
                ),
            });
        }

        // Validate router model is not empty
        if self.router.model.is_empty() {
            errors.push(ConfigValidationError {
                field: "router.model".to_string(),
                message: "Router model cannot be empty.".to_string(),
            });
        }

        // Validate RLM provider
        if !["groq", "anthropic", "local"].contains(&self.rlm.provider.as_str()) {
            errors.push(ConfigValidationError {
                field: "rlm.provider".to_string(),
                message: format!(
                    "Invalid provider '{}'. Expected 'groq', 'anthropic', or 'local'.",
                    self.rlm.provider
                ),
            });
        }

        // Validate RLM model is not empty
        if self.rlm.model.is_empty() {
            errors.push(ConfigValidationError {
                field: "rlm.model".to_string(),
                message: "RLM model cannot be empty.".to_string(),
            });
        }

        // Validate router strategy
        let valid_strategies = [
            "llm",
            "always-rlm",
            "always-passthrough",
            "heuristic",
            "hybrid",
        ];
        if !valid_strategies.contains(&self.router.strategy.as_str()) {
            errors.push(ConfigValidationError {
                field: "router.strategy".to_string(),
                message: format!(
                    "Invalid strategy '{}'. Expected one of: {}.",
                    self.router.strategy,
                    valid_strategies.join(", ")
                ),
            });
        }

        // Check for provider-specific configuration
        if (self.router.provider == "groq" || self.rlm.provider == "groq")
            && self.groq.api_key.is_none()
            && std::env::var("GROQ_API_KEY").is_err()
        {
            errors.push(ConfigValidationError {
                    field: "groq.api_key".to_string(),
                    message: "Groq API key required for router/RLM. Set [groq] api_key or GROQ_API_KEY env var.".to_string(),
                });
        }

        if (self.router.provider == "anthropic" || self.rlm.provider == "anthropic")
            && self.anthropic.api_key.is_none()
            && std::env::var("ANTHROPIC_API_KEY").is_err()
        {
            errors.push(ConfigValidationError {
                    field: "anthropic.api_key".to_string(),
                    message: "Anthropic API key required for router/RLM. Set [anthropic] api_key or ANTHROPIC_API_KEY env var.".to_string(),
                });
        }

        errors
    }

    /// Check if the deprecated [backend] section is being used.
    pub fn has_deprecated_backend_config(&self) -> bool {
        // Check if backend has non-default values
        let default = BackendConfig::default();
        self.backend.backend_type != default.backend_type
            || self.backend.model != default.model
            || self.backend.base_url != default.base_url
            || self.backend.model_path != default.model_path
    }

    /// Emit a deprecation warning if using old [backend] section.
    pub fn warn_deprecated_backend(&self) {
        if self.has_deprecated_backend_config() {
            tracing::warn!(
                "The [backend] config section is deprecated. \
                 Use [router] and [rlm] sections instead:\n  \
                 [router]\n  \
                 provider = \"groq\"\n  \
                 model = \"llama-3.1-8b-instant\"\n\n  \
                 [rlm]\n  \
                 provider = \"groq\"\n  \
                 model = \"qwen/qwen3-32b\""
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.backend.backend_type, "groq");
        assert_eq!(config.router.strategy, "llm");
        assert_eq!(config.router.provider, "groq");
        assert_eq!(config.router.model, "llama-3.1-8b-instant");
        assert_eq!(config.rlm.provider, "groq");
        assert_eq!(config.rlm.model, "qwen/qwen3-32b");
        assert_eq!(config.budget.max_depth, 5);
    }

    #[test]
    fn test_parse_minimal_config() {
        let toml = r#"
[backend]
type = "anthropic"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.backend.backend_type, "anthropic");
        // Defaults should still apply
        assert_eq!(config.router.strategy, "llm");
    }

    #[test]
    fn test_parse_full_config() {
        let toml = r#"
[project]
root = "/home/user/myproject"

[graph]
path = "code.db"
extensions = ["rs", "py"]

[router]
strategy = "llm"
enabled = true
provider = "groq"
model = "llama-3.1-8b-instant"

[rlm]
provider = "groq"
model = "qwen/qwen3-32b"

[budget]
max_tokens = 50000
max_depth = 3
max_tool_calls = 20
max_duration_secs = 120
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.project.root, PathBuf::from("/home/user/myproject"));
        assert_eq!(config.graph.path, PathBuf::from("code.db"));
        assert_eq!(config.graph.extensions, vec!["rs", "py"]);
        assert_eq!(config.router.strategy, "llm");
        assert_eq!(config.router.provider, "groq");
        assert_eq!(config.router.model, "llama-3.1-8b-instant");
        assert_eq!(config.rlm.provider, "groq");
        assert_eq!(config.rlm.model, "qwen/qwen3-32b");
        assert_eq!(config.budget.max_tokens, 50000);
        assert_eq!(config.budget.max_depth, 3);
    }

    #[test]
    fn test_default_graph_path() {
        let config = Config::default();
        // Default is relative, resolved within .muninn/
        assert_eq!(config.graph.path, PathBuf::from("graph.db"));
    }

    #[test]
    fn test_resolve_graph_path() {
        let config = Config::default();
        let muninn_dir = PathBuf::from("/project/.muninn");
        let resolved = config.resolve_graph_path(Some(&muninn_dir));
        assert_eq!(resolved, PathBuf::from("/project/.muninn/graph.db"));
    }

    #[test]
    fn test_validate_invalid_provider() {
        let mut config = Config::default();
        config.router.provider = "invalid".to_string();

        let errors = config.validate();
        assert!(errors.iter().any(|e| e.field == "router.provider"));
    }

    #[test]
    fn test_validate_empty_model() {
        let mut config = Config::default();
        config.rlm.model = "".to_string();

        let errors = config.validate();
        assert!(errors.iter().any(|e| e.field == "rlm.model"));
    }

    #[test]
    fn test_deprecated_backend_detection() {
        let mut config = Config::default();
        assert!(!config.has_deprecated_backend_config());

        config.backend.model = Some("custom-model".to_string());
        assert!(config.has_deprecated_backend_config());
    }
}
