//! Ingest a SCIP (Source Code Intelligence Protocol) index into
//! the muninn code graph.
//!
//! SCIP is an indexer-produced protobuf describing symbols and
//! their occurrences across a workspace. The canonical Rust
//! producer is `rust-analyzer scip <path>` which uses
//! rust-analyzer's full semantic resolver — so cross-crate
//! references, `use` aliases, trait/impl semantics, etc. all come
//! resolved correctly out of the box, no per-language tree-sitter
//! extractor needed.
//!
//! ## Shape of a SCIP index (just the fields we care about)
//!
//! An [`Index`] contains many [`Document`]s. Each document
//! corresponds to a source file and has:
//!   - `occurrences`: positions in the file annotated with a
//!     symbol string and a role bitset (definition vs reference)
//!   - `symbols`: `SymbolInformation` for the symbols defined in
//!     the file (kind, display name, documentation)
//!
//! A symbol string looks like:
//!
//! ```text
//! rust-analyzer cargo muninn_core 0.0.1 muninn_core/daemon/socket_path_for_repo().
//! ```
//!
//! ## What this ingest produces
//!
//! For each Definition occurrence in each Document, we insert a
//! [`Symbol`] node carrying the SCIP symbol string (as both
//! `qualified_name` and `id` source material), name (the display
//! name), kind (mapped from SCIP's kind enum), and source location.
//!
//! For each Reference occurrence inside a function's lexical range,
//! we emit a CALLS edge from the enclosing function's node to the
//! referenced symbol's node — provided the referenced symbol is
//! itself defined somewhere in the SCIP index. Cross-crate
//! references where the target is an external dependency become
//! edges to an `unresolved__*` placeholder (graphqlite drops them
//! on insert; same behavior as the tree-sitter pipeline).
//!
//! ## What this ingest doesn't yet do
//!
//! - Relationships (`implements`, `type_definition`, `references`):
//!   would map to IMPLEMENTS-style edges. SCIP carries them as a
//!   side-channel on `SymbolInformation.relationships`. Future work.
//! - External symbol bookkeeping (`Index.external_symbols`).
//! - Multi-language merging (SCIP indexes from multiple languages
//!   point at different documents but can cross-reference).

use std::collections::HashMap;
use std::path::Path;

use protobuf::Message;
use scip::types::{Index, Occurrence, SymbolInformation, symbol_information};

use crate::edges::{CallType, Edge, EdgeKind};
use crate::store::{GraphStore, StoreError};
use crate::symbols::{Symbol, SymbolKind, Visibility};

