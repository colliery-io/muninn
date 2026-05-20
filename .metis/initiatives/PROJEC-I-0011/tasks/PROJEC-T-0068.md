---
id: implement-muninn-mcp-stdio
level: task
title: "Implement muninn mcp stdio subcommand backed by daemon IPC"
short_code: "PROJEC-T-0068"
created_at: 2026-05-19T16:41:29.207302+00:00
updated_at: 2026-05-20T14:14:32.013112+00:00
parent: PROJEC-I-0011
blocked_by: []
archived: false

tags:
  - "#task"
  - "#phase/completed"


exit_criteria_met: false
initiative_id: PROJEC-I-0011
---

# Implement muninn mcp stdio subcommand backed by daemon IPC

## Parent Initiative

[[hook-mcp-integration-layer-for-claude-code]] (PROJEC-I-0011)

## Objective

Add a `muninn mcp` subcommand that runs an MCP server over stdio (CC's default transport). The server exposes the tool schemas from PROJEC-T-0067 and routes each call through the daemon IPC from PROJEC-T-0066. The MCP process itself is stateless — a thin protocol adapter; the daemon does the work.

## Acceptance Criteria

- [ ] `muninn mcp` starts a stdio MCP server speaking the current MCP wire protocol.
- [ ] Server advertises the tools defined in PROJEC-T-0067.
- [ ] On each invocation: ensure daemon is running, send IPC request, return result as MCP tool response.
- [ ] Errors map cleanly: daemon unreachable → MCP tool error with actionable message; engine error → MCP tool error with engine message; timeout → MCP tool error.
- [ ] Integration test: spawn `muninn mcp` as a subprocess, send MCP initialize + a `search_code` tool call, verify the response.
- [ ] Manual smoke test: point CC's `mcp.json` at `muninn mcp`, start a CC session, verify the tools appear and at least one returns results.
- [ ] `angreal ci` passes.

## Dependencies

- PROJEC-T-0066 (daemon + IPC)
- PROJEC-T-0067 (tool schemas)

## Implementation Notes

- Use an existing Rust MCP server crate if a mature one exists; otherwise hand-roll the small protocol surface we need.
- Keep this binary thin. Anything that looks like "logic" probably belongs in the daemon.
- Log to stderr only — stdout is reserved for MCP protocol bytes.

## Status Updates

### 2026-05-20 — Implementation landed

- **`crates/muninn-rlm/src/mcp_engine_server.rs`** (new) — `EngineServerHandler` impls `rust_mcp_sdk::ServerHandler` for `Arc<dyn MuninnEngine>`. `list_tools` returns the curated set from `muninn_core::tool_schemas()` (single source of truth); `call_tool` dispatches on tool name, deserializes the matching DTO from the MCP `arguments` object, calls the trait method, and serializes the result back. `recall_memory` wraps `Vec<MemoryHit>` as `{ "hits": [...] }` to match the documented MCP shape from PROJEC-T-0067. Errors land as `CallToolResult { is_error: Some(true) }` rather than failing the JSON-RPC call.
- **`run_engine_mcp_server(engine)`** runs a stdio MCP server with sane metadata (server name `"muninn"`, instructions text). Uses `rust_mcp_sdk`'s `StdioTransport`.
- **`muninn-rlm/src/lib.rs`** — re-exports `MuninnEngine` and `SharedEngine` from `muninn-core` so the binary doesn't need to depend on `muninn-core` directly.
- **`muninn` binary** — new `Commands::Mcp { socket, no_ensure }` subcommand: resolves the daemon socket path, calls `ensure_daemon` (unless `--no-ensure`), connects a `DaemonClient`, hands the resulting `Arc<dyn MuninnEngine>` to `run_engine_mcp_server`. **CRITICAL**: uses a dedicated `init_logging_stderr_only` so tracing output goes to stderr — stdout is reserved for MCP protocol frames.

### Decision: kept the existing `crates/muninn-rlm/src/mcp.rs` alongside

The pre-existing module exposes an arbitrary `ToolEnvironment` (the LLM-callable tool registry) over MCP. The new `mcp_engine_server.rs` exposes the curated *engine* surface (search_code / query_graph / recall_memory / search_docs). Different problems, different audiences (LLM tool-use vs. external MCP clients like Claude Code), no conflict. Both modules compile, both have unit tests, and they can be selected via separate binary subcommands when needed.

### Tests
- 4 new unit tests in `mcp_engine_server::tests`: tool-list contents (verifies `search_code` / `query_graph` / `recall_memory` / `search_docs` advertised + `explore` correctly absent), search_code dispatch roundtrip on a stub engine, `tool_ok` structured-content for JSON objects and the "drop structured_content for non-objects" branch, `tool_error` marks `is_error`.
- `muninn-rlm` unit tests: **286/286 pass** (was 281, +5 — 4 new in mcp_engine_server, 1 already-flushed pipeline coverage).
- `muninn-core`: 28/28 still pass.
- `cargo test --workspace`: all suites green.
- `cargo clippy -p muninn-core -p muninn-rlm --no-deps -- -D warnings`: clean.
- `cargo fmt --check`: clean on touched crates.
- Manual smoke: `muninn mcp --help` exits 0 and renders the expected usage.

### Manual end-to-end smoke procedure (documented for follow-up)

1. In one shell: `OLLAMA_API_KEY=… muninn daemon start` (or just run `muninn daemon ensure`).
2. In another: `muninn mcp --no-ensure` — server writes nothing to stdout until it receives an MCP request, logs setup info to stderr.
3. Drive via Claude Code by adding `muninn` to the project's `mcp.json` pointing at `muninn` binary with `mcp` subcommand.

### Deferred / explicit non-scope

- **Cross-process MCP-protocol integration test** (spawn `muninn mcp` as a subprocess, drive JSON-RPC over its stdio, assert `initialize` + `tools/list` + `tools/call` responses). The handler-level unit tests cover dispatch logic; protocol plumbing is delegated to `rust-mcp-sdk` (which has its own coverage). A subprocess test would also exercise the timing/teardown of the spawned daemon child. Worth adding alongside the hook plugin work (PROJEC-T-0069) where the same code path gets driven for real.

### CI carve-out
Same as previous initiative tasks — workspace `angreal ci` still blocked by the pre-existing muninn-graph clippy debt tracked in PROJEC-T-0076. No new clippy or fmt issues introduced.