//! muninn: Privacy-first recursive context gateway
//!
//! Muninn sits between your coding agent (like Claude Code) and local LLMs,
//! providing intelligent request routing and deep context exploration.

mod config;
mod session;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing::{debug, info};
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

/// Known agent commands that trigger passthrough mode.
/// When any of these appear in argv, everything after is passed to the agent.
const AGENT_COMMANDS: &[&str] = &["claude", "cursor", "aider"];

/// Split command line args at agent command boundary.
/// Returns (muninn_args, Option<(agent_cmd, agent_args)>)
fn split_args_at_agent() -> (Vec<String>, Option<(String, Vec<String>)>) {
    let args: Vec<String> = std::env::args().collect();

    // Find where an agent command starts
    if let Some(idx) = args
        .iter()
        .position(|a| AGENT_COMMANDS.contains(&a.as_str()))
    {
        let muninn_args = args[..idx].to_vec();
        let agent_cmd = args[idx].clone();
        let agent_args = args[idx + 1..].to_vec();
        (muninn_args, Some((agent_cmd, agent_args)))
    } else {
        (args, None)
    }
}

use config::Config;
use muninn_graph::doc_store::{DocStore, Ecosystem};
use muninn_graph::registry::{
    IndexerConfig, LlmsTxtIndexer, LlmsTxtIndexerConfig, PyDocIndexer, PyIndexerConfig,
    RustDocIndexer,
};
use muninn_graph::{FileEvent, FileWatcher, GraphBuilder, GraphStore};
use muninn_rlm::{
    AnthropicBackend, AnthropicConfig, BudgetConfig as RlmBudgetConfig, FileTokenManager,
    GroqBackend, GroqConfig, OAuthConfig, OllamaBackend, OllamaConfig, PkceChallenge, ProxyConfig,
    ProxyServer, RouterConfig, RouterStrategy, SharedDocStore, SharedGraphStore, TokenManager,
    ToolRegistry, build_authorization_url, create_doc_tools, create_fs_tools, create_graph_tools,
    create_token_manager, exchange_code_for_tokens, generate_state, parse_code_state,
    wrap_doc_store, wrap_store,
};

/// Convert config budget to RLM budget type.
fn config_to_rlm_budget(config: &config::BudgetConfig) -> RlmBudgetConfig {
    RlmBudgetConfig {
        max_tokens: Some(config.max_tokens as u64),
        max_depth: Some(config.max_depth),
        max_tool_calls: Some(config.max_tool_calls),
        max_duration_secs: Some(config.max_duration_secs),
    }
}

/// Create a backend from provider and model configuration.
///
/// Returns None if required credentials are missing.
fn create_backend_from_config(
    provider: &str,
    model: &str,
    config: &Config,
    _config_dir: Option<&std::path::Path>,
) -> Result<Option<Arc<dyn muninn_rlm::LLMBackend>>> {
    match provider {
        "groq" => {
            let key = config
                .groq
                .api_key
                .clone()
                .or_else(|| std::env::var("GROQ_API_KEY").ok());
            match key {
                Some(k) => {
                    let groq_config = GroqConfig::new(k).with_model(model);
                    Ok(Some(Arc::new(GroqBackend::new(groq_config)?)))
                }
                None => Ok(None),
            }
        }
        "anthropic" => {
            let key = config
                .anthropic
                .api_key
                .clone()
                .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok());
            match key {
                Some(k) => Ok(Some(Arc::new(AnthropicBackend::new(
                    AnthropicConfig::new(k),
                )?))),
                None => Ok(None),
            }
        }
        "ollama" => {
            // Resolve base_url + api_key from [ollama] (with env var fallback
            // for the key). Local Ollama works keyless; Ollama Cloud requires
            // OLLAMA_API_KEY and is the new default base_url.
            let base_url = config.ollama.resolved_base_url().to_string();
            let api_key = config.ollama.resolved_api_key();
            if config.ollama.needs_api_key() && api_key.is_none() {
                // The validator already surfaces this, but guard the factory
                // too so we never silently hit cloud without credentials.
                return Ok(None);
            }
            let mut ollama_config = OllamaConfig::new()
                .with_base_url(base_url)
                .with_model(model);
            if let Some(k) = api_key {
                ollama_config = ollama_config.with_api_key(k);
            }
            Ok(Some(Arc::new(OllamaBackend::new(ollama_config)?)))
        }
        other => {
            anyhow::bail!("Unknown provider: {}", other)
        }
    }
}

/// Privacy-first recursive context gateway for agentic coding
///
/// Usage with agents: `muninn [OPTIONS] <agent> [AGENT_ARGS]...`
/// Example: `muninn --verbose claude -c` runs claude with -c flag
#[derive(Parser)]
#[command(name = "muninn")]
#[command(version, about, long_about = None)]
struct Cli {
    /// Enable verbose logging
    #[arg(short, long, global = true)]
    verbose: bool,

    /// Path to .muninn directory (default: search for .muninn/config.toml)
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    /// Port for the proxy server when launching agents (0 = auto-select)
    #[arg(long, global = true, default_value = "0")]
    port: u16,

    /// Groq API key for RLM backend (or use GROQ_API_KEY env var)
    #[arg(long, global = true, env = "GROQ_API_KEY")]
    groq_key: Option<String>,

    /// Routing strategy: heuristic, llm, hybrid, always-rlm, always-passthrough
    #[arg(long, global = true)]
    router: Option<String>,

    /// Working directory for file operations
    #[arg(long, global = true)]
    workdir: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the proxy server standalone (without launching an agent)
    Proxy {
        /// Host to bind to
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
    },

    /// Build or update the code graph index
    Index {
        /// Directory to index (default: project root from config or current dir)
        #[arg(long)]
        path: Option<PathBuf>,

        /// Output path for the graph database
        #[arg(long)]
        output: Option<PathBuf>,

        /// Watch for changes and update incrementally
        #[arg(long)]
        watch: bool,
    },

    /// Initialize a new .muninn directory with config file
    Init {
        /// Force overwrite existing config
        #[arg(long)]
        force: bool,
    },

    /// Authenticate with Claude MAX subscription (OAuth flow)
    #[command(name = "oauth")]
    Auth {
        /// Show current token status instead of re-authenticating
        #[arg(long)]
        status: bool,

        /// Delete stored OAuth tokens
        #[arg(long)]
        logout: bool,
    },

    /// Manage library documentation index
    Docs {
        #[command(subcommand)]
        command: DocsCommand,
    },
}

/// Subcommands for documentation management.
#[derive(Subcommand)]
enum DocsCommand {
    /// Index a Rust crate from crates.io
    #[command(name = "index-crate")]
    IndexCrate {
        /// Name of the crate to index (e.g., 'tokio', 'serde')
        name: String,

        /// Specific version to index (default: latest)
        #[arg(long)]
        version: Option<String>,

        /// Path to the doc store database (default: .muninn/docs.db)
        #[arg(long)]
        db: Option<PathBuf>,
    },