/// Errors from SCIP ingest.
#[derive(Debug, thiserror::Error)]
pub enum ScipIngestError {
    #[error("read scip file: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse scip protobuf: {0}")]
    Protobuf(#[from] protobuf::Error),
    #[error("store: {0}")]
    Store(#[from] StoreError),
}

/// Aggregate counts for a SCIP ingest pass.
#[derive(Debug, Default)]
pub struct IngestStats {
    pub documents: usize,
    pub symbol_nodes: usize,
    pub call_edges: usize,
}

/// Ingest a SCIP index from `scip_path` into `store`.
///
/// `project_root` is joined to each document's `relative_path` to
/// produce the absolute `file_path` on `Symbol`s — match the daemon's
/// notion of the workspace root so MCP / hook callers see consistent
/// paths.
pub fn ingest_scip(
    scip_path: &Path,
    project_root: &Path,
    store: &GraphStore,
) -> Result<IngestStats, ScipIngestError> {
    let bytes = std::fs::read(scip_path)?;
    let index = Index::parse_from_bytes(&bytes)?;

    let mut stats = IngestStats::default();

    // Pass 1: walk all documents, collect Definition occurrences
    // into a workspace-wide map keyed by SCIP symbol string.
    // We need this map populated before resolving references, so
    // pass 2 can emit edges that point at the right node.
    let mut scip_symbol_to_node: HashMap<String, String> = HashMap::new();
    let mut nodes_to_insert: Vec<Symbol> = Vec::new();

    for doc in &index.documents {
        stats.documents += 1;
        let abs_file = project_root.join(&doc.relative_path);
        let abs_file_str = abs_file.to_string_lossy().to_string();

        // Build an index of SymbolInformation by symbol string for
        // quick lookup of display_name and kind during occurrence
        // processing.
        let info_by_symbol: HashMap<&str, &SymbolInformation> = doc
            .symbols
            .iter()
            .map(|info| (info.symbol.as_str(), info))
            .collect();

        for occ in &doc.occurrences {
            if !is_definition(occ) {
                continue;
            }
            let info = info_by_symbol.get(occ.symbol.as_str()).copied();
            let symbol_node = scip_occurrence_to_symbol(occ, info, &abs_file_str);
            if let Some(s) = symbol_node {
                scip_symbol_to_node.insert(occ.symbol.clone(), s.id());
                nodes_to_insert.push(s);
            }
        }
    }

    // Persist all nodes before edges so edge target lookups succeed.
    if !nodes_to_insert.is_empty() {
        store.insert_nodes_batch(&nodes_to_insert)?;
        stats.symbol_nodes = nodes_to_insert.len();
    }

    // Build a workspace-wide map of SymbolInformation by SCIP
    // symbol string. We need it across documents (not just
    // per-doc) so we can ask "is this callee a function/method?"
    // even when the callee lives in a different file.
    let mut info_by_symbol: HashMap<String, &SymbolInformation> = HashMap::new();
    for doc in &index.documents {
        for info in &doc.symbols {
            info_by_symbol.entry(info.symbol.clone()).or_insert(info);
        }
    }

    // Pass 2: walk reference occurrences, find the enclosing
    // function (or method/type) by lexical containment, and emit
    // CALLS edges — but only when the callee is itself a callable
    // (function / method / constructor). rust-analyzer's SCIP
    // doesn't populate `syntax_kind` reliably, so we filter by
    // the *callee's* SymbolInformation.kind instead. This is the
    // right semantic anyway: type references shouldn't show up
    // as CALLS edges.
    let mut edges_to_insert: Vec<Edge> = Vec::new();

    for doc in &index.documents {
        // Collect definition occurrences from THIS document and
        // index them by line range for the containment check.
        let local_defs: Vec<(&Occurrence, &SymbolInformation)> = doc
            .occurrences
            .iter()
            .filter(|o| is_definition(o))
            .filter_map(|o| {
                doc.symbols
                    .iter()
                    .find(|i| i.symbol == o.symbol)
                    .map(|i| (o, i))
            })
            .collect();

        for occ in &doc.occurrences {
            if is_definition(occ) {
                continue;
            }
            // Resolve callee: must be a Definition we recorded in pass 1.
            let Some(target_id) = scip_symbol_to_node.get(&occ.symbol) else {
                continue;
            };
            // Callee must be a callable kind (function/method/etc.)
            // — skips type references, field accesses, etc.
            let callee_info = info_by_symbol.get(&occ.symbol);
            if !callee_info.is_some_and(|i| is_callable_kind(i)) {
                continue;
            }

            // Find the enclosing definition in this document — the
            // function/method whose range covers this occurrence's line.
            let occ_line = range_start_line(&occ.range);
            let caller = local_defs.iter().find(|(def_occ, info)| {
                if !is_callable_kind(info) {
                    return false;
                }
                let (start, end) = def_range(def_occ);
                start <= occ_line && occ_line <= end
            });

            let Some((_def_occ, caller_info)) = caller else {
                continue;
            };
            let Some(caller_id) = scip_symbol_to_node.get(&caller_info.symbol) else {
                continue;
            };

            edges_to_insert.push(Edge {
                source_id: caller_id.clone(),
                target_id: target_id.clone(),
                kind: EdgeKind::Calls {
                    call_type: CallType::Direct,
                    line: (occ_line + 1) as usize,
                },
            });
        }
    }

    if !edges_to_insert.is_empty() {
        store.insert_edges_batch_slow(&edges_to_insert)?;
        stats.call_edges = edges_to_insert.len();
    }

    Ok(stats)
}

/// SCIP roles bitset: Definition = bit 0 (value 1).
const SYMBOL_ROLE_DEFINITION: i32 = 0x1;

fn is_definition(occ: &Occurrence) -> bool {
    occ.symbol_roles & SYMBOL_ROLE_DEFINITION != 0
}

/// Compute (start_line, end_line) from a SCIP occurrence.
///
/// SCIP encodes two ranges per occurrence:
///   - `range`: the span of the symbol's NAME token (often a
///     single line — e.g. the function's `pub fn foo` line).
///   - `enclosing_range`: the span of the symbol's BODY (the
///     whole function block). Only populated on Definition
///     occurrences; empty on References.
///
/// For containment checks against reference lines we want the
/// body span, so prefer `enclosing_range` when present. Fall
/// back to `range` (the name span) when it isn't.
fn def_range(occ: &Occurrence) -> (i32, i32) {
    let r = if !occ.enclosing_range.is_empty() {
        &occ.enclosing_range
    } else {
        &occ.range
    };
    let start = r.first().copied().unwrap_or(0);
    let end = if r.len() >= 4 {
        r[2]
    } else {
        start // single-line occurrence
    };
    (start, end)
}

fn range_start_line(range: &[i32]) -> i32 {
    range.first().copied().unwrap_or(0)
}

/// Map a SCIP `SymbolInformation.kind` to our [`SymbolKind`].
/// Definitions without a `SymbolInformation` (rare — usually
/// indexer-internal) fall back to `Function`.
fn scip_kind_to_symbol_kind(kind: symbol_information::Kind) -> SymbolKind {
    use symbol_information::Kind as K;
    match kind {
        K::Function | K::StaticMethod | K::AbstractMethod => SymbolKind::Function,
        K::Method => SymbolKind::Method,
        K::Class | K::Struct | K::Object => SymbolKind::Struct,
        K::Enum => SymbolKind::Enum,
        K::Interface | K::Trait => SymbolKind::Interface,
        K::Module | K::Namespace | K::Package => SymbolKind::Module,
        K::Variable | K::Field | K::Constant | K::Parameter | K::Property => SymbolKind::Variable,
        K::Type | K::TypeAlias | K::TypeParameter => SymbolKind::Type,
        K::Macro => SymbolKind::Macro,
        _ => SymbolKind::Function,
    }
}

fn is_callable_kind(info: &SymbolInformation) -> bool {
    use symbol_information::Kind as K;
    matches!(
        info.kind.enum_value_or_default(),
        K::Function | K::Method | K::Constructor | K::StaticMethod | K::AbstractMethod
    )
}

/// Build a [`Symbol`] from a SCIP definition occurrence + its
/// `SymbolInformation` (when present). Returns `None` if we can't
/// derive a usable display name.
fn scip_occurrence_to_symbol(
    occ: &Occurrence,
    info: Option<&SymbolInformation>,
    file_path: &str,
) -> Option<Symbol> {
    let (start, end) = def_range(occ);
    let start_line = (start + 1).max(1) as usize;
    let end_line = (end + 1).max(start_line as i32) as usize;

    let display_name = info
        .and_then(|i| {
            if i.display_name.is_empty() {
                None
            } else {
                Some(i.display_name.clone())
            }
        })
        .or_else(|| derive_name_from_symbol_string(&occ.symbol))?;

    let kind = info
        .map(|i| scip_kind_to_symbol_kind(i.kind.enum_value_or_default()))
        .unwrap_or(SymbolKind::Function);

    Some(Symbol {
        name: display_name,
        kind,
        file_path: file_path.to_string(),
        start_line,
        end_line,
        signature: info.and_then(|i| {
            i.signature_documentation
                .as_ref()
                .map(|d| d.text.clone())
                .filter(|s| !s.is_empty())
        }),
        qualified_name: Some(occ.symbol.clone()),
        doc_comment: info.and_then(|i| {
            if i.documentation.is_empty() {
                None
            } else {
                Some(i.documentation.join("\n"))
            }
        }),
        visibility: Visibility::Public, // SCIP doesn't distinguish at the occurrence level
    })
}

/// SCIP symbol strings end with descriptors like `foo()`,
/// `Foo#bar().`, `Foo#`, etc. Pull a reasonable display name out as
/// a fallback when `SymbolInformation.display_name` is missing.
/// Best-effort — the proper path is to use `display_name`.
fn derive_name_from_symbol_string(symbol: &str) -> Option<String> {
    // The last descriptor segment, between the final `/` (or first
    // space after the package prefix) and any trailing `()` / `#`
    // / `.` / `:`.
    let after_slash = symbol.rsplit('/').next()?;
    let cleaned: String = after_slash
        .trim_end_matches('.')
        .trim_end_matches(')')
        .trim_end_matches('(')
        .trim_end_matches('#')
        .trim_end_matches(':')
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn definition_role_bit_check() {
        let mut occ = Occurrence::new();
        occ.symbol_roles = 0;
        assert!(!is_definition(&occ));
        occ.symbol_roles = 1;
        assert!(is_definition(&occ));
        occ.symbol_roles = 5; // Definition | ReadAccess
        assert!(is_definition(&occ));
    }

    #[test]
    fn range_decoding_four_element() {
        let mut occ = Occurrence::new();
        occ.range = vec![5, 0, 10, 0]; // start_line=5, end_line=10
        assert_eq!(def_range(&occ), (5, 10));
    }

    #[test]
    fn range_decoding_three_element() {
        let mut occ = Occurrence::new();
        occ.range = vec![3, 0, 20]; // single-line, end_char=20
        assert_eq!(def_range(&occ), (3, 3));
    }

    #[test]
    fn name_derivation_from_scip_symbol() {
        assert_eq!(
            derive_name_from_symbol_string(
                "rust-analyzer cargo muninn_core 0.0.1 muninn_core/daemon/socket_path_for_repo()."
            ),
            Some("socket_path_for_repo".to_string())
        );
        assert_eq!(
            derive_name_from_symbol_string("rust-analyzer cargo crate 0.1.0 crate/Foo#"),
            Some("Foo".to_string())
        );
    }
}
