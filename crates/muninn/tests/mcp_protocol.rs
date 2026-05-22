//! UAT — MCP server stdio protocol.
//!
//! Drives `muninn mcp` as a real subprocess and speaks newline-
//! delimited JSON-RPC 2.0 on its stdin/stdout. Catches breakage in
//! anywhere along the chain: the rust-mcp-sdk transport, our
//! [`EngineServerHandler`], the tool-schema generation in
//! `muninn-core`, and the daemon round-trip.
//!
//! `tools/list` exercises a path that doesn't actually call any
//! engine method (the response is built from
//! `muninn_core::tool_schemas()`), so this test doesn't burn LLM
//! budget — but it still needs a running daemon for the MCP server
//! to connect to.
//!
//! Gated `#[ignore]` because:
//! - the daemon startup needs credentials, even if we never invoke
//!   the model (the engine constructs a backend eagerly);
//! - subprocess work belongs in the UAT pipeline.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const ALIVE_TIMEOUT: Duration = Duration::from_secs(15);
const DEAD_TIMEOUT: Duration = Duration::from_secs(10);
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(15);
const POLL_INTERVAL: Duration = Duration::from_millis(50);

fn muninn_bin() -> &'static str {
    env!("CARGO_BIN_EXE_muninn")
}

fn isolated_socket() -> PathBuf {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("muninn.sock");
    std::mem::forget(dir);
    path
}

fn status_verb(socket: &std::path::Path) -> String {
    let out = Command::new(muninn_bin())
        .args(["daemon", "status", "--socket"])
        .arg(socket)
        .output()
        .expect("daemon status");
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_string()
}

fn wait_until<F: Fn() -> bool>(label: &str, timeout: Duration, cond: F) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if cond() {
            return;
        }
        std::thread::sleep(POLL_INTERVAL);
    }
    panic!("{label}: did not converge within {timeout:?}");
}

fn skip_if_no_backend(test: &str) -> bool {
    let has_any = std::env::var_os("OLLAMA_API_KEY").is_some()
        || std::env::var_os("GROQ_API_KEY").is_some()
        || std::env::var_os("ANTHROPIC_API_KEY").is_some();
    if !has_any {
        eprintln!("[uat::{test}] skipping: no backend credentials — run via `angreal test uat`");
        true
    } else {
        false
    }
}

/// Bring up a daemon scoped to `socket`. Returns a guard that stops
/// the daemon on drop so tests don't leak background processes.
struct DaemonGuard {
    socket: PathBuf,
}

impl DaemonGuard {
    fn start(socket: PathBuf) -> Self {
        let _ = Command::new(muninn_bin())
            .args(["daemon", "ensure", "--socket"])
            .arg(&socket)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("daemon ensure");
        wait_until("daemon alive", ALIVE_TIMEOUT, || {
            status_verb(&socket) == "alive"
        });
        Self { socket }
    }
}

impl Drop for DaemonGuard {
    fn drop(&mut self) {
        let _ = Command::new(muninn_bin())
            .args(["daemon", "stop", "--socket"])
            .arg(&self.socket)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        wait_until("daemon dead", DEAD_TIMEOUT, || {
            status_verb(&self.socket) == "dead"
        });
    }
}

/// Minimal JSON-RPC client over the muninn-mcp child process's
/// stdio.
struct McpClient {
    child: Child,
    stdin: std::process::ChildStdin,
    stdout: BufReader<std::process::ChildStdout>,
    next_id: u64,
}

impl McpClient {
    fn spawn(socket: &std::path::Path) -> Self {
        let mut child = Command::new(muninn_bin())
            .args(["mcp", "--no-ensure", "--socket"])
            .arg(socket)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn muninn mcp");
        let stdin = child.stdin.take().expect("child stdin");
        let stdout = BufReader::new(child.stdout.take().expect("child stdout"));
        Self {
            child,
            stdin,
            stdout,
            next_id: 1,
        }
    }

    fn send(&mut self, payload: serde_json::Value) {
        let line = serde_json::to_string(&payload).expect("serialize payload");
        writeln!(self.stdin, "{line}").expect("write line");
        self.stdin.flush().ok();
    }

