//! Augmentation retrieval block for `muninn hook decide`.
//!
//! When the decision model returns `augment`, this module builds the
//! "Muninn context" markdown block we attach to Claude Code's
//! `additionalContext` slot. It pulls supporting evidence from the
//! running engine via `recall_memory` + `query_graph` calls, formats
//! the results as a compact markdown block, and caps the payload at
//! 2 KB so we never balloon the agent's context.
//!
//! Per the initiative design notes (PROJEC-I-0011):
//! - Retrieval-only path — no second LLM call.
//! - Connects through the daemon (`muninn_rlm::daemon::DaemonClient`),
//!   not direct DB access.
//! - Empty results fall back to passthrough rather than emitting an
//!   empty block.
//!
//! Today the engine's `recall_memory` / `query_graph` impls are
//! stubbed (PROJEC-T-0065 carve-out), so the live path almost always
//! returns `Ok(None)`. The formatting logic is fully implemented and
//! unit-tested so flipping the stubs to real impls "just works."

use std::sync::Arc;
use std::time::Duration;

use muninn_rlm::SharedEngine;
use muninn_rlm::daemon::{DaemonClient, is_alive};

/// Hard cap on the rendered block in bytes.
pub const AUGMENT_BLOCK_BYTE_CAP: usize = 2048;

/// Per-call budget for each engine retrieval (recall_memory, query_graph).
/// Bounded short so a slow store can't push the hook past its outer
/// 500 ms wall-clock budget.
const RETRIEVAL_BUDGET: Duration = Duration::from_millis(150);

/// Top-N hits to request from each retrieval method. Generous enough
/// to fill the block but not so big that we waste IPC bandwidth before
/// the cap-truncate step.
const RETRIEVAL_LIMIT: u32 = 8;

/// Try to connect to the daemon and build an augmentation block for
/// the given `(tool_name, tool_args, augment_hint)` triple.
///
/// Returns:
/// - `Ok(Some(block))` when retrieval produced at least one usable
///   section. The string is guaranteed `<= AUGMENT_BLOCK_BYTE_CAP`.
/// - `Ok(None)` when nothing was retrieved — the caller should fall
///   back to passthrough or to a hint-only response.
/// - `Err(_)` only on unexpected daemon / IPC errors. Callers treat
///   errors the same as `None` per NFR-002.
pub async fn try_build_augment_block(
    socket_path: &std::path::Path,
    tool_name: &str,
    tool_args: &serde_json::Value,
    augment_hint: Option<&str>,
) -> anyhow::Result<Option<String>> {
    // Connect to the daemon. We deliberately do *not* call
    // `ensure_daemon` here — the cost of spawning a fresh daemon mid-
    // hook would blow the 500 ms budget. If no daemon is up, augment
    // degrades to passthrough/hint-only via Ok(None).
    if !is_alive(socket_path).await {
        return Ok(None);
    }
    let client = match DaemonClient::connect(socket_path).await {
        Ok(c) => Arc::new(c) as SharedEngine,
        Err(_) => return Ok(None),
    };

    // Build a memory-search query from the augment hint when provided,
    // falling back to the tool args. The hint is the model's pointer
    // at "what muninn might know" — a more relevant query than the
    // raw tool input.
    let memory_query = augment_hint
        .map(|s| s.to_string())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| format!("{tool_name}: {}", short_args(tool_args)));

    // Pick a graph target from the tool args. For Grep/Read/Glob the
    // most useful target is the pattern/path the agent was about to
    // hit — graph callers/refs against it.
    let graph_target = extract_graph_target(tool_name, tool_args);

    // Issue retrievals in parallel but bounded by RETRIEVAL_BUDGET so
    // a single slow store doesn't drag the whole augmentation path
    // past the outer 500 ms cap.
    let memory_fut = tokio::time::timeout(RETRIEVAL_BUDGET, async {
        client
            .recall_memory(muninn_core::types::MemoryQuery {
                query: memory_query.clone(),
                limit: Some(RETRIEVAL_LIMIT),
            })
            .await
    });

    // We always run the query_graph branch (even when there's no
    // useful target — in that case it just resolves to an empty
    // result instantly). Keeping both arms the same future type lets
    // us join them without trait-object gymnastics.
    let graph_target_owned = graph_target.clone();
    let graph_client = Arc::clone(&client);
    let graph_fut = tokio::time::timeout(RETRIEVAL_BUDGET, async move {
        match graph_target_owned {
            Some(t) => {
                graph_client
                    .query_graph(muninn_core::types::GraphQuery {
                        target: t,
                        kind: muninn_core::types::GraphQueryKind::Callers,
                        max_hops: Some(1),
                    })
                    .await
            }
            None => Ok(muninn_core::types::GraphResult {
                nodes: vec![],
                edges: vec![],
            }),
        }
    });

    let (memory_res, graph_res) = futures::future::join(memory_fut, graph_fut).await;

    // Treat timeouts and engine errors as "no data" — never propagate.
    let memory_hits = memory_res.ok().and_then(|r| r.ok()).unwrap_or_default();
    let graph_result =
        graph_res
            .ok()
            .and_then(|r| r.ok())
            .unwrap_or(muninn_core::types::GraphResult {
                nodes: vec![],
                edges: vec![],
            });

    Ok(format_augment_block(
        &graph_result,
        &memory_hits,
        AUGMENT_BLOCK_BYTE_CAP,
    ))
}

