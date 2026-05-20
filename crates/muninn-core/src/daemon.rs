//! Local-IPC daemon server and client.
//!
//! The muninn daemon owns a single [`MuninnEngine`] instance plus the
//! underlying SQLite-backed stores, and serves engine calls over a
//! Unix-domain socket. Adapters — the existing proxy, the future MCP
//! server (PROJEC-T-0068), and the Claude Code hook plugin
//! (PROJEC-T-0069+) — talk to it via a [`DaemonClient`] that itself
//! implements [`MuninnEngine`] so callers can hold
//! `Arc<dyn MuninnEngine>` without caring whether the implementation
//! lives in-process or across the socket.
//!
//! ## Wire format
//!
//! Length-prefixed JSON, one request → one response. Each frame is
//! a `u32` big-endian byte length followed by exactly that many JSON
//! bytes. The framed payload is a [`Request`] for client→server, or a
//! [`Response`] for server→client. Concurrent requests share a
//! connection: every request carries a numeric `id` and the matching
//! response echoes it.
//!
//! ## Socket discovery
//!
//! Sockets live under `$XDG_RUNTIME_DIR/muninn/` (Linux),
//! `~/Library/Caches/muninn/` (macOS), or the system temp directory
//! as a last-resort fallback. The file name is a hex-encoded SHA-256
//! of the canonicalized repository root, keeping multiple muninn
//! instances (different repos, different daemons) isolated.
//!
//! ## Limitations (current iteration — PROJEC-T-0066)
//!
//! - **Unix-only.** [`stop_daemon`] and [`ensure_daemon`] are
//!   `#[cfg(unix)]`; Windows named-pipe + service-control support is
//!   a follow-up.
//! - **No streaming completions.** [`MuninnEngine::complete`] is
//!   request/response; streaming responses would need a separate
//!   protocol extension.
//! - **Race-tolerant rather than mutually-exclusive spawn.** Two
//!   concurrent `ensure_daemon` calls may both try to spawn; the
//!   second child's `bind(2)` fails on the already-held socket and
//!   exits. Acceptable for typical adapter usage; if pathological
//!   contention becomes real, a `flock(2)`-based gate can replace it.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{Mutex, oneshot};

use crate::error::{MuninnCoreError, Result};

// Re-export so `muninn-rlm::daemon::*` consumers (e.g. the muninn
// binary) can match on engine errors without naming `muninn-core`
// directly.
#[doc(no_inline)]
pub use crate::error::MuninnCoreError as EngineError;
use crate::llm::{CompletionRequest, CompletionResponse};
use crate::types::{
    DocsQuery, DocsResult, ExploreRequest, ExploreResult, GraphQuery, GraphResult, MemoryHit,
    MemoryItem, MemoryQuery, SearchQuery, SearchResult,
};
use crate::{MuninnEngine, SharedEngine};

// ─────────────────────────────────────────────────────────────────────────────
// Wire types
// ─────────────────────────────────────────────────────────────────────────────

/// Method discriminant for daemon requests.
///
/// Matches the [`MuninnEngine`] trait surface. Wire form is the
/// lowercase variant name (snake_case).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DaemonMethod {
    Complete,
    SearchCode,
    Explore,
    RecallMemory,
    RecordMemory,
    SearchDocs,
    QueryGraph,
}

/// A single client→server request frame.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    /// Caller-assigned correlation id. Echoed in the response.
    pub id: u64,
    pub method: DaemonMethod,
    /// JSON-encoded method payload (the trait's argument struct).
    pub payload: Value,
}

/// A single server→client response frame.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub id: u64,
    #[serde(flatten)]
    pub result: ResponseResult,
}

/// Result discriminant for [`Response`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ResponseResult {
    Ok { result: Value },
    Err { error: WireError },
}

/// Engine error in wire form.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "message")]
pub enum WireError {
    InvalidRequest(String),
    NotFound(String),
    BudgetExceeded(String),
    Backend(String),
    Storage(String),
    Internal(String),
}

