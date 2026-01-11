//! Tree-sitter based multi-language parser.
//!
//! This module provides the parsing infrastructure for extracting symbols
//! from source code. It supports multiple languages through tree-sitter
//! grammars and uses lazy initialization for efficient resource usage.

use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

use thiserror::Error;

/// Errors that can occur during parsing.
#[derive(Debug, Error)]
pub enum ParseError {
    #[error("Unsupported file extension: {0}")]
    UnsupportedExtension(String),

    #[error("Failed to read file: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Failed to parse source code")]
    ParseFailed,

    #[error("Failed to set parser language: {0}")]
    LanguageError(String),

    #[error("Failed to compile query: {0}")]
    QueryError(String),
}

/// Supported programming languages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    Rust,
    Python,
    C,
    Cpp,
}

impl Language {
    /// Detect language from file extension.
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_lowercase().as_str() {
            "rs" => Some(Language::Rust),
            "py" | "pyi" => Some(Language::Python),
            "c" | "h" => Some(Language::C),
            "cpp" | "cc" | "cxx" | "c++" | "hpp" | "hh" | "hxx" | "h++" => Some(Language::Cpp),
            _ => None,
        }
    }

    /// Detect language from file path.
    pub fn from_path(path: &Path) -> Option<Self> {
        path.extension()
            .and_then(|ext| ext.to_str())
            .and_then(Self::from_extension)
    }

    /// Get the tree-sitter language for this language.
    pub fn tree_sitter_language(&self) -> tree_sitter::Language {
        match self {
            Language::Rust => tree_sitter_rust::LANGUAGE.into(),
            Language::Python => tree_sitter_python::LANGUAGE.into(),
            Language::C => tree_sitter_c::LANGUAGE.into(),
            Language::Cpp => tree_sitter_cpp::LANGUAGE.into(),
        }
    }

    /// Get file extensions associated with this language.
    pub fn extensions(&self) -> &'static [&'static str] {
        match self {
            Language::Rust => &["rs"],
            Language::Python => &["py", "pyi"],
            Language::C => &["c", "h"],
            Language::Cpp => &["cpp", "cc", "cxx", "c++", "hpp", "hh", "hxx", "h++"],
        }
    }

    /// Get the display name for this language.
    pub fn name(&self) -> &'static str {
        match self {
            Language::Rust => "Rust",
            Language::Python => "Python",
            Language::C => "C",
            Language::Cpp => "C++",
        }
    }
}

/// Configuration for a language including tree-sitter queries.
pub struct LanguageConfig {
    /// The tree-sitter language.
    pub language: tree_sitter::Language,
    /// Query for extracting symbols (structs, functions, etc.).
    pub symbols_query: tree_sitter::Query,
}

impl LanguageConfig {
    /// Create a new language configuration.
    pub fn new(lang: Language) -> Result<Self, ParseError> {
        let ts_lang = lang.tree_sitter_language();
        let query_source = Self::get_symbols_query(lang);

        let symbols_query = tree_sitter::Query::new(&ts_lang, query_source)
            .map_err(|e| ParseError::QueryError(e.to_string()))?;

        Ok(Self {
            language: ts_lang,
            symbols_query,
        })
    }

    /// Get the symbols query source for a language.
    fn get_symbols_query(lang: Language) -> &'static str {
        match lang {
            Language::Rust => include_str!("queries/rust_symbols.scm"),
            Language::Python => include_str!("queries/python_symbols.scm"),
            Language::C => include_str!("queries/c_symbols.scm"),
            Language::Cpp => include_str!("queries/cpp_symbols.scm"),
        }
    }
}

/// Lazily initialized language configuration.
///
/// Uses `OnceLock` to defer query compilation until first use,
/// avoiding startup overhead for unused languages.
pub struct LazyLanguageConfig {
    language: Language,
    config: OnceLock<Result<LanguageConfig, String>>,
}

impl LazyLanguageConfig {
    /// Create a new lazy config for the given language.
    pub fn new(language: Language) -> Self {
        Self {
            language,
            config: OnceLock::new(),
        }
    }

    /// Get the configuration, initializing if needed.
    pub fn get(&self) -> Result<&LanguageConfig, ParseError> {
        self.config
            .get_or_init(|| LanguageConfig::new(self.language).map_err(|e| e.to_string()))
            .as_ref()
            .map_err(|e| ParseError::QueryError(e.clone()))
    }
}

/// A parsed source file with its AST.
pub struct ParsedFile {
    /// The language of the source file.
    pub language: Language,
    /// The tree-sitter syntax tree.
    pub tree: tree_sitter::Tree,
    /// The source code (owned for lifetime management).
    pub source: String,
    /// The file path (if parsed from a file).
    pub path: Option<String>,
}

impl ParsedFile {
    /// Get the root node of the syntax tree.
    pub fn root_node(&self) -> tree_sitter::Node<'_> {
        self.tree.root_node()
    }

    /// Get the source code as bytes.
    pub fn source_bytes(&self) -> &[u8] {
        self.source.as_bytes()
    }

    /// Get text for a node.
    pub fn node_text(&self, node: tree_sitter::Node) -> &str {
        node.utf8_text(self.source_bytes()).unwrap_or("")
    }
}

/// Multi-language source code parser.
///
/// Manages tree-sitter parsers and language configurations for
/// parsing source files across supported languages.
pub struct Parser {
    /// The tree-sitter parser instance.
    ts_parser: tree_sitter::Parser,
    /// Language configurations (lazily initialized).
    configs: HashMap<Language, LazyLanguageConfig>,
}