/// Pure formatter — render the retrieval results as a markdown block.
///
/// Returns `None` when there's nothing usable to attach (all sections
/// empty) so the caller can passthrough cleanly. Otherwise returns a
/// block bounded by `byte_cap`; if rendering would exceed the cap,
/// the tail gets a `… (truncated)` marker so the agent knows we
/// trimmed.
///
/// Output shape matches the design in [`PROJEC-I-0011`]:
///
/// ```text
/// ─── Muninn context ───
/// Related symbols: foo::bar (crates/x/src/y.rs:42), …
/// Callers: alpha (a.rs:10), beta (b.rs:88)
/// Prior memory:
///   - 2026-04-02 ADR-0001 declares this module owns auth
/// ─────────────────────
/// ```
pub fn format_augment_block(
    graph: &muninn_core::types::GraphResult,
    memory: &[muninn_core::types::MemoryHit],
    byte_cap: usize,
) -> Option<String> {
    let mut sections: Vec<String> = Vec::new();

    // Related symbols section — the graph nodes.
    let symbols: Vec<String> = graph
        .nodes
        .iter()
        .map(|n| match &n.location {
            Some(loc) => format!("{} ({loc})", n.id),
            None => n.id.clone(),
        })
        .collect();
    if !symbols.is_empty() {
        sections.push(format!("Related symbols: {}", symbols.join(", ")));
    }

    // Callers section — `kind=calls` edges (or any "calls"-flavored
    // edge in our generic edge list).
    let callers: Vec<String> = graph
        .edges
        .iter()
        .filter(|e| e.kind == "calls" || e.kind == "caller")
        .map(|e| e.from.clone())
        .collect();
    if !callers.is_empty() {
        sections.push(format!("Callers: {}", callers.join(", ")));
    }

    // Memory section.
    if !memory.is_empty() {
        let mut lines = vec!["Prior memory:".to_string()];
        for hit in memory {
            // One-line snippet per hit; trim the content to keep lines
            // readable. Anything that runs over still gets capped by
            // the overall byte_cap below.
            let snippet = hit.content.lines().next().unwrap_or("").trim();
            if snippet.is_empty() {
                continue;
            }
            lines.push(format!("  - {snippet}"));
        }
        if lines.len() > 1 {
            sections.push(lines.join("\n"));
        }
    }

    if sections.is_empty() {
        return None;
    }

    let header = "─── Muninn context ───";
    let footer = "─────────────────────";
    let body = sections.join("\n");
    let mut rendered = format!("{header}\n{body}\n{footer}");

    if rendered.len() > byte_cap {
        rendered = truncate_with_marker(&rendered, footer, byte_cap);
    }

    Some(rendered)
}

