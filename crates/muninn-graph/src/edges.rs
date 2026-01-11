//! Edge types representing relationships between symbols.
//!
//! This module defines the edge types that connect nodes in the code graph.
//! Edges capture structural relationships (contains, imports), call relationships
//! (direct, FFI, dynamic), type relationships (inherits, implements), and
//! special cases (macros, generated code).

use serde::{Deserialize, Serialize};

/// The type of function/method call.
///
/// Distinguishes between different calling conventions which have
/// different implications for analysis and refactoring.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallType {
    /// Direct function call: `foo()`
    Direct,
    /// Method call on self: `self.foo()`
    Method,
    /// Static/associated function call: `Type::foo()`
    StaticMethod,
    /// Dynamic dispatch (vtable, trait object, reflection)
    Dynamic,
    /// Foreign function interface (extern "C", PyO3, ctypes)
    FFI,
    /// External API call (HTTP, gRPC)
    API,
}

impl CallType {
    /// Returns the string representation for queries and display.
    pub fn as_str(&self) -> &'static str {
        match self {
            CallType::Direct => "direct",
            CallType::Method => "method",
            CallType::StaticMethod => "static_method",
            CallType::Dynamic => "dynamic",
            CallType::FFI => "ffi",
            CallType::API => "api",
        }
    }

    /// Returns true if this call type crosses a language boundary.
    pub fn is_cross_language(&self) -> bool {
        matches!(self, CallType::FFI | CallType::API)
    }

    /// Returns true if the call target may not be statically resolvable.
    pub fn is_dynamic(&self) -> bool {
        matches!(self, CallType::Dynamic | CallType::API)
    }
}

/// The kind of relationship between two symbols.
///
/// Each edge kind represents a specific semantic relationship that
/// can be queried and traversed in the code graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum EdgeKind {
    /// Parent contains child (File -> Function, Class -> Method)
    Contains,

    /// Import/use relationship between files/modules
    Imports {
        /// The import path as written in source
        path: String,
        /// Optional alias (e.g., `use foo as bar`)
        #[serde(skip_serializing_if = "Option::is_none")]
        alias: Option<String>,
    },

    /// Function/method call relationship
    Calls {
        /// The type of call
        call_type: CallType,
        /// Line number where the call occurs
        line: usize,
    },

    /// Class inheritance relationship
    Inherits,

    /// Interface/trait implementation
    Implements,

    /// Type reference in signature or body
    UsesType,

    /// Object/struct instantiation
    Instantiates,

    /// Variable/constant reference
    References,

    /// Macro expansion (macro -> expanded code location)
    ExpandsTo,

    /// Generated code tracking
    GeneratedBy {
        /// The tool/macro that generated the code
        generator: String,
    },
}

impl EdgeKind {
    /// Returns the string representation for queries.
    pub fn as_str(&self) -> &'static str {
        match self {
            EdgeKind::Contains => "contains",
            EdgeKind::Imports { .. } => "imports",
            EdgeKind::Calls { .. } => "calls",
            EdgeKind::Inherits => "inherits",
            EdgeKind::Implements => "implements",
            EdgeKind::UsesType => "uses_type",
            EdgeKind::Instantiates => "instantiates",
            EdgeKind::References => "references",
            EdgeKind::ExpandsTo => "expands_to",
            EdgeKind::GeneratedBy { .. } => "generated_by",
        }
    }

    /// Returns true if this edge represents a structural containment.
    pub fn is_structural(&self) -> bool {
        matches!(self, EdgeKind::Contains)
    }

    /// Returns true if this edge represents a dependency.
    pub fn is_dependency(&self) -> bool {
        matches!(
            self,
            EdgeKind::Imports { .. }
                | EdgeKind::Calls { .. }
                | EdgeKind::UsesType
                | EdgeKind::Instantiates
                | EdgeKind::References
        )
    }

    /// Returns true if this edge represents a type relationship.
    pub fn is_type_relationship(&self) -> bool {
        matches!(
            self,
            EdgeKind::Inherits | EdgeKind::Implements | EdgeKind::UsesType
        )
    }
}

