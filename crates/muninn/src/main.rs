//! muninn: Privacy-first recursive context gateway
//!
//! Muninn sits between your coding agent (like Claude Code) and local LLMs,
//! providing intelligent request routing and deep context exploration.

mod config;
mod install;
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
use muninn_graph::{GraphBuilder, GraphStore};
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
            if let Some(r) = config.ollama.max_retries {
                ollama_config = ollama_config.with_max_retries(r);
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

        /// Wipe the graph database and rebuild from scratch.
        /// Useful when symbols accumulated stale duplicates (e.g.
        /// across schema changes or after the cross-file resolver
        /// improved). Without this, `muninn index` is additive —
        /// it upserts nodes and edges but doesn't remove anything
        /// that no longer matches a current source file.
        #[arg(long)]
        reset: bool,
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

    /// Manage the muninn daemon (engine over local IPC)
    Daemon {
        #[command(subcommand)]
        command: DaemonCommand,
    },

    /// UserPromptSubmit hook plumbing — invoked once per user turn
    /// by the muninn-cc plugin.
    Hook {
        #[command(subcommand)]
        command: HookCommand,
    },

    /// Register the muninn MCP server (and print plugin install
    /// instructions) into a target Claude Code config.
    ///
    /// Default scope is the current project (writes `.mcp.json`).
    /// Use `--global` to write `~/.claude.json` instead.
    #[command(name = "install-cc")]
    InstallCc {
        /// Write to `~/.claude.json` instead of the project `.mcp.json`.
        #[arg(long)]
        global: bool,
        /// Print what would change without writing anything.
        #[arg(long)]
        dry_run: bool,
    },

    /// Remove the muninn MCP entry from a target Claude Code config.
    #[command(name = "uninstall-cc")]
    UninstallCc {
        /// Operate on `~/.claude.json` instead of the project `.mcp.json`.
        #[arg(long)]
        global: bool,
        /// Print what would change without writing anything.
        #[arg(long)]
        dry_run: bool,
    },

    /// Run a stdio MCP server backed by the muninn engine.
    ///
    /// Auto-ensures the daemon is running, connects a client, and
    /// exposes the curated engine tool set (search_code, query_graph)
    /// over the Model Context Protocol.
    /// Intended to be launched by an MCP client (e.g. Claude Code's
    /// mcp.json).
    Mcp {
        /// Override the daemon socket path. Defaults to the repo-scoped
        /// path under `$XDG_RUNTIME_DIR/muninn/`.
        #[arg(long)]
        socket: Option<PathBuf>,

        /// Skip the `daemon ensure` step. Use when the daemon is
        /// already known to be running (e.g. when this command is
        /// invoked by `daemon ensure` itself, or in tests).
        #[arg(long)]
        no_ensure: bool,
    },
}

/// Subcommands for the local-IPC engine daemon.
#[derive(Subcommand)]
enum DaemonCommand {
    /// Start the daemon in the foreground. Ctrl-C to stop.
    Start {
        /// Override the socket path. Defaults to the repo-scoped path
        /// under `$XDG_RUNTIME_DIR/muninn/` (or platform equivalent).
        #[arg(long)]
        socket: Option<PathBuf>,
    },
    /// Report whether a daemon is reachable at the socket path.
    Status {
        /// Override the socket path (see `start --socket`).
        #[arg(long)]
        socket: Option<PathBuf>,
    },
    /// Stop the daemon associated with the socket path. Sends SIGTERM
    /// and escalates to SIGKILL if it doesn't exit within a few seconds.
    Stop {
        /// Override the socket path (see `start --socket`).
        #[arg(long)]
        socket: Option<PathBuf>,
    },
    /// Make sure a daemon is alive at the socket path, spawning one
    /// (detached) if not. Idempotent.
    Ensure {
        /// Override the socket path (see `start --socket`).
        #[arg(long)]
        socket: Option<PathBuf>,
    },
}

/// Subcommands for Claude Code hook integration.
#[derive(Subcommand)]
enum HookCommand {
    /// Read a CC UserPromptSubmit hook-input from stdin and emit a
    /// turn-start `additionalContext` block on stdout. Fires once per
    /// user message before Claude starts, so muninn gets to pre-load
    /// project context into the agent's working set on a local model.
    ///
    /// Silent-passthrough on any failure — never blocks the turn.
    Submit,
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
            reset,
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