    /// Index a Python package from PyPI
    #[command(name = "index-package")]
    IndexPackage {
        /// Name of the package to index (e.g., 'requests', 'flask')
        name: String,

        /// Specific version to index (default: latest)
        #[arg(long)]
        version: Option<String>,

        /// Path to the doc store database (default: .muninn/docs.db)
        #[arg(long)]
        db: Option<PathBuf>,

        /// Python executable (deprecated, no longer needed - tree-sitter is used)
        #[arg(long, default_value = "python3", hide = true)]
        python: String,
    },

    /// List all indexed libraries
    List {
        /// Filter by ecosystem (rust, python)
        #[arg(short, long)]
        ecosystem: Option<String>,

        /// Path to the doc store database (default: .muninn/docs.db)
        #[arg(long)]
        db: Option<PathBuf>,
    },

    /// Search documentation in indexed libraries
    Search {
        /// Library name to search (e.g., 'tokio', 'requests')
        library: String,

        /// Search query (e.g., 'spawn async task', 'HTTP request')
        query: String,

        /// Maximum results to return
        #[arg(short = 'n', long, default_value = "20")]
        limit: usize,

        /// Path to the doc store database (default: .muninn/docs.db)
        #[arg(long)]
        db: Option<PathBuf>,
    },

    /// Remove an indexed library
    Remove {
        /// Name of the library to remove
        name: String,

        /// Path to the doc store database (default: .muninn/docs.db)
        #[arg(long)]
        db: Option<PathBuf>,

        /// Skip confirmation prompt
        #[arg(short, long)]
        force: bool,
    },

    /// Update (re-index) an existing library to a new version
    Update {
        /// Name of the library to update
        name: String,

        /// Specific version to update to (default: latest)
        #[arg(long)]
        version: Option<String>,

        /// Path to the doc store database (default: .muninn/docs.db)
        #[arg(long)]
        db: Option<PathBuf>,

        /// Python executable (deprecated, no longer needed - tree-sitter is used)
        #[arg(long, default_value = "python3", hide = true)]
        python: String,
    },

    /// Index documentation from an llms.txt URL (fast-path for LLM-optimized docs)
    #[command(name = "index-llms")]
    IndexLlms {
        /// URL to fetch llms.txt from (can be base URL or direct llms.txt URL)
        url: String,

        /// Path to the doc store database (default: .muninn/docs.db)
        #[arg(long)]
        db: Option<PathBuf>,

        /// Fast mode: only index descriptions, don't fetch linked content
        #[arg(long)]
        fast: bool,

        /// Maximum number of links to fetch (0 = unlimited)
        #[arg(long, default_value = "100")]
        max_links: usize,
    },
}

/// Initialize logging for standalone commands (not agent mode).
/// Logs to stderr for interactive use.
fn init_logging(verbose: bool) {
    let filter = if verbose {
        EnvFilter::new("debug")
    } else {
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))
    };

    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(filter)
        .init();
}

/// Initialize logging for proxy/daemon mode.
/// Logs to rotating files in .muninn/logs/ with daily rotation.
fn init_file_logging(muninn_dir: &std::path::Path, verbose: bool) {
    let logs_dir = muninn_dir.join("logs");

    // Create logs directory if it doesn't exist
    if let Err(e) = std::fs::create_dir_all(&logs_dir) {
        eprintln!("Warning: Failed to create logs directory: {}", e);
        // Fall back to stderr logging
        init_logging(verbose);
        return;
    }

    let filter = if verbose {
        EnvFilter::new("debug")
    } else {
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))
    };

    // Daily rotation with prefix "muninn"
    let file_appender = RollingFileAppender::new(Rotation::DAILY, &logs_dir, "muninn.log");

    // Use non-blocking writer to avoid blocking on log writes
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

    // Store guard in a static to prevent it from being dropped
    // (dropping the guard would stop logging)
    static GUARD: std::sync::OnceLock<tracing_appender::non_blocking::WorkerGuard> =
        std::sync::OnceLock::new();
    let _ = GUARD.set(_guard);

    tracing_subscriber::registry()
        .with(fmt::layer().with_writer(non_blocking).with_ansi(false))
        .with(filter)
        .init();
}

/// Initialize logging for agent mode - logs to file to keep terminal clean.
fn init_agent_logging(muninn_dir: &std::path::Path) {
    use tracing_subscriber::layer::SubscriberExt;

    // Create logs directory
    let log_dir = muninn_dir.join("logs");
    std::fs::create_dir_all(&log_dir).ok();

    // Create file appender
    let file_appender = RollingFileAppender::new(Rotation::DAILY, log_dir, "muninn.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

    // Keep guard alive - store in static to prevent drop
    static AGENT_GUARD: std::sync::OnceLock<tracing_appender::non_blocking::WorkerGuard> =
        std::sync::OnceLock::new();
    let _ = AGENT_GUARD.set(_guard);

    // Log to file with debug level
    let filter = EnvFilter::new("debug");

    tracing_subscriber::registry()
        .with(fmt::layer().with_writer(non_blocking).with_ansi(false))
        .with(filter)
        .init();
}

/// Initialize logging for session-based mode.
/// Logs to a single file in the session directory (no rotation).
fn init_session_logging(session_dir: &std::path::Path, verbose: bool) {
    use std::fs::OpenOptions;

    // Session directory should already be created
    let log_path = session_dir.join("muninn.log");

    let filter = if verbose {
        EnvFilter::new("debug")
    } else {
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))
    };

    // Open file for appending
    let file = match OpenOptions::new().create(true).append(true).open(&log_path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("Warning: Failed to create log file: {}", e);
            // Fall back to stderr logging
            init_logging(verbose);
            return;
        }
    };

    // Use non-blocking writer
    let (non_blocking, _guard) = tracing_appender::non_blocking(file);

    // Store guard in a static to prevent it from being dropped
    static SESSION_GUARD: std::sync::OnceLock<tracing_appender::non_blocking::WorkerGuard> =
        std::sync::OnceLock::new();
    let _ = SESSION_GUARD.set(_guard);

    tracing_subscriber::registry()
        .with(fmt::layer().with_writer(non_blocking).with_ansi(false))
        .with(filter)
        .init();
}

fn parse_router_strategy(s: &str) -> RouterStrategy {
    match s.to_lowercase().as_str() {
        "llm" => RouterStrategy::Llm,
        "always-rlm" | "rlm" => RouterStrategy::AlwaysRlm,
        "always-passthrough" | "passthrough" => RouterStrategy::AlwaysPassthrough,
        _ => {
            tracing::warn!("Unknown router strategy '{}', using llm", s);
            RouterStrategy::Llm
        }
    }
}

/// Create a tool registry with all available tools.
fn create_tools(
    workdir: &PathBuf,
    graph_store: Option<SharedGraphStore>,
    doc_store: Option<SharedDocStore>,
) -> ToolRegistry {
    let mut registry = ToolRegistry::new();

    // Add filesystem tools (internal, for RLM use)
    for tool in create_fs_tools(workdir) {
        registry.register_arc(Arc::from(tool));
    }

    // Add graph tools if we have a graph store (external, exposed via MCP)
    if let Some(store) = graph_store {
        for tool in create_graph_tools(store) {
            registry.register_arc(Arc::from(tool));
        }
    }

    // Add doc tools if we have a doc store (for library documentation search)
    if let Some(store) = doc_store {
        for tool in create_doc_tools(store) {
            registry.register_arc(Arc::from(tool));
        }
    }

    registry
}

