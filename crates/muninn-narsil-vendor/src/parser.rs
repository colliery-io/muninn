use anyhow::{anyhow, Result};
use std::path::Path;
use std::sync::{Arc, OnceLock};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Parser, Query, QueryCursor, Tree};

use crate::symbols::{Symbol, SymbolKind};

/// Supported languages and their tree-sitter configurations
#[derive(Debug, Clone)]
pub struct LanguageConfig {
    pub name: String,
    pub language: Language,
    pub extensions: Vec<&'static str>,
    pub symbol_query: &'static str,
}

/// Language configuration with lazily-compiled query
/// Query is compiled on first use, warnings logged once, then cached
struct LazyLanguageConfig {
    config: LanguageConfig,
    /// Lazily compiled query (None if compilation failed)
    compiled_query: OnceLock<Option<Arc<Query>>>,
}

impl LazyLanguageConfig {
    fn new(config: LanguageConfig) -> Self {
        Self {
            config,
            compiled_query: OnceLock::new(),
        }
    }

    /// Get the compiled query, compiling on first access
    fn get_query(&self) -> Option<&Arc<Query>> {
        self.compiled_query
            .get_or_init(
                || match Query::new(&self.config.language, self.config.symbol_query) {
                    Ok(q) => Some(Arc::new(q)),
                    Err(e) => {
                        tracing::warn!(
                            "Query compilation failed for {} (this warning appears once): {:?}",
                            self.config.name,
                            e
                        );
                        None
                    }
                },
            )
            .as_ref()
    }
}

/// A parsed file with extracted information
#[derive(Debug, Clone)]
pub struct ParsedFile {
    /// Path of the parsed file (stored for reference, may be used by consumers)
    pub path: String,
    /// Language identifier for the file
    pub language: String,
    /// Symbols extracted from the file
    pub symbols: Vec<Symbol>,
    /// The tree-sitter parse tree (used for AST-aware chunking)
    pub tree: Option<Tree>,
}

/// Multi-language parser using tree-sitter
pub struct LanguageParser {
    configs: Vec<LazyLanguageConfig>,
}