            // If --reset, remove the existing DB file before opening
            // so the new GraphStore is empty. Keeps semantics clean:
            // we don't have a per-table truncate, and rebuild is fast
            // enough that wipe-and-rebuild is the simplest correct path.
            if reset && graph_path.exists() {
                info!("Resetting graph at {}", graph_path.display());
                std::fs::remove_file(&graph_path)?;
                // Also remove any sqlite sidecar files (WAL, SHM).
                for suffix in ["-wal", "-shm", "-journal"] {
                    let sidecar = graph_path.with_extension(format!(
                        "{}{}",
                        graph_path
                            .extension()
                            .and_then(|e| e.to_str())
                            .unwrap_or(""),
                        suffix
                    ));
                    let _ = std::fs::remove_file(sidecar);
                }
                // Drop the Merkle snapshot too — otherwise the
                // incremental gate would see "no changes" against an
                // empty graph and skip the rebuild we just asked for.
                let _ = std::fs::remove_file(muninn_dir.join("incremental-state.bin"));
            }

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

            // Incremental gate: walk the tree, hash everything, compare
            // against the previous Merkle snapshot. If nothing changed,
            // skip extraction entirely — the graph is already up to date.
            // On a fresh run (no prior snapshot) or `--reset` (snapshot
            // deleted alongside graph.db) this falls through to the full
            // build.
            let state_path = muninn_dir.join("incremental-state.bin");
            let no_op_parse = |_p: &std::path::Path| Ok(Vec::new());
            let new_tree =
                muninn_narsil_vendor::incremental::MerkleTree::build(&source_path, no_op_parse)?;

            let skip = if reset {
                false
            } else if state_path.exists() {
                match muninn_narsil_vendor::incremental::MerkleTree::load(&state_path) {
                    Ok(old_tree) => {
                        let cs = old_tree.diff(&new_tree);
                        if cs.is_empty() {
                            info!(
                                "Graph already up to date for {} (no source changes since last index)",
                                source_path.display()
                            );
                            true
                        } else {
                            info!(
                                "Detected {} added / {} modified / {} deleted files since last index",
                                cs.added.len(),
                                cs.modified.len(),
                                cs.deleted.len(),
                            );
                            false
                        }
                    }
                    Err(e) => {
                        info!("Could not load incremental state ({e}); doing full reindex");
                        false
                    }
                }
            } else {
                false
            };

            if skip {
                // We're done — no need to spin the extractor.
                return Ok(());
            }

            // Drive the vendored narsil extractor over the source tree.
            // This is the only indexing path muninn supports.
            let mut builder = GraphBuilder::new(store)?;
            let stats = builder.build_directory(&source_path)?;
            info!(
                "Indexed {} files, {} nodes, {} edges",
                stats.files_processed, stats.nodes_added, stats.edges_added
            );

            // Persist the new snapshot so the next `muninn index` can
            // short-circuit if nothing changed.
            if let Err(e) = new_tree.save(&state_path) {
                tracing::warn!(
                    "Failed to save incremental state to {}: {e}",
                    state_path.display()
                );
            }

