//! Graph storage using graphqlite.
//!
//! Provides persistent storage for the code graph using SQLite with Cypher query support.

use std::collections::HashMap;
use std::path::Path;

use graphqlite::{CypherResult, Graph, Value};

/// Type alias for the node ID map returned by bulk insert.
pub type NodeIdMap = HashMap<String, i64>;

use crate::edges::{Edge, EdgeKind};
use crate::symbols::{Symbol, SymbolKind, Visibility};

/// Error type for graph store operations.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("Database error: {0}")]
    Database(#[from] graphqlite::Error),
    #[error("Invalid data: {0}")]
    InvalidData(String),
}

pub type Result<T> = std::result::Result<T, StoreError>;

/// Persistent storage for the code graph.
///
/// Uses graphqlite to store symbols as nodes and relationships as edges,
/// supporting Cypher queries for graph traversal.
pub struct GraphStore {
    graph: Graph,
}

impl GraphStore {
    /// Open or create a graph database at the specified path.
    ///
    /// The database file will be created if it doesn't exist.
    /// Use `:memory:` for an in-memory database.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let graph = Graph::open(path)?;
        Ok(Self { graph })
    }

    /// Create an in-memory graph database.
    pub fn open_in_memory() -> Result<Self> {
        let graph = Graph::open_in_memory()?;
        Ok(Self { graph })
    }

    /// Insert or update a symbol as a node in the graph.
    ///
    /// Returns the node ID (generated from the symbol).
    pub fn insert_node(&self, symbol: &Symbol) -> Result<String> {
        let node_id = symbol.id();
        let label = symbol_kind_to_label(symbol.kind);
        let props = symbol_to_properties(symbol);

        self.graph.upsert_node(&node_id, props, label)?;

        Ok(node_id)
    }

    /// Insert multiple symbols as nodes in a batch operation.
    ///
    /// Uses graphqlite's bulk insert for optimal performance.
    /// Returns a map from external node IDs to internal row IDs for edge insertion.
    pub fn insert_nodes_batch(&self, symbols: &[Symbol]) -> Result<NodeIdMap> {
        if symbols.is_empty() {
            return Ok(HashMap::new());
        }

        // Convert symbols to the format expected by insert_nodes_bulk:
        // (external_id, properties, label)
        let nodes: Vec<_> = symbols
            .iter()
            .map(|symbol| {
                let id = symbol.id();
                let label = symbol_kind_to_label(symbol.kind);
                let props = symbol_to_properties(symbol);
                (id, props, label.to_string())
            })
            .collect();

        // Use references for the API
        let node_refs: Vec<_> = nodes
            .iter()
            .map(|(id, props, label)| {
                let prop_refs: Vec<(&str, &str)> =
                    props.iter().map(|(k, v)| (*k, v.as_str())).collect();
                (id.as_str(), prop_refs, label.as_str())
            })
            .collect();

        let id_map = self.graph.insert_nodes_bulk(node_refs)?;
        Ok(id_map)
    }

    /// Insert an edge between two nodes.
    pub fn insert_edge(&self, edge: &Edge) -> Result<()> {
        let rel_type = edge_kind_to_rel_type(&edge.kind);
        let props = edge_to_properties(&edge.kind);

        self.graph
            .upsert_edge(&edge.source_id, &edge.target_id, props, rel_type)?;

        Ok(())
    }

    /// Insert multiple edges in a batch operation.
    ///
    /// Uses graphqlite's bulk insert for optimal performance.
    /// Requires the node ID map from insert_nodes_batch.
    pub fn insert_edges_batch(&self, edges: &[Edge], id_map: &NodeIdMap) -> Result<usize> {
        if edges.is_empty() {
            return Ok(0);
        }

        // Convert edges to the format expected by insert_edges_bulk:
        // (source_id, target_id, properties, rel_type)
        let edge_data: Vec<_> = edges
            .iter()
            .map(|edge| {
                let rel_type = edge_kind_to_rel_type(&edge.kind);
                let props = edge_to_properties(&edge.kind);
                (
                    edge.source_id.clone(),
                    edge.target_id.clone(),
                    props,
                    rel_type.to_string(),
                )
            })
            .collect();

        // Use references for the API
        let edge_refs: Vec<_> = edge_data
            .iter()
            .map(|(src, tgt, props, rel)| {
                let prop_refs: Vec<(&str, &str)> =
                    props.iter().map(|(k, v)| (*k, v.as_str())).collect();
                (src.as_str(), tgt.as_str(), prop_refs, rel.as_str())
            })
            .collect();

        let inserted = self.graph.insert_edges_bulk(edge_refs, id_map)?;
        Ok(inserted)
    }

    /// Insert multiple edges without an ID map (uses transaction for performance).
    ///
    /// Use insert_edges_batch with an ID map for better performance when bulk inserting.
    /// This method wraps all inserts in a transaction to reduce I/O overhead.
    pub fn insert_edges_batch_slow(&self, edges: &[Edge]) -> Result<()> {
        if edges.is_empty() {
            return Ok(());
        }

        // Wrap in transaction for better performance
        self.graph.connection().execute("BEGIN")?;
        let result = edges.iter().try_for_each(|edge| self.insert_edge(edge));
        if result.is_ok() {
            self.graph.connection().execute("COMMIT")?;
        } else {
            let _ = self.graph.connection().execute("ROLLBACK");
        }
        result
    }

    /// Delete all nodes and edges associated with a file.
    ///
    /// This is used when a file is modified or deleted to clear stale data.
    /// Wraps deletes in a transaction for performance.
    pub fn delete_file(&self, file_path: &str) -> Result<usize> {
        // Find all nodes from this file using inline property matching
        let cypher = format!(
            "MATCH (n {{file_path: '{}'}}) RETURN n.id",
            graphqlite::escape_string(file_path)
        );

        let result = self.graph.query(&cypher)?;

        // Collect IDs first to avoid modifying during iteration
        let ids: Vec<String> = result
            .iter()
            .filter_map(|row| row.get::<String>("n.id").ok())
            .collect();

        if ids.is_empty() {
            return Ok(0);
        }

        // Wrap in transaction for performance
        self.graph.connection().execute("BEGIN")?;
        let mut deleted = 0;
        for id in &ids {
            if let Err(e) = self.graph.delete_node(id) {
                let _ = self.graph.connection().execute("ROLLBACK");
                return Err(e.into());
            }
            deleted += 1;
        }
        self.graph.connection().execute("COMMIT")?;

        Ok(deleted)
    }

    /// Delete a specific node by ID.
    pub fn delete_node(&self, node_id: &str) -> Result<()> {
        self.graph.delete_node(node_id)?;
        Ok(())
    }

    /// Delete an edge between two nodes.
    pub fn delete_edge(&self, source_id: &str, target_id: &str) -> Result<()> {
        self.graph.delete_edge(source_id, target_id)?;
        Ok(())
    }

    /// Check if a node exists.
    pub fn has_node(&self, node_id: &str) -> Result<bool> {
        Ok(self.graph.has_node(node_id)?)
    }

    /// Get a node by ID.
    pub fn get_node(&self, node_id: &str) -> Result<Option<Value>> {
        Ok(self.graph.get_node(node_id)?)
    }

    /// Execute a Cypher query.
    pub fn query(&self, cypher: &str) -> Result<CypherResult> {
        Ok(self.graph.query(cypher)?)
    }

    /// Find all callers of a function/method.
    pub fn find_callers(&self, callee_id: &str) -> Result<Vec<Value>> {
        // Use inline property matching for callee node
        let cypher = format!(
            "MATCH (caller)-[:CALLS]->(callee {{id: '{}'}}) RETURN caller",
            graphqlite::escape_string(callee_id)
        );
        let result = self.graph.query(&cypher)?;
        Ok(result
            .iter()
            .filter_map(|r| r.get_value("caller").cloned())
            .collect())
    }

    /// Find all functions/methods called by a caller.
    pub fn find_callees(&self, caller_id: &str) -> Result<Vec<Value>> {
        // Use inline property matching for caller node
        let cypher = format!(
            "MATCH (caller {{id: '{}'}})-[:CALLS]->(callee) RETURN callee",
            graphqlite::escape_string(caller_id)
        );
        let result = self.graph.query(&cypher)?;
        Ok(result
            .iter()
            .filter_map(|r| r.get_value("callee").cloned())
            .collect())
    }

    /// Find implementations of a trait/interface.
    pub fn find_implementations(&self, trait_id: &str) -> Result<Vec<Value>> {
        // Use inline property matching for trait node
        let cypher = format!(
            "MATCH (impl)-[:IMPLEMENTS]->(t {{id: '{}'}}) RETURN impl",
            graphqlite::escape_string(trait_id)
        );
        let result = self.graph.query(&cypher)?;
        Ok(result
            .iter()
            .filter_map(|r| r.get_value("impl").cloned())
            .collect())
    }

    /// Find all symbols in a file.
    pub fn find_symbols_in_file(&self, file_path: &str) -> Result<Vec<Value>> {
        // Use inline property matching for file_path
        let cypher = format!(
            "MATCH (n {{file_path: '{}'}}) RETURN n ORDER BY n.start_line",
            graphqlite::escape_string(file_path)
        );
        let result = self.graph.query(&cypher)?;
        Ok(result
            .iter()
            .filter_map(|r| r.get_value("n").cloned())
            .collect())
    }

    /// Find symbols by name (exact match).
    pub fn find_by_name(&self, name: &str) -> Result<Vec<Value>> {
        // Use inline property matching for exact name match
        let cypher = format!(
            "MATCH (n {{name: '{}'}}) RETURN n",
            graphqlite::escape_string(name)
        );
        let result = self.graph.query(&cypher)?;
        Ok(result
            .iter()
            .filter_map(|r| r.get_value("n").cloned())
            .collect())
    }

    /// Get graph statistics.
    pub fn stats(&self) -> Result<GraphStats> {
        let stats = self.graph.stats()?;
        Ok(GraphStats {
            node_count: stats.nodes,
            edge_count: stats.edges,
        })
    }

    /// Get the underlying graphqlite Graph for advanced operations.
    pub fn inner(&self) -> &Graph {
        &self.graph
    }
}

