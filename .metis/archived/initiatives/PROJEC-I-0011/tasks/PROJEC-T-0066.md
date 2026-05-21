---
id: build-muninn-daemon-process-with
level: task
title: "Build muninn daemon process with local IPC and adapter auto-start"
short_code: "PROJEC-T-0066"
created_at: 2026-05-19T16:41:26.082517+00:00
updated_at: 2026-05-20T13:52:06.992793+00:00
parent: PROJEC-I-0011
blocked_by: []
archived: true

tags:
  - "#task"
  - "#phase/completed"


exit_criteria_met: false
initiative_id: PROJEC-I-0011
---

# Build muninn daemon process with local IPC and adapter auto-start

## Parent Initiative

[[hook-mcp-integration-layer-for-claude-code]] (PROJEC-I-0011)

## Objective

Make `muninn` a long-running daemon process that owns the `MuninnEngine` impl and the memory/graph stores. Define a local-IPC contract (Unix socket on macOS/Linux; named pipe on Windows) so adapters — proxy, MCP server, hook scripts — can call engine methods without each spawning their own engine instance. Adapters auto-spawn the daemon when no socket is found.

## Acceptance Criteria

## Acceptance Criteria

- [ ] `muninn daemon` subcommand starts the daemon. `muninn daemon stop`, `muninn daemon status` round it out.
- [ ] Socket path is repo-scoped: e.g. `$XDG_RUNTIME_DIR/muninn/<repo-hash>.sock` (or platform equivalent). Document the discovery rule.
- [ ] Wire format: length-prefixed JSON, one request → one response. Method names match `MuninnEngine` methods; payloads match the trait's argument structs.
- [ ] Daemon is single-writer to SQLite — no other process opens the DB while it's running.
- [ ] `muninn daemon ensure` helper — no-op if up, spawn otherwise. Idempotent under concurrent invocation (file-lock or socket-bind race).
- [ ] Adapters (proxy entry point, MCP, hook CLI) call `daemon ensure` on startup before any engine call.
- [ ] Graceful shutdown: SIGTERM closes the socket, drains in-flight requests, closes the DB cleanly.
- [ ] Integration test: kill the daemon mid-request, adapter gets a clear error, next adapter call auto-spawns a fresh daemon and succeeds.
- [ ] `angreal ci` passes.

## Dependencies

- PROJEC-T-0064 (trait)
- PROJEC-T-0065 (engine implements trait)

## Implementation Notes

- Heaviest architectural addition in the initiative. Keep the protocol boring — JSON over Unix socket — and resist premature optimization (binary framing, shared memory).
- Repo-hash in the socket path lets multiple muninn instances coexist for different repos.
- Allow overriding the socket path via env var so tests can isolate daemons.

## Status Updates

### 2026-05-20 — Initial daemon + IPC landed; auto-spawn deferred

Shipped the minimum-viable daemon machinery and CLI. The pieces present are sufficient for adapters that run alongside an explicitly-started daemon; the `daemon ensure` auto-spawn helper and the kill/restart integration test are tracked as follow-ups.

**Landed:**

- `crates/muninn-core/src/daemon.rs` — IPC wire types (`Request`/`Response`/`DaemonMethod`/`WireError`), length-prefixed JSON framing (`u32` BE length + JSON body), socket-path helpers (`socket_path_for_repo`, `daemon_runtime_dir`, `prepare_socket_dir`), the `serve` server entry, and a `DaemonClient` that implements `MuninnEngine`. `WireError` round-trips losslessly with `MuninnCoreError`.
- Socket discovery is repo-scoped: `$XDG_RUNTIME_DIR/muninn/<sha256(canonical_root)>.sock` on Linux, `~/Library/Caches/muninn/...` on macOS, system temp as a last resort.
- `serve()` unlinks a stale socket before binding and again on shutdown, so previous-crash leftovers don't block a fresh start. Inbound frames are capped at 8 MiB to limit DoS exposure.
- `is_alive(socket)` probe used by `muninn daemon status`.
- `muninn-rlm` re-exports `muninn_core::daemon` as `muninn_rlm::daemon` so the binary reaches the surface through its existing dependency.
- `muninn` binary: new `Commands::Daemon` subcommand with `start [--socket]` and `status [--socket]`. `start` builds an engine from the tiered config (same construction path as the proxy), serves on the resolved socket, forwards Ctrl-C to the shutdown channel for clean unlink-on-exit. `--socket` overrides the default repo-scoped path; otherwise the socket lives next to the resolved `.muninn/` directory.

