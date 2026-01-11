//! Integration tests for muninn-graph
//!
//! Tests the public API of the graph crate.

use std::fs;
use std::path::Path;

use muninn_graph::{Language, Parser, RustExtractor};

/// Parse muninn-graph's own source code to verify the Rust extractor works on real code.
#[test]
fn parse_muninn_graph_source() {
    let mut parser = Parser::new();

    // Get the crate source directory
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let src_dir = Path::new(manifest_dir).join("src");

    // Parse lib.rs
    let lib_path = src_dir.join("lib.rs");
    let parsed = parser.parse_file(&lib_path).expect("Should parse lib.rs");

    assert_eq!(parsed.language, Language::Rust);

    // Extract symbols from lib.rs
    let symbols = RustExtractor::extract_symbols(&parsed.tree, &parsed.source, "lib.rs")
        .expect("Should extract symbols");

    // lib.rs should have module declarations
    assert!(
        !symbols.is_empty(),
        "lib.rs should have at least some symbols"
    );

    // Check that we find the expected modules
    let module_names: Vec<_> = symbols
        .iter()
        .filter(|s| s.kind == muninn_graph::SymbolKind::Module)
        .map(|s| s.name.as_str())
        .collect();

    assert!(
        module_names.contains(&"edges"),
        "Should find 'edges' module declaration"
    );
    assert!(
        module_names.contains(&"symbols"),
        "Should find 'symbols' module declaration"
    );
    assert!(
        module_names.contains(&"parser"),
        "Should find 'parser' module declaration"
    );
}

/// Parse symbols.rs and verify symbol extraction.
#[test]
fn parse_symbols_module() {
    let mut parser = Parser::new();

    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let symbols_path = Path::new(manifest_dir).join("src").join("symbols.rs");

    let parsed = parser
        .parse_file(&symbols_path)
        .expect("Should parse symbols.rs");

    let symbols = RustExtractor::extract_symbols(&parsed.tree, &parsed.source, "symbols.rs")
        .expect("Should extract symbols");

    // Check for expected types
    let struct_names: Vec<_> = symbols
        .iter()
        .filter(|s| s.kind == muninn_graph::SymbolKind::Struct)
        .map(|s| s.name.as_str())
        .collect();

    assert!(
        struct_names.contains(&"Symbol"),
        "Should find Symbol struct"
    );

    // Check for enums
    let enum_names: Vec<_> = symbols
        .iter()
        .filter(|s| s.kind == muninn_graph::SymbolKind::Enum)
        .map(|s| s.name.as_str())
        .collect();

    assert!(
        enum_names.contains(&"SymbolKind"),
        "Should find SymbolKind enum"
    );
}

/// Parse edges.rs and verify symbol extraction.
#[test]
fn parse_edges_module() {
    let mut parser = Parser::new();

    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let edges_path = Path::new(manifest_dir).join("src").join("edges.rs");

    let parsed = parser
        .parse_file(&edges_path)
        .expect("Should parse edges.rs");

    let symbols = RustExtractor::extract_symbols(&parsed.tree, &parsed.source, "edges.rs")
        .expect("Should extract symbols");

    // Check for expected enums
    let enum_names: Vec<_> = symbols
        .iter()
        .filter(|s| s.kind == muninn_graph::SymbolKind::Enum)
        .map(|s| s.name.as_str())
        .collect();

    assert!(
        enum_names.contains(&"EdgeKind"),
        "Should find EdgeKind enum"
    );
    assert!(
        enum_names.contains(&"CallType"),
        "Should find CallType enum"
    );

    // Check for Edge struct
    let struct_names: Vec<_> = symbols
        .iter()
        .filter(|s| s.kind == muninn_graph::SymbolKind::Struct)
        .map(|s| s.name.as_str())
        .collect();

    assert!(struct_names.contains(&"Edge"), "Should find Edge struct");
}

/// Verify import extraction on a real file.
#[test]
fn extract_imports_from_real_file() {
    let mut parser = Parser::new();

    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let rust_extractor_path = Path::new(manifest_dir)
        .join("src")
        .join("lang")
        .join("rust.rs");

    let parsed = parser
        .parse_file(&rust_extractor_path)
        .expect("Should parse rust.rs");

    let imports = RustExtractor::extract_imports(&parsed.tree, &parsed.source)
        .expect("Should extract imports");

    // rust.rs has imports from std and crate
    assert!(!imports.is_empty(), "rust.rs should have import statements");

    // Check for specific imports
    let import_paths: Vec<_> = imports.iter().map(|i| i.path.as_str()).collect();
    assert!(
        import_paths.iter().any(|p| p.contains("OnceLock")),
        "Should import OnceLock"
    );
}

/// Parse all Rust files in src/ directory.
#[test]
fn parse_all_source_files() {
    let mut parser = Parser::new();

    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let src_dir = Path::new(manifest_dir).join("src");

    let mut file_count = 0;
    let mut total_symbols = 0;

    fn visit_dir(
        dir: &Path,
        parser: &mut Parser,
        file_count: &mut usize,
        total_symbols: &mut usize,
    ) {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    visit_dir(&path, parser, file_count, total_symbols);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    let parsed = parser.parse_file(&path).expect("Should parse Rust file");
                    let file_name = path.file_name().unwrap().to_string_lossy();
                    let symbols =
                        RustExtractor::extract_symbols(&parsed.tree, &parsed.source, &file_name)
                            .expect("Should extract symbols");

                    *file_count += 1;
                    *total_symbols += symbols.len();
                }
            }
        }
    }

    visit_dir(&src_dir, &mut parser, &mut file_count, &mut total_symbols);

    // We should have parsed multiple files
    assert!(file_count >= 4, "Should parse at least 4 Rust files");

    // We should have extracted many symbols
    assert!(
        total_symbols >= 10,
        "Should extract at least 10 symbols total"
    );

    eprintln!(
        "Parsed {} files, extracted {} symbols",
        file_count, total_symbols
    );
}