impl LanguageParser {
    pub fn new() -> Result<Self> {
        let configs = vec![
            // Rust
            LanguageConfig {
                name: "rust".to_string(),
                language: tree_sitter_rust::LANGUAGE.into(),
                extensions: vec!["rs"],
                symbol_query: r#"
                    (function_item name: (identifier) @function.name) @function.def
                    (struct_item name: (type_identifier) @struct.name) @struct.def
                    (enum_item name: (type_identifier) @enum.name) @enum.def
                    (trait_item name: (type_identifier) @trait.name) @trait.def
                    (impl_item type: (type_identifier) @impl.name) @impl.def
                    (type_item name: (type_identifier) @type.name) @type.def
                    (const_item name: (identifier) @const.name) @const.def
                    (static_item name: (identifier) @static.name) @static.def
                    (mod_item name: (identifier) @mod.name) @mod.def
                "#,
            },
            // Python
            LanguageConfig {
                name: "python".to_string(),
                language: tree_sitter_python::LANGUAGE.into(),
                extensions: vec!["py", "pyi"],
                symbol_query: r#"
                    (function_definition name: (identifier) @function.name) @function.def
                    (class_definition name: (identifier) @class.name) @class.def
                "#,
            },
            // JavaScript
            LanguageConfig {
                name: "javascript".to_string(),
                language: tree_sitter_javascript::LANGUAGE.into(),
                extensions: vec!["js", "jsx", "mjs"],
                symbol_query: r#"
                    (function_declaration name: (identifier) @function.name) @function.def
                    (class_declaration name: (identifier) @class.name) @class.def
                    (method_definition name: (property_identifier) @method.name) @method.def
                    (arrow_function) @arrow.def
                    (variable_declarator name: (identifier) @var.name) @var.def
                "#,
            },
            // TypeScript
            LanguageConfig {
                name: "typescript".to_string(),
                language: tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
                extensions: vec!["ts"],
                symbol_query: r#"
                    (function_declaration name: (identifier) @function.name) @function.def
                    (class_declaration name: (type_identifier) @class.name) @class.def
                    (method_definition name: (property_identifier) @method.name) @method.def
                    (interface_declaration name: (type_identifier) @interface.name) @interface.def
                    (type_alias_declaration name: (type_identifier) @type.name) @type.def
                    (enum_declaration name: (identifier) @enum.name) @enum.def
                "#,
            },
            // TSX
            LanguageConfig {
                name: "tsx".to_string(),
                language: tree_sitter_typescript::LANGUAGE_TSX.into(),
                extensions: vec!["tsx"],
                symbol_query: r#"
                    (function_declaration name: (identifier) @function.name) @function.def
                    (class_declaration name: (type_identifier) @class.name) @class.def
                    (method_definition name: (property_identifier) @method.name) @method.def
                    (interface_declaration name: (type_identifier) @interface.name) @interface.def
                    (type_alias_declaration name: (type_identifier) @type.name) @type.def
                "#,
            },
            // Go
            LanguageConfig {
                name: "go".to_string(),
                language: tree_sitter_go::LANGUAGE.into(),
                extensions: vec!["go"],
                symbol_query: r#"
                    (function_declaration name: (identifier) @function.name) @function.def
                    (method_declaration name: (field_identifier) @method.name) @method.def
                    (type_declaration (type_spec name: (type_identifier) @type.name)) @type.def
                "#,
            },
            // C
            LanguageConfig {
                name: "c".to_string(),
                language: tree_sitter_c::LANGUAGE.into(),
                extensions: vec!["c", "h"],
                symbol_query: r#"
                    (function_definition declarator: (function_declarator declarator: (identifier) @function.name)) @function.def
                    (struct_specifier name: (type_identifier) @struct.name) @struct.def
                    (enum_specifier name: (type_identifier) @enum.name) @enum.def
                    (type_definition declarator: (type_identifier) @type.name) @type.def
                "#,
            },
            // C++
            LanguageConfig {
                name: "cpp".to_string(),
                language: tree_sitter_cpp::LANGUAGE.into(),
                extensions: vec!["cpp", "cc", "cxx", "hpp", "hxx", "hh"],
                symbol_query: r#"
                    (function_definition declarator: (function_declarator declarator: (identifier) @function.name)) @function.def
                    (class_specifier name: (type_identifier) @class.name) @class.def
                    (struct_specifier name: (type_identifier) @struct.name) @struct.def
                    (enum_specifier name: (type_identifier) @enum.name) @enum.def
                    (namespace_definition name: (namespace_identifier) @namespace.name) @namespace.def
                "#,
            },
            // Java
            LanguageConfig {
                name: "java".to_string(),
                language: tree_sitter_java::LANGUAGE.into(),
                extensions: vec!["java"],
                symbol_query: r#"
                    (method_declaration name: (identifier) @method.name) @method.def
                    (class_declaration name: (identifier) @class.name) @class.def
                    (interface_declaration name: (identifier) @interface.name) @interface.def
                    (enum_declaration name: (identifier) @enum.name) @enum.def
                "#,
            },
        ];

        // Wrap configs in lazy wrappers (queries compiled on first use, not during init)
        let lazy_configs = configs.into_iter().map(LazyLanguageConfig::new).collect();

        Ok(Self {
            configs: lazy_configs,
        })
    }

    /// Get language config for a file extension
    fn get_config(&self, path: &Path) -> Option<&LazyLanguageConfig> {
        let ext = path.extension()?.to_str()?;
        self.configs
            .iter()
            .find(|c| c.config.extensions.contains(&ext))
    }

    /// Parse a file and extract symbols
    pub fn parse_file(&self, path: &Path, content: &str) -> Result<ParsedFile> {
        let lazy_config = self
            .get_config(path)
            .ok_or_else(|| anyhow!("Unsupported file type: {:?}", path))?;

        let mut parser = Parser::new();
        parser.set_language(&lazy_config.config.language)?;

        let tree = parser
            .parse(content, None)
            .ok_or_else(|| anyhow!("Failed to parse file"))?;

        let symbols = self.extract_symbols(&tree, content, lazy_config)?;

        Ok(ParsedFile {
            path: path.to_string_lossy().to_string(),
            language: lazy_config.config.name.clone(),
            symbols,
            tree: Some(tree),
        })
    }