    fn request(&mut self, method: &str, params: serde_json::Value) -> serde_json::Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }));
        // Read responses until we find one with matching id.
        let deadline = Instant::now() + RESPONSE_TIMEOUT;
        loop {
            if Instant::now() > deadline {
                panic!("timeout waiting for response to id {id}");
            }
            let mut line = String::new();
            let n = self
                .stdout
                .read_line(&mut line)
                .expect("read mcp response line");
            if n == 0 {
                panic!("MCP stdout closed before response to id {id}");
            }
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let value: serde_json::Value =
                serde_json::from_str(line).expect("MCP response is JSON");
            if value.get("id").and_then(|v| v.as_u64()) == Some(id) {
                return value;
            }
            // Notifications (no id) — ignore.
        }
    }

    fn notify(&mut self, method: &str, params: serde_json::Value) {
        self.send(serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        }));
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        // Kill the server outright — by the time we get here we've
        // already gotten the responses we care about, and a graceful
        // shutdown via stdin-close requires futzing with Option-typed
        // fields we'd rather not introduce just for cleanup.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Full handshake: `initialize` succeeds, the server reports its
/// own name/version, and `tools/list` advertises the curated muninn
/// surface (search_code / query_graph). `search_docs` is
/// deliberately gated out of v1 per the muninn-as-RLM focus.
#[test]
#[ignore = "UAT — MCP stdio protocol; invoke via `angreal test uat`"]
fn mcp_initialize_and_list_tools() {
    if skip_if_no_backend("mcp_initialize_and_list_tools") {
        return;
    }
    let sock = isolated_socket();
    let _daemon = DaemonGuard::start(sock.clone());
    let mut mcp = McpClient::spawn(&sock);

    // 1. initialize
    let init = mcp.request(
        "initialize",
        serde_json::json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "uat-mcp-protocol", "version": "0.0"}
        }),
    );
    assert_eq!(init["jsonrpc"], "2.0");
    let server_info = &init["result"]["serverInfo"];
    assert_eq!(
        server_info["name"].as_str(),
        Some("muninn"),
        "server should identify as 'muninn': {init}"
    );

    // 2. notifications/initialized (no response expected)
    mcp.notify("notifications/initialized", serde_json::json!({}));

    // 3. tools/list — the headline assertion. We don't care about
    //    order, just presence.
    let tools = mcp.request("tools/list", serde_json::json!({}));
    let names: Vec<String> = tools["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .filter_map(|t| t.get("name").and_then(|n| n.as_str()).map(String::from))
        .collect();
    let expected = ["search_code", "query_graph"];
    for want in expected {
        assert!(
            names.iter().any(|n| n == want),
            "tools/list missing {want}; got {names:?}"
        );
    }
    // `explore` is deliberately not surfaced via MCP — the recursive
    // engine is the expensive code path and an LLM planner is prone
    // to invoking it for vague questions. See mcp.rs design notes.
    assert!(
        !names.iter().any(|n| n == "explore"),
        "explore should not be exposed via MCP, got {names:?}"
    );
    // `search_docs` is gated out of v1 — muninn-as-RLM focus.
    assert!(
        !names.iter().any(|n| n == "search_docs"),
        "search_docs should not be exposed via MCP in v1, got {names:?}"
    );
}

/// Call `search_code` over MCP and assert the engine actually walks
/// the filesystem and returns real hits. The daemon is started
/// against the project's own `.muninn/config.toml`, whose
/// `project.root` resolves to the workspace root — so a search for
/// the literal `fn main` should land on at least one Rust source
/// file in this repo.
#[test]
#[ignore = "UAT — MCP stdio protocol; invoke via `angreal test uat`"]
fn mcp_tools_call_search_code_returns_filesystem_hits() {
    if skip_if_no_backend("mcp_tools_call_search_code_returns_filesystem_hits") {
        return;
    }
    let sock = isolated_socket();
    let _daemon = DaemonGuard::start(sock.clone());
    let mut mcp = McpClient::spawn(&sock);

    let _ = mcp.request(
        "initialize",
        serde_json::json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "uat", "version": "0.0"}
        }),
    );
    mcp.notify("notifications/initialized", serde_json::json!({}));

    let resp = mcp.request(
        "tools/call",
        serde_json::json!({
            "name": "search_code",
            "arguments": {
                "pattern": "fn main",
                "is_regex": false,
                "language": "rust",
                "limit": 10
            }
        }),
    );
    assert!(
        resp.get("error").is_none(),
        "expected JSON-RPC success: {resp}"
    );
    let result = &resp["result"];
    let is_error = result
        .get("isError")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    assert!(
        !is_error,
        "search_code should not surface a tool error now that it's wired: {result}"
    );

    let structured = &result["structuredContent"];
    let hits = structured
        .get("hits")
        .and_then(|v| v.as_array())
        .unwrap_or_else(|| panic!("structuredContent missing 'hits': {result}"));
    assert!(
        !hits.is_empty(),
        "expected at least one `fn main` hit in this workspace: {result}"
    );
    let first = &hits[0];
    assert!(
        first
            .get("path")
            .and_then(|p| p.as_str())
            .is_some_and(|p| p.ends_with(".rs")),
        "expected a .rs path on the first hit: {first}"
    );
    assert!(
        first
            .get("snippet")
            .and_then(|s| s.as_str())
            .is_some_and(|s| s.contains("fn main")),
        "expected snippet to contain `fn main`: {first}"
    );
}

