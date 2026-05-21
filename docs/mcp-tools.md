# Muninn MCP tools

Muninn exposes its engine to Claude Code (and other MCP clients) as a small
set of tools. The MCP server itself (`muninn mcp`, shipping in PROJEC-T-0068)
is a thin protocol adapter; the schemas described here live in
[`crates/muninn-core/src/mcp.rs`](../crates/muninn-core/src/mcp.rs) and are
derived from the engine DTOs in `crates/muninn-core/src/types.rs` via
`schemars`, so the wire shape and the trait surface can't drift.

## When to call which tool

| Tool | Use when… |
|------|-----------|
| [`search_code`](#search_code) | You want ranked, scoped text/regex matches in the working tree. |
| [`query_graph`](#query_graph) | You need to reason about call relationships: callers, callees, definitions, references. |
| [`search_docs`](#search_docs) | You need API or usage info for an indexed library on crates.io or PyPI. |

Two tools are intentionally *not* surfaced via MCP:

- `explore` (the recursive engine) — expensive code path; an LLM
  planner is prone to calling it for vague questions and blowing
  through budget. The proxy + UserPromptSubmit hook drive `explore`
  internally when they decide it's appropriate, so the agent gets
  the value without being able to over-invoke it.
- `recall_memory` — the memory store has no user-facing write source
  in v1, so advertising the tool would surface a feature that always
  returns empty. The trait method exists and works; it's just not
  exposed to agents until there's a clear write story.

## Stability

- Tool **names**, **descriptions**, and the documented input/output
  shapes are **stable**. Changes require a new tool name or a version
  bump in the schema.
- Internal scoring details (e.g. exact numeric range of `score` fields)
  are **best-effort** — don't depend on specific values.

## Tools

### `search_code`

> Use this when you need to find where a symbol, string, or pattern occurs
> in the working tree. Faster and more focused than `Grep` when you want
> results ranked by relevance and scoped to a path glob or language.
> Returns line-level hits with snippets.

**Input** (see `SearchQuery` in `types.rs`):

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `pattern` | string | yes | The pattern to search for. |
| `is_regex` | bool | no | When `true`, `pattern` is treated as a regex. Default: `false`. |
| `path_glob` | string | no | Glob filter (e.g. `src/**/*.rs`). |
| `language` | string | no | Language tag (e.g. `"rust"`, `"python"`). |
| `limit` | u32 | no | Max hits. Engine picks a default if unset. |

**Output** (`SearchResult`): `hits: SearchHit[]`, `truncated: bool`.
Each `SearchHit` has `path`, `line`, `snippet`.

**Examples:**

```json
{ "pattern": "fn main", "is_regex": false, "limit": 20 }
```

```json
{
  "pattern": "^impl .* for .*Backend$",
  "is_regex": true,
  "path_glob": "crates/**/*.rs",
  "language": "rust"
}
```

### `query_graph`

> Use this when you need to know how a symbol relates to other code:
> who calls it, what it calls, where it's defined, or where it's
> referenced. Returns a graph of nodes and edges rather than raw text
> matches. Prefer this over `Grep` for call-chain reasoning.

**Input** (`GraphQuery`):

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `target` | string | yes | Symbol name (e.g. `"RecursiveEngine::run"`) or `file:line`. |
| `kind` | enum | yes | `"callers"`, `"callees"`, `"defines"`, `"references"`. |
| `max_hops` | u32 | no | Maximum graph hops. Engine default if unset. |

**Output** (`GraphResult`): `nodes: GraphNode[]`, `edges: GraphEdge[]`.

**Examples:**

```json
{ "target": "RecursiveEngine::run", "kind": "callers" }
```

```json
{ "target": "crates/muninn/src/main.rs:71", "kind": "defines", "max_hops": 1 }
```

### `search_docs`

> Use this when you need API or usage information for an indexed library
> (crates.io / PyPI). Returns ranked documentation chunks with the
> library name, version, and item path. Filter by ecosystem or library
> when you already know which one you want.

**Input** (`DocsQuery`): `query: string`, optional `ecosystem`,
`library`, `limit`.

**Output** (`DocsResult`): `hits: DocsHit[]` where each hit has
`library`, `version`, `item_path`, `snippet`, `score`.

**Examples:**

```json
{
  "query": "spawning blocking tasks",
  "ecosystem": "rust",
  "library": "tokio",
  "limit": 5
}
```

```json
{ "query": "datetime parsing iso8601" }
```

## See also

- ADR-0003 ([`hook + MCP integration model`](../.metis/adrs/PROJEC-A-0003.md))
  — why muninn exposes MCP at all and how it relates to the proxy.
- Initiative PROJEC-I-0011 — the broader hook + MCP integration work.
- [`crates/muninn-core/src/mcp.rs`](../crates/muninn-core/src/mcp.rs)
  — the source-of-truth schema definitions.
