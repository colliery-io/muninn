//! MCP tool schemas for muninn's engine surface.
//!
//! Each [`McpToolSchema`] describes one tool exposed to Claude Code (or
//! any MCP client) by the upcoming `muninn mcp` server: name, an
//! agent-facing description, JSON Schema for input/output (derived from
//! the Rust DTOs in [`crate::types`] via `schemars`), and 1–2 example
//! calls.
//!
//! ## Design notes
//!
//! - The schema set is intentionally small. The MCP surface is what the
//!   agent's planner sees; fewer, richer tools beat many narrow ones.
//! - `explore` (recursive exploration) is **not** exposed via MCP. The
//!   recursive engine is the expensive code path and an LLM planner is
//!   prone to invoking it for any vague question, blowing through
//!   budget. The hook plugin (PROJEC-T-0070) drives `explore` directly
//!   via the engine trait when its decision model determines a rewrite
//!   is warranted. If/when there's a concrete need for an MCP-exposed
//!   exploration tool, add one then — with explicit budget arguments
//!   surfaced to the agent.
//! - Schemas derive from [`crate::types`] so the wire shape and the
//!   trait surface can't drift.
//!
//! Stability:
//! - Tool names, the `name` and `description` fields, and the documented
//!   input/output shapes are **stable**.
//! - Internal scoring details (e.g. exact numeric range of `score`)
//!   are **best-effort** — clients should not depend on specific values.

use schemars::{JsonSchema, schema_for};
use serde::Serialize;
use serde_json::Value;

use crate::types::{
    DocsQuery, DocsResult, GraphQuery, GraphResult, MemoryHit, MemoryQuery, SearchQuery,
    SearchResult,
};

/// Stability classification for a tool schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SchemaStability {
    /// Wire shape is committed; changes require a tool version bump or
    /// a new tool name.
    Stable,
    /// Wire shape may change as the surface matures. Adapters should
    /// expect non-breaking additions but not depend on exact field set.
    Experimental,
}

/// Self-describing schema for one MCP tool.
///
/// The MCP server (PROJEC-T-0068) consumes these by name to advertise
/// tools to clients. The schemas also feed the docs page at
/// `docs/mcp-tools.md`.
#[derive(Debug, Clone, Serialize)]
pub struct McpToolSchema {
    /// Tool name as it appears in the MCP wire (e.g. `"search_code"`).
    pub name: &'static str,
    /// Agent-facing description. Written as a prompt fragment — the
    /// planner reads this to decide when to call the tool.
    pub description: &'static str,
    /// JSON Schema for the tool's input object.
    pub input_schema: Value,
    /// JSON Schema for the tool's output object.
    pub output_schema: Value,
    /// 1–2 example invocations, each a literal value matching `input_schema`.
    pub examples: Vec<Value>,
    /// Stability classification.
    pub stability: SchemaStability,
}

/// Return every MCP tool schema muninn currently exposes.
///
/// Order is documentation-style (most commonly used first); the MCP
/// server may sort however it likes.
pub fn tool_schemas() -> Vec<McpToolSchema> {
    vec![
        search_code_schema(),
        query_graph_schema(),
        recall_memory_schema(),
        search_docs_schema(),
    ]
}

fn schema_value<T: JsonSchema>() -> Value {
    serde_json::to_value(schema_for!(T)).expect("schemars schema serialization is infallible")
}

fn search_code_schema() -> McpToolSchema {
    McpToolSchema {
        name: "search_code",
        description: "\
Use this when you need to find where a symbol, string, or pattern occurs in \
the working tree. Faster and more focused than Grep when you want results \
ranked by relevance and scoped to a path glob or language. Returns line-level \
hits with snippets.",
        input_schema: schema_value::<SearchQuery>(),
        output_schema: schema_value::<SearchResult>(),
        examples: vec![
            serde_json::json!({
                "pattern": "fn main",
                "is_regex": false,
                "limit": 20
            }),
            serde_json::json!({
                "pattern": "^impl .* for .*Backend$",
                "is_regex": true,
                "path_glob": "crates/**/*.rs",
                "language": "rust"
            }),
        ],
        stability: SchemaStability::Stable,
    }
}