            if watch {
                anyhow::bail!(
                    "--watch is not supported — re-run `muninn index` after \
                     structural changes you want reflected in the graph."
                );
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

# Provider credentials.
#
# Uncomment one block below to set credentials in this file, OR
# export OLLAMA_API_KEY / GROQ_API_KEY / ANTHROPIC_API_KEY in the
# environment muninn runs from. Note: Claude Code's hook + MCP
# subprocesses may not inherit your interactive shell's env, so
# in-file credentials are usually the most reliable.
#
# The section header AND fields are commented out together so that
# adding a fresh `[ollama]` (etc.) section later in this file
# won't be silently overridden by an empty section above. Either
# uncomment in place, or paste a fresh block.

# [ollama]
# api_key = "..."                              # Ollama Cloud key — leave base_url commented to talk to Ollama Cloud.

# For LOCAL Ollama instead of Ollama Cloud, uncomment BOTH lines below.
# The Ollama Cloud key above must then be commented out (or it will be
# sent as a stray bearer token to a server that doesn't want it).
# [ollama]
# base_url = "http://localhost:11434/v1"

# [groq]
# api_key = "gsk_..."

# [anthropic]
# api_key = "sk-..."
"#;

            std::fs::write(&config_path, default_config)?;
            // Use println — `muninn init` is a one-shot command and
            // doesn't initialize the tracing subscriber, so info!
            // would silently swallow these.
            println!("Initialized {}", muninn_dir.display());
            println!("Wrote   {}", config_path.display());
            println!();
            println!("Next steps:");
            println!(
                "  1. Add a provider credential to .muninn/config.toml \
                 (under [ollama]/[groq]/[anthropic] api_key), or export \
                 OLLAMA_API_KEY / GROQ_API_KEY / ANTHROPIC_API_KEY in your \
                 shell. The default config targets Ollama Cloud."
            );
            println!(
                "  2. For Claude Code: `muninn install-cc` here, then inside CC \
                 run `/plugin marketplace add colliery-io/muninn` and \
                 `/plugin install muninn-cc`."
            );
            println!(
                "  3. (Optional) `muninn index` to populate the code graph so \
                 the MCP `query_graph` tool returns non-empty results."
            );
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
                        "{:<20} {:<10} {:<10} INDEXED AT",
                        "LIBRARY", "VERSION", "ECOSYSTEM",
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

        Commands::Daemon { command } => {
            // For `daemon start`, route logs to a rolling file under
            // `.muninn/logs/` so failures inside the spawned daemon
            // are diagnosable. `ensure_daemon` nulls the child's
            // stdout/stderr, so without file logging the daemon is
            // invisible. For other daemon subcommands (`status`,
            // `stop`, `ensure`) keep stderr logging — those run in
            // the user's foreground shell.
            if matches!(command, DaemonCommand::Start { .. }) {
                let muninn_dir = config_dir
                    .as_deref()
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from(config::MUNINN_DIR));
                init_file_logging(&muninn_dir, cli.verbose);
            } else {
                init_logging(cli.verbose);
            }
            run_daemon_command(command, &config, config_dir.as_deref()).await?;
        }

        Commands::Hook { command } => {
            // Hook decisions must be quiet on stdout — Claude Code reads
            // the hook response from there — so route tracing to stderr.
            init_logging_stderr_only(cli.verbose);
            run_hook_command(command, &config, config_dir.as_deref()).await?;
        }

        Commands::InstallCc { global, dry_run } => {
            init_logging(cli.verbose);
            let scope = if global {
                install::InstallScope::Global
            } else {
                install::InstallScope::Project
            };
            let outcome = install::install(scope, config_dir.as_deref(), dry_run)?;
            println!("{}", install::describe_install(&outcome, scope));
            println!();
            println!("{}", install::plugin_install_notice());
        }

        Commands::UninstallCc { global, dry_run } => {
            init_logging(cli.verbose);
            let scope = if global {
                install::InstallScope::Global
            } else {
                install::InstallScope::Project
            };
            let outcome = install::uninstall(scope, config_dir.as_deref(), dry_run)?;
            println!("{}", install::describe_uninstall(&outcome, scope));
        }

        Commands::Mcp { socket, no_ensure } => {
            // CRITICAL: log to stderr only. stdout is reserved for MCP
            // protocol frames; mixing tracing output in would corrupt
            // every response.
            init_logging_stderr_only(cli.verbose);

            let socket_path = resolve_daemon_socket(socket, config_dir.as_deref());
            if !no_ensure {
                let exe = std::env::current_exe()
                    .map_err(|e| anyhow::anyhow!("locate muninn binary: {}", e))?;
                let mut extra: Vec<std::ffi::OsString> = Vec::new();
                if let Some(d) = config_dir.as_deref() {
                    extra.push("--config".into());
                    extra.push(d.as_os_str().to_owned());
                }
                muninn_rlm::daemon::ensure_daemon_with_args(&socket_path, &exe, &extra)
                    .await
                    .map_err(|e| anyhow::anyhow!("daemon ensure: {}", e))?;
            }
            let client = muninn_rlm::daemon::DaemonClient::connect(&socket_path)
                .await
                .map_err(|e| anyhow::anyhow!("daemon connect: {}", e))?;
            let engine: muninn_rlm::SharedEngine = Arc::new(client);
            muninn_rlm::mcp_engine_server::run_engine_mcp_server(engine)
                .await
                .map_err(|e| anyhow::anyhow!("mcp server: {}", e))?;
        }
    }

    Ok(())
}