impl Parser {
    /// Create a new parser with all supported languages.
    pub fn new() -> Self {
        let mut configs = HashMap::new();
        configs.insert(Language::Rust, LazyLanguageConfig::new(Language::Rust));
        configs.insert(Language::Python, LazyLanguageConfig::new(Language::Python));
        configs.insert(Language::C, LazyLanguageConfig::new(Language::C));
        configs.insert(Language::Cpp, LazyLanguageConfig::new(Language::Cpp));

        Self {
            ts_parser: tree_sitter::Parser::new(),
            configs,
        }
    }

    /// Parse a file from the filesystem.
    pub fn parse_file(&mut self, path: &Path) -> Result<ParsedFile, ParseError> {
        let language = Language::from_path(path).ok_or_else(|| {
            ParseError::UnsupportedExtension(
                path.extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("none")
                    .to_string(),
            )
        })?;

        let source = std::fs::read_to_string(path)?;
        let mut parsed = self.parse_source(&source, language)?;
        parsed.path = Some(path.to_string_lossy().to_string());
        Ok(parsed)
    }

    /// Parse source code string with a specified language.
    pub fn parse_source(
        &mut self,
        source: &str,
        language: Language,
    ) -> Result<ParsedFile, ParseError> {
        // Get the language config (this compiles queries on first use)
        let config = self
            .configs
            .get(&language)
            .ok_or_else(|| ParseError::UnsupportedExtension(language.name().to_string()))?
            .get()?;

        // Set the parser language
        self.ts_parser
            .set_language(&config.language)
            .map_err(|e| ParseError::LanguageError(e.to_string()))?;

        // Parse the source
        let tree = self
            .ts_parser
            .parse(source, None)
            .ok_or(ParseError::ParseFailed)?;

        Ok(ParsedFile {
            language,
            tree,
            source: source.to_string(),
            path: None,
        })
    }

    /// Get the language configuration for a language.
    pub fn get_config(&self, language: Language) -> Result<&LanguageConfig, ParseError> {
        self.configs
            .get(&language)
            .ok_or_else(|| ParseError::UnsupportedExtension(language.name().to_string()))?
            .get()
    }

    /// Check if a file extension is supported.
    pub fn supports_extension(ext: &str) -> bool {
        Language::from_extension(ext).is_some()
    }

    /// Get all supported extensions.
    pub fn supported_extensions() -> Vec<&'static str> {
        let mut exts = Vec::new();
        for lang in [Language::Rust, Language::Python, Language::C, Language::Cpp] {
            exts.extend_from_slice(lang.extensions());
        }
        exts
    }
}

impl Default for Parser {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_language_from_extension() {
        assert_eq!(Language::from_extension("rs"), Some(Language::Rust));
        assert_eq!(Language::from_extension("py"), Some(Language::Python));
        assert_eq!(Language::from_extension("c"), Some(Language::C));
        assert_eq!(Language::from_extension("cpp"), Some(Language::Cpp));
        assert_eq!(Language::from_extension("hpp"), Some(Language::Cpp));
        assert_eq!(Language::from_extension("unknown"), None);
    }

    #[test]
    fn test_language_from_path() {
        assert_eq!(
            Language::from_path(Path::new("src/main.rs")),
            Some(Language::Rust)
        );
        assert_eq!(
            Language::from_path(Path::new("script.py")),
            Some(Language::Python)
        );
        assert_eq!(Language::from_path(Path::new("noext")), None);
    }

    #[test]
    fn test_parser_parse_rust_source() {
        let mut parser = Parser::new();
        let source = r#"
            pub fn hello() {
                println!("Hello, world!");
            }
        "#;

        let parsed = parser.parse_source(source, Language::Rust).unwrap();
        assert_eq!(parsed.language, Language::Rust);
        assert!(!parsed.tree.root_node().has_error());
    }

    #[test]
    fn test_parser_parse_python_source() {
        let mut parser = Parser::new();
        let source = r#"
def hello():
    print("Hello, world!")

class Greeter:
    def greet(self, name):
        return f"Hello, {name}!"
        "#;

        let parsed = parser.parse_source(source, Language::Python).unwrap();
        assert_eq!(parsed.language, Language::Python);
        assert!(!parsed.tree.root_node().has_error());
    }

    #[test]
    fn test_parser_parse_c_source() {
        let mut parser = Parser::new();
        let source = r#"
            #include <stdio.h>

            void hello() {
                printf("Hello, world!\n");
            }
        "#;

        let parsed = parser.parse_source(source, Language::C).unwrap();
        assert_eq!(parsed.language, Language::C);
        assert!(!parsed.tree.root_node().has_error());
    }

    #[test]
    fn test_parsed_file_node_text() {
        let mut parser = Parser::new();
        let source = "fn main() {}";
        let parsed = parser.parse_source(source, Language::Rust).unwrap();

        let root = parsed.root_node();
        assert_eq!(parsed.node_text(root), source);
    }

    #[test]
    fn test_supported_extensions() {
        let exts = Parser::supported_extensions();
        assert!(exts.contains(&"rs"));
        assert!(exts.contains(&"py"));
        assert!(exts.contains(&"c"));
        assert!(exts.contains(&"cpp"));
    }

    #[test]
    fn test_supports_extension() {
        assert!(Parser::supports_extension("rs"));
        assert!(Parser::supports_extension("py"));
        assert!(!Parser::supports_extension("java"));
    }
}