/// Load or open the graph store, optionally starting background indexing if missing.
fn open_graph_store(path: &PathBuf) -> Result<Option<SharedGraphStore>> {
    if path.exists() {
        info!("Opening graph store at {}", path.display());
        let store = GraphStore::open(path)?;
        Ok(Some(wrap_store(store)))
    } else {
        Ok(None)
    }
}

/// Open the doc store if it exists.
fn open_doc_store(path: &PathBuf) -> Result<Option<SharedDocStore>> {
    if path.exists() {
        info!("Opening doc store at {}", path.display());
        let store = DocStore::open(path)?;
        Ok(Some(wrap_doc_store(store)))
    } else {
        debug!(
            "No doc store at {} - doc tools will not be available",
            path.display()
        );
        Ok(None)
    }
}

/// Start background indexing if graph store doesn't exist.
fn start_background_indexing(graph_path: PathBuf, source_path: PathBuf, extensions: Vec<String>) {
    if graph_path.exists() {
        return;
    }

    info!(
        "Starting background indexing of {} -> {}",
        source_path.display(),
        graph_path.display()
    );

    std::thread::spawn(move || {
        // Create parent directory if needed
        if let Some(parent) = graph_path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                tracing::error!("Failed to create graph directory: {}", e);
                return;
            }
        }

        // Open/create the graph store
        let store = match GraphStore::open(&graph_path) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("Failed to create graph store: {}", e);
                return;
            }
        };

        // Build the index
        let mut builder = GraphBuilder::new(store);

        tracing::debug!("Indexing extensions: {:?}", extensions);

        match builder.build_directory(&source_path) {
            Ok(stats) => {
                info!(
                    "Background indexing complete: {} files, {} nodes, {} edges",
                    stats.files_processed, stats.nodes_added, stats.edges_added
                );
            }
            Err(e) => {
                tracing::error!("Background indexing failed: {}", e);
            }
        }
    });
}