/// An edge connecting two symbols in the code graph.
///
/// Edges are directional, going from `source_id` to `target_id`.
/// The semantic meaning depends on the `kind`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    /// The source node ID (where the edge originates)
    pub source_id: String,

    /// The target node ID (where the edge points to)
    pub target_id: String,

    /// The kind of relationship this edge represents
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

    /// Create a CONTAINS edge (parent contains child).
    pub fn contains(parent_id: impl Into<String>, child_id: impl Into<String>) -> Self {
        Self::new(parent_id, child_id, EdgeKind::Contains)
    }

    /// Create an IMPORTS edge.
    pub fn imports(
        source_id: impl Into<String>,
        target_id: impl Into<String>,
        path: impl Into<String>,
        alias: Option<String>,
    ) -> Self {
        Self::new(
            source_id,
            target_id,
            EdgeKind::Imports {
                path: path.into(),
                alias,
            },
        )
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

    /// Create an INHERITS edge.
    pub fn inherits(child_id: impl Into<String>, parent_id: impl Into<String>) -> Self {
        Self::new(child_id, parent_id, EdgeKind::Inherits)
    }

    /// Create an IMPLEMENTS edge.
    pub fn implements(implementor_id: impl Into<String>, interface_id: impl Into<String>) -> Self {
        Self::new(implementor_id, interface_id, EdgeKind::Implements)
    }

    /// Create a GENERATED_BY edge.
    pub fn generated_by(
        generated_id: impl Into<String>,
        generator_id: impl Into<String>,
        generator_name: impl Into<String>,
    ) -> Self {
        Self::new(
            generated_id,
            generator_id,
            EdgeKind::GeneratedBy {
                generator: generator_name.into(),
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_call_type_as_str() {
        assert_eq!(CallType::Direct.as_str(), "direct");
        assert_eq!(CallType::FFI.as_str(), "ffi");
        assert_eq!(CallType::StaticMethod.as_str(), "static_method");
    }

    #[test]
    fn test_call_type_classification() {
        assert!(CallType::FFI.is_cross_language());
        assert!(CallType::API.is_cross_language());
        assert!(!CallType::Direct.is_cross_language());

        assert!(CallType::Dynamic.is_dynamic());
        assert!(!CallType::Direct.is_dynamic());
    }

    #[test]
    fn test_edge_kind_as_str() {
        assert_eq!(EdgeKind::Contains.as_str(), "contains");
        assert_eq!(
            EdgeKind::Imports {
                path: "foo".into(),
                alias: None
            }
            .as_str(),
            "imports"
        );
        assert_eq!(
            EdgeKind::Calls {
                call_type: CallType::Direct,
                line: 10
            }
            .as_str(),
            "calls"
        );
    }

    #[test]
    fn test_edge_kind_classification() {
        assert!(EdgeKind::Contains.is_structural());
        assert!(!EdgeKind::Inherits.is_structural());

        assert!(
            EdgeKind::Imports {
                path: "x".into(),
                alias: None
            }
            .is_dependency()
        );
        assert!(
            EdgeKind::Calls {
                call_type: CallType::Direct,
                line: 1
            }
            .is_dependency()
        );
        assert!(!EdgeKind::Inherits.is_dependency());

        assert!(EdgeKind::Inherits.is_type_relationship());
        assert!(EdgeKind::Implements.is_type_relationship());
        assert!(!EdgeKind::Contains.is_type_relationship());
    }

    #[test]
    fn test_edge_creation() {
        let edge = Edge::new("src:func:foo:10", "src:func:bar:20", EdgeKind::Contains);
        assert_eq!(edge.source_id, "src:func:foo:10");
        assert_eq!(edge.target_id, "src:func:bar:20");
        assert_eq!(edge.kind.as_str(), "contains");
    }

    #[test]
    fn test_edge_factory_methods() {
        let contains = Edge::contains("parent", "child");
        assert!(matches!(contains.kind, EdgeKind::Contains));

        let imports = Edge::imports("file_a", "file_b", "std::io", Some("io".into()));
        assert!(matches!(imports.kind, EdgeKind::Imports { .. }));

        let calls = Edge::calls("caller", "callee", CallType::Method, 42);
        if let EdgeKind::Calls { call_type, line } = calls.kind {
            assert_eq!(call_type, CallType::Method);
            assert_eq!(line, 42);
        } else {
            panic!("Expected Calls edge");
        }

        let inherits = Edge::inherits("child_class", "parent_class");
        assert!(matches!(inherits.kind, EdgeKind::Inherits));

        let implements = Edge::implements("struct_id", "trait_id");
        assert!(matches!(implements.kind, EdgeKind::Implements));

        let generated = Edge::generated_by("gen_file", "macro_id", "sqlx");
        if let EdgeKind::GeneratedBy { generator } = generated.kind {
            assert_eq!(generator, "sqlx");
        } else {
            panic!("Expected GeneratedBy edge");
        }
    }

    #[test]
    fn test_edge_serialization() {
        let edge = Edge::calls("a", "b", CallType::FFI, 100);
        let json = serde_json::to_string(&edge).expect("serialize");
        let deserialized: Edge = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(deserialized.source_id, edge.source_id);
        assert_eq!(deserialized.target_id, edge.target_id);
    }

    #[test]
    fn test_call_type_serialization() {
        let call_type = CallType::StaticMethod;
        let json = serde_json::to_string(&call_type).expect("serialize");
        assert_eq!(json, "\"static_method\"");
    }

    #[test]
    fn test_edge_kind_serialization() {
        let kind = EdgeKind::Imports {
            path: "std::collections".to_string(),
            alias: Some("col".to_string()),
        };
        let json = serde_json::to_string(&kind).expect("serialize");
        assert!(json.contains("\"type\":\"imports\""));
        assert!(json.contains("\"path\":\"std::collections\""));
        assert!(json.contains("\"alias\":\"col\""));
    }
}