/// Truncate `rendered` to `byte_cap` while preserving the footer and
/// indicating truncation. Truncation happens on a char boundary so
/// the result stays valid UTF-8.
fn truncate_with_marker(rendered: &str, footer: &str, byte_cap: usize) -> String {
    const MARKER: &str = "… (truncated)";
    // Reserve space for the marker, a newline, and the footer line.
    let reserved = MARKER.len() + 1 + footer.len();
    if byte_cap <= reserved {
        // Cap is absurdly small — just return the footer alone. Avoids
        // panic on zero-budget callers.
        return footer.to_string();
    }
    let available = byte_cap - reserved;
    // Cut at the last char boundary <= `available` to keep UTF-8 valid.
    let body_end = floor_char_boundary(rendered, available);
    let body = &rendered[..body_end];
    // Re-append the marker + footer; the original footer is already
    // inside `body` if `rendered` is short enough, but the byte_cap
    // path means it isn't, so we add it.
    format!("{}{MARKER}\n{footer}", body.trim_end_matches('\n'))
}

/// std::str::floor_char_boundary is unstable; small replacement.
fn floor_char_boundary(s: &str, idx: usize) -> usize {
    if idx >= s.len() {
        return s.len();
    }
    let mut i = idx;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Shorten the JSON-encoded tool args to something reasonable for a
/// memory query. We want enough signal for retrieval ranking; not the
/// whole serialized blob.
fn short_args(args: &serde_json::Value) -> String {
    let s = args.to_string();
    if s.len() > 200 {
        format!("{}…", &s[..floor_char_boundary(&s, 200)])
    } else {
        s
    }
}

/// Pick a graph-query target from the tool args, when one is obvious.
/// For Grep/Read/Glob the most useful target is the `pattern` (for
/// Grep) or `path` (for Read/Glob). Returns `None` for tools whose
/// arguments don't yield a sensible target.
fn extract_graph_target(tool_name: &str, args: &serde_json::Value) -> Option<String> {
    let s = match tool_name {
        "Grep" => args.get("pattern").and_then(|v| v.as_str()),
        "Read" | "Glob" => args
            .get("file_path")
            .or_else(|| args.get("path"))
            .or_else(|| args.get("pattern"))
            .and_then(|v| v.as_str()),
        _ => None,
    }?;
    let trimmed = s.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use muninn_core::types::{GraphEdge, GraphNode, GraphResult, MemoryHit};

    fn mh(id: &str, content: &str, score: f32) -> MemoryHit {
        MemoryHit {
            id: id.into(),
            content: content.into(),
            score,
        }
    }

    fn gr_empty() -> GraphResult {
        GraphResult {
            nodes: vec![],
            edges: vec![],
        }
    }

    #[test]
    fn format_returns_none_when_everything_empty() {
        assert_eq!(
            format_augment_block(&gr_empty(), &[], AUGMENT_BLOCK_BYTE_CAP),
            None
        );
    }

    #[test]
    fn format_renders_full_block_under_cap() {
        let graph = GraphResult {
            nodes: vec![
                GraphNode {
                    id: "foo::bar".into(),
                    location: Some("crates/x/src/y.rs:42".into()),
                },
                GraphNode {
                    id: "baz".into(),
                    location: None,
                },
            ],
            edges: vec![GraphEdge {
                from: "alpha".into(),
                to: "foo::bar".into(),
                kind: "calls".into(),
            }],
        };
        let memory = vec![mh("m1", "ADR-0003 says hook+MCP is primary", 0.9)];
        let rendered = format_augment_block(&graph, &memory, AUGMENT_BLOCK_BYTE_CAP)
            .expect("expected a block");

        assert!(rendered.starts_with("─── Muninn context ───"));
        assert!(rendered.ends_with("─────────────────────"));
        assert!(rendered.contains("Related symbols: foo::bar (crates/x/src/y.rs:42), baz"));
        assert!(rendered.contains("Callers: alpha"));
        assert!(rendered.contains("Prior memory:"));
        assert!(rendered.contains("- ADR-0003 says hook+MCP is primary"));
        assert!(rendered.len() <= AUGMENT_BLOCK_BYTE_CAP);
    }

    #[test]
    fn format_handles_partial_sections() {
        // Only memory present — graph is empty.
        let memory = vec![mh("m1", "only memory has anything", 1.0)];
        let rendered = format_augment_block(&gr_empty(), &memory, AUGMENT_BLOCK_BYTE_CAP)
            .expect("expected a block");
        assert!(!rendered.contains("Related symbols:"));
        assert!(!rendered.contains("Callers:"));
        assert!(rendered.contains("Prior memory:"));
    }

    #[test]
    fn format_skips_memory_entries_with_blank_first_line() {
        let memory = vec![
            mh("m1", "", 1.0),
            mh("m2", "   ", 1.0),
            mh("m3", "real content", 1.0),
        ];
        let rendered = format_augment_block(&gr_empty(), &memory, AUGMENT_BLOCK_BYTE_CAP)
            .expect("expected a block");
        // Only the real entry survives.
        assert!(rendered.contains("- real content"));
        // And we don't end up with a header followed by zero items.
        let mem_line_count = rendered
            .lines()
            .filter(|l| l.trim_start().starts_with('-'))
            .count();
        assert_eq!(mem_line_count, 1);
    }

    #[test]
    fn format_truncates_when_over_cap_and_marks_with_ellipsis() {
        // Build a graph result whose serialized symbols line is huge
        // enough to overflow a small cap.
        let nodes: Vec<GraphNode> = (0..200)
            .map(|i| GraphNode {
                id: format!("symbol_{i:04}"),
                location: Some(format!("crates/x/src/file_{i:04}.rs:{i}")),
            })
            .collect();
        let graph = GraphResult {
            nodes,
            edges: vec![],
        };
        let cap = 512;
        let rendered = format_augment_block(&graph, &[], cap).expect("expected a block");
        assert!(
            rendered.len() <= cap,
            "rendered {} > cap {cap}",
            rendered.len()
        );
        assert!(rendered.contains("… (truncated)"));
        // Footer should still be the last line so the block reads as
        // self-contained.
        assert!(rendered.ends_with("─────────────────────"));
    }

    #[test]
    fn truncate_respects_utf8_char_boundaries() {
        // Build a rendered string with multi-byte chars right at the
        // truncation point so we'd panic if we sliced at a non-boundary.
        let body = "─".repeat(500); // each char is 3 bytes in UTF-8
        let rendered = format!("─── Muninn context ───\n{body}\n─────────────────────");
        let cap = 100;
        let footer = "─────────────────────";
        let truncated = truncate_with_marker(&rendered, footer, cap);
        // Just exercising: should not panic, should still end with footer.
        assert!(truncated.ends_with(footer));
        assert!(truncated.len() <= cap || cap < footer.len() + 16);
    }

    #[test]
    fn truncate_with_absurdly_small_cap_returns_footer_only() {
        let footer = "─────────────────────";
        let out = truncate_with_marker("anything", footer, 5);
        assert_eq!(out, footer);
    }

    #[test]
    fn extract_graph_target_grep_uses_pattern() {
        let args = serde_json::json!({"pattern": "fn main", "path": "src/"});
        assert_eq!(
            extract_graph_target("Grep", &args),
            Some("fn main".to_string())
        );
    }

    #[test]
    fn extract_graph_target_read_uses_path() {
        let args = serde_json::json!({"file_path": "src/lib.rs"});
        assert_eq!(
            extract_graph_target("Read", &args),
            Some("src/lib.rs".to_string())
        );
    }

    #[test]
    fn extract_graph_target_unknown_tool_returns_none() {
        let args = serde_json::json!({"pattern": "x"});
        assert_eq!(extract_graph_target("MysteryTool", &args), None);
    }

    #[test]
    fn extract_graph_target_empty_string_returns_none() {
        let args = serde_json::json!({"pattern": "   "});
        assert_eq!(extract_graph_target("Grep", &args), None);
    }
}