/// Call `query_graph` over MCP with `kind=callers` against a target
/// the repo's indexed graph should know about. The test tolerates an
/// empty graph database (fresh checkout, never indexed): in that
/// case `nodes` is empty and we skip the substantive assertion with
/// a breadcrumb. When the graph IS populated we assert the response
/// shape (structuredContent with `nodes` + `edges` arrays).
#[test]
#[ignore = "UAT — MCP stdio protocol; invoke via `angreal test uat`"]
fn mcp_tools_call_query_graph_returns_graph_payload() {
    if skip_if_no_backend("mcp_tools_call_query_graph_returns_graph_payload") {
        return;
    }
    let sock = isolated_socket();
    let _daemon = DaemonGuard::start(sock.clone());
    let mut mcp = McpClient::spawn(&sock);

    let _ = mcp.request(
        "initialize",
        serde_json::json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "uat", "version": "0.0"}
        }),
    );
    mcp.notify("notifications/initialized", serde_json::json!({}));

    // Ask for callers of a function we know exists in this workspace.
    // If the graph index is empty the call should still succeed with
    // zero nodes — the engine returns an empty result, not an error.
    let resp = mcp.request(
        "tools/call",
        serde_json::json!({
            "name": "query_graph",
            "arguments": {
                "target": "socket_path_for_repo",
                "kind": "callers"
            }
        }),
    );
    assert!(
        resp.get("error").is_none(),
        "expected JSON-RPC success: {resp}"
    );
    let result = &resp["result"];
    let is_error = result
        .get("isError")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    assert!(
        !is_error,
        "query_graph should not surface a tool error now that it's wired: {result}"
    );

    let structured = &result["structuredContent"];
    let nodes = structured
        .get("nodes")
        .and_then(|v| v.as_array())
        .unwrap_or_else(|| panic!("structuredContent missing 'nodes' array: {result}"));
    let edges = structured
        .get("edges")
        .and_then(|v| v.as_array())
        .unwrap_or_else(|| panic!("structuredContent missing 'edges' array: {result}"));

    if nodes.is_empty() {
        eprintln!(
            "[uat] query_graph returned empty — graph index is likely unpopulated. \
             Skipping payload-shape assertions. (Run `muninn index` to populate.)"
        );
        return;
    }

    // Graph has data: the target itself should appear as a node, and
    // every edge should reference ids that appear in nodes.
    let node_ids: Vec<String> = nodes
        .iter()
        .filter_map(|n| n.get("id").and_then(|i| i.as_str()).map(String::from))
        .collect();
    assert!(
        node_ids.iter().any(|n| n.contains("socket_path_for_repo")),
        "expected target symbol in returned nodes; got {node_ids:?}"
    );
    for e in edges {
        let from = e
            .get("from")
            .and_then(|v| v.as_str())
            .expect("edge.from string");
        let to = e
            .get("to")
            .and_then(|v| v.as_str())
            .expect("edge.to string");
        assert!(
            node_ids.iter().any(|n| n == from),
            "edge.from {from:?} not in nodes {node_ids:?}"
        );
        assert!(
            node_ids.iter().any(|n| n == to),
            "edge.to {to:?} not in nodes {node_ids:?}"
        );
    }
    eprintln!(
        "[uat] query_graph: {n} nodes, {e} edges",
        n = nodes.len(),
        e = edges.len()
    );
}