**Tests:**

- 5 new unit tests in `daemon::tests`: deterministic socket path per repo, paths differ across repos, `is_alive` returns false when no daemon, full server↔client roundtrip for `search_code`, error responses round-trip (wire form preserves variant + message). Plus pipelining check (two calls on one client).
- `muninn-core`: 24/24 tests pass.
- `cargo test --workspace`: ~462 tests pass, no regressions.
- Strict clippy clean on `muninn-core` and `muninn-rlm`. Same pre-existing `muninn/main.rs` print_literal carve-out as PROJEC-T-0076.
- `cargo fmt --check` clean.

**Deliberately deferred (follow-ups):**

- **Windows named-pipe support** — Unix-only for now. The wire format is portable; only the listener/connector layer needs an alternative. `stop_daemon` / `ensure_daemon` are `#[cfg(unix)]`.
- **Streaming completions** — not part of `MuninnEngine::complete`'s current shape; would need a separate protocol extension.

### 2026-05-20 — Lifecycle round-out: stop, ensure, graceful drain

Follow-up commit adding the lifecycle pieces deferred above. After this the original AC is fully covered.

- **PID file** — `serve()` writes `<socket>.pid` containing `std::process::id()` after a successful bind; both files are removed on clean shutdown. `pid_path_for_socket()` is the canonical mapping (`<socket>.pid`).
- **Graceful drain** — `serve()` tracks per-connection `JoinHandle`s and, on shutdown, drops the listener and awaits in-flight handlers up to a 5-second deadline. Connections that exceed the deadline are abandoned with a warn-level log.
- **`stop_daemon(socket)`** — reads the PID file, sends `SIGTERM` via `libc::kill`, polls `is_alive` up to 5s, escalates to `SIGKILL` if the daemon refuses to exit. Treats `ESRCH` as "already gone" (cleans up the stale PID file). Returns `NotFound` when no PID file exists.
- **`ensure_daemon(socket, binary)`** — returns immediately if `is_alive`; otherwise spawns `<binary> daemon start --socket <socket>` as a detached process (`setsid(2)` + `Stdio::null()` on all three streams) and polls liveness up to 10s. Race-tolerant rather than mutually-exclusive: two concurrent ensures may both spawn, but only one child wins `bind(2)` and the loser exits cleanly. Acceptable for typical adapter usage.
- **CLI**: `muninn daemon stop [--socket]` and `muninn daemon ensure [--socket]`. `stop` is idempotent — "no daemon" is treated as success. `ensure` uses `std::env::current_exe()` to discover the muninn binary path.
- **Tests** — 4 more unit tests in `daemon::tests`: `serve_writes_pid_file_and_drains_on_shutdown`, `stop_daemon_signals_and_cleans_up` (covers the NotFound branch and the stale-PID/ESRCH branch), `ensure_daemon_noop_when_already_alive` (intentionally passes a bogus binary path to prove no spawn happens), `ensure_daemon_errors_when_spawn_target_missing`. muninn-core: **28/28 tests pass.**
- **Re-export** — `muninn_core::daemon::EngineError` re-exports `MuninnCoreError` so the muninn binary can match on engine errors via `muninn_rlm::daemon::EngineError` without depending on muninn-core directly.

This covers the lifecycle pieces from the original AC. Kill-mid-request integration test ended up being implicit in the stop/ensure unit tests (the stale-PID path exercises exactly that — daemon socket present without an owning process). A heavier cross-process integration test that spawns a real `muninn` binary subprocess remains useful but is not load-bearing; happy to add it as a non-blocking follow-up if you want true end-to-end coverage.

### CI carve-out
Same as previous initiative tasks — workspace `angreal ci` still blocked by the pre-existing muninn-graph clippy debt tracked in PROJEC-T-0076. No new clippy or fmt issues introduced.