/// Handle `muninn hook …` subcommands. All paths in this handler
/// return `Ok(())` even on failure — `decide` is contractually
/// allowed to emit nothing and exit 0, which Claude Code reads as
/// "allow original tool unchanged" (NFR-002 silent passthrough).
async fn run_hook_command(
    command: HookCommand,
    config: &Config,
    config_dir: Option<&std::path::Path>,
) -> Result<()> {
    match command {
        HookCommand::Submit => {
            run_hook_submit(config, config_dir).await;
            Ok(())
        }
    }
}

/// Resolve the daemon socket path the hook should target — the
/// repo-scoped path that `muninn daemon ensure` would compute, so
/// the hook and the daemon agree on where to find each other
/// without any extra CLI plumbing.
///
/// `MUNINN_HOOK_TEST_SOCKET`, when set, overrides the resolved path.
/// Used by UAT to drive the hook against an isolated tempdir daemon
/// without needing the canonical repo-scoped socket to exist.
fn hook_socket_path(_config: &Config, config_dir: Option<&std::path::Path>) -> PathBuf {
    if let Some(override_path) = std::env::var_os("MUNINN_HOOK_TEST_SOCKET") {
        return PathBuf::from(override_path);
    }
    let repo_root = config_dir
        .and_then(|p| p.parent().map(PathBuf::from))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    muninn_rlm::daemon::socket_path_for_repo(&repo_root)
}

/// What the hook tells Claude Code to do. Either "let the turn
/// proceed unchanged" (Passthrough — empty stdout sentinel) or
/// "attach this block as `additionalContext`" (Augment).
enum HookResponse {
    Passthrough,
    Augment(String),
}

impl HookResponse {
    /// Serialize to CC's hook-response JSON envelope on stdout.
    /// `event` is the CC hook event name (e.g. "UserPromptSubmit").
    fn write_to_stdout_for_event(self, event: &str) {
        use std::io::Write;
        match self {
            HookResponse::Passthrough => {}
            HookResponse::Augment(context) => {
                let body = serde_json::json!({
                    "hookSpecificOutput": {
                        "hookEventName": event,
                        "additionalContext": context,
                    }
                });
                let _ = writeln!(std::io::stdout(), "{body}");
            }
        }
    }
}

/// CC UserPromptSubmit hook input shape. Fields we don't read are
/// tolerated so future CC additions don't break parsing.
#[derive(serde::Deserialize)]
struct UserPromptInput {
    #[serde(default)]
    prompt: String,
}