fn query_graph_schema() -> McpToolSchema {
    McpToolSchema {
        name: "query_graph",
        description: "\
Use this when you need to know how a symbol relates to other code: who \
calls it, what it calls, where it's defined, or where it's referenced. \
Returns a graph of nodes and edges rather than raw text matches. Prefer \
this over Grep for call-chain reasoning.",
        input_schema: schema_value::<GraphQuery>(),
        output_schema: schema_value::<GraphResult>(),
        examples: vec![
            serde_json::json!({
                "target": "RecursiveEngine::run",
                "kind": "callers"
            }),
            serde_json::json!({
                "target": "crates/muninn/src/main.rs:71",
                "kind": "defines",
                "max_hops": 1
            }),
        ],
        stability: SchemaStability::Stable,
    }
}

fn recall_memory_schema() -> McpToolSchema {
    McpToolSchema {
        name: "recall_memory",
        description: "\
Use this to look up prior decisions, observations, or context muninn has \
stored about this repo (ADR-style notes, architecture facts, past \
explorations). Returns ranked memory hits. Pair with record_memory when \
you discover something worth keeping.",
        input_schema: schema_value::<MemoryQuery>(),
        output_schema: {
            // recall_memory returns Vec<MemoryHit> — wrap it in the
            // standard `{ "hits": [...] }` shape for MCP, matching how
            // other tools surface lists.
            let mut wrapper = schemars::schema::SchemaObject::default();
            wrapper.metadata().title = Some("RecallMemoryResult".into());
            wrapper.object().properties.insert(
                "hits".into(),
                schemars::schema::Schema::Object(schemars::schema::SchemaObject {
                    instance_type: Some(schemars::schema::InstanceType::Array.into()),
                    array: Some(Box::new(schemars::schema::ArrayValidation {
                        items: Some(schemars::schema::SingleOrVec::Single(Box::new(
                            schema_for!(MemoryHit).schema.into(),
                        ))),
                        ..Default::default()
                    })),
                    ..Default::default()
                }),
            );
            wrapper.object().required.insert("hits".into());
            serde_json::to_value(wrapper).expect("schema serialization is infallible")
        },
        examples: vec![serde_json::json!({
            "query": "how does the proxy handle OAuth?",
            "limit": 3
        })],
        stability: SchemaStability::Stable,
    }
}

fn search_docs_schema() -> McpToolSchema {
    McpToolSchema {
        name: "search_docs",
        description: "\
Use this when you need API or usage information for an indexed library \
(crates.io / PyPI). Returns ranked documentation chunks with the library \
name, version, and item path. Filter by ecosystem or library when you \
already know which one you want.",
        input_schema: schema_value::<DocsQuery>(),
        output_schema: schema_value::<DocsResult>(),
        examples: vec![
            serde_json::json!({
                "query": "spawning blocking tasks",
                "ecosystem": "rust",
                "library": "tokio",
                "limit": 5
            }),
            serde_json::json!({
                "query": "datetime parsing iso8601"
            }),
        ],
        stability: SchemaStability::Stable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_schemas_lists_all_expected_tools() {
        let names: Vec<&'static str> = tool_schemas().iter().map(|s| s.name).collect();
        assert_eq!(
            names,
            vec!["search_code", "query_graph", "recall_memory", "search_docs"]
        );
    }

    #[test]
    fn tool_schemas_do_not_expose_explore() {
        // explore is intentionally hook-only; see the module-level
        // design notes.
        let names: Vec<&'static str> = tool_schemas().iter().map(|s| s.name).collect();
        assert!(
            !names.contains(&"explore"),
            "explore should not be exposed via MCP"
        );
    }

    #[test]
    fn every_schema_has_non_empty_description_and_input_schema() {
        for s in tool_schemas() {
            assert!(
                !s.description.is_empty(),
                "tool {} has empty description",
                s.name
            );
            // Every input schema is an object with at least one property.
            assert!(
                s.input_schema.get("properties").is_some(),
                "tool {} input schema missing 'properties'",
                s.name
            );
        }
    }

    #[test]
    fn every_example_is_a_json_object() {
        for s in tool_schemas() {
            assert!(!s.examples.is_empty(), "tool {} has no examples", s.name);
            for ex in &s.examples {
                assert!(
                    ex.is_object(),
                    "tool {} example is not an object: {}",
                    s.name,
                    ex
                );
            }
        }
    }

    #[test]
    fn schemas_serialize_to_json() {
        // Round-trip the whole set as JSON to make sure nothing in the
        // schema construction produces invalid JSON.
        let json = serde_json::to_string(&tool_schemas()).expect("schemas serialize");
        assert!(json.contains("\"search_code\""));
        assert!(json.contains("\"query_graph\""));
    }
}
