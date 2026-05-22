//! `muninn install-cc` / `muninn uninstall-cc` implementation.
//!
//! These subcommands register the muninn MCP server into a target
//! Claude Code config so the agent can call `search_code` and
//! `query_graph` over MCP. The muninn-cc plugin, which registers the
//! UserPromptSubmit hook, is installed separately via Claude Code's
//! `/plugin` tooling because CC's local-plugin registration format
//! is less standardized than its `.mcp.json` format; we print clear
//! instructions for that step rather than poking unknown internals.
//!
//! ## File layout
//!
//! - **Project scope (default)**: writes `<repo>/.mcp.json`.
//! - **Global scope (`--global`)**: writes `~/.claude.json`, adding
//!   the entry under `mcpServers` (creating the section if absent).
//!
//! Both scopes are JSON. We preserve unrelated keys verbatim — only
//! the `mcpServers.muninn` key is touched. Before writing, the
//! original file is copied to `<path>.bak` (only when there's
//! something to back up).
//!
//! The format we write matches CC's documented `.mcp.json` shape:
//!
//! ```json
//! {
//!   "mcpServers": {
//!     "muninn": {
//!       "command": "muninn",
//!       "args": ["mcp"],
//!       "env": {}
//!     }
//!   }
//! }
//! ```

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use serde_json::{Map, Value, json};

/// MCP server key — also the name CC will surface in the tool picker.
pub const MUNINN_SERVER_NAME: &str = "muninn";

/// Where to write the registration.
#[derive(Debug, Clone, Copy)]
pub enum InstallScope {
    /// `<repo>/.mcp.json` next to the project root.
    Project,
    /// `~/.claude.json` — applies to every CC session for the current
    /// user.
    Global,
}

impl InstallScope {
    fn label(self) -> &'static str {
        match self {
            InstallScope::Project => "project",
            InstallScope::Global => "global",
        }
    }
}

/// Resolve the on-disk config path for a given scope.
///
/// For [`InstallScope::Project`], we anchor at the repo containing the
/// resolved `.muninn/` directory when one is available; otherwise we
/// fall back to the current working directory. For
/// [`InstallScope::Global`], we use `dirs::home_dir()` and write to
/// `~/.claude.json`.
pub fn resolve_config_path(scope: InstallScope, config_dir: Option<&Path>) -> Result<PathBuf> {
    match scope {
        InstallScope::Project => {
            let repo_root = config_dir
                .and_then(|p| p.parent().map(PathBuf::from))
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
            Ok(repo_root.join(".mcp.json"))
        }
        InstallScope::Global => {
            let home = dirs::home_dir()
                .ok_or_else(|| anyhow!("could not resolve home directory for --global install"))?;
            Ok(home.join(".claude.json"))
        }
    }
}

/// Build the MCP server entry value we write into `mcpServers.muninn`.
pub fn muninn_mcp_entry() -> Value {
    json!({
        "command": "muninn",
        "args": ["mcp"],
        "env": {}
    })
}

/// Result of an install/uninstall operation. Returned so the caller
/// can render a helpful summary.
#[derive(Debug, PartialEq)]
pub enum InstallOutcome {
    /// The entry was added (or rewritten because it differed).
    Wrote {
        path: PathBuf,
        backup: Option<PathBuf>,
    },
    /// The entry was already present and matched; nothing to do.
    AlreadyPresent { path: PathBuf },
    /// `--dry-run` — show what we would have written.
    DryRun {
        path: PathBuf,
        action: &'static str,
        proposed: Value,
    },
}

#[derive(Debug, PartialEq)]
pub enum UninstallOutcome {
    Removed {
        path: PathBuf,
        backup: Option<PathBuf>,
    },
    NothingToRemove {
        path: PathBuf,
    },
    DryRun {
        path: PathBuf,
        action: &'static str,
    },
}

/// Install (or no-op) the muninn MCP entry into the target config.
pub fn install(
    scope: InstallScope,
    config_dir: Option<&Path>,
    dry_run: bool,
) -> Result<InstallOutcome> {
    let path = resolve_config_path(scope, config_dir)?;
    let (mut root, existed) = read_or_empty_object(&path)?;

    let entry = muninn_mcp_entry();
    let servers = ensure_mcp_servers(&mut root);

    if let Some(existing) = servers.get(MUNINN_SERVER_NAME) {
        if existing == &entry {
            return Ok(InstallOutcome::AlreadyPresent { path });
        }
    }

    let action = if servers.contains_key(MUNINN_SERVER_NAME) {
        "rewrite mcpServers.muninn"
    } else {
        "add mcpServers.muninn"
    };
    servers.insert(MUNINN_SERVER_NAME.to_string(), entry.clone());

    if dry_run {
        return Ok(InstallOutcome::DryRun {
            path,
            action,
            proposed: entry,
        });
    }

    let backup = if existed { backup_file(&path)? } else { None };
    write_json_pretty(&path, &root)?;
    Ok(InstallOutcome::Wrote { path, backup })
}

