---
id: build-muninn-daemon-process-with
level: task
title: "Build muninn daemon process with local IPC and adapter auto-start"
short_code: "PROJEC-T-0066"
created_at: 2026-05-19T16:41:26.082517+00:00
updated_at: 2026-05-19T16:41:26.082517+00:00
parent: PROJEC-I-0011
blocked_by: []
archived: false

tags:
  - "#task"
  - "#phase/todo"


exit_criteria_met: false
initiative_id: PROJEC-I-0011
---

# Build muninn daemon process with local IPC and adapter auto-start

## Parent Initiative

[[hook-mcp-integration-layer-for-claude-code]] (PROJEC-I-0011)

## Objective

Make `muninn` a long-running daemon process that owns the `MuninnEngine` impl and the memory/graph stores. Define a local-IPC contract (Unix socket on macOS/Linux; named pipe on Windows) so adapters — proxy, MCP server, hook scripts — can call engine methods without each spawning their own engine instance. Adapters auto-spawn the daemon when no socket is found.

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

*To be added during implementation.*