/// Body of `muninn hook submit`. Fires once per user turn before
/// Claude starts. Drives a brief RLM exploration of the user's
/// prompt against the cheap configured backend and injects the
/// resulting summary as `additionalContext`. The point is to off-load
/// "go find the relevant files and patterns" from Claude to muninn's
/// local model — Claude then composes its response with the
/// exploration findings already in context.
///
/// Failure mode is silent passthrough (NFR-002): if the daemon is
/// down, the model errors, or the exploration runs over budget, the
/// user's turn proceeds without muninn pre-injection.
async fn run_hook_submit(config: &Config, config_dir: Option<&std::path::Path>) {
    // Generous outer cap. The user is already waiting for Claude's
    // first token, and real RLM exploration on a local/cheap model
    // regularly takes 20-60s for code-shaped questions, so 240s of
    // pre-exploration is the realistic floor — anything tighter
    // makes muninn silently disappear for the prompts it's most
    // useful on. Still bounded so a hung daemon can't strand the turn.
    //
    // `MUNINN_HOOK_DEADLINE_MS` lets UAT shrink the cap so the
    // timeout-backstop path can be exercised in a few seconds
    // instead of the full default; not intended for production use.
    const SUBMIT_DEADLINE_DEFAULT: std::time::Duration = std::time::Duration::from_secs(240);
    let deadline = std::env::var("MUNINN_HOOK_DEADLINE_MS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .map(std::time::Duration::from_millis)
        .unwrap_or(SUBMIT_DEADLINE_DEFAULT);

    let outcome = tokio::time::timeout(deadline, submit_inner(config, config_dir)).await;

    let response = match outcome {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => {
            tracing::debug!(error = %e, "hook submit errored — passthrough");
            HookResponse::Passthrough
        }
        Err(_) => {
            tracing::debug!("hook submit timed out — passthrough");
            HookResponse::Passthrough
        }
    };
    response.write_to_stdout_for_event("UserPromptSubmit");
}

/// User-message prefix when we drive the engine for the
/// answer-shaped RLM exploration. The engine replaces the caller's
/// `system` field with `CORE_RLM_BEHAVIOR`, so we embed the
/// instructions in the user message instead. The framing is
/// deliberately strong: muninn is producing the *answer* the
/// downstream agent should deliver, not advisory context for it to
/// re-verify.
const SUBMIT_RLM_INSTRUCTION: &str = "\
[muninn turn-start exploration]\n\
\n\
You are priming the downstream agent (Claude Code) with the code \
context it needs to answer this user prompt. You only get one \
shot — your answer goes straight into the agent's conversation \
and stays there for the whole session, so include enough \
substance that follow-up turns can work from it without you. \
The agent has no way to call you again for this prompt; if it \
needs a fresh exploration later it will re-trigger you with a \
new user prompt (often with `@muninn explore`).\n\
\n\
Front-load substance over cleverness:\n\
- Quote actual code (not just file paths). Verbatim snippets \
  with surrounding context beat references that the agent has \
  to chase.\n\
- Cite file paths and line numbers for each snippet so the \
  agent can navigate when it needs to verify.\n\
- If the prompt asks for code changes, include the concrete \
  edit plan: file path + the diff or the replacement snippet \
  inline.\n\
- It's fine to over-include relevant context; the agent's \
  follow-up turns will discard what they don't need.\n\
- Keep the answer focused and under ~1200 tokens.\n\
- End with FINAL(<the complete answer>).";

async fn submit_inner(
    config: &Config,
    config_dir: Option<&std::path::Path>,
) -> Result<HookResponse> {
    use tokio::io::AsyncReadExt;
    let mut buf = String::new();
    tokio::io::stdin()
        .read_to_string(&mut buf)
        .await
        .map_err(|e| anyhow::anyhow!("read stdin: {e}"))?;
    let input: UserPromptInput = serde_json::from_str(&buf)
        .map_err(|e| anyhow::anyhow!("parse user-prompt-submit input: {e}"))?;

    // Floor cases — fast-path passthrough without burning an LLM
    // call. Keeps the hook honest for chat-shaped messages and the
    // explicit `@muninn passthrough` marker.
    let prompt = input.prompt.trim();
    if prompt.is_empty() || prompt.contains("@muninn passthrough") || prompt.len() < 8 {
        return Ok(HookResponse::Passthrough);
    }

    // Connect to the daemon. `submit_inner` itself doesn't ensure
    // the daemon — the plugin's shell entry (`user-prompt-submit.sh`)
    // runs `muninn daemon ensure` ahead of us, which is idempotent
    // when the daemon is already alive. If for any reason no daemon
    // is up at this point (the ensure call failed, race, etc.) we
    // degrade to silent passthrough per NFR-002.
    let socket = hook_socket_path(config, config_dir);
    if !muninn_rlm::daemon::is_alive(&socket).await {
        return Ok(HookResponse::Passthrough);
    }
    let client = muninn_rlm::daemon::DaemonClient::connect(&socket)
        .await
        .map_err(|e| anyhow::anyhow!("daemon connect: {e}"))?;

    // ── Stage 1: router gate ──
    //
    // Reuse the proxy's router with its tuned RLM-biased prompt. The
    // router runs against the resolved `[router]` model (cheap by
    // default; small fast model). Passthrough cases skip the
    // expensive RLM call entirely.
    let resolved_router = config.resolved_router();
    let router_backend = create_backend_from_config(
        &resolved_router.provider,
        &resolved_router.model,
        config,
        config_dir,
    )?
    .ok_or_else(|| {
        anyhow::anyhow!(
            "no router backend (provider={}, model={})",
            resolved_router.provider,
            resolved_router.model
        )
    })?;
    let router = muninn_rlm::Router::with_config(muninn_rlm::RouterConfig {
        strategy: muninn_rlm::RouterStrategy::Llm,
        enabled: true,
        router_model: Some(resolved_router.model.clone()),
    })
    .with_llm(router_backend);

    let probe_request = muninn_rlm::CompletionRequest::new(
        &resolved_router.model,
        vec![muninn_rlm::Message::user(prompt)],
        128,
    );
    let decision = router.route(&probe_request).await;
    if decision.is_passthrough() {
        return Ok(HookResponse::Passthrough);
    }

    // ── Stage 2: RLM exploration via the daemon ──
    //
    // Router said "code context matters." Drive the recursive engine
    // to produce a complete answer; muninn replaces Claude's
    // exploration work for this turn.
    let resolved_rlm = config.resolved_rlm();
    let user_message = format!("{}\n\nUser prompt:\n{}", SUBMIT_RLM_INSTRUCTION, prompt);
    let rlm_request = muninn_rlm::CompletionRequest::new(
        &resolved_rlm.model,
        vec![muninn_rlm::Message::user(user_message)],
        2048,
    )
    .with_muninn(muninn_rlm::MuninnConfig::recursive());

    let response = {
        use muninn_rlm::MuninnEngine;
        client
            .complete(rlm_request)
            .await
            .map_err(|e| anyhow::anyhow!("rlm complete: {e}"))?
    };
    let text = response.text();
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(HookResponse::Passthrough);
    }

    // Prefer the FINAL(...) capture; otherwise relay the body.
    let answer = extract_final_capture(trimmed)
        .unwrap_or_else(|| trimmed.to_string())
        .trim()
        .to_string();
    if answer.is_empty() {
        return Ok(HookResponse::Passthrough);
    }

    // Answer-shaped framing: muninn primes the conversation with
    // code context for this turn. The contract is explicit:
    //   - This is a one-shot priming dump, not a re-callable tool.
    //   - The answer persists in the agent's context for the whole
    //     session, so follow-up turns work from what's already here.
    //   - When a fresh exploration is genuinely warranted, the user
    //     (or the agent reasoning on the user's behalf) re-triggers
    //     with `@muninn explore <prompt>`.
    //   - Quality caveats: file paths + verbatim snippets are
    //     reliable; line numbers may drift by a few lines.
    let block = format!(
        "─── muninn turn-start answer ───\n\
         Muninn primed this turn with the code context below. Prefer \
         it as your starting point rather than re-doing the same \
         exploration — file paths, structural claims, and verbatim \
         code snippets are reliable; line numbers may be approximate. \
         Verify what you'd reasonably double-check before acting; \
         skip what you wouldn't.\n\
         \n\
         Scope: this priming is one-shot per user turn. It lives in \
         your context for the rest of the session — follow-up turns \
         should work from what's already here using your normal \
         tools, and the user can re-trigger muninn explicitly with \
         `@muninn explore <prompt>` when fresh exploration is \
         warranted.\n\
         \n\
         {answer}\n\
         ─────────────────────────────────"
    );
    Ok(HookResponse::Augment(block))
}