/// Start file watcher to keep graph in sync with source changes.
///
/// Collects file changes over a debounce window and batch processes them.
fn start_file_watcher(graph_path: PathBuf, source_path: PathBuf, debounce_ms: u64) {
    use std::collections::HashSet;
    use std::time::{Duration, Instant};

    std::thread::spawn(move || {
        // Wait for graph to exist before starting watcher
        while !graph_path.exists() {
            std::thread::sleep(Duration::from_secs(1));
        }

        let watcher = match FileWatcher::new(&source_path) {
            Ok(w) => w,
            Err(e) => {
                tracing::error!("Failed to create file watcher: {}", e);
                return;
            }
        };

        info!("File watcher started for {}", source_path.display());

        let debounce_duration = Duration::from_millis(debounce_ms);
        let mut pending_modified: HashSet<PathBuf> = HashSet::new();
        let mut pending_deleted: HashSet<PathBuf> = HashSet::new();
        let mut last_event_time = Instant::now();

        loop {
            // Try to get next event with timeout
            match watcher.try_next_event() {
                Some(event) => {
                    last_event_time = Instant::now();
                    match event {
                        FileEvent::Modified(path) | FileEvent::Created(path) => {
                            pending_deleted.remove(&path);
                            pending_modified.insert(path);
                        }
                        FileEvent::Deleted(path) => {
                            pending_modified.remove(&path);
                            pending_deleted.insert(path);
                        }
                    }
                }
                None => {
                    // No event available - check if we should flush pending changes
                    if (!pending_modified.is_empty() || !pending_deleted.is_empty())
                        && last_event_time.elapsed() >= debounce_duration
                    {
                        // Flush pending changes
                        let modified: Vec<_> = pending_modified.drain().collect();
                        let deleted: Vec<_> = pending_deleted.drain().collect();

                        if let Err(e) = process_file_changes(&graph_path, &modified, &deleted) {
                            tracing::error!("Failed to process file changes: {}", e);
                        }
                    }
                    // Sleep briefly to avoid busy loop
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        }
    });
}

/// Process batched file changes by updating the graph.
fn process_file_changes(
    graph_path: &PathBuf,
    modified: &[PathBuf],
    deleted: &[PathBuf],
) -> Result<()> {
    if modified.is_empty() && deleted.is_empty() {
        return Ok(());
    }

    let store = GraphStore::open(graph_path)?;
    let mut builder = GraphBuilder::new(store);

    // Process deletions first
    for path in deleted {
        let path_str = path.to_string_lossy();
        tracing::debug!("Removing from graph: {}", path_str);
        if let Err(e) = builder.store().delete_file(&path_str) {
            tracing::warn!("Failed to delete {}: {}", path_str, e);
        }
    }

    // Process modifications (rebuild_file handles delete + rebuild)
    let mut stats = muninn_graph::BuildStats::default();
    for path in modified {
        tracing::debug!("Rebuilding in graph: {}", path.display());
        match builder.rebuild_file(path) {
            Ok(file_stats) => stats.merge(&file_stats),
            Err(e) => tracing::warn!("Failed to rebuild {}: {}", path.display(), e),
        }
    }

    if stats.files_processed > 0 {
        info!(
            "Graph updated: {} files, {} nodes, {} edges",
            stats.files_processed, stats.nodes_added, stats.edges_added
        );
    }

    Ok(())
}

/// Load config from file or auto-discover from `.muninn/config.toml`.
///
/// Returns the config and the path to the `.muninn` directory (for resolving relative paths).
fn load_config(override_path: Option<&PathBuf>) -> (Config, Option<PathBuf>) {
    if let Some(path) = override_path {
        // Explicit path override - treat as path to .muninn directory
        let config_file = if path.is_dir() {
            path.join(config::CONFIG_FILE)
        } else {
            path.clone()
        };
        let muninn_dir = config_file.parent().unwrap_or(path).to_path_buf();

        match Config::from_file(&config_file) {
            Ok(config) => {
                info!("Loaded config from {}", config_file.display());
                (config, Some(muninn_dir))
            }
            Err(e) => {
                tracing::error!(
                    "Failed to load config from {}: {}",
                    config_file.display(),
                    e
                );
                std::process::exit(1);
            }
        }
    } else {
        // Auto-discover by walking up directory tree
        match Config::find_and_load() {
            Ok(Some((config, muninn_dir))) => {
                info!("Found config at {}", muninn_dir.display());
                (config, Some(muninn_dir))
            }
            Ok(None) => {
                tracing::debug!("No .muninn/config.toml found, using defaults");
                (Config::default(), None)
            }
            Err(e) => {
                tracing::warn!("Error searching for config: {}, using defaults", e);
                (Config::default(), None)
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Split args at agent command boundary BEFORE clap parsing
    let (muninn_args, agent_info) = split_args_at_agent();

    // Parse only the muninn portion with clap
    let cli = Cli::parse_from(&muninn_args);

    // Defer logging init - different commands need different logging modes
    let _is_agent_mode = agent_info.is_some();

    let (config, config_dir) = load_config(cli.config.as_ref());

    // If an agent command was found, run in agent mode
    if let Some((agent_cmd, agent_args)) = agent_info {
        return run_with_agent(AgentLaunchConfig {
            port: cli.port,
            groq_key: cli.groq_key,
            router_strategy: cli.router,
            workdir: cli.workdir,
            agent_cmd,
            agent_args,
            config,
            config_dir,
            verbose: cli.verbose,
        })
        .await;
    }

    // Otherwise handle subcommands
    let Some(command) = cli.command else {
        // No command and no agent - show help
        use clap::CommandFactory;
        Cli::command().print_help()?;
        println!("\n\nSupported agents: {}", AGENT_COMMANDS.join(", "));
        return Ok(());
    };

    match command {
        Commands::Proxy { host } => {
            let muninn_dir = config_dir
                .clone()
                .unwrap_or_else(|| PathBuf::from(config::MUNINN_DIR));

            // Generate session ID and create session directory
            let session_id = session::SessionId::generate();
            let session_dir = session::session_dir(&muninn_dir, &session_id);
            std::fs::create_dir_all(&session_dir)?;

            // Initialize session-based logging
            init_session_logging(&session_dir, cli.verbose);

            let addr: SocketAddr = format!("{}:{}", host, cli.port).parse()?;
            info!("Starting Muninn proxy server on {}", addr);

            // Emit deprecation warning if using old [backend] section
            config.warn_deprecated_backend();

            // Use CLI args or fall back to config
            let router_strategy = cli
                .router
                .map(|s| parse_router_strategy(&s))
                .unwrap_or_else(|| parse_router_strategy(&config.router.strategy));

            let work_path = cli.workdir.unwrap_or_else(|| {
                config_dir
                    .as_ref()
                    .map(|d| d.join(&config.project.root))
                    .unwrap_or_else(|| config.project.root.clone())
            });
            // Canonicalize to resolve relative paths like "." or ".."
            let work_path = work_path.canonicalize().unwrap_or(work_path);

            // Resolve provider+model via the tiered config (router/rlm
            // inherit from [default] when not overridden).
            let resolved_router = config.resolved_router();
            let resolved_rlm = config.resolved_rlm();

            // Create separate backends for router and RLM
            // If CLI provides groq_key, use it for both; otherwise use config
            let (router_backend, rlm_backend) = if let Some(key) = cli.groq_key.clone() {
                info!("Using Groq backend from CLI for both router and RLM");
                let router_groq = GroqConfig::new(key.clone()).with_model(&resolved_router.model);
                let rlm_groq = GroqConfig::new(key).with_model(&resolved_rlm.model);
                (
                    Some(
                        Arc::new(GroqBackend::new(router_groq)?) as Arc<dyn muninn_rlm::LLMBackend>
                    ),
                    Some(Arc::new(GroqBackend::new(rlm_groq)?) as Arc<dyn muninn_rlm::LLMBackend>),
                )
            } else {
                // Create router backend
                let router_backend = create_backend_from_config(
                    &resolved_router.provider,
                    &resolved_router.model,
                    &config,
                    config_dir.as_deref(),
                )?;

                // Create RLM backend
                let rlm_backend = create_backend_from_config(
                    &resolved_rlm.provider,
                    &resolved_rlm.model,
                    &config,
                    config_dir.as_deref(),
                )?;

                (router_backend, rlm_backend)
            };

            // Log which models are being used
            info!(
                "Router: {} via {}",
                resolved_router.model, resolved_router.provider
            );
            info!("RLM: {} via {}", resolved_rlm.model, resolved_rlm.provider);

            // Configure the router with its dedicated backend
            let router_strategy_str = format!("{:?}", router_strategy);
            let router_config = RouterConfig {
                strategy: router_strategy,
                enabled: config.router.enabled,
                router_model: Some(resolved_router.model.clone()),
            };

            // Open graph store if available
            let graph_path = config.resolve_graph_path(config_dir.as_deref());
            let graph_store = open_graph_store(&graph_path)?;

            // Open doc store if available (default: .muninn/docs.db)
            let doc_path = config_dir
                .as_ref()
                .map(|d| d.join("docs.db"))
                .unwrap_or_else(|| PathBuf::from(".muninn/docs.db"));
            let doc_store = open_doc_store(&doc_path)?;

            // Create tools
            let tools: Arc<dyn muninn_rlm::ToolEnvironment> =
                Arc::new(create_tools(&work_path, graph_store, doc_store));

            // Create token manager for OAuth support
            let muninn_dir = config_dir
                .clone()
                .unwrap_or_else(|| PathBuf::from(config::MUNINN_DIR));
            let token_manager = create_token_manager(&muninn_dir);

            // Configure and start the proxy with OAuth support
            let rlm_budget = config_to_rlm_budget(&config.budget);
            info!(
                "Budget config: max_depth={}, max_tool_calls={}, max_tokens={}",
                config.budget.max_depth, config.budget.max_tool_calls, config.budget.max_tokens
            );

            // Write session metadata
            let session_metadata = session::SessionMetadata::new(&session_id, work_path.clone())
                .with_router_strategy(&router_strategy_str)
                .with_rlm_model(&resolved_rlm.model);
            session::write_metadata(&session_dir, &session_metadata)?;

            info!("Session: {} -> {:?}", session_id, session_dir);

            // Configure trace writer for session mode
            let trace_writer_config =
                muninn_tracing::WriterConfig::session(session_dir.join("traces.jsonl"));

            let proxy_config = ProxyConfig::new(addr)
                .with_token_manager(token_manager)
                .with_budget(rlm_budget)
                .with_work_dir(&work_path)
                .with_session_dir(&session_dir)
                .with_trace_writer(trace_writer_config);

            // Build server with separate router and RLM backends
            let server = match (router_backend, rlm_backend) {
                (Some(router_be), Some(rlm_be)) => ProxyServer::with_separate_backends(
                    proxy_config,
                    router_be,
                    rlm_be,
                    tools,
                    router_config,
                ),
                (_, Some(rlm_be)) => {
                    // No router backend, use RLM backend for both
                    info!("Router backend not available, using RLM backend for routing");
                    ProxyServer::with_router(proxy_config, rlm_be, tools, router_config)
                }
                _ => {
                    info!("No RLM backend configured, running in passthrough-only mode");
                    ProxyServer::passthrough_only(proxy_config)
                }
            };
            server.run().await?;
        }

        Commands::Index {
            path,
            output,
            watch,
        } => {
            // Index uses file logging
            let muninn_dir = config_dir
                .clone()
                .unwrap_or_else(|| PathBuf::from(config::MUNINN_DIR));
            init_file_logging(&muninn_dir, cli.verbose);

            let source_path = path.unwrap_or_else(|| {
                config_dir
                    .as_ref()
                    .map(|d| d.join(&config.project.root))
                    .unwrap_or_else(|| config.project.root.clone())
            });
            // Canonicalize to resolve relative paths like "." or ".."
            let source_path = source_path.canonicalize().unwrap_or(source_path);

            let graph_path =
                output.unwrap_or_else(|| config.resolve_graph_path(config_dir.as_deref()));

            info!(
                "Indexing {} -> {}",
                source_path.display(),
                graph_path.display()
            );

            // Create parent directory if needed
            if let Some(parent) = graph_path.parent() {
                std::fs::create_dir_all(parent)?;
            }

            // GraphStore::open creates the database if it doesn't exist
            let store = GraphStore::open(&graph_path)?;

            // Build the index
            let mut builder = GraphBuilder::new(store);

            info!("Indexing extensions: {:?}", config.graph.extensions);

            // Index the directory (GraphBuilder auto-detects languages from extensions)
            let stats = builder.build_directory(&source_path)?;
            info!(
                "Indexed {} files, {} nodes, {} edges (parse: {}ms, store: {}ms)",
                stats.files_processed,
                stats.nodes_added,
                stats.edges_added,
                stats.parse_time_ms,
                stats.store_time_ms
            );

            if watch {
                info!("Watch mode not yet implemented");
                // TODO: Implement file watching with notify crate
            }
        }

        Commands::Init { force } => {
            use config::{CONFIG_FILE, MUNINN_DIR};

            let muninn_dir = PathBuf::from(MUNINN_DIR);
            let config_path = muninn_dir.join(CONFIG_FILE);

            if config_path.exists() && !force {
                anyhow::bail!(".muninn/config.toml already exists. Use --force to overwrite.");
            }

            // Create .muninn directory if it doesn't exist
            if !muninn_dir.exists() {
                std::fs::create_dir_all(&muninn_dir)?;
                info!("Created {}/", muninn_dir.display());
            }

            let default_config = r#"# Muninn configuration
# All paths are relative to this .muninn/ directory unless absolute

[project]
root = ".."  # Parent directory (the actual project root)

[graph]
path = "graph.db"  # Stored in .muninn/graph.db
extensions = ["rs", "py", "ts", "js", "go", "c", "cpp", "h"]

# Default LLM provider/model. Router and RLM inherit from this unless they
# override `provider` / `model` in their own sections. The out-of-the-box
# default is a single Ollama Cloud model — works on the free tier (concurrent
# model cap = 1) and maximizes prompt-cache reuse.
[default]
provider = "ollama"  # Options: "ollama", "groq", "anthropic", "local"
model = "gemma4:31b"

# Router configuration (for deciding passthrough vs RLM)
[router]
strategy = "llm"  # Options: "llm", "always-rlm", "always-passthrough"
enabled = true
# Override provider/model below to specialize the router on a cheaper/faster
# model. Leaving them unset inherits from [default].
# provider = "groq"
# model = "llama-3.1-8b-instant"

# RLM (Recursive Language Model) configuration
[rlm]
# Override to point the recursive-exploration loop at a larger model.
# Leaving these unset inherits from [default].
# provider = "groq"
# model = "qwen/qwen3-32b"

[budget]
max_tokens = 100000
max_depth = 5
max_tool_calls = 50
max_duration_secs = 300

# Provider credentials (set here or use env vars)
[ollama]
# Ollama Cloud is the default. Set api_key or use OLLAMA_API_KEY env var.
# To run against a local Ollama daemon instead, override base_url:
# base_url = "http://localhost:11434/v1"
# api_key = "..."

# [groq]
# api_key = "gsk_..."  # Or use GROQ_API_KEY env var

# [anthropic]
# api_key = "sk-..."  # Or use ANTHROPIC_API_KEY env var
"#;

            std::fs::write(&config_path, default_config)?;
            info!("Created {}", config_path.display());
            info!("Next steps:");
            info!("  1. Edit .muninn/config.toml to configure your project");
            info!("  2. Run 'muninn index' to build the code graph");
            info!("  3. Run 'muninn oauth' to authenticate with Claude MAX");
            info!("  4. Run 'muninn claude' to start coding with context");
        }

        Commands::Auth { status, logout } => {
            use config::MUNINN_DIR;

            // Ensure .muninn directory exists
            let muninn_dir = config_dir.unwrap_or_else(|| PathBuf::from(MUNINN_DIR));
            if !muninn_dir.exists() {
                std::fs::create_dir_all(&muninn_dir)?;
            }

            let token_manager = FileTokenManager::new(&muninn_dir);

            if logout {
                // Delete stored tokens
                if token_manager.has_tokens() {
                    token_manager.delete_tokens().await?;
                    info!("OAuth tokens deleted. You are now logged out.");
                } else {
                    info!("No OAuth tokens found.");
                }
                return Ok(());
            }

            if status {
                // Show token status
                match token_manager.get_token_info().await? {
                    Some(info) => {
                        info!("OAuth Status:");
                        info!("  Created: {}", info.created_at);
                        info!("  Expires in: {}", info.expires_in_display());
                        info!("  Scope: {}", info.scope);
                    }
                    None => {
                        info!("No OAuth tokens found. Run 'muninn oauth' to authenticate.");
                    }
                }
                return Ok(());
            }

            // Run OAuth flow
            run_oauth_flow(&token_manager).await?;
        }

        Commands::Docs { command } => {
            // Initialize logging for CLI commands
            init_logging(cli.verbose);

            // Resolve docs database path
            let resolve_db_path = |db: Option<PathBuf>| -> PathBuf {
                db.unwrap_or_else(|| {
                    config_dir
                        .as_ref()
                        .map(|d| d.join("docs.db"))
                        .unwrap_or_else(|| PathBuf::from(".muninn/docs.db"))
                })
            };

            match command {
                DocsCommand::IndexCrate { name, version, db } => {
                    let db_path = resolve_db_path(db);

                    // Create parent directory if needed
                    if let Some(parent) = db_path.parent() {
                        std::fs::create_dir_all(parent)?;
                    }

                    info!("Opening doc store at {}", db_path.display());

                    info!(
                        "Indexing crate '{}' {}...",
                        name,
                        version.as_deref().unwrap_or("(latest)")
                    );

                    // Run in blocking task to avoid tokio runtime conflicts with reqwest::blocking
                    let result = tokio::task::spawn_blocking(move || {
                        let store = DocStore::open(&db_path)?;
                        let config = IndexerConfig {
                            keep_source: false,
                            work_dir: None,
                            rustdoc_flags: Vec::new(),
                        };
                        let indexer = RustDocIndexer::with_config(config);
                        indexer.index_crate(&store, &name, version.as_deref())
                    })
                    .await??;

                    info!(
                        "Successfully indexed {} v{}",
                        result.crate_name, result.version
                    );
                    info!(
                        "  {} items extracted, {} items indexed",
                        result.items_extracted, result.items_indexed
                    );
                }

                DocsCommand::IndexPackage {
                    name,
                    version,
                    db,
                    python,
                } => {
                    let db_path = resolve_db_path(db);

                    // Create parent directory if needed
                    if let Some(parent) = db_path.parent() {
                        std::fs::create_dir_all(parent)?;
                    }

                    info!("Opening doc store at {}", db_path.display());

                    info!(
                        "Indexing package '{}' {}...",
                        name,
                        version.as_deref().unwrap_or("(latest)")
                    );

                    // Clone name for error message since it's moved into closure
                    let name_for_error = name.clone();

                    // Run in blocking task to avoid tokio runtime conflicts with reqwest::blocking
                    // Note: `python` argument is ignored - tree-sitter is used for extraction
                    let _ = python; // Silence unused variable warning
                    let result = tokio::task::spawn_blocking(move || {
                        let store = DocStore::open(&db_path)?;
                        let config = PyIndexerConfig {
                            keep_source: false,
                            work_dir: None,
                            ..Default::default()
                        };
                        let indexer = PyDocIndexer::with_config(config);
                        indexer.index_package(&store, &name, version.as_deref())
                    })
                    .await?;

                    match result {
                        Ok(stats) => {
                            info!(
                                "Successfully indexed {} v{}",
                                stats.package_name, stats.version
                            );
                            info!(
                                "  {} items extracted, {} items indexed",
                                stats.items_extracted, stats.items_indexed
                            );
                        }
                        Err(e) => {
                            anyhow::bail!("Failed to index package '{}': {}", name_for_error, e);
                        }
                    }
                }

                DocsCommand::List { ecosystem, db } => {
                    let db_path = resolve_db_path(db);

                    if !db_path.exists() {
                        info!("No doc store found at {}", db_path.display());
                        info!(
                            "Use 'muninn docs index-crate' or 'muninn docs index-package' to index libraries."
                        );
                        return Ok(());
                    }

                    let store = DocStore::open(&db_path)?;
                    let libraries = store.list_libraries()?;

                    // Filter by ecosystem if specified
                    let ecosystem_filter = ecosystem.as_ref().and_then(|e| Ecosystem::from_str(e));
                    let filtered: Vec<_> = libraries
                        .into_iter()
                        .filter(|lib| {
                            ecosystem_filter
                                .map(|eco| lib.ecosystem == eco)
                                .unwrap_or(true)
                        })
                        .collect();

                    if filtered.is_empty() {
                        if let Some(eco) = ecosystem_filter {
                            info!("No {} libraries indexed.", eco.as_str());
                        } else {
                            info!("No libraries indexed.");
                            info!(
                                "Use 'muninn docs index-crate' or 'muninn docs index-package' to index libraries."
                            );
                        }
                        return Ok(());
                    }

                    println!(
                        "{:<20} {:<10} {:<10} {}",
                        "LIBRARY", "VERSION", "ECOSYSTEM", "INDEXED AT"
                    );
                    println!("{}", "-".repeat(60));
                    for lib in &filtered {
                        println!(
                            "{:<20} {:<10} {:<10} {}",
                            lib.library,
                            lib.version,
                            lib.ecosystem.as_str(),
                            lib.indexed_at
                        );
                    }
                    println!();
                    info!("{} libraries indexed", filtered.len());
                }

                DocsCommand::Search {
                    library,
                    query,
                    limit,
                    db,
                } => {
                    let db_path = resolve_db_path(db);

                    if !db_path.exists() {
                        anyhow::bail!(
                            "No doc store found at {}. Use 'muninn docs index-crate' or 'muninn docs index-package' first.",
                            db_path.display()
                        );
                    }

                    let store = DocStore::open(&db_path)?;

                    // Check if library exists
                    let lib = store.get_library(&library)?;
                    if lib.is_none() {
                        anyhow::bail!(
                            "Library '{}' is not indexed. Use 'muninn docs index-crate' or 'muninn docs index-package' first.",
                            library
                        );
                    }

                    let lib_info = lib.unwrap();
                    info!(
                        "Searching '{}' in {} v{} ({})...",
                        query,
                        library,
                        lib_info.version,
                        lib_info.ecosystem.as_str()
                    );

                    let results = store.search(&library, &query, limit)?;

                    if results.is_empty() {
                        info!("No results found for '{}'", query);
                        return Ok(());
                    }

                    println!();
                    for (i, result) in results.iter().enumerate() {
                        println!(
                            "{}. {} ({})",
                            i + 1,
                            result.chunk.item_path,
                            result.chunk.item_type.as_str()
                        );
                        if let Some(ref sig) = result.chunk.signature {
                            println!("   {}", sig);
                        }
                        // Truncate doc text for display
                        let doc = &result.chunk.doc_text;
                        let doc_preview = if doc.len() > 200 {
                            format!("{}...", &doc[..200])
                        } else {
                            doc.clone()
                        };
                        // Indent and wrap doc text
                        for line in doc_preview.lines().take(4) {
                            println!("   {}", line);
                        }
                        println!();
                    }
                    info!("Found {} results", results.len());
                }

                DocsCommand::Remove { name, db, force } => {
                    let db_path = resolve_db_path(db);

                    if !db_path.exists() {
                        anyhow::bail!("No doc store found at {}.", db_path.display());
                    }

                    let store = DocStore::open(&db_path)?;

                    // Check if library exists
                    let lib = store.get_library(&name)?;
                    if lib.is_none() {
                        anyhow::bail!("Library '{}' is not indexed.", name);
                    }

                    let lib_info = lib.unwrap();

                    // Confirm unless --force
                    if !force {
                        use std::io::{self, Write};
                        print!(
                            "Remove {} v{} ({})? [y/N] ",
                            lib_info.library,
                            lib_info.version,
                            lib_info.ecosystem.as_str()
                        );
                        io::stdout().flush()?;

                        let mut input = String::new();
                        io::stdin().read_line(&mut input)?;
                        let input = input.trim().to_lowercase();

                        if input != "y" && input != "yes" {
                            info!("Aborted.");
                            return Ok(());
                        }
                    }

                    if store.delete_library(&name)? {
                        info!(
                            "Removed {} v{} ({})",
                            lib_info.library,
                            lib_info.version,
                            lib_info.ecosystem.as_str()
                        );
                    } else {
                        anyhow::bail!("Failed to remove library '{}'", name);
                    }
                }

                DocsCommand::Update {
                    name,
                    version,
                    db,
                    python,
                } => {
                    // Note: `python` argument is ignored - tree-sitter is used for extraction
                    let _ = python;
                    let db_path = resolve_db_path(db);

                    if !db_path.exists() {
                        anyhow::bail!(
                            "No doc store found at {}. Use 'muninn docs index-crate' or 'muninn docs index-package' first.",
                            db_path.display()
                        );
                    }

                    // First check library info (quick, no HTTP)
                    let (ecosystem, old_version) = {
                        let store = DocStore::open(&db_path)?;
                        let lib = store.get_library(&name)?;
                        if lib.is_none() {
                            anyhow::bail!(
                                "Library '{}' is not indexed. Use 'muninn docs index-crate' or 'muninn docs index-package' to index it first.",
                                name
                            );
                        }
                        let lib_info = lib.unwrap();
                        (lib_info.ecosystem, lib_info.version.clone())
                    };

                    if ecosystem == Ecosystem::Web {
                        anyhow::bail!(
                            "Cannot update web documentation '{}'. Use 'muninn docs index-llms' to re-index.",
                            name
                        );
                    }

                    info!(
                        "Updating {} from v{} to {}...",
                        name,
                        old_version,
                        version.as_deref().unwrap_or("latest")
                    );

                    // Run indexing in blocking task to avoid tokio runtime conflicts
                    let result = tokio::task::spawn_blocking(
                        move || -> anyhow::Result<(String, String, usize, usize)> {
                            let store = DocStore::open(&db_path)?;

                            // Delete the old entry
                            store.delete_library(&name)?;

                            // Re-index based on ecosystem
                            match ecosystem {
                                Ecosystem::Rust => {
                                    let config = IndexerConfig {
                                        keep_source: false,
                                        work_dir: None,
                                        rustdoc_flags: Vec::new(),
                                    };
                                    let indexer = RustDocIndexer::with_config(config);
                                    let stats =
                                        indexer.index_crate(&store, &name, version.as_deref())?;
                                    Ok((
                                        stats.crate_name,
                                        stats.version,
                                        stats.items_extracted,
                                        stats.items_indexed,
                                    ))
                                }
                                Ecosystem::Python => {
                                    let config = PyIndexerConfig {
                                        keep_source: false,
                                        work_dir: None,
                                        ..Default::default()
                                    };
                                    let indexer = PyDocIndexer::with_config(config);
                                    let stats =
                                        indexer.index_package(&store, &name, version.as_deref())?;
                                    Ok((
                                        stats.package_name,
                                        stats.version,
                                        stats.items_extracted,
                                        stats.items_indexed,
                                    ))
                                }
                                Ecosystem::Web => {
                                    unreachable!("Web ecosystem handled above")
                                }
                            }
                        },
                    )
                    .await??;

                    info!(
                        "Updated {} from v{} to v{}",
                        result.0, old_version, result.1
                    );
                    info!("  {} items extracted, {} items indexed", result.2, result.3);
                }

                DocsCommand::IndexLlms {
                    url,
                    db,
                    fast,
                    max_links,
                } => {
                    let db_path = resolve_db_path(db);

                    // Create parent directory if needed
                    if let Some(parent) = db_path.parent() {
                        std::fs::create_dir_all(parent)?;
                    }

                    info!("Opening doc store at {}", db_path.display());

                    let fast_mode = fast;
                    info!(
                        "Indexing llms.txt from {} {}...",
                        url,
                        if fast_mode { "(fast mode)" } else { "" }
                    );

                    // Run in blocking task to avoid tokio runtime conflicts with reqwest::blocking
                    let result = tokio::task::spawn_blocking(move || {
                        let store = DocStore::open(&db_path)?;
                        let config = LlmsTxtIndexerConfig {
                            fetch_linked_content: !fast,
                            max_links,
                            ..Default::default()
                        };
                        let indexer = LlmsTxtIndexer::with_config(config);
                        indexer.index_url(&store, &url)
                    })
                    .await?;

                    match result {
                        Ok(stats) => {
                            info!("Successfully indexed '{}'", stats.name);
                            info!(
                                "  {} links found, {} indexed, {} failed",
                                stats.links_found, stats.links_indexed, stats.links_failed
                            );
                        }
                        Err(e) => {
                            anyhow::bail!("Failed to index llms.txt: {}", e);
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

/// Configuration for launching an agent with muninn proxy.
struct AgentLaunchConfig {
    /// Port for the proxy server (0 = auto-select).
    port: u16,
    /// Groq API key override for RLM backend.
    groq_key: Option<String>,
    /// Router strategy override.
    router_strategy: Option<String>,
    /// Working directory override.
    workdir: Option<PathBuf>,
    /// The agent command to run (e.g., "claude", "cursor").
    agent_cmd: String,
    /// Arguments to pass to the agent.
    agent_args: Vec<String>,
    /// Muninn config.
    config: Config,
    /// Directory containing the config file.
    config_dir: Option<PathBuf>,
    /// Verbose logging flag.
    verbose: bool,
}

/// Run an agent with muninn proxy transparently injected.
async fn run_with_agent(launch: AgentLaunchConfig) -> Result<()> {
    use std::process::Stdio;
    use tokio::net::TcpListener;
    use tokio::process::Command;
    use tokio::signal;

    // Get or create muninn directory FIRST - we need it for logging
    let muninn_dir = match launch.config_dir.clone() {
        Some(dir) => dir,
        None => {
            // No config found - auto-init in current directory
            let cwd = std::env::current_dir()?;
            let muninn_dir = cwd.join(".muninn");
            if !muninn_dir.exists() {
                std::fs::create_dir_all(&muninn_dir)?;
                // Create default config
                let config_path = muninn_dir.join("config.toml");
                let default_config = config::Config::default();
                let toml_str = toml::to_string_pretty(&default_config)?;
                std::fs::write(&config_path, toml_str)?;
            }
            muninn_dir
        }
    };

    // Initialize logging to file (keeps terminal clean for agent)
    if launch.verbose {
        // In verbose mode, also log to terminal
        init_logging(true);
    } else {
        init_agent_logging(&muninn_dir);
    }

    // Find an available port if port is 0
    let listener = TcpListener::bind(format!("127.0.0.1:{}", launch.port)).await?;
    let actual_port = listener.local_addr()?.port();
    drop(listener); // Release the port so the proxy can bind to it

    let addr: SocketAddr = format!("127.0.0.1:{}", actual_port).parse()?;
    info!("Starting muninn proxy on {} for {}", addr, launch.agent_cmd);

    // Resolve working directory - canonicalize to resolve relative paths like "." or ".."
    let work_path = launch
        .workdir
        .clone()
        .unwrap_or_else(|| muninn_dir.join(&launch.config.project.root));
    let work_path = work_path.canonicalize().unwrap_or(work_path);

    // Emit deprecation warning if using old [backend] section
    launch.config.warn_deprecated_backend();

    // Configure router strategy
    let router_strategy = launch
        .router_strategy
        .map(|s| parse_router_strategy(&s))
        .unwrap_or_else(|| parse_router_strategy(&launch.config.router.strategy));

    let resolved_router = launch.config.resolved_router();
    let resolved_rlm = launch.config.resolved_rlm();

    let router_config = RouterConfig {
        strategy: router_strategy,
        enabled: launch.config.router.enabled,
        router_model: Some(resolved_router.model.clone()),
    };

    // Open graph store if available, or start background indexing
    let graph_path = launch.config.resolve_graph_path(Some(&muninn_dir));
    let graph_store = open_graph_store(&graph_path)?;

    // Open doc store if available (default: .muninn/docs.db)
    let doc_path = muninn_dir.join("docs.db");
    let doc_store = open_doc_store(&doc_path)?;

    // Start background indexing if graph doesn't exist
    if graph_store.is_none() {
        start_background_indexing(
            graph_path.clone(),
            work_path.clone(),
            launch.config.graph.extensions.clone(),
        );
    }

    // Start file watcher to keep graph in sync with source changes
    // Uses 1 second debounce to batch rapid changes
    start_file_watcher(graph_path, work_path.clone(), 1000);

    // Create separate backends for router and RLM
    // If CLI provides groq_key, use it for both; otherwise use config
    let (router_backend, rlm_backend) = if let Some(key) = launch.groq_key.clone() {
        info!("Using Groq backend from CLI for both router and RLM");
        let router_groq = GroqConfig::new(key.clone()).with_model(&resolved_router.model);
        let rlm_groq = GroqConfig::new(key).with_model(&resolved_rlm.model);
        (
            Some(Arc::new(GroqBackend::new(router_groq)?) as Arc<dyn muninn_rlm::LLMBackend>),
            Some(Arc::new(GroqBackend::new(rlm_groq)?) as Arc<dyn muninn_rlm::LLMBackend>),
        )
    } else {
        // Create router backend
        let router_backend = create_backend_from_config(
            &resolved_router.provider,
            &resolved_router.model,
            &launch.config,
            Some(&muninn_dir),
        )?;

        // Create RLM backend
        let rlm_backend = create_backend_from_config(
            &resolved_rlm.provider,
            &resolved_rlm.model,
            &launch.config,
            Some(&muninn_dir),
        )?;

        (router_backend, rlm_backend)
    };

    // Log which models are being used
    info!(
        "Router: {} via {}",
        resolved_router.model, resolved_router.provider
    );
    info!("RLM: {} via {}", resolved_rlm.model, resolved_rlm.provider);

    // Create tools
    let tools: Arc<dyn muninn_rlm::ToolEnvironment> =
        Arc::new(create_tools(&work_path, graph_store, doc_store));

    // Token manager uses the muninn_dir we resolved earlier
    let token_manager = FileTokenManager::new(&muninn_dir);

    // Check if API key is available as fallback
    let has_api_key = std::env::var("ANTHROPIC_API_KEY").is_ok();

    // Check if we need to authenticate
    // OAuth is only required if no API key is available AND no OAuth tokens exist
    let needs_auth = if has_api_key {
        // API key available - OAuth is optional, just use tokens if they exist
        if token_manager.has_tokens() {
            info!("Using OAuth tokens (API key available as fallback)");
        } else {
            info!("Using API key for authentication");
        }
        false
    } else if !token_manager.has_tokens() {
        info!("No OAuth tokens or API key found - starting authentication flow...");
        true
    } else {
        // Check if tokens are valid
        match token_manager.get_valid_access_token().await {
            Ok(_) => {
                info!("Using existing OAuth tokens");
                false
            }
            Err(e) => {
                info!("OAuth tokens invalid ({}), re-authenticating...", e);
                true
            }
        }
    };

    if needs_auth {
        run_oauth_flow(&token_manager).await?;
    }

    let shared_token_manager = create_token_manager(&muninn_dir);

    // Create and start the proxy server with OAuth support
    let rlm_budget = config_to_rlm_budget(&launch.config.budget);
    info!(
        "Budget config: max_depth={}, max_tool_calls={}, max_tokens={}",
        launch.config.budget.max_depth,
        launch.config.budget.max_tool_calls,
        launch.config.budget.max_tokens
    );

    let proxy_config = ProxyConfig::new(addr)
        .with_token_manager(shared_token_manager)
        .with_budget(rlm_budget)
        .with_work_dir(&work_path);

    // Build server with separate router and RLM backends
    let server = match (router_backend, rlm_backend) {
        (Some(router_be), Some(rlm_be)) => ProxyServer::with_separate_backends(
            proxy_config,
            router_be,
            rlm_be,
            tools,
            router_config,
        ),
        (_, Some(rlm_be)) => {
            // No router backend, use RLM backend for both
            info!("Router backend not available, using RLM backend for routing");
            ProxyServer::with_router(proxy_config, rlm_be, tools, router_config)
        }
        _ => {
            info!("No RLM backend configured, running in passthrough-only mode");
            ProxyServer::passthrough_only(proxy_config)
        }
    };

    // Channel to signal proxy is ready
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();

    // Spawn proxy in background
    let proxy_handle = tokio::spawn(async move {
        // Signal ready before starting (server binds immediately)
        let _ = ready_tx.send(());
        if let Err(e) = server.run().await {
            tracing::error!("Proxy server error: {}", e);
        }
    });

    // Wait for proxy to be ready
    ready_rx.await?;

    // Give the proxy a moment to fully bind
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Build the proxy URL
    let proxy_url = format!("http://127.0.0.1:{}", actual_port);
    info!("Proxy ready at {}", proxy_url);

    // Get the API key to pass through (agent still needs this for auth header)
    // When using OAuth, we use a placeholder since the proxy handles real auth
    let api_key = std::env::var("ANTHROPIC_API_KEY").unwrap_or_else(|_| "muninn-proxy".to_string());

    // Clear screen before launching agent for clean TUI handoff
    // This ensures no shell prompt residue when Claude takes over the terminal
    print!("\x1b[2J\x1b[H");
    std::io::Write::flush(&mut std::io::stdout())?;

    // Launch agent with environment configured
    // Claude Code uses ANTHROPIC_AUTH_TOKEN (not API_KEY) for custom endpoints
    let mut cmd = Command::new(&launch.agent_cmd);
    cmd.args(&launch.agent_args)
        .env("ANTHROPIC_BASE_URL", &proxy_url)
        .env("ANTHROPIC_AUTH_TOKEN", &api_key)
        .env("NO_PROXY", "127.0.0.1") // Prevent proxy interference
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    // Spawn agent and wait for it
    let mut child = cmd.spawn().map_err(|e| {
        anyhow::anyhow!(
            "Failed to launch '{}'. Is it installed? Error: {}",
            launch.agent_cmd,
            e
        )
    })?;

    // Wait for agent to exit or for ctrl+c
    tokio::select! {
        status = child.wait() => {
            match status {
                Ok(exit) => {
                    if exit.success() {
                        info!("{} exited successfully", launch.agent_cmd);
                    } else {
                        info!("{} exited with status: {}", launch.agent_cmd, exit);
                    }
                }
                Err(e) => {
                    tracing::error!("Error waiting for {}: {}", launch.agent_cmd, e);
                }
            }
        }
        _ = signal::ctrl_c() => {
            info!("Received interrupt, shutting down...");
            let _ = child.kill().await;
        }
    }

    // Shutdown proxy
    proxy_handle.abort();
    info!("Muninn proxy stopped");

    Ok(())
}

/// Run the OAuth PKCE flow for Claude MAX authentication.
async fn run_oauth_flow(token_manager: &FileTokenManager) -> Result<()> {
    use std::io::{self, Write};

    let oauth_config = OAuthConfig::default();

    // Generate PKCE challenge and state
    let pkce = PkceChallenge::generate();
    let state = generate_state();

    // Build authorization URL
    let auth_url = build_authorization_url(&oauth_config, &pkce.challenge, &state);

    println!();
    println!("=== Claude MAX OAuth Authentication ===");
    println!();
    println!("To authenticate with your Claude MAX subscription:");
    println!();
    println!("1. Open this URL in your browser:");
    println!();
    println!("   {}", auth_url);
    println!();
    println!("2. Log in and authorize the application");
    println!();
    println!("3. After authorizing, you'll see a page with a code and state.");
    println!("   Copy them and paste below in this format: code#state");
    println!();
    println!("   Example: abc123xyz...#def456uvw...");
    println!();
    println!("=========================================");
    println!();

    // Read user input
    print!("Paste code#state here: ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    // Parse code and state
    let (code, returned_state) = parse_code_state(&input).map_err(|e| anyhow::anyhow!("{}", e))?;

    // Verify state matches (CSRF protection)
    if returned_state != state {
        anyhow::bail!("State mismatch - possible CSRF attack. Please try again.");
    }

    println!();
    info!("Authorization code received, exchanging for tokens...");

    // Exchange code for tokens
    let tokens = exchange_code_for_tokens(&oauth_config, &code, &pkce.verifier, &state)
        .await
        .map_err(|e| anyhow::anyhow!("Token exchange failed: {}", e))?;

    // Save tokens
    token_manager
        .save_tokens(&tokens)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    println!();
    info!("Authentication successful!");
    info!("Tokens saved to {}", token_manager.token_path().display());
    println!();
    info!("You can now use 'muninn claude' with your MAX subscription.");
    info!("Tokens will auto-refresh when they expire (8-hour lifetime).");

    Ok(())
}
