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
    /// Default LLM provider/model. Router and RLM inherit from this when not
    /// explicitly overridden, so the out-of-the-box config is a single-model
    /// setup that works on free-tier Ollama Cloud.
    #[serde(default)]
    pub default: DefaultLlmConfig,
    /// Groq-specific settings.
    pub groq: GroqProviderConfig,
    /// Anthropic-specific settings.
    pub anthropic: AnthropicProviderConfig,
    /// Ollama-specific settings (covers both local and Ollama Cloud).
    #[serde(default)]
    pub ollama: OllamaProviderConfig,
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
///
/// `provider` and `model` are optional overrides. When unset, they inherit
/// from `[default]`. Downstream callers should consume the post-inheritance
/// view via [`Config::resolved_router`] rather than reading these fields
/// directly.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct RouterConfig {
    /// Routing strategy: "llm", "always-rlm", "always-passthrough".
    pub strategy: String,
    /// Enable/disable routing.
    pub enabled: bool,
    /// Provider override for LLM-based routing. If `None`, inherits from `[default]`.
    pub provider: Option<String>,
    /// Model override for LLM-based routing. If `None`, inherits from `[default]`.
    pub model: Option<String>,
}

impl Default for RouterConfig {
    fn default() -> Self {
        Self {
            strategy: "llm".to_string(),
            enabled: true,
            provider: None,
            model: None,
        }
    }
}

/// RLM (Recursive Language Model) configuration.
///
/// `provider` and `model` are optional overrides. When unset, they inherit
/// from `[default]`. Consume via [`Config::resolved_rlm`].
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct RlmConfig {
    /// Provider override for RLM exploration. If `None`, inherits from `[default]`.
    pub provider: Option<String>,
    /// Model override for recursive exploration. If `None`, inherits from `[default]`.
    pub model: Option<String>,
}

/// Default LLM provider/model baseline.
///
/// Router and RLM inherit from this when they don't set their own
/// `provider`/`model`. The out-of-the-box default is a single Ollama Cloud
/// model that serves both surfaces — works on the free tier (concurrent
/// model cap = 1) and maximises prompt-cache reuse across calls.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct DefaultLlmConfig {
    /// Provider name (e.g. "ollama", "groq", "anthropic").
    pub provider: String,
    /// Model identifier.
    pub model: String,
}

impl Default for DefaultLlmConfig {
    fn default() -> Self {
        Self {
            provider: "ollama".to_string(),
            model: "gemma4:31b".to_string(),
        }
    }
}

/// Fully-resolved LLM provider/model after applying inheritance from `[default]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedLlmConfig {
    pub provider: String,
    pub model: String,
}

/// Ollama provider configuration. Covers both local Ollama (no key required)
/// and Ollama Cloud (`https://ollama.com/v1`, requires `OLLAMA_API_KEY`).
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct OllamaProviderConfig {
    /// API key for Ollama Cloud. Falls back to the `OLLAMA_API_KEY` env var
    /// if unset here.
    pub api_key: Option<String>,
    /// Base URL override. If unset, defaults to Ollama Cloud
    /// (`https://ollama.com/v1`). Set to `http://localhost:11434/v1` for
    /// local Ollama.
    pub base_url: Option<String>,
}

/// Default Ollama Cloud base URL.
pub const OLLAMA_CLOUD_BASE_URL: &str = "https://ollama.com/v1";
/// Default local Ollama base URL. Exposed for docs and install tooling.
#[allow(dead_code)]
pub const OLLAMA_LOCAL_BASE_URL: &str = "http://localhost:11434/v1";

impl OllamaProviderConfig {
    /// Resolve the effective base URL, defaulting to Ollama Cloud.
    pub fn resolved_base_url(&self) -> &str {
        self.base_url.as_deref().unwrap_or(OLLAMA_CLOUD_BASE_URL)
    }

    /// Resolve the effective API key, consulting `OLLAMA_API_KEY` if the
    /// config value is unset.
    pub fn resolved_api_key(&self) -> Option<String> {
        self.api_key
            .clone()
            .or_else(|| std::env::var("OLLAMA_API_KEY").ok())
            .filter(|s| !s.is_empty())
    }

