//! Edge types representing relationships between symbols.
//!
//! Calls-only graph: the vendored narsil extractor produces a call
//! graph and nothing else, so this module mirrors that. Earlier
//! variants (Contains, Imports, Inherits, Implements, UsesType,
//! Instantiates, References, ExpandsTo, GeneratedBy) were removed
//! when we vendored narsil — the data was never produced by the
//! new pipeline and the dead enum arms just rotted query code.
//! Restore selectively if the extractor learns to emit them.

use serde::{Deserialize, Serialize};

/// The type of function/method call.
///
/// Mirrors the subset of narsil's `CallType` variants we actually
/// emit through the adapter. Async/Spawn/Closure/Unknown all map
/// down to `Direct` in `builder.rs::map_call_type` for now — refine
/// when our query surface needs to distinguish them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallType {
    /// Direct function call: `foo()`
    Direct,
    /// Method call on self: `self.foo()`
    Method,
    /// Static/associated function call: `Type::foo()`
    StaticMethod,
}

impl CallType {
    /// Returns the string representation for queries and display.
    pub fn as_str(&self) -> &'static str {
        match self {
            CallType::Direct => "direct",
            CallType::Method => "method",
            CallType::StaticMethod => "static_method",
        }
    }
}

/// The kind of relationship between two symbols.
///
/// Currently the graph models call relationships only. The enum
/// stays an enum (rather than collapsing to a struct) so adding
/// future edge kinds — IMPLEMENTS, IMPORTS, etc., when the
/// extractor learns to emit them — stays additive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum EdgeKind {
    /// Function/method call relationship.
    Calls {
        /// The type of call.
        call_type: CallType,
        /// Line number where the call occurs.
        line: usize,
    },
}

impl EdgeKind {
    /// Returns the string representation for queries.
    pub fn as_str(&self) -> &'static str {
        match self {
            EdgeKind::Calls { .. } => "calls",
        }
    }
}

/// An edge connecting two symbols in the code graph.
///
/// Edges are directional, going from `source_id` to `target_id`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    /// The source node ID (where the edge originates).
    pub source_id: String,
    /// The target node ID (where the edge points to).
    pub target_id: String,
    /// The kind of relationship this edge represents.
    pub kind: EdgeKind,
}

impl Edge {
    /// Create a new edge.
    pub fn new(source_id: impl Into<String>, target_id: impl Into<String>, kind: EdgeKind) -> Self {
        Self {
            source_id: source_id.into(),
            target_id: target_id.into(),
            kind,
        }
    }

    /// Create a CALLS edge.
    pub fn calls(
        caller_id: impl Into<String>,
        callee_id: impl Into<String>,
        call_type: CallType,
        line: usize,
    ) -> Self {
        Self::new(caller_id, callee_id, EdgeKind::Calls { call_type, line })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_call_type_as_str() {
        assert_eq!(CallType::Direct.as_str(), "direct");
        assert_eq!(CallType::Method.as_str(), "method");
        assert_eq!(CallType::StaticMethod.as_str(), "static_method");
    }

    #[test]
    fn test_edge_kind_as_str() {
        assert_eq!(
            EdgeKind::Calls {
                call_type: CallType::Direct,
                line: 10,
            }
            .as_str(),
            "calls"
        );
    }

    #[test]
    fn test_edge_creation() {
        let edge = Edge::new(
            "src:func:foo:10",
            "src:func:bar:20",
            EdgeKind::Calls {
                call_type: CallType::Direct,
                line: 5,
            },
        );
        assert_eq!(edge.source_id, "src:func:foo:10");
        assert_eq!(edge.target_id, "src:func:bar:20");
        assert_eq!(edge.kind.as_str(), "calls");
    }

    #[test]
    fn test_edge_factory_calls() {
        let calls = Edge::calls("caller", "callee", CallType::Method, 42);
        // EdgeKind only has the Calls variant now, so the destructure
        // is irrefutable — but we still want to confirm the fields
        // come through correctly.
        let EdgeKind::Calls { call_type, line } = calls.kind;
        assert_eq!(call_type, CallType::Method);
        assert_eq!(line, 42);
    }

    #[test]
    fn test_call_type_serialization() {
        let call_type = CallType::StaticMethod;
        let json = serde_json::to_string(&call_type).expect("serialize");
        assert_eq!(json, "\"static_method\"");
    }

    #[test]
    fn test_edge_kind_serialization() {
        let kind = EdgeKind::Calls {
            call_type: CallType::Direct,
            line: 42,
        };
        let json = serde_json::to_string(&kind).expect("serialize");
        assert!(json.contains("\"type\":\"calls\""));
        assert!(json.contains("\"line\":42"));
    }
}
