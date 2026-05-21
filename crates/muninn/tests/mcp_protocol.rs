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
/// surface (search_code / query_graph / search_docs). `recall_memory`
/// is excluded in v1 — memory has no real write source, so
/// advertising it would surface a tool that always returns empty.
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
    let expected = ["search_code", "query_graph", "search_docs"];
    for want in expected {
        assert!(
            names.iter().any(|n| n == want),
            "tools/list missing {want}; got {names:?}"
        );
    }
    // `explore` is deliberately not surfaced (PROJEC-T-0067).
    assert!(
        !names.iter().any(|n| n == "explore"),
        "explore should not be exposed via MCP, got {names:?}"
    );
    // `recall_memory` is not surfaced in v1 — memory store has no
    // user-facing write source so advertising it would be misleading.
    assert!(
        !names.iter().any(|n| n == "recall_memory"),
        "recall_memory should not be exposed via MCP in v1, got {names:?}"
    );
}

/// Call `search_code` over MCP. The engine's `search_code` is still
/// stubbed (PROJEC-T-0065 carve-out), so the response should arrive
/// as a `CallToolResult { is_error: Some(true) }` carrying the
/// "not yet wired" message — *not* a JSON-RPC error. The contract
/// per `mcp_engine_server.rs` is that engine errors land in
/// CallToolResult, not at the protocol layer.
#[test]
#[ignore = "UAT — MCP stdio protocol; invoke via `angreal test uat`"]
fn mcp_tools_call_search_code_surfaces_engine_error_as_tool_error() {
    if skip_if_no_backend("mcp_tools_call_search_code_surfaces_engine_error_as_tool_error") {
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
            "arguments": {"pattern": "fn main", "is_regex": false}
        }),
    );
    // JSON-RPC success at the protocol layer.
    assert!(
        resp.get("error").is_none(),
        "expected JSON-RPC success: {resp}"
    );
    let result = &resp["result"];
    // Tool-level error: `isError: true`.
    assert_eq!(
        result["isError"], true,
        "engine error should surface as tool isError: {resp}"
    );
    // The text content should mention the carve-out so future
    // engine-wiring follow-ups can confirm they replaced this path
    // by re-running the test and seeing it pass (i.e. isError: false).
    let content_text = result["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(
        content_text.contains("not yet wired") || content_text.contains("PROJEC-T-0065"),
        "expected the not-wired-yet sentinel in the tool error text; got {content_text:?}"
    );
}