    /// True when the resolved base URL points at Ollama Cloud (or any
    /// non-localhost host), which means an API key is required.
    pub fn needs_api_key(&self) -> bool {
        let url = self.resolved_base_url();
        !(url.contains("localhost") || url.contains("127.0.0.1"))
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

    /// Resolve the router's effective provider+model after applying
    /// inheritance from `[default]`. Use this everywhere downstream — never
    /// read `Config.router.provider/model` directly.
    pub fn resolved_router(&self) -> ResolvedLlmConfig {
        ResolvedLlmConfig {
            provider: self
                .router
                .provider
                .clone()
                .unwrap_or_else(|| self.default.provider.clone()),
            model: self
                .router
                .model
                .clone()
                .unwrap_or_else(|| self.default.model.clone()),
        }
    }

    /// Resolve the RLM's effective provider+model after applying inheritance
    /// from `[default]`. Use this everywhere downstream.
    pub fn resolved_rlm(&self) -> ResolvedLlmConfig {
        ResolvedLlmConfig {
            provider: self
                .rlm
                .provider
                .clone()
                .unwrap_or_else(|| self.default.provider.clone()),
            model: self
                .rlm
                .model
                .clone()
                .unwrap_or_else(|| self.default.model.clone()),
        }
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

        let router = self.resolved_router();
        let rlm = self.resolved_rlm();
        let valid_providers = ["groq", "anthropic", "ollama", "local"];

        // Validate router provider
        if !valid_providers.contains(&router.provider.as_str()) {
            errors.push(ConfigValidationError {
                field: "router.provider".to_string(),
                message: format!(
                    "Invalid provider '{}'. Expected one of: {}.",
                    router.provider,
                    valid_providers.join(", ")
                ),
            });
        }

        if router.model.is_empty() {
            errors.push(ConfigValidationError {
                field: "router.model".to_string(),
                message: "Router model cannot be empty (set [router] model or [default] model)."
                    .to_string(),
            });
        }

        // Validate RLM provider
        if !valid_providers.contains(&rlm.provider.as_str()) {
            errors.push(ConfigValidationError {
                field: "rlm.provider".to_string(),
                message: format!(
                    "Invalid provider '{}'. Expected one of: {}.",
                    rlm.provider,
                    valid_providers.join(", ")
                ),
            });
        }

        if rlm.model.is_empty() {
            errors.push(ConfigValidationError {
                field: "rlm.model".to_string(),
                message: "RLM model cannot be empty (set [rlm] model or [default] model)."
                    .to_string(),
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
        if (router.provider == "groq" || rlm.provider == "groq")
            && self.groq.api_key.is_none()
            && std::env::var("GROQ_API_KEY").is_err()
        {
            errors.push(ConfigValidationError {
                    field: "groq.api_key".to_string(),
                    message: "Groq API key required for router/RLM. Set [groq] api_key or GROQ_API_KEY env var.".to_string(),
                });
        }

        if (router.provider == "anthropic" || rlm.provider == "anthropic")
            && self.anthropic.api_key.is_none()
            && std::env::var("ANTHROPIC_API_KEY").is_err()
        {
            errors.push(ConfigValidationError {
                    field: "anthropic.api_key".to_string(),
                    message: "Anthropic API key required for router/RLM. Set [anthropic] api_key or ANTHROPIC_API_KEY env var.".to_string(),
                });
        }

        if (router.provider == "ollama" || rlm.provider == "ollama")
            && self.ollama.needs_api_key()
            && self.ollama.resolved_api_key().is_none()
        {
            errors.push(ConfigValidationError {
                field: "ollama.api_key".to_string(),
                message: format!(
                    "Ollama Cloud API key required (resolved base_url = {}). \
                     Set [ollama] api_key, OLLAMA_API_KEY env var, or override [ollama] base_url \
                     to a local Ollama endpoint (e.g. http://localhost:11434/v1).",
                    self.ollama.resolved_base_url()
                ),
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
        // Router and RLM start with no explicit provider/model — they inherit
        // from [default], which lands them on Ollama Cloud + gemma4:31b.
        assert_eq!(config.router.provider, None);
        assert_eq!(config.router.model, None);
        assert_eq!(config.rlm.provider, None);
        assert_eq!(config.rlm.model, None);
        assert_eq!(config.default.provider, "ollama");
        assert_eq!(config.default.model, "gemma4:31b");
        assert_eq!(config.resolved_router().provider, "ollama");
        assert_eq!(config.resolved_router().model, "gemma4:31b");
        assert_eq!(config.resolved_rlm().provider, "ollama");
        assert_eq!(config.resolved_rlm().model, "gemma4:31b");
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

[default]
provider = "groq"
model = "llama-3.1-8b-instant"

[router]
strategy = "llm"
enabled = true

[rlm]
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
        // Router inherits both fields from [default].
        assert_eq!(config.resolved_router().provider, "groq");
        assert_eq!(config.resolved_router().model, "llama-3.1-8b-instant");
        // RLM overrides only the model; provider inherits.
        assert_eq!(config.resolved_rlm().provider, "groq");
        assert_eq!(config.resolved_rlm().model, "qwen/qwen3-32b");
        assert_eq!(config.budget.max_tokens, 50000);
        assert_eq!(config.budget.max_depth, 3);
    }

    #[test]
    fn test_inheritance_default_only() {
        // With no router/rlm overrides, both inherit from [default].
        let toml = r#"
[default]
provider = "anthropic"
model = "claude-haiku-4-5-20251001"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        let r = config.resolved_router();
        let m = config.resolved_rlm();
        assert_eq!(r.provider, "anthropic");
        assert_eq!(r.model, "claude-haiku-4-5-20251001");
        assert_eq!(m.provider, "anthropic");
        assert_eq!(m.model, "claude-haiku-4-5-20251001");
    }

    #[test]
    fn test_inheritance_router_override_beats_default() {
        let toml = r#"
[default]
provider = "ollama"
model = "gemma4:31b"

[router]
provider = "groq"
model = "llama-3.1-8b-instant"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.resolved_router().provider, "groq");
        assert_eq!(config.resolved_router().model, "llama-3.1-8b-instant");
        // RLM still inherits.
        assert_eq!(config.resolved_rlm().provider, "ollama");
        assert_eq!(config.resolved_rlm().model, "gemma4:31b");
    }

    #[test]
    fn test_inheritance_backwards_compat_no_default_section() {
        // Pre-tiered configs that only set [router]/[rlm] should still resolve
        // via the built-in DefaultLlmConfig fallback.
        let toml = r#"
[router]
provider = "groq"
model = "llama-3.1-8b-instant"

[rlm]
provider = "groq"
model = "qwen/qwen3-32b"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.resolved_router().provider, "groq");
        assert_eq!(config.resolved_rlm().model, "qwen/qwen3-32b");
        // The built-in DefaultLlmConfig is still present underneath.
        assert_eq!(config.default.provider, "ollama");
    }

    #[test]
    fn test_validate_requires_ollama_api_key_for_cloud() {
        // SAFETY: we don't actually want OLLAMA_API_KEY leaking in from the
        // host env into this assertion. Remove it for the duration of the test.
        let prev = std::env::var("OLLAMA_API_KEY").ok();
        // SAFETY: tests run in the same process; remove + restore the env var.
        unsafe {
            std::env::remove_var("OLLAMA_API_KEY");
        }
        let config = Config::default();
        let errors = config.validate();
        assert!(
            errors.iter().any(|e| e.field == "ollama.api_key"),
            "expected ollama.api_key error, got {:?}",
            errors
        );
        if let Some(v) = prev {
            // SAFETY: restoring the prior env var.
            unsafe {
                std::env::set_var("OLLAMA_API_KEY", v);
            }
        }
    }

    #[test]
    fn test_validate_local_ollama_keyless_ok() {
        let prev = std::env::var("OLLAMA_API_KEY").ok();
        // SAFETY: see test above.
        unsafe {
            std::env::remove_var("OLLAMA_API_KEY");
        }
        let mut config = Config::default();
        config.ollama.base_url = Some("http://localhost:11434/v1".to_string());
        let errors = config.validate();
        assert!(
            !errors.iter().any(|e| e.field == "ollama.api_key"),
            "local ollama should not require an api key; got {:?}",
            errors
        );
        if let Some(v) = prev {
            // SAFETY: restoring.
            unsafe {
                std::env::set_var("OLLAMA_API_KEY", v);
            }
        }
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
        config.router.provider = Some("invalid".to_string());

        let errors = config.validate();
        assert!(errors.iter().any(|e| e.field == "router.provider"));
    }

    #[test]
    fn test_validate_empty_model() {
        let mut config = Config::default();
        // An empty override propagates through inheritance as the resolved
        // RLM model, so the validator should catch it.
        config.rlm.model = Some("".to_string());

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