/// Graph statistics.
#[derive(Debug, Clone)]
pub struct GraphStats {
    pub node_count: i64,
    pub edge_count: i64,
}

/// Convert a SymbolKind to a node label.
fn symbol_kind_to_label(kind: SymbolKind) -> &'static str {
    match kind {
        SymbolKind::File => "File",
        SymbolKind::Module => "Module",
        SymbolKind::Class => "Class",
        SymbolKind::Struct => "Struct",
        SymbolKind::Interface => "Interface",
        SymbolKind::Enum => "Enum",
        SymbolKind::Function => "Function",
        SymbolKind::Method => "Method",
        SymbolKind::Variable => "Variable",
        SymbolKind::Type => "Type",
        SymbolKind::Macro => "Macro",
    }
}

/// Convert a Symbol to property key-value pairs (as strings for graphqlite).
fn symbol_to_properties(symbol: &Symbol) -> Vec<(&'static str, String)> {
    let mut props = vec![
        ("name", symbol.name.clone()),
        ("kind", symbol.kind.as_str().to_string()),
        ("file_path", symbol.file_path.clone()),
        ("start_line", symbol.start_line.to_string()),
        ("end_line", symbol.end_line.to_string()),
        ("visibility", visibility_to_string(&symbol.visibility)),
    ];

    if let Some(ref sig) = symbol.signature {
        props.push(("signature", sig.clone()));
    }

    if let Some(ref qn) = symbol.qualified_name {
        props.push(("qualified_name", qn.clone()));
    }

    if let Some(ref doc) = symbol.doc_comment {
        props.push(("doc_comment", doc.clone()));
    }

    props
}