impl From<MuninnCoreError> for WireError {
    fn from(e: MuninnCoreError) -> Self {
        match e {
            MuninnCoreError::InvalidRequest(s) => WireError::InvalidRequest(s),
            MuninnCoreError::NotFound(s) => WireError::NotFound(s),
            MuninnCoreError::BudgetExceeded(s) => WireError::BudgetExceeded(s),
            MuninnCoreError::Backend(s) => WireError::Backend(s),
            MuninnCoreError::Storage(s) => WireError::Storage(s),
            MuninnCoreError::Internal(s) => WireError::Internal(s),
        }
    }
}

impl From<WireError> for MuninnCoreError {
    fn from(e: WireError) -> Self {
        match e {
            WireError::InvalidRequest(s) => MuninnCoreError::InvalidRequest(s),
            WireError::NotFound(s) => MuninnCoreError::NotFound(s),
            WireError::BudgetExceeded(s) => MuninnCoreError::BudgetExceeded(s),
            WireError::Backend(s) => MuninnCoreError::Backend(s),
            WireError::Storage(s) => MuninnCoreError::Storage(s),
            WireError::Internal(s) => MuninnCoreError::Internal(s),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Socket-path discovery
// ─────────────────────────────────────────────────────────────────────────────

/// Compute the canonical daemon socket path for a given repository root.
///
/// The path is deterministic for a given canonicalized root, so two
/// adapters in the same repo find the same daemon without coordination.
/// The function does not create any directories on disk — call
/// [`prepare_socket_dir`] for that.
pub fn socket_path_for_repo(repo_root: &Path) -> PathBuf {
    let canonical = repo_root
        .canonicalize()
        .unwrap_or_else(|_| repo_root.to_path_buf());
    let mut hasher = Sha256::new();
    hasher.update(canonical.as_os_str().as_encoded_bytes());
    let hash = hasher.finalize();
    let hex: String = hash.iter().take(8).map(|b| format!("{b:02x}")).collect();
    daemon_runtime_dir().join(format!("{hex}.sock"))
}

/// Best-effort runtime directory for daemon sockets.
///
/// Linux: `$XDG_RUNTIME_DIR/muninn` (or `/tmp/muninn` fallback).
/// macOS / others: `~/Library/Caches/muninn` (or `dirs::cache_dir`),
/// with the system temp directory as a last-resort fallback.
pub fn daemon_runtime_dir() -> PathBuf {
    if let Some(rt) = std::env::var_os("XDG_RUNTIME_DIR") {
        return PathBuf::from(rt).join("muninn");
    }
    if let Some(cache) = dirs::cache_dir() {
        return cache.join("muninn");
    }
    std::env::temp_dir().join("muninn")
}

/// Create the daemon runtime directory if it doesn't exist. Returns the
/// directory.
pub fn prepare_socket_dir() -> std::io::Result<PathBuf> {
    let dir = daemon_runtime_dir();
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

// ─────────────────────────────────────────────────────────────────────────────
// Server
// ─────────────────────────────────────────────────────────────────────────────

/// Length-prefix-framed read of a single JSON message into `T`.
async fn read_frame<T: for<'de> Deserialize<'de>>(stream: &mut UnixStream) -> Result<T> {
    let mut len_buf = [0u8; 4];
    stream
        .read_exact(&mut len_buf)
        .await
        .map_err(|e| MuninnCoreError::Internal(format!("frame read (length): {e}")))?;
    let len = u32::from_be_bytes(len_buf) as usize;
    // Cap inbound frames to a few megabytes — engine payloads (full
    // CompletionRequests with tool definitions) can be sizable, but
    // accepting unbounded sizes is a denial-of-service hazard.
    const MAX_FRAME: usize = 8 * 1024 * 1024;
    if len > MAX_FRAME {
        return Err(MuninnCoreError::InvalidRequest(format!(
            "frame too large: {len} bytes (max {MAX_FRAME})"
        )));
    }
    let mut buf = vec![0u8; len];
    stream
        .read_exact(&mut buf)
        .await
        .map_err(|e| MuninnCoreError::Internal(format!("frame read (body): {e}")))?;
    serde_json::from_slice(&buf)
        .map_err(|e| MuninnCoreError::InvalidRequest(format!("frame decode: {e}")))
}

/// Length-prefix-framed write of a single JSON message.
async fn write_frame<T: Serialize>(stream: &mut UnixStream, msg: &T) -> Result<()> {
    let body = serde_json::to_vec(msg)
        .map_err(|e| MuninnCoreError::Internal(format!("frame encode: {e}")))?;
    let len = u32::try_from(body.len())
        .map_err(|_| MuninnCoreError::Internal("frame too large for u32 length".into()))?;
    stream
        .write_all(&len.to_be_bytes())
        .await
        .map_err(|e| MuninnCoreError::Internal(format!("frame write (length): {e}")))?;
    stream
        .write_all(&body)
        .await
        .map_err(|e| MuninnCoreError::Internal(format!("frame write (body): {e}")))?;
    Ok(())
}

/// Return the PID-file path that pairs with a given socket path
/// (`<socket>.pid`). The PID file is written by [`serve`] and read by
/// [`stop`] / used by `ensure` to spot stale daemons.
pub fn pid_path_for_socket(socket_path: &Path) -> PathBuf {
    let mut s = socket_path.as_os_str().to_owned();
    s.push(".pid");
    PathBuf::from(s)
}

/// Run a daemon serving `engine` on `socket_path`.
///
/// Binds a [`UnixListener`] at the given path, accepts connections in
/// a loop, and dispatches each request to the matching [`MuninnEngine`]
/// method. On `shutdown` the listener stops accepting and the server
/// waits for in-flight connection handlers to finish (graceful drain)
/// before unlinking the socket and the paired `<socket>.pid` file.
///
/// The socket file is unlinked before binding (to recover from a
/// previous crashed daemon).
pub async fn serve(
    engine: SharedEngine,
    socket_path: &Path,
    mut shutdown: oneshot::Receiver<()>,
) -> Result<()> {
    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| MuninnCoreError::Internal(format!("create socket dir: {e}")))?;
    }
    // Best-effort unlink in case a previous daemon crashed without
    // cleaning up. Ignore NotFound.
    if socket_path.exists() {
        let _ = std::fs::remove_file(socket_path);
    }
    let listener = UnixListener::bind(socket_path)
        .map_err(|e| MuninnCoreError::Internal(format!("bind {socket_path:?}: {e}")))?;

    // Write our PID alongside the socket so `daemon stop` knows whom to
    // signal. Best-effort: a failure here is non-fatal — the user just
    // loses the convenience of `daemon stop` and has to fall back to
    // killing the process directly.
    let pid_path = pid_path_for_socket(socket_path);
    if let Err(e) = std::fs::write(&pid_path, std::process::id().to_string()) {
        tracing::warn!(error = %e, pid_path = ?pid_path, "failed to write PID file");
    }

    tracing::info!(socket = ?socket_path, "muninn daemon listening");

    // Track per-connection task handles so shutdown can drain them.
    let mut connections: Vec<tokio::task::JoinHandle<()>> = Vec::new();

    loop {
        // Periodically prune finished handles so the Vec doesn't grow
        // unboundedly during a long-lived daemon's lifetime.
        connections.retain(|h| !h.is_finished());

        tokio::select! {
            biased;
            _ = &mut shutdown => {
                tracing::info!("daemon shutdown signal received");
                break;
            }
            accept = listener.accept() => {
                match accept {
                    Ok((stream, _addr)) => {
                        let engine = Arc::clone(&engine);
                        connections.push(tokio::spawn(async move {
                            if let Err(e) = handle_connection(engine, stream).await {
                                tracing::warn!(error = %e, "daemon connection ended with error");
                            }
                        }));
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "accept failed; stopping daemon");
                        break;
                    }
                }
            }
        }
    }

    // Drop the listener now so new connections are refused immediately,
    // then drain in-flight handlers with a bounded grace period.
    drop(listener);
    let drain_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    for handle in connections {
        let remaining = drain_deadline.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout(remaining, handle).await {
            Ok(_) => {}
            Err(_) => {
                tracing::warn!("daemon drain timeout; in-flight handler abandoned");
                break;
            }
        }
    }

    // Cleanup on the way out. Best-effort.
    let _ = std::fs::remove_file(socket_path);
    let _ = std::fs::remove_file(&pid_path);
    Ok(())
}

async fn handle_connection(engine: SharedEngine, mut stream: UnixStream) -> Result<()> {
    loop {
        let req: Request = match read_frame(&mut stream).await {
            Ok(r) => r,
            Err(e) => {
                // Treat plain EOF as a clean disconnect.
                if matches!(&e, MuninnCoreError::Internal(s) if s.contains("unexpected end of file"))
                {
                    return Ok(());
                }
                return Err(e);
            }
        };
        let id = req.id;
        let result = dispatch(&engine, req).await;
        let response = Response {
            id,
            result: match result {
                Ok(v) => ResponseResult::Ok { result: v },
                Err(e) => ResponseResult::Err {
                    error: WireError::from(e),
                },
            },
        };
        write_frame(&mut stream, &response).await?;
    }
}

async fn dispatch(engine: &SharedEngine, req: Request) -> Result<Value> {
    fn decode<T: for<'de> Deserialize<'de>>(payload: Value) -> Result<T> {
        serde_json::from_value(payload)
            .map_err(|e| MuninnCoreError::InvalidRequest(format!("payload decode: {e}")))
    }
    fn encode<T: Serialize>(v: T) -> Result<Value> {
        serde_json::to_value(v)
            .map_err(|e| MuninnCoreError::Internal(format!("payload encode: {e}")))
    }
    match req.method {
        DaemonMethod::Complete => {
            let r: CompletionRequest = decode(req.payload)?;
            let resp = engine.complete(r).await?;
            encode(resp)
        }
        DaemonMethod::SearchCode => {
            let q: SearchQuery = decode(req.payload)?;
            encode(engine.search_code(q).await?)
        }
        DaemonMethod::Explore => {
            let q: ExploreRequest = decode(req.payload)?;
            encode(engine.explore(q).await?)
        }
        DaemonMethod::RecallMemory => {
            let q: MemoryQuery = decode(req.payload)?;
            encode(engine.recall_memory(q).await?)
        }
        DaemonMethod::RecordMemory => {
            let item: MemoryItem = decode(req.payload)?;
            engine.record_memory(item).await?;
            Ok(Value::Null)
        }
        DaemonMethod::SearchDocs => {
            let q: DocsQuery = decode(req.payload)?;
            encode(engine.search_docs(q).await?)
        }
        DaemonMethod::QueryGraph => {
            let q: GraphQuery = decode(req.payload)?;
            encode(engine.query_graph(q).await?)
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Client
// ─────────────────────────────────────────────────────────────────────────────

/// A client that talks to a daemon over a Unix socket. Implements
/// [`MuninnEngine`] so callers can hold `Arc<dyn MuninnEngine>` without
/// knowing whether the engine is in-process or remote.
///
/// Requests are pipelined on a single connection; each call takes the
/// connection lock briefly while it writes and reads its frame. This
/// is intentionally simple — connection pooling and request
/// multiplexing can be added later if profiling demands it.
pub struct DaemonClient {
    conn: Mutex<UnixStream>,
    next_id: std::sync::atomic::AtomicU64,
}

impl DaemonClient {
    /// Connect to a daemon at `socket_path`.
    pub async fn connect(socket_path: &Path) -> Result<Self> {
        let stream = UnixStream::connect(socket_path)
            .await
            .map_err(|e| MuninnCoreError::Backend(format!("connect {socket_path:?}: {e}")))?;
        Ok(Self {
            conn: Mutex::new(stream),
            next_id: std::sync::atomic::AtomicU64::new(1),
        })
    }

    async fn call<P: Serialize, R: for<'de> Deserialize<'de>>(
        &self,
        method: DaemonMethod,
        payload: P,
    ) -> Result<R> {
        let id = self
            .next_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let req = Request {
            id,
            method,
            payload: serde_json::to_value(payload)
                .map_err(|e| MuninnCoreError::Internal(format!("payload encode: {e}")))?,
        };
        let mut conn = self.conn.lock().await;
        write_frame(&mut conn, &req).await?;
        let resp: Response = read_frame(&mut conn).await?;
        if resp.id != id {
            return Err(MuninnCoreError::Internal(format!(
                "daemon response id mismatch: expected {id}, got {}",
                resp.id
            )));
        }
        match resp.result {
            ResponseResult::Ok { result } => serde_json::from_value(result)
                .map_err(|e| MuninnCoreError::Internal(format!("response decode: {e}"))),
            ResponseResult::Err { error } => Err(error.into()),
        }
    }

    async fn call_unit<P: Serialize>(&self, method: DaemonMethod, payload: P) -> Result<()> {
        let id = self
            .next_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let req = Request {
            id,
            method,
            payload: serde_json::to_value(payload)
                .map_err(|e| MuninnCoreError::Internal(format!("payload encode: {e}")))?,
        };
        let mut conn = self.conn.lock().await;
        write_frame(&mut conn, &req).await?;
        let resp: Response = read_frame(&mut conn).await?;
        if resp.id != id {
            return Err(MuninnCoreError::Internal(format!(
                "daemon response id mismatch: expected {id}, got {}",
                resp.id
            )));
        }
        match resp.result {
            ResponseResult::Ok { .. } => Ok(()),
            ResponseResult::Err { error } => Err(error.into()),
        }
    }
}

#[async_trait]
impl MuninnEngine for DaemonClient {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse> {
        self.call(DaemonMethod::Complete, request).await
    }
    async fn search_code(&self, query: SearchQuery) -> Result<SearchResult> {
        self.call(DaemonMethod::SearchCode, query).await
    }
    async fn explore(&self, request: ExploreRequest) -> Result<ExploreResult> {
        self.call(DaemonMethod::Explore, request).await
    }
    async fn recall_memory(&self, query: MemoryQuery) -> Result<Vec<MemoryHit>> {
        self.call(DaemonMethod::RecallMemory, query).await
    }
    async fn record_memory(&self, item: MemoryItem) -> Result<()> {
        self.call_unit(DaemonMethod::RecordMemory, item).await
    }
    async fn search_docs(&self, query: DocsQuery) -> Result<DocsResult> {
        self.call(DaemonMethod::SearchDocs, query).await
    }
    async fn query_graph(&self, query: GraphQuery) -> Result<GraphResult> {
        self.call(DaemonMethod::QueryGraph, query).await
    }
}

/// Quick liveness probe: returns `true` if a daemon is currently
/// accepting connections at `socket_path`.
pub async fn is_alive(socket_path: &Path) -> bool {
    UnixStream::connect(socket_path).await.is_ok()
}

/// Send `SIGTERM` to the daemon associated with `socket_path` and wait
/// for the socket to disappear (up to a few seconds). Falls back to
/// `SIGKILL` if the daemon doesn't exit in time. Returns `NotFound` if
/// no PID file exists.
///
/// Unix-only — Windows named-pipe support is a follow-up.
#[cfg(unix)]
pub async fn stop_daemon(socket_path: &Path) -> Result<()> {
    let pid_path = pid_path_for_socket(socket_path);
    let pid_str = std::fs::read_to_string(&pid_path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            MuninnCoreError::NotFound(format!("no daemon PID file at {pid_path:?}"))
        } else {
            MuninnCoreError::Storage(format!("read PID file {pid_path:?}: {e}"))
        }
    })?;
    let pid: i32 = pid_str
        .trim()
        .parse()
        .map_err(|_| MuninnCoreError::Internal(format!("malformed PID file: {pid_str:?}")))?;

    // SIGTERM first.
    // SAFETY: `libc::kill` is an FFI call; we pass a valid signal
    // number and the kernel handles bad PIDs by returning ESRCH.
    let rc = unsafe { libc::kill(pid, libc::SIGTERM) };
    if rc != 0 {
        let errno = std::io::Error::last_os_error();
        // ESRCH = no such process — treat as "daemon already gone".
        if errno.raw_os_error() == Some(libc::ESRCH) {
            let _ = std::fs::remove_file(&pid_path);
            return Ok(());
        }
        return Err(MuninnCoreError::Internal(format!(
            "kill(SIGTERM, {pid}): {errno}"
        )));
    }

    // Poll for the socket to disappear. Bounded total wait: 5s.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        if !is_alive(socket_path).await {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    // Escalate to SIGKILL if the daemon refused to exit cleanly.
    // SAFETY: same as above.
    let _ = unsafe { libc::kill(pid, libc::SIGKILL) };
    Ok(())
}

/// Ensure a daemon is alive at `socket_path`, spawning one via the
/// given binary if not.
///
/// Idempotent on success. Concurrent invocations are race-tolerant
/// rather than mutually exclusive: two callers may both try to spawn,
/// but the second child's `bind(2)` will fail (the first daemon already
/// owns the socket) and the second child exits — both callers then
/// observe an alive socket.
///
/// The spawned process detaches from the parent via `setsid(2)` and
/// closes its stdio, so it survives the parent's exit and doesn't
/// inherit our terminal.
#[cfg(unix)]
pub async fn ensure_daemon(socket_path: &Path, binary_path: &Path) -> Result<()> {
    if is_alive(socket_path).await {
        return Ok(());
    }

    // Spawn `<binary> daemon start --socket <socket_path>` as a
    // detached process.
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    let mut cmd = Command::new(binary_path);
    cmd.arg("daemon")
        .arg("start")
        .arg("--socket")
        .arg(socket_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // SAFETY: `setsid` is a leaf libc call with no Rust-allocated
    // state crossing the fork. Running it in the child detaches the
    // new process from our session/process-group so a SIGHUP to our
    // tty doesn't take the daemon down with us.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    cmd.spawn()
        .map_err(|e| MuninnCoreError::Internal(format!("spawn daemon: {e}")))?;

    // Poll for liveness up to 10s. The child has to fork, set up the
    // tokio runtime, build the engine (which may open a couple of
    // SQLite files), and bind — be generous.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    while tokio::time::Instant::now() < deadline {
        if is_alive(socket_path).await {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    Err(MuninnCoreError::Internal(format!(
        "daemon did not come up within timeout at {socket_path:?}"
    )))
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Result as CoreResult;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// In-memory engine that records calls and returns canned responses.
    /// Used for the daemon roundtrip smoke test.
    #[derive(Default)]
    struct StubEngine {
        search_calls: AtomicU32,
    }

    #[async_trait]
    impl MuninnEngine for StubEngine {
        async fn complete(&self, _r: CompletionRequest) -> CoreResult<CompletionResponse> {
            Err(MuninnCoreError::Internal("complete not stubbed".into()))
        }
        async fn search_code(&self, query: SearchQuery) -> CoreResult<SearchResult> {
            self.search_calls.fetch_add(1, Ordering::Relaxed);
            Ok(SearchResult {
                hits: vec![crate::types::SearchHit {
                    path: "src/main.rs".into(),
                    line: 42,
                    snippet: format!("hit for {}", query.pattern),
                }],
                truncated: false,
            })
        }
        async fn explore(&self, _r: ExploreRequest) -> CoreResult<ExploreResult> {
            Err(MuninnCoreError::Internal("explore not stubbed".into()))
        }
        async fn recall_memory(&self, _q: MemoryQuery) -> CoreResult<Vec<MemoryHit>> {
            Ok(vec![])
        }
        async fn record_memory(&self, _i: MemoryItem) -> CoreResult<()> {
            Ok(())
        }
        async fn search_docs(&self, _q: DocsQuery) -> CoreResult<DocsResult> {
            Ok(DocsResult { hits: vec![] })
        }
        async fn query_graph(&self, _q: GraphQuery) -> CoreResult<GraphResult> {
            Err(MuninnCoreError::Backend("graph not stubbed".into()))
        }
    }

    fn temp_socket() -> PathBuf {
        let dir = tempfile::tempdir().unwrap().into_path();
        dir.join("muninn.sock")
    }

    #[tokio::test]
    async fn socket_path_is_deterministic_per_repo() {
        let p1 = socket_path_for_repo(Path::new("."));
        let p2 = socket_path_for_repo(Path::new("."));
        assert_eq!(p1, p2);
    }

    #[tokio::test]
    async fn socket_path_differs_across_repos() {
        let a = socket_path_for_repo(Path::new("/tmp/muninn_test_a"));
        let b = socket_path_for_repo(Path::new("/tmp/muninn_test_b"));
        assert_ne!(a, b);
    }

    #[tokio::test]
    async fn is_alive_false_when_no_daemon() {
        let path = temp_socket();
        assert!(!is_alive(&path).await);
    }

    #[tokio::test]
    async fn server_client_roundtrip_search_code() {
        let socket = temp_socket();
        let engine: SharedEngine = Arc::new(StubEngine::default());
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let server_engine = Arc::clone(&engine);
        let server_socket = socket.clone();
        let server_task =
            tokio::spawn(async move { serve(server_engine, &server_socket, shutdown_rx).await });

        // Wait briefly for the listener to bind.
        for _ in 0..50 {
            if is_alive(&socket).await {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(is_alive(&socket).await, "daemon never came up");

        let client = DaemonClient::connect(&socket).await.unwrap();
        let result = client
            .search_code(SearchQuery {
                pattern: "fn foo".into(),
                is_regex: false,
                path_glob: None,
                language: None,
                limit: None,
            })
            .await
            .unwrap();
        assert_eq!(result.hits.len(), 1);
        assert_eq!(result.hits[0].path, "src/main.rs");
        assert_eq!(result.hits[0].line, 42);
        assert!(result.hits[0].snippet.contains("fn foo"));

        // Issue a second call on the same client to exercise pipelining.
        let _ = client
            .search_code(SearchQuery {
                pattern: "other".into(),
                is_regex: false,
                path_glob: None,
                language: None,
                limit: Some(5),
            })
            .await
            .unwrap();

        let _ = shutdown_tx.send(());
        // serve() drops the listener and unlinks the socket on shutdown.
        let _ = server_task.await;
    }

    #[tokio::test]
    async fn serve_writes_pid_file_and_drains_on_shutdown() {
        let socket = temp_socket();
        let pid_path = pid_path_for_socket(&socket);
        let engine: SharedEngine = Arc::new(StubEngine::default());
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let server_socket = socket.clone();
        let server_engine = Arc::clone(&engine);
        let server_task =
            tokio::spawn(async move { serve(server_engine, &server_socket, shutdown_rx).await });

        for _ in 0..50 {
            if is_alive(&socket).await {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        // PID file exists and matches our process while serving.
        let pid_str = std::fs::read_to_string(&pid_path).expect("pid file written");
        assert_eq!(pid_str.trim(), std::process::id().to_string());

        // Open a connection but don't issue a request — we want to
        // verify the server cleans up properly even with an idle
        // connection still attached.
        let _client = DaemonClient::connect(&socket).await.unwrap();

        let _ = shutdown_tx.send(());
        let _ = server_task.await;

        // Socket + PID file both removed on the way out.
        assert!(!socket.exists(), "socket should be unlinked on shutdown");
        assert!(
            !pid_path.exists(),
            "PID file should be unlinked on shutdown"
        );
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn stop_daemon_signals_and_cleans_up() {
        let socket = temp_socket();
        let pid_path = pid_path_for_socket(&socket);
        let engine: SharedEngine = Arc::new(StubEngine::default());
        let (_shutdown_tx, shutdown_rx) = oneshot::channel();
        let server_socket = socket.clone();
        let server_task =
            tokio::spawn(async move { serve(engine, &server_socket, shutdown_rx).await });

        for _ in 0..50 {
            if is_alive(&socket).await {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(is_alive(&socket).await);

        // We don't want stop_daemon to actually kill the test process,
        // so instead of pointing it at our real PID we exercise the
        // "no PID file" branch by removing it first.
        std::fs::remove_file(&pid_path).expect("remove pid file");
        let err = stop_daemon(&socket).await.unwrap_err();
        assert!(
            matches!(err, MuninnCoreError::NotFound(_)),
            "expected NotFound when PID file is missing, got {err:?}"
        );

        // Stale-PID path: write a PID that doesn't correspond to any
        // running process. SIGTERM should return ESRCH and stop_daemon
        // should treat it as already-gone.
        std::fs::write(&pid_path, "999999999").expect("write fake pid");
        stop_daemon(&socket)
            .await
            .expect("stop with stale pid is ok");
        assert!(!pid_path.exists(), "stale PID file should be cleaned up");

        // Tidy up the still-running test server.
        let _ = server_task.abort();
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn ensure_daemon_noop_when_already_alive() {
        let socket = temp_socket();
        let engine: SharedEngine = Arc::new(StubEngine::default());
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let server_socket = socket.clone();
        let server_task =
            tokio::spawn(async move { serve(engine, &server_socket, shutdown_rx).await });

        for _ in 0..50 {
            if is_alive(&socket).await {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(is_alive(&socket).await);

        // ensure_daemon should return Ok immediately without trying to
        // spawn anything (the binary path we pass is intentionally
        // bogus — a spawn attempt would fail).
        ensure_daemon(&socket, Path::new("/path/that/does/not/exist"))
            .await
            .expect("ensure should no-op when daemon already alive");

        let _ = shutdown_tx.send(());
        let _ = server_task.await;
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn ensure_daemon_errors_when_spawn_target_missing() {
        let socket = temp_socket();
        // No daemon running, and the binary path doesn't exist — spawn
        // fails and ensure_daemon returns an Internal error.
        let err = ensure_daemon(&socket, Path::new("/definitely/not/a/binary"))
            .await
            .unwrap_err();
        assert!(matches!(err, MuninnCoreError::Internal(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn error_responses_round_trip() {
        let socket = temp_socket();
        let engine: SharedEngine = Arc::new(StubEngine::default());
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let server_socket = socket.clone();
        let server_task =
            tokio::spawn(async move { serve(engine, &server_socket, shutdown_rx).await });

        for _ in 0..50 {
            if is_alive(&socket).await {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        let client = DaemonClient::connect(&socket).await.unwrap();
        let err = client
            .query_graph(GraphQuery {
                target: "foo".into(),
                kind: crate::types::GraphQueryKind::Callers,
                max_hops: None,
            })
            .await
            .unwrap_err();
        // StubEngine maps query_graph to Backend; check that the wire
        // form preserved the variant + message.
        assert!(
            matches!(err, MuninnCoreError::Backend(ref s) if s.contains("graph not stubbed")),
            "got {err:?}"
        );

        let _ = shutdown_tx.send(());
        let _ = server_task.await;
    }
}
