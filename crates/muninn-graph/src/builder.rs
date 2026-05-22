//! Graph builder.
//!
//! Walks a source tree, runs each supported file through the
//! vendored narsil `LanguageParser` to get a tree-sitter parse,
//! then feeds the lot to narsil's `CallGraph` for symbol +
//! call-edge extraction with scope-hint resolution.
//!
//! The narsil call graph is held in memory while we adapt its
//! `CallNode`/`CallEdge` into our `Symbol`/`Edge` types and
//! persist them through [`GraphStore`].
//!
//! What this module does NOT do anymore: hand-written tree-sitter
//! queries, manual cross-file resolution, SCIP ingest. All of that
//! was removed when we vendored narsil — see
//! `crates/muninn-narsil-vendor/NOTICE.md`.

use std::path::Path;

use muninn_narsil_vendor::callgraph::{CallGraph, CallNode};
use muninn_narsil_vendor::parser::LanguageParser;
use muninn_narsil_vendor::tree_sitter::Tree;

use crate::edges::{CallType, Edge, EdgeKind};
use crate::store::{GraphStore, StoreError};
use crate::symbols::{Symbol, SymbolKind, Visibility};

/// Error type for graph building operations.
#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    #[error("store: {0}")]
    Store(#[from] StoreError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("extractor: {0}")]
    Extractor(String),
}

impl From<anyhow::Error> for BuildError {
    fn from(e: anyhow::Error) -> Self {
        BuildError::Extractor(format!("{e:#}"))
    }
}

pub type Result<T> = std::result::Result<T, BuildError>;

/// Aggregate counts for an indexing run.
#[derive(Debug, Clone, Default)]
pub struct BuildStats {
    pub files_processed: usize,
    pub nodes_added: usize,
    pub edges_added: usize,
}

/// Walks source, drives the vendored extractor, persists to the store.
pub struct GraphBuilder {
    parser: LanguageParser,
    store: GraphStore,
}

impl GraphBuilder {
    pub fn new(store: GraphStore) -> Result<Self> {
        let parser = LanguageParser::new().map_err(BuildError::from)?;
        Ok(Self { parser, store })
    }

    pub fn store(&self) -> &GraphStore {
        &self.store
    }

    pub fn store_mut(&mut self) -> &mut GraphStore {
        &mut self.store
    }

    pub fn into_store(self) -> GraphStore {
        self.store
    }

    /// Index every supported file under `root`. Returns counts.
    pub fn build_directory(&mut self, root: &Path) -> Result<BuildStats> {
        let parsed_files = self.collect_parsed_files(root)?;
        self.persist_call_graph(&parsed_files)
    }

    fn collect_parsed_files(&self, root: &Path) -> Result<Vec<(String, String, Tree)>> {
        let mut out = Vec::new();
        if root.is_file() {
            if let Some(triple) = self.parse_one(root)? {
                out.push(triple);
            }
            return Ok(out);
        }
        self.walk_recursive(root, &mut out)?;
        Ok(out)
    }