/// Convert Visibility to a string representation.
fn visibility_to_string(vis: &Visibility) -> String {
    match vis {
        Visibility::Public => "public".to_string(),
        Visibility::Private => "private".to_string(),
        Visibility::Crate => "crate".to_string(),
        Visibility::Restricted(path) => format!("restricted:{}", path),
    }
}

/// Convert an EdgeKind to a relationship type.
fn edge_kind_to_rel_type(kind: &EdgeKind) -> &'static str {
    match kind {
        EdgeKind::Contains => "_CONTAINS", // Prefixed to avoid Cypher reserved word
        EdgeKind::Imports { .. } => "IMPORTS",
        EdgeKind::Calls { .. } => "CALLS",
        EdgeKind::Inherits => "INHERITS",
        EdgeKind::Implements => "IMPLEMENTS",
        EdgeKind::UsesType => "USES_TYPE",
        EdgeKind::Instantiates => "INSTANTIATES",
        EdgeKind::References => "REFERENCES",
        EdgeKind::ExpandsTo => "EXPANDS_TO",
        EdgeKind::GeneratedBy { .. } => "GENERATED_BY",
    }
}

/// Convert an EdgeKind to property key-value pairs (as strings).
fn edge_to_properties(kind: &EdgeKind) -> Vec<(&'static str, String)> {
    match kind {
        EdgeKind::Imports { path, alias } => {
            let mut props = vec![("import_path", path.clone())];
            if let Some(a) = alias {
                props.push(("alias", a.clone()));
            }
            props
        }
        EdgeKind::Calls { call_type, line } => {
            vec![
                ("call_type", call_type.as_str().to_string()),
                ("line", line.to_string()),
            ]
        }
        EdgeKind::GeneratedBy { generator } => {
            vec![("generator", generator.clone())]
        }
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edges::CallType;
    use serial_test::serial;

    fn create_test_symbol(name: &str, kind: SymbolKind) -> Symbol {
        Symbol {
            name: name.to_string(),
            kind,
            file_path: "test.rs".to_string(),
            start_line: 1,
            end_line: 10,
            signature: None,
            qualified_name: None,
            doc_comment: None,
            visibility: Visibility::Public,
        }
    }

    #[test]
    #[serial]
    fn test_store_open_in_memory() {
        let store = GraphStore::open_in_memory().expect("Should open in-memory store");
        let stats = store.stats().expect("Should get stats");
        assert_eq!(stats.node_count, 0);
        assert_eq!(stats.edge_count, 0);
    }

    #[test]
    #[serial]
    fn test_insert_node() {
        let store = GraphStore::open_in_memory().unwrap();
        let symbol = create_test_symbol("Foo", SymbolKind::Struct);

        let node_id = store.insert_node(&symbol).expect("Should insert node");
        assert!(!node_id.is_empty());

        assert!(store.has_node(&node_id).expect("Should check node"));

        let stats = store.stats().unwrap();
        assert_eq!(stats.node_count, 1);
    }

    #[test]
    #[serial]
    fn test_insert_edge() {
        let store = GraphStore::open_in_memory().unwrap();

        let func = create_test_symbol("main", SymbolKind::Function);
        let callee = create_test_symbol("helper", SymbolKind::Function);

        let func_id = store.insert_node(&func).unwrap();
        let callee_id = store.insert_node(&callee).unwrap();

        let edge = Edge::calls(&func_id, &callee_id, CallType::Direct, 5);
        store.insert_edge(&edge).expect("Should insert edge");

        let stats = store.stats().unwrap();
        assert_eq!(stats.node_count, 2);
        assert_eq!(stats.edge_count, 1);
    }

    #[test]
    #[serial]
    fn test_delete_node() {
        let store = GraphStore::open_in_memory().unwrap();
        let symbol = create_test_symbol("Foo", SymbolKind::Struct);

        let node_id = store.insert_node(&symbol).unwrap();
        assert!(store.has_node(&node_id).unwrap());

        store.delete_node(&node_id).expect("Should delete node");
        assert!(!store.has_node(&node_id).unwrap());
    }

    #[test]
    #[serial]
    fn test_delete_file() {
        let store = GraphStore::open_in_memory().unwrap();

        // Insert symbols from two different files
        let sym1 = Symbol {
            file_path: "file1.rs".to_string(),
            ..create_test_symbol("Foo", SymbolKind::Struct)
        };
        let sym2 = Symbol {
            file_path: "file1.rs".to_string(),
            ..create_test_symbol("Bar", SymbolKind::Struct)
        };
        let sym3 = Symbol {
            file_path: "file2.rs".to_string(),
            ..create_test_symbol("Baz", SymbolKind::Struct)
        };

        store.insert_node(&sym1).unwrap();
        store.insert_node(&sym2).unwrap();
        store.insert_node(&sym3).unwrap();

        assert_eq!(store.stats().unwrap().node_count, 3);

        // Delete file1.rs symbols
        let deleted = store.delete_file("file1.rs").expect("Should delete file");
        assert_eq!(deleted, 2);
        assert_eq!(store.stats().unwrap().node_count, 1);
    }

    #[test]
    #[serial]
    fn test_find_callers() {
        let store = GraphStore::open_in_memory().unwrap();

        let main_fn = create_test_symbol("main", SymbolKind::Function);
        let helper_fn = create_test_symbol("helper", SymbolKind::Function);
        let util_fn = create_test_symbol("util", SymbolKind::Function);

        let main_id = store.insert_node(&main_fn).unwrap();
        let helper_id = store.insert_node(&helper_fn).unwrap();
        let util_id = store.insert_node(&util_fn).unwrap();

        // main calls helper, util calls helper
        store
            .insert_edge(&Edge::calls(&main_id, &helper_id, CallType::Direct, 5))
            .unwrap();
        store
            .insert_edge(&Edge::calls(&util_id, &helper_id, CallType::Direct, 10))
            .unwrap();

        let callers = store.find_callers(&helper_id).expect("Should find callers");
        assert_eq!(callers.len(), 2);
    }

    #[test]
    #[serial]
    fn test_find_callees() {
        let store = GraphStore::open_in_memory().unwrap();

        let main_fn = create_test_symbol("main", SymbolKind::Function);
        let helper_fn = create_test_symbol("helper", SymbolKind::Function);
        let util_fn = create_test_symbol("util", SymbolKind::Function);

        let main_id = store.insert_node(&main_fn).unwrap();
        let helper_id = store.insert_node(&helper_fn).unwrap();
        let util_id = store.insert_node(&util_fn).unwrap();

        // main calls helper and util
        store
            .insert_edge(&Edge::calls(&main_id, &helper_id, CallType::Direct, 5))
            .unwrap();
        store
            .insert_edge(&Edge::calls(&main_id, &util_id, CallType::Direct, 10))
            .unwrap();

        let callees = store.find_callees(&main_id).expect("Should find callees");
        assert_eq!(callees.len(), 2);
    }

    #[test]
    #[serial]
    fn test_find_implementations() {
        let store = GraphStore::open_in_memory().unwrap();

        let trait_sym = create_test_symbol("Greet", SymbolKind::Interface);
        let impl1 = create_test_symbol("Person", SymbolKind::Struct);
        let impl2 = create_test_symbol("Robot", SymbolKind::Struct);

        let trait_id = store.insert_node(&trait_sym).unwrap();
        let impl1_id = store.insert_node(&impl1).unwrap();
        let impl2_id = store.insert_node(&impl2).unwrap();

        store
            .insert_edge(&Edge::implements(&impl1_id, &trait_id))
            .unwrap();
        store
            .insert_edge(&Edge::implements(&impl2_id, &trait_id))
            .unwrap();

        let impls = store
            .find_implementations(&trait_id)
            .expect("Should find implementations");
        assert_eq!(impls.len(), 2);
    }

    #[test]
    #[serial]
    fn test_find_symbols_in_file() {
        let store = GraphStore::open_in_memory().unwrap();

        let sym1 = Symbol {
            file_path: "lib.rs".to_string(),
            start_line: 1,
            ..create_test_symbol("A", SymbolKind::Struct)
        };
        let sym2 = Symbol {
            file_path: "lib.rs".to_string(),
            start_line: 10,
            ..create_test_symbol("B", SymbolKind::Function)
        };
        let sym3 = Symbol {
            file_path: "other.rs".to_string(),
            start_line: 1,
            ..create_test_symbol("C", SymbolKind::Struct)
        };

        store.insert_node(&sym1).unwrap();
        store.insert_node(&sym2).unwrap();
        store.insert_node(&sym3).unwrap();

        let symbols = store
            .find_symbols_in_file("lib.rs")
            .expect("Should find symbols");
        assert_eq!(symbols.len(), 2);
    }

    #[test]
    #[serial]
    fn test_batch_insert() {
        let store = GraphStore::open_in_memory().unwrap();

        let symbols = vec![
            create_test_symbol("A", SymbolKind::Struct),
            create_test_symbol("B", SymbolKind::Struct),
            create_test_symbol("C", SymbolKind::Function),
        ];

        let ids = store
            .insert_nodes_batch(&symbols)
            .expect("Should batch insert");
        assert_eq!(ids.len(), 3);
        assert_eq!(store.stats().unwrap().node_count, 3);
    }

    #[test]
    #[serial]
    fn test_cypher_query() {
        let store = GraphStore::open_in_memory().unwrap();

        let sym = create_test_symbol("MyStruct", SymbolKind::Struct);
        store.insert_node(&sym).unwrap();

        let result = store
            .query("MATCH (n:Struct) RETURN n.name")
            .expect("Should query");

        let names: Vec<String> = result
            .iter()
            .filter_map(|r| r.get::<String>("n.name").ok())
            .collect();

        assert!(names.contains(&"MyStruct".to_string()));
    }
}