/// Extract the FINAL(...) capture from an RLM response if present.
/// Mirrors the engine's own pattern.
fn extract_final_capture(text: &str) -> Option<String> {
    let re = regex::Regex::new(r#"(?m)^FINAL\(["']?([\s\S]+?)["']?\)$"#).ok()?;
    re.captures(text)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Set up tracing for the MCP subcommand. Writes only to stderr —
/// stdout is reserved for MCP protocol bytes.
fn init_logging_stderr_only(verbose: bool) {
    let filter = if verbose {
        EnvFilter::new("debug")
    } else {
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))
    };
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_writer(std::io::stderr))
        .try_init();
}

/// Resolve the daemon socket path: explicit override > repo-scoped default.
fn resolve_daemon_socket(
    explicit: Option<PathBuf>,
    config_dir: Option<&std::path::Path>,
) -> PathBuf {
    if let Some(p) = explicit {
        return p;
    }
    // Use the directory containing `.muninn/` as the repo root so two
    // adapters running against the same config find the same socket.
    let repo_root = config_dir
        .and_then(|p| p.parent().map(PathBuf::from))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    muninn_rlm::daemon::socket_path_for_repo(&repo_root)
}

/// Handle `muninn daemon …` subcommands.
async fn run_daemon_command(
    command: DaemonCommand,
    config: &Config,
    config_dir: Option<&std::path::Path>,
) -> Result<()> {
    match command {
        DaemonCommand::Status { socket } => {
            let path = resolve_daemon_socket(socket, config_dir);
            if muninn_rlm::daemon::is_alive(&path).await {
                println!("alive\t{}", path.display());
            } else {
                println!("dead\t{}", path.display());
            }
            Ok(())
        }
        DaemonCommand::Stop { socket } => {
            let path = resolve_daemon_socket(socket, config_dir);
            match muninn_rlm::daemon::stop_daemon(&path).await {
                Ok(()) => {
                    info!("daemon stopped ({})", path.display());
                    Ok(())
                }
                Err(muninn_rlm::daemon::EngineError::NotFound(msg)) => {
                    // Treat "no daemon" as success — `stop` is supposed
                    // to leave the system in a "daemon not running" state.
                    info!("no daemon to stop: {}", msg);
                    Ok(())
                }
                Err(e) => Err(anyhow::anyhow!("daemon stop: {}", e)),
            }
        }
        DaemonCommand::Ensure { socket } => {
            let path = resolve_daemon_socket(socket, config_dir);

            // If the daemon is already alive but the config file has
            // been modified since the daemon started, the running
            // daemon is using a stale config snapshot. Stop it so the
            // spawn below picks up the current config. Without this,
            // edits to .muninn/config.toml (e.g. adding api_key after
            // an initial dry-run boot) silently don't take effect.
            if muninn_rlm::daemon::is_alive(&path).await {
                let pid_file = path.with_extension("sock.pid");
                let staleness = (|| -> Option<()> {
                    let cfg_path = config_dir?.join(config::CONFIG_FILE);
                    let cfg_mtime = std::fs::metadata(&cfg_path).ok()?.modified().ok()?;
                    let pid_mtime = std::fs::metadata(&pid_file).ok()?.modified().ok()?;
                    (cfg_mtime > pid_mtime).then_some(())
                })();
                if staleness.is_some() {
                    info!("config modified after daemon start; restarting to pick up changes");
                    let _ = muninn_rlm::daemon::stop_daemon(&path).await;
                }
            }

            // Pre-validate the config so credential / provider errors
            // surface as actionable messages instead of as a 10s
            // "daemon did not come up within timeout" with no
            // breadcrumb. The spawned daemon would fail with the
            // same errors, but its stdout/stderr are nulled.
            let errors = config.validate();
            if !errors.is_empty() {
                let mut msg =
                    String::from("muninn daemon ensure: config validation failed before spawn:\n");
                for e in &errors {
                    msg.push_str(&format!("  - {e}\n"));
                }
                msg.push_str(
                    "Fix the above and retry. Tip: set provider credentials \
                     in your .muninn/config.toml (under [ollama]/[groq]/[anthropic] \
                     api_key) rather than env vars when running from Claude Code — \
                     CC's subprocess environment may not inherit shell exports.",
                );
                anyhow::bail!(msg);
            }

            let exe = std::env::current_exe()
                .map_err(|e| anyhow::anyhow!("locate muninn binary: {}", e))?;
            // Propagate --config so the spawned daemon reads the
            // same config we did. Without this the daemon falls back
            // to CWD-based discovery and silently disagrees.
            let mut extra: Vec<std::ffi::OsString> = Vec::new();
            if let Some(d) = config_dir {
                extra.push("--config".into());
                extra.push(d.as_os_str().to_owned());
            }
            muninn_rlm::daemon::ensure_daemon_with_args(&path, &exe, &extra)
                .await
                .map_err(|e| anyhow::anyhow!("daemon ensure: {}", e))?;
            info!("daemon alive at {}", path.display());
            Ok(())
        }
        DaemonCommand::Start { socket } => {
            let socket_path = resolve_daemon_socket(socket, config_dir);

            // Build a default engine using the resolved tiered config.
            let resolved_rlm = config.resolved_rlm();
            let rlm_backend = create_backend_from_config(
                &resolved_rlm.provider,
                &resolved_rlm.model,
                config,
                config_dir,
            )?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "no backend available for daemon (provider={}, model={}). \
                     Configure credentials and retry.",
                    resolved_rlm.provider,
                    resolved_rlm.model
                )
            })?;

            // Build minimal tools + stores aligned with the proxy path's
            // construction. The daemon shares the same engine shape.
            let work_path = config_dir
                .map(|d| d.join(&config.project.root))
                .unwrap_or_else(|| config.project.root.clone());
            let work_path = work_path.canonicalize().unwrap_or(work_path);

            let graph_path = config.resolve_graph_path(config_dir);
            let graph_store = open_graph_store(&graph_path)?;

            let doc_path = config_dir
                .map(|d| d.join("docs.db"))
                .unwrap_or_else(|| PathBuf::from(".muninn/docs.db"));
            let doc_store = open_doc_store(&doc_path)?;

            // Keep a handle to the graph store for the engine; the
            // tools layer needs its own clone, so split before
            // consuming into create_tools.
            let engine_graph_store = graph_store.clone();
            let tools: Arc<dyn muninn_rlm::ToolEnvironment> =
                Arc::new(create_tools(&work_path, graph_store, doc_store));

            let engine = muninn_rlm::engine::default_engine_with_graph(
                rlm_backend,
                tools,
                Some(config_to_rlm_budget(&config.budget)),
                Some(work_path.clone()),
                engine_graph_store,
            );

            // The daemon does NOT auto-reindex. Narsil's extraction
            // is fast enough to re-run on demand (a few seconds even
            // for medium repos), and the watch-and-rebuild pattern
            // introduced its own correctness gaps. Users re-run
            // `muninn index` when they want fresh graph state.
            if !graph_path.exists() {
                tracing::info!(
                    "graph DB missing at {}; daemon will run with an empty graph. Run `muninn index` to populate.",
                    graph_path.display()
                );
            }

            info!("daemon starting at {}", socket_path.display());
            let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
            // Forward Ctrl-C *and* SIGTERM (what `muninn daemon stop`
            // sends) to the shutdown channel so the socket + PID file
            // get unlinked on the way out. Without the SIGTERM arm,
            // `stop` would kill the process before serve()'s cleanup
            // had a chance to run.
            tokio::spawn(async move {
                #[cfg(unix)]
                {
                    use tokio::signal::unix::{SignalKind, signal};
                    let mut sigterm = match signal(SignalKind::terminate()) {
                        Ok(s) => s,
                        Err(e) => {
                            tracing::warn!(error = %e, "could not install SIGTERM handler");
                            // Fall back to ctrl_c only.
                            let _ = tokio::signal::ctrl_c().await;
                            let _ = shutdown_tx.send(());
                            return;
                        }
                    };
                    tokio::select! {
                        _ = tokio::signal::ctrl_c() => {}
                        _ = sigterm.recv() => {}
                    }
                }
                #[cfg(not(unix))]
                {
                    let _ = tokio::signal::ctrl_c().await;
                }
                let _ = shutdown_tx.send(());
            });
            muninn_rlm::daemon::serve(engine, &socket_path, shutdown_rx)
                .await
                .map_err(|e| anyhow::anyhow!("daemon: {}", e))?;
            info!("daemon stopped");
            Ok(())
        }
    }
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

    // Note: this legacy agent-launch path does NOT auto-bootstrap
    // the graph. Run `muninn index` once before launching if you
    // want a populated graph. The watcher / background-build paths
    // were removed when we adopted narsil's extractor.
    let _ = (&graph_store, &graph_path, &launch.config.graph.extensions);

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