    /// Parse a file and return just the tree (for call graph analysis)
    pub fn parse_to_tree(&self, path: &Path, content: &str) -> Result<Tree> {
        let lazy_config = self
            .get_config(path)
            .ok_or_else(|| anyhow!("Unsupported file type: {:?}", path))?;

        let mut parser = Parser::new();
        parser.set_language(&lazy_config.config.language)?;

        parser
            .parse(content, None)
            .ok_or_else(|| anyhow!("Failed to parse file"))
    }

    /// Extract symbols using tree-sitter queries
    fn extract_symbols(
        &self,
        tree: &Tree,
        source: &str,
        lazy_config: &LazyLanguageConfig,
    ) -> Result<Vec<Symbol>> {
        let mut symbols = Vec::new();
        let source_bytes = source.as_bytes();

        // Get lazily-compiled query (errors logged once on first access)
        let query = match lazy_config.get_query() {
            Some(q) => q,
            None => return Ok(symbols), // Query compilation failed, return empty
        };

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(query, tree.root_node(), source_bytes);

        while let Some(match_) = matches.next() {
            let mut name: Option<String> = None;
            let mut kind: Option<SymbolKind> = None;
            let mut start_line = 0;
            let mut end_line = 0;
            let mut signature: Option<String> = None;

            for capture in match_.captures {
                let capture_name = query.capture_names()[capture.index as usize];
                let node = capture.node;
                let text = node.utf8_text(source_bytes).unwrap_or("");

                if capture_name.ends_with(".name") {
                    name = Some(text.to_string());
                    kind = Some(parse_symbol_kind(capture_name));
                } else if capture_name.ends_with(".def") {
                    start_line = node.start_position().row + 1;
                    end_line = node.end_position().row + 1;

                    // Extract first line as signature (safe byte boundary).
                    // Manual char-boundary search (std's `floor_char_boundary`
                    // is 1.91-stable; our MSRV is 1.85).
                    let first_line_end = text.find('\n').unwrap_or(text.len());
                    let mut sig_end = first_line_end.min(200);
                    while sig_end > 0 && !text.is_char_boundary(sig_end) {
                        sig_end -= 1;
                    }
                    signature = Some(text[..sig_end].to_string());
                }
            }

            if let (Some(name), Some(kind)) = (name, kind) {
                symbols.push(Symbol {
                    name,
                    kind,
                    file_path: String::new(), // Will be set by caller
                    start_line,
                    end_line,
                    signature,
                    qualified_name: None,
                    doc_comment: None,
                });
            }
        }

        Ok(symbols)
    }
}