    fn walk_recursive(&self, dir: &Path, out: &mut Vec<(String, String, Tree)>) -> Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                let skip = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with('.') || n == "target" || n == "node_modules");
                if skip {
                    continue;
                }
                self.walk_recursive(&path, out)?;
            } else if is_supported_source_file(&path) {
                if let Some(triple) = self.parse_one(&path)? {
                    out.push(triple);
                }
            }
        }
        Ok(())
    }

    fn parse_one(&self, path: &Path) -> Result<Option<(String, String, Tree)>> {
        let Ok(content) = std::fs::read_to_string(path) else {
            return Ok(None); // binary / unreadable — skip silently
        };
        let tree = match self.parser.parse_to_tree(path, &content) {
            Ok(t) => t,
            Err(_) => return Ok(None), // unsupported language for narsil's parser
        };
        Ok(Some((path.to_string_lossy().to_string(), content, tree)))
    }

    fn persist_call_graph(&self, files: &[(String, String, Tree)]) -> Result<BuildStats> {
        let cg = CallGraph::new();
        cg.build_from_files(files).map_err(BuildError::from)?;

        let mut symbols: Vec<Symbol> = Vec::new();
        let mut edges: Vec<Edge> = Vec::new();
        // narsil keys its DashMap by `"<file_path>::<function_name>"` —
        // both for the node lookup AND for `CallEdge.target` / `.called_by`.
        // Map that qualified KEY (the entry's key, not `CallNode.name`)
        // to our Symbol::id() so CallEdge.target lookups hit.
        let mut qkey_to_id: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();

        for entry in cg.iter_nodes() {
            let qkey = entry.key().clone();
            let node: &CallNode = entry.value();
            // call_degree = inbound + outbound edges. This is the same
            // signal narsil's `get_hotspots` uses to rank functions —
            // surfacing it as a node property lets `find_symbols` /
            // `graph_query` cheaply find the load-bearing code without
            // a second pass.
            let degree = node.calls.len() + node.called_by.len();
            let sym = call_node_to_symbol(node, degree);
            qkey_to_id.insert(qkey, sym.id());
            symbols.push(sym);
        }

        for entry in cg.iter_nodes() {
            let qkey = entry.key();
            let node: &CallNode = entry.value();
            let Some(source_id) = qkey_to_id.get(qkey).cloned() else {
                continue;
            };
            for ce in &node.calls {
                let Some(target_id) = qkey_to_id.get(&ce.target).cloned() else {
                    // Narsil's resolver couldn't pin this callee to a
                    // workspace symbol (commonly: stdlib / dep / extern).
                    // Skipping is the right move — graphqlite would drop
                    // the edge and unresolved placeholders just clutter.
                    continue;
                };
                edges.push(Edge {
                    source_id: source_id.clone(),
                    target_id,
                    kind: EdgeKind::Calls {
                        call_type: map_call_type(&ce.call_type),
                        line: ce.line,
                    },
                });
            }
        }

        let mut stats = BuildStats {
            files_processed: files.len(),
            nodes_added: 0,
            edges_added: 0,
        };
        if !symbols.is_empty() {
            self.store.insert_nodes_batch(&symbols)?;
            stats.nodes_added = symbols.len();
        }
        if !edges.is_empty() {
            self.store.insert_edges_batch_slow(&edges)?;
            stats.edges_added = edges.len();
        }
        Ok(stats)
    }
}

fn is_supported_source_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("rs" | "py" | "js" | "ts" | "tsx" | "jsx" | "go" | "c" | "cpp" | "h" | "hpp" | "java")
    )
}

/// Adapt narsil's `CallNode` to our `Symbol`. Narsil stores the
/// fully-qualified name (post-scope-hint resolution) as `name`; we
/// keep that as `qualified_name` and derive a short display name.
/// `degree` is `len(node.calls) + len(node.called_by)` — passed in
/// rather than computed here so the caller can iterate the call
/// graph once.
fn call_node_to_symbol(node: &CallNode, degree: usize) -> Symbol {
    let short = node
        .name
        .rsplit("::")
        .next()
        .unwrap_or(&node.name)
        .rsplit('.')
        .next()
        .unwrap_or(&node.name)
        .to_string();
    Symbol {
        name: short,
        kind: SymbolKind::Function,
        file_path: node.file_path.clone(),
        start_line: node.line,
        end_line: node.line,
        signature: None,
        qualified_name: Some(node.name.clone()),
        doc_comment: None,
        visibility: Visibility::Public,
        cyclomatic: Some(node.metrics.cyclomatic),
        cognitive: Some(node.metrics.cognitive),
        call_degree: Some(degree),
    }
}

fn map_call_type(t: &muninn_narsil_vendor::callgraph::CallType) -> CallType {
    use muninn_narsil_vendor::callgraph::CallType as N;
    match t {
        N::Direct => CallType::Direct,
        N::Method => CallType::Method,
        N::StaticMethod => CallType::StaticMethod,
        N::Closure => CallType::Direct,
        // Async/Spawn/Unknown collapse to Direct — our edge model
        // doesn't differentiate further. Refine when we surface
        // those call types in queries.
        N::Async | N::Spawn | N::Unknown => CallType::Direct,
    }
}