/// Remove the muninn MCP entry from the target config, leaving other
/// `mcpServers` entries intact.
pub fn uninstall(
    scope: InstallScope,
    config_dir: Option<&Path>,
    dry_run: bool,
) -> Result<UninstallOutcome> {
    let path = resolve_config_path(scope, config_dir)?;
    if !path.exists() {
        return Ok(UninstallOutcome::NothingToRemove { path });
    }

    let (mut root, _existed) = read_or_empty_object(&path)?;
    let servers = match root.get_mut("mcpServers").and_then(|v| v.as_object_mut()) {
        Some(s) => s,
        None => return Ok(UninstallOutcome::NothingToRemove { path }),
    };
    if !servers.contains_key(MUNINN_SERVER_NAME) {
        return Ok(UninstallOutcome::NothingToRemove { path });
    }
    let action = "remove mcpServers.muninn";

    if dry_run {
        return Ok(UninstallOutcome::DryRun { path, action });
    }

    servers.remove(MUNINN_SERVER_NAME);
    // If mcpServers is now empty, leave it empty — preserving the key
    // is consistent with CC's expectations and means the user's hand-
    // edited config doesn't get "tidied" unexpectedly.

    let backup = backup_file(&path)?;
    write_json_pretty(&path, &root)?;
    Ok(UninstallOutcome::Removed { path, backup })
}

/// Read the config file as a JSON object (or treat a missing file as
/// `{}`). Returns `(root, existed)`.
fn read_or_empty_object(path: &Path) -> Result<(Value, bool)> {
    if !path.exists() {
        return Ok((Value::Object(Map::new()), false));
    }
    let raw =
        std::fs::read_to_string(path).with_context(|| format!("read existing config {path:?}"))?;
    if raw.trim().is_empty() {
        return Ok((Value::Object(Map::new()), true));
    }
    let parsed: Value =
        serde_json::from_str(&raw).with_context(|| format!("parse JSON config {path:?}"))?;
    if !parsed.is_object() {
        anyhow::bail!(
            "config {path:?} is not a JSON object — refusing to overwrite. \
             Backup and rewrite by hand if needed."
        );
    }
    Ok((parsed, true))
}

/// Borrow the `mcpServers` object out of `root`, creating it if absent.
fn ensure_mcp_servers(root: &mut Value) -> &mut Map<String, Value> {
    let obj = root
        .as_object_mut()
        .expect("read_or_empty_object guarantees an object");
    obj.entry("mcpServers".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    obj.get_mut("mcpServers")
        .and_then(|v| v.as_object_mut())
        .expect("mcpServers entry just created or already an object")
}

/// Copy `path` to `path.bak` for safety before mutating it.
fn backup_file(path: &Path) -> Result<Option<PathBuf>> {
    let mut backup_path = path.as_os_str().to_owned();
    backup_path.push(".bak");
    let backup_path = PathBuf::from(backup_path);
    std::fs::copy(path, &backup_path).with_context(|| format!("write backup {backup_path:?}"))?;
    Ok(Some(backup_path))
}

/// Pretty-print JSON with a trailing newline. We try to keep keys in
/// the same insertion order serde_json's Map preserves, which means
/// unrelated keys stay where the user put them.
fn write_json_pretty(path: &Path, value: &Value) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).with_context(|| format!("create parent dir {parent:?}"))?;
    }
    let mut serialized = serde_json::to_string_pretty(value).context("serialize JSON config")?;
    serialized.push('\n');
    std::fs::write(path, serialized).with_context(|| format!("write config {path:?}"))
}

/// Render a friendly install summary for stdout.
pub fn describe_install(outcome: &InstallOutcome, scope: InstallScope) -> String {
    match outcome {
        InstallOutcome::Wrote { path, backup } => {
            let mut s = format!(
                "installed muninn MCP entry ({}) into {}",
                scope.label(),
                path.display()
            );
            if let Some(b) = backup {
                s.push_str(&format!("\n  backup: {}", b.display()));
            }
            s
        }
        InstallOutcome::AlreadyPresent { path } => format!(
            "muninn MCP entry already present in {} ({}) — no changes",
            path.display(),
            scope.label()
        ),
        InstallOutcome::DryRun {
            path,
            action,
            proposed,
        } => format!(
            "[dry-run] would {} in {} ({})\n  proposed value: {}",
            action,
            path.display(),
            scope.label(),
            proposed
        ),
    }
}