fn parse_symbol_kind(capture_name: &str) -> SymbolKind {
    let prefix = capture_name.split('.').next().unwrap_or("");
    match prefix {
        "function" => SymbolKind::Function,
        "method" => SymbolKind::Method,
        "class" => SymbolKind::Class,
        "struct" => SymbolKind::Struct,
        "enum" => SymbolKind::Enum,
        "interface" => SymbolKind::Interface,
        "trait" => SymbolKind::Trait,
        "type" => SymbolKind::TypeAlias,
        "const" | "static" => SymbolKind::Constant,
        "mod" | "module" | "namespace" => SymbolKind::Module,
        "impl" => SymbolKind::Implementation,
        "var" | "arrow" => SymbolKind::Variable,
        _ => SymbolKind::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_rust() {
        let parser = LanguageParser::new().unwrap();
        let content = r#"
            pub struct MyStruct {
                field: u32,
            }

            pub fn my_function() -> i32 {
                42
            }

            impl MyStruct {
                pub fn method(&self) {}
            }
        "#;

        let parsed = parser.parse_file(Path::new("test.rs"), content).unwrap();
        assert_eq!(parsed.language, "rust");
        assert!(!parsed.symbols.is_empty());

        let names: Vec<_> = parsed.symbols.iter().map(|s| &s.name).collect();
        assert!(names.contains(&&"MyStruct".to_string()));
        assert!(names.contains(&&"my_function".to_string()));
    }

    #[test]
    fn test_parse_python() {
        let parser = LanguageParser::new().unwrap();
        let content = r#"
class MyClass:
    def __init__(self):
        pass

    def method(self):
        return 42

def standalone_function():
    pass
        "#;

        let parsed = parser.parse_file(Path::new("test.py"), content).unwrap();
        assert_eq!(parsed.language, "python");

        let names: Vec<_> = parsed.symbols.iter().map(|s| &s.name).collect();
        assert!(names.contains(&&"MyClass".to_string()));
        assert!(names.contains(&&"standalone_function".to_string()));
    }

    /// Issue #18a regression coverage. Java was registered in the parser but
    /// had no unit tests, so a regression in the symbol query (or in the
    /// tree-sitter-java grammar version) could land silently. Asserts that
    /// every node-kind in our query (method, class, interface, enum)
    /// produces a symbol on a representative Maven-shaped class file.
    #[test]
    fn test_parse_java() {
        let parser = LanguageParser::new().unwrap();
        let content = r#"
package com.example.demo;

import java.util.List;

public class Greeter {
    public String hello(String name) {
        return "Hi " + name;
    }
    private int counter = 0;
}

interface Greetable {
    String hello(String name);
}

enum Color {
    RED, GREEN, BLUE;
}
"#;
        let parsed = parser
            .parse_file(
                Path::new("src/main/java/com/example/demo/Greeter.java"),
                content,
            )
            .expect("Java should parse");
        assert_eq!(parsed.language, "java");

        let names: Vec<_> = parsed.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(
            names.contains(&"Greeter"),
            "missing class Greeter; got {names:?}"
        );
        assert!(
            names.contains(&"hello"),
            "missing method hello; got {names:?}"
        );
        assert!(
            names.contains(&"Greetable"),
            "missing interface Greetable; got {names:?}"
        );
        assert!(
            names.contains(&"Color"),
            "missing enum Color; got {names:?}"
        );
    }

    #[test]
    fn test_parse_cpp() {
        let parser = LanguageParser::new().unwrap();
        let content = r#"
namespace MyNamespace {
    class MyClass {
    public:
        void myMethod() {}
    };

    struct MyStruct {
        int x;
        int y;
    };

    enum MyEnum {
        VALUE_A,
        VALUE_B
    };
}

void standaloneFunction() {
    // do something
}
        "#;

        let parsed = parser.parse_file(Path::new("test.cpp"), content).unwrap();
        assert_eq!(parsed.language, "cpp");
        assert!(!parsed.symbols.is_empty());

        let names: Vec<_> = parsed.symbols.iter().map(|s| &s.name).collect();
        assert!(
            names.contains(&&"MyNamespace".to_string()),
            "Should find namespace, found: {:?}",
            names
        );
        assert!(names.contains(&&"MyClass".to_string()), "Should find class");
        assert!(
            names.contains(&&"MyStruct".to_string()),
            "Should find struct"
        );
        assert!(names.contains(&&"MyEnum".to_string()), "Should find enum");
        assert!(
            names.contains(&&"standaloneFunction".to_string()),
            "Should find function"
        );
    }

    #[test]
    fn test_signature_truncation_at_multibyte_utf8_boundary() {
        let parser = LanguageParser::new().unwrap();
        // Build a JS function whose name + params span past byte 200 using Cyrillic chars.
        // Each Cyrillic char is 2 bytes in UTF-8, so 100 Cyrillic chars = 200 bytes.
        // Place "function " (9 bytes) then 96 Cyrillic chars (192 bytes) = 201 bytes total,
        // which puts byte 200 right in the middle of the last Cyrillic char.
        let cyrillic_name: String = "Б".repeat(96);
        let content = format!("function {}() {{\n  return 1;\n}}\n", cyrillic_name);

        // This must not panic — the truncation must land on a valid char boundary
        let parsed = parser
            .parse_file(Path::new("test_utf8.js"), &content)
            .unwrap();
        assert_eq!(parsed.language, "javascript");
        assert!(
            !parsed.symbols.is_empty(),
            "Should find the function symbol"
        );

        // Verify the signature was truncated safely (no panic, valid UTF-8)
        let sym = parsed.symbols.iter().find(|s| s.name == cyrillic_name);
        assert!(sym.is_some(), "Should find the Cyrillic-named function");
        if let Some(sig) = &sym.unwrap().signature {
            assert!(
                sig.len() <= 200,
                "Signature should be truncated to <= 200 bytes"
            );
            // Verify it's valid UTF-8 (it is, since it's a String, but let's be explicit)
            assert!(std::str::from_utf8(sig.as_bytes()).is_ok());
        }
    }
}