pub fn describe_uninstall(outcome: &UninstallOutcome, scope: InstallScope) -> String {
    match outcome {
        UninstallOutcome::Removed { path, backup } => {
            let mut s = format!(
                "removed muninn MCP entry ({}) from {}",
                scope.label(),
                path.display()
            );
            if let Some(b) = backup {
                s.push_str(&format!("\n  backup: {}", b.display()));
            }
            s
        }
        UninstallOutcome::NothingToRemove { path } => format!(
            "no muninn MCP entry to remove in {} ({})",
            path.display(),
            scope.label()
        ),
        UninstallOutcome::DryRun { path, action } => format!(
            "[dry-run] would {} in {} ({})",
            action,
            path.display(),
            scope.label()
        ),
    }
}

/// Hook + plugin install notice — printed alongside the MCP summary
/// because the UserPromptSubmit plugin lives at the repo root and must
/// be loaded with `claude --plugin-dir <abs-path>` at session start.
/// There is no in-session slash command that adds a local plugin source.
pub fn plugin_install_notice() -> String {
    "Plugin (muninn-cc) install:\n  \
     The UserPromptSubmit hook plugin at plugins/muninn-cc/ is loaded by Claude Code \
     at session start. Restart CC with:\n  \
       claude --plugin-dir <absolute-path>/plugins/muninn-cc\n  \
     (Use an absolute path. `/reload-plugins` inside the session picks up plugin \
     edits without restarting. See plugins/muninn-cc/README.md for details.)"
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn read_json(path: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    /// Install into a missing file → creates it and writes our entry.
    #[test]
    fn install_into_missing_file_creates_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join(".mcp.json");
        // Synthesize a config_dir whose parent is `dir.path()` so the
        // project scope resolves to <dir>/.mcp.json. We pass through a
        // helper to avoid juggling InstallScope::Project's CWD lookup.
        let outcome = install_with_path(&path, false).unwrap();
        match outcome {
            InstallOutcome::Wrote { backup, .. } => assert!(backup.is_none()),
            other => panic!("expected Wrote, got {other:?}"),
        }
        let root = read_json(&path);
        assert_eq!(root["mcpServers"]["muninn"], muninn_mcp_entry());
    }

    /// Install over an existing config with other entries preserves
    /// them and adds ours alongside.
    #[test]
    fn install_preserves_unrelated_entries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".mcp.json");
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&json!({
                "mcpServers": {
                    "other": {"command": "other", "args": []}
                },
                "unrelatedTopLevel": 42
            }))
            .unwrap(),
        )
        .unwrap();

        let outcome = install_with_path(&path, false).unwrap();
        match outcome {
            InstallOutcome::Wrote { backup, .. } => assert!(backup.is_some()),
            other => panic!("expected Wrote, got {other:?}"),
        }
        let root = read_json(&path);
        assert_eq!(root["mcpServers"]["muninn"], muninn_mcp_entry());
        assert_eq!(root["mcpServers"]["other"]["command"], "other");
        assert_eq!(root["unrelatedTopLevel"], 42);
    }

    /// Re-installing on an already-installed config is a no-op.
    #[test]
    fn install_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".mcp.json");
        // First install
        install_with_path(&path, false).unwrap();
        // Second install
        let outcome = install_with_path(&path, false).unwrap();
        assert!(matches!(outcome, InstallOutcome::AlreadyPresent { .. }));
    }

    /// If muninn's entry exists but differs (e.g. old args), install
    /// rewrites it.
    #[test]
    fn install_rewrites_when_entry_differs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".mcp.json");
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&json!({
                "mcpServers": {
                    "muninn": {"command": "muninn-old", "args": ["old"]}
                }
            }))
            .unwrap(),
        )
        .unwrap();
        let outcome = install_with_path(&path, false).unwrap();
        assert!(matches!(outcome, InstallOutcome::Wrote { .. }));
        let root = read_json(&path);
        assert_eq!(root["mcpServers"]["muninn"]["command"], "muninn");
    }

    /// `--dry-run` doesn't touch disk.
    #[test]
    fn install_dry_run_does_not_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".mcp.json");
        let outcome = install_with_path(&path, true).unwrap();
        assert!(matches!(outcome, InstallOutcome::DryRun { .. }));
        assert!(!path.exists(), "dry-run should not have created the file");
    }

    /// Refuse to clobber a config that isn't a JSON object.
    #[test]
    fn install_rejects_non_object_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".mcp.json");
        std::fs::write(&path, "[\"not\", \"an\", \"object\"]").unwrap();
        let err = install_with_path(&path, false).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("not a JSON object"), "got {msg}");
    }

    /// Uninstall removes only our entry and leaves the rest intact.
    #[test]
    fn uninstall_preserves_other_entries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".mcp.json");
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&json!({
                "mcpServers": {
                    "muninn": muninn_mcp_entry(),
                    "other": {"command": "other"}
                }
            }))
            .unwrap(),
        )
        .unwrap();
        let outcome = uninstall_with_path(&path, false).unwrap();
        assert!(matches!(outcome, UninstallOutcome::Removed { .. }));
        let root = read_json(&path);
        assert!(root["mcpServers"]["other"].is_object());
        assert!(root["mcpServers"].get("muninn").is_none());
    }

    /// Uninstall on a clean config reports NothingToRemove.
    #[test]
    fn uninstall_on_clean_config_is_noop() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".mcp.json");
        std::fs::write(&path, "{}").unwrap();
        let outcome = uninstall_with_path(&path, false).unwrap();
        assert!(matches!(outcome, UninstallOutcome::NothingToRemove { .. }));
    }

    /// Uninstall on a missing config is also a no-op.
    #[test]
    fn uninstall_on_missing_config_is_noop() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nope.mcp.json");
        let outcome = uninstall_with_path(&path, false).unwrap();
        assert!(matches!(outcome, UninstallOutcome::NothingToRemove { .. }));
    }

    /// Round-trip: install → uninstall returns to a state where
    /// only the pre-existing unrelated keys remain.
    #[test]
    fn install_then_uninstall_restores_pre_install_state() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".mcp.json");
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&json!({
                "mcpServers": {"other": {"command": "other"}},
                "extra": 1
            }))
            .unwrap(),
        )
        .unwrap();
        install_with_path(&path, false).unwrap();
        uninstall_with_path(&path, false).unwrap();
        let root = read_json(&path);
        assert!(root["mcpServers"]["other"].is_object());
        assert!(root["mcpServers"].get("muninn").is_none());
        assert_eq!(root["extra"], 1);
    }

    /// Confirm describe_install renders something useful in each branch.
    #[test]
    fn describe_install_handles_all_variants() {
        let p = PathBuf::from("/tmp/x.json");
        for o in [
            InstallOutcome::Wrote {
                path: p.clone(),
                backup: None,
            },
            InstallOutcome::AlreadyPresent { path: p.clone() },
            InstallOutcome::DryRun {
                path: p.clone(),
                action: "test",
                proposed: muninn_mcp_entry(),
            },
        ] {
            let s = describe_install(&o, InstallScope::Project);
            assert!(!s.is_empty());
        }
    }

    // ─── Test helpers ────────────────────────────────────────────────
    //
    // The public install/uninstall fns take an InstallScope + config_dir
    // and compute the path themselves. For unit tests we want explicit
    // control over the path so we don't have to fake a .muninn/ dir.
    // These thin wrappers duplicate the body of install/uninstall with
    // the path resolution stripped out.

    fn install_with_path(path: &Path, dry_run: bool) -> Result<InstallOutcome> {
        let (mut root, existed) = read_or_empty_object(path)?;
        let entry = muninn_mcp_entry();
        let servers = ensure_mcp_servers(&mut root);
        if let Some(existing) = servers.get(MUNINN_SERVER_NAME)
            && existing == &entry
        {
            return Ok(InstallOutcome::AlreadyPresent {
                path: path.to_path_buf(),
            });
        }
        let action = if servers.contains_key(MUNINN_SERVER_NAME) {
            "rewrite mcpServers.muninn"
        } else {
            "add mcpServers.muninn"
        };
        servers.insert(MUNINN_SERVER_NAME.to_string(), entry.clone());
        if dry_run {
            return Ok(InstallOutcome::DryRun {
                path: path.to_path_buf(),
                action,
                proposed: entry,
            });
        }
        let backup = if existed { backup_file(path)? } else { None };
        write_json_pretty(path, &root)?;
        Ok(InstallOutcome::Wrote {
            path: path.to_path_buf(),
            backup,
        })
    }

    fn uninstall_with_path(path: &Path, dry_run: bool) -> Result<UninstallOutcome> {
        if !path.exists() {
            return Ok(UninstallOutcome::NothingToRemove {
                path: path.to_path_buf(),
            });
        }
        let (mut root, _existed) = read_or_empty_object(path)?;
        let servers = match root.get_mut("mcpServers").and_then(|v| v.as_object_mut()) {
            Some(s) => s,
            None => {
                return Ok(UninstallOutcome::NothingToRemove {
                    path: path.to_path_buf(),
                });
            }
        };
        if !servers.contains_key(MUNINN_SERVER_NAME) {
            return Ok(UninstallOutcome::NothingToRemove {
                path: path.to_path_buf(),
            });
        }
        let action = "remove mcpServers.muninn";
        if dry_run {
            return Ok(UninstallOutcome::DryRun {
                path: path.to_path_buf(),
                action,
            });
        }
        servers.remove(MUNINN_SERVER_NAME);
        let backup = backup_file(path)?;
        write_json_pretty(path, &root)?;
        Ok(UninstallOutcome::Removed {
            path: path.to_path_buf(),
            backup,
        })
    }
}
