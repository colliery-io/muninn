//! Python documentation extractor using tree-sitter.
//!
//! Extracts documentation from Python packages using tree-sitter for parsing,
//! with no external Python dependencies. Extracts:
//! - Module, class, function, and method docstrings
//! - Function/method signatures with type annotations
//!
//! # Example
//!
//! ```no_run
//! use muninn_graph::registry::pydoc::{PyDocExtractor, items_to_chunks};
//!
//! let mut extractor = PyDocExtractor::new();
//!
//! // Extract docs from a package path
//! let items = extractor.extract_from_path("/path/to/package")?;
//!
//! // Convert to DocChunkInput for storage
//! let chunks = items_to_chunks(items);
//! # Ok::<(), muninn_graph::registry::pydoc::PyDocError>(())
//! ```

use std::path::{Path, PathBuf};

use crate::doc_store::{DocChunkInput, ItemType};
use crate::parser::Parser;

/// Error type for Python doc extraction.
#[derive(Debug, thiserror::Error)]
pub enum PyDocError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Parse error: {0}")]
    Parse(String),

    #[error("Package not found: {0}")]
    PackageNotFound(String),
}

pub type Result<T> = std::result::Result<T, PyDocError>;

// Re-export old error name for compatibility
pub use PyDocError as GriffeError;

/// An extracted documentation item from Python code.
#[derive(Debug, Clone)]
pub struct ExtractedPyItem {
    /// Fully qualified path (e.g., "requests.api.get")
    pub path: String,
    /// Item type
    pub item_type: ItemType,
    /// Documentation text (may be empty)
    pub docstring: String,
    /// Function/method signature
    pub signature: Option<String>,
    /// Parent class (for methods)
    pub parent_class: Option<String>,
}

impl ExtractedPyItem {
    /// Convert to DocChunkInput for storage.
    pub fn to_chunk(&self) -> DocChunkInput {
        DocChunkInput {
            item_path: self.path.clone(),
            item_type: self.item_type,
            doc_text: self.docstring.clone(),
            signature: self.signature.clone(),
            embedding: None,
        }
    }
}

/// Python documentation extractor using tree-sitter.
///
/// This extractor uses tree-sitter to parse Python source files directly,
/// requiring no external Python runtime or dependencies.
pub struct PyDocExtractor {
    parser: Parser,
}

// Re-export old name for compatibility
pub use PyDocExtractor as GriffeExtractor;

impl PyDocExtractor {
    /// Create a new extractor.
    pub fn new() -> Self {
        Self {
            parser: Parser::new(),
        }
    }

    /// Compatibility method - does nothing since we don't use griffe.
    #[deprecated(note = "No longer needed - tree-sitter is always available")]
    pub fn check_griffe(&self) -> Result<bool> {
        Ok(true)
    }

    /// Compatibility constructor - python argument is ignored.
    #[deprecated(note = "Python executable is no longer needed")]
    pub fn with_python(_python: impl Into<String>) -> Self {
        Self::new()
    }

    /// Compatibility method - flags are ignored.
    #[deprecated(note = "Flags are no longer needed")]
    pub fn with_flags(self, _flags: Vec<String>) -> Self {
        self
    }

    /// Extract documentation from a package at the given path.
    ///
    /// The path should point to a directory containing Python source files.
    pub fn extract_from_path(&mut self, package_path: impl AsRef<Path>) -> Result<Vec<ExtractedPyItem>> {
        let package_path = package_path.as_ref();

        if !package_path.exists() {
            return Err(PyDocError::PackageNotFound(
                package_path.display().to_string(),
            ));
        }

        // Find the package root and name
        let (package_root, package_name) = self.find_package_root(package_path)?;

        let mut items = Vec::new();

        // Walk all Python files in the package
        self.walk_python_files(&package_root, &package_name, &mut items)?;

        Ok(items)
    }

    /// Extract documentation from a single Python file.
    pub fn extract_from_file(
        &mut self,
        file_path: impl AsRef<Path>,
        module_path: &str,
    ) -> Result<Vec<ExtractedPyItem>> {
        let file_path = file_path.as_ref();
        let source = std::fs::read_to_string(file_path)?;

        self.extract_from_source(&source, module_path)
    }

    /// Extract documentation from Python source code.
    pub fn extract_from_source(
        &mut self,
        source: &str,
        module_path: &str,
    ) -> Result<Vec<ExtractedPyItem>> {
        let parsed = self
            .parser
            .parse_source(source, crate::parser::Language::Python)
            .map_err(|e| PyDocError::Parse(e.to_string()))?;

        let mut items = Vec::new();
        let tree = &parsed.tree;
        let root = tree.root_node();

        // Extract module docstring
        if let Some(docstring) = self.extract_module_docstring(&root, source) {
            items.push(ExtractedPyItem {
                path: module_path.to_string(),
                item_type: ItemType::Module,
                docstring,
                signature: None,
                parent_class: None,
            });
        }

        // Walk the AST to extract classes and functions
        self.extract_from_node(&root, source, module_path, None, &mut items);

        Ok(items)
    }

    /// Find the package root directory and name.
    fn find_package_root(&self, path: &Path) -> Result<(PathBuf, String)> {
        // If path contains __init__.py, it's the package root
        if path.join("__init__.py").exists() {
            let name = path
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "package".to_string());
            return Ok((path.to_path_buf(), name));
        }

        // Look for subdirectory with __init__.py
        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.flatten() {
                let entry_path = entry.path();
                if entry_path.is_dir() && entry_path.join("__init__.py").exists() {
                    let name = entry_path
                        .file_name()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "package".to_string());
                    return Ok((entry_path, name));
                }
            }
        }

        // Look for src/ directory pattern (common in modern Python projects)
        let src_path = path.join("src");
        if src_path.exists() {
            if let Ok(entries) = std::fs::read_dir(&src_path) {
                for entry in entries.flatten() {
                    let entry_path = entry.path();
                    if entry_path.is_dir() && entry_path.join("__init__.py").exists() {
                        let name = entry_path
                            .file_name()
                            .map(|s| s.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "package".to_string());
                        return Ok((entry_path, name));
                    }
                }
            }
        }

        // Fallback: treat the directory as a namespace package (no __init__.py required)
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "package".to_string());
        Ok((path.to_path_buf(), name))
    }

    /// Walk Python files in a directory recursively.
    fn walk_python_files(
        &mut self,
        dir: &Path,
        module_prefix: &str,
        items: &mut Vec<ExtractedPyItem>,
    ) -> Result<()> {
        let entries = std::fs::read_dir(dir)?;

        for entry in entries.flatten() {
            let path = entry.path();
            let file_name = path
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();

            // Skip hidden files and __pycache__
            if file_name.starts_with('.') || file_name == "__pycache__" {
                continue;
            }

            if path.is_dir() {
                // Check if it's a subpackage (has __init__.py)
                if path.join("__init__.py").exists() {
                    let submodule = format!("{}.{}", module_prefix, file_name);
                    self.walk_python_files(&path, &submodule, items)?;
                }
            } else if path.extension().is_some_and(|ext| ext == "py") {
                // Build module path from file name
                let module_name = if file_name == "__init__.py" {
                    module_prefix.to_string()
                } else {
                    let stem = file_name.trim_end_matches(".py");
                    format!("{}.{}", module_prefix, stem)
                };

                // Extract documentation from this file
                if let Ok(file_items) = self.extract_from_file(&path, &module_name) {
                    items.extend(file_items);
                }
            }
        }

        Ok(())
    }

    /// Extract module-level docstring (first string literal at top level).
    fn extract_module_docstring(&self, root: &tree_sitter::Node, source: &str) -> Option<String> {
        let mut cursor = root.walk();
        cursor.goto_first_child();

        loop {
            let node = cursor.node();

            // Skip comments and whitespace
            if node.kind() == "comment" {
                if !cursor.goto_next_sibling() {
                    break;
                }
                continue;
            }

            // Look for expression_statement containing a string
            if node.kind() == "expression_statement" {
                if let Some(child) = node.child(0) {
                    if child.kind() == "string" {
                        return Some(self.extract_string_content(&child, source));
                    }
                }
            }

            // If we hit any other statement type, no module docstring
            break;
        }

        None
    }

    /// Extract items from an AST node recursively.
    fn extract_from_node(
        &self,
        node: &tree_sitter::Node,
        source: &str,
        module_path: &str,
        parent_class: Option<&str>,
        items: &mut Vec<ExtractedPyItem>,
    ) {
        let mut cursor = node.walk();
        cursor.goto_first_child();

        loop {
            let child = cursor.node();

            match child.kind() {
                "class_definition" => {
                    self.extract_class(&child, source, module_path, items);
                }
                "function_definition" => {
                    self.extract_function(&child, source, module_path, parent_class, items);
                }
                "decorated_definition" => {
                    // Handle decorated classes and functions
                    if let Some(definition) = child.child_by_field_name("definition") {
                        match definition.kind() {
                            "class_definition" => {
                                self.extract_class(&definition, source, module_path, items);
                            }
                            "function_definition" => {
                                self.extract_function(
                                    &definition,
                                    source,
                                    module_path,
                                    parent_class,
                                    items,
                                );
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }

            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }

    /// Extract a class and its methods.
    fn extract_class(
        &self,
        node: &tree_sitter::Node,
        source: &str,
        module_path: &str,
        items: &mut Vec<ExtractedPyItem>,
    ) {
        let name = match node.child_by_field_name("name") {
            Some(n) => self.node_text(&n, source),
            None => return,
        };

        let class_path = format!("{}.{}", module_path, name);

        // Extract class docstring
        let docstring = node
            .child_by_field_name("body")
            .and_then(|body| self.extract_block_docstring(&body, source))
            .unwrap_or_default();

        // Only add if has docstring
        if !docstring.is_empty() {
            items.push(ExtractedPyItem {
                path: class_path.clone(),
                item_type: ItemType::Class,
                docstring,
                signature: None,
                parent_class: None,
            });
        }

        // Extract methods from class body
        if let Some(body) = node.child_by_field_name("body") {
            self.extract_from_node(&body, source, &class_path, Some(&name), items);
        }
    }

    /// Extract a function or method.
    fn extract_function(
        &self,
        node: &tree_sitter::Node,
        source: &str,
        parent_path: &str,
        parent_class: Option<&str>,
        items: &mut Vec<ExtractedPyItem>,
    ) {
        let name = match node.child_by_field_name("name") {
            Some(n) => self.node_text(&n, source),
            None => return,
        };

        let func_path = format!("{}.{}", parent_path, name);

        // Build signature
        let signature = self.build_signature(node, source, &name);

        // Extract docstring
        let docstring = node
            .child_by_field_name("body")
            .and_then(|body| self.extract_block_docstring(&body, source))
            .unwrap_or_default();

        // Determine if it's a method or function
        let item_type = if parent_class.is_some() {
            ItemType::Method
        } else {
            ItemType::Function
        };

        // Only add if has docstring or signature
        if !docstring.is_empty() || signature.is_some() {
            items.push(ExtractedPyItem {
                path: func_path,
                item_type,
                docstring,
                signature,
                parent_class: parent_class.map(|s| s.to_string()),
            });
        }
    }

    /// Build function signature from AST.
    fn build_signature(
        &self,
        node: &tree_sitter::Node,
        source: &str,
        name: &str,
    ) -> Option<String> {
        let params = node.child_by_field_name("parameters")?;
        let params_text = self.node_text(&params, source);

        let return_type = node
            .child_by_field_name("return_type")
            .map(|n| self.node_text(&n, source));

        let sig = match return_type {
            Some(ret) => format!("def {}{} -> {}", name, params_text, ret),
            None => format!("def {}{}", name, params_text),
        };

        Some(sig)
    }

    /// Extract docstring from a block (first string literal).
    fn extract_block_docstring(&self, block: &tree_sitter::Node, source: &str) -> Option<String> {
        let mut cursor = block.walk();
        cursor.goto_first_child();

        loop {
            let node = cursor.node();

            // Look for expression_statement containing a string
            if node.kind() == "expression_statement" {
                if let Some(child) = node.child(0) {
                    if child.kind() == "string" {
                        let content = self.extract_string_content(&child, source);
                        if !content.is_empty() {
                            return Some(content);
                        }
                    }
                }
            }

            // Only check the first statement
            break;
        }

        None
    }

    /// Extract string content, removing quotes.
    fn extract_string_content(&self, node: &tree_sitter::Node, source: &str) -> String {
        let text = self.node_text(node, source);

        // Remove triple quotes first, then single quotes
        let trimmed = text
            .trim_start_matches("\"\"\"")
            .trim_end_matches("\"\"\"")
            .trim_start_matches("'''")
            .trim_end_matches("'''")
            .trim_start_matches('"')
            .trim_end_matches('"')
            .trim_start_matches('\'')
            .trim_end_matches('\'')
            .trim();

        trimmed.to_string()
    }

    /// Get text content of a node.
    fn node_text(&self, node: &tree_sitter::Node, source: &str) -> String {
        source[node.byte_range()].to_string()
    }
}

impl Default for PyDocExtractor {
    fn default() -> Self {
        Self::new()
    }
}

/// Convert extracted items to DocChunkInput for storage.
pub fn items_to_chunks(items: Vec<ExtractedPyItem>) -> Vec<DocChunkInput> {
    items
        .into_iter()
        .filter(|item| !item.docstring.is_empty())
        .map(|item| item.to_chunk())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extractor_creation() {
        let extractor = PyDocExtractor::new();
        drop(extractor);
    }

    #[test]
    fn test_extract_module_docstring() {
        let source = r#""""Module docstring.

This module does things.
"""

import os

def foo():
    pass
"#;

        let mut extractor = PyDocExtractor::new();
        let items = extractor.extract_from_source(source, "mymodule").unwrap();

        let module = items.iter().find(|i| i.item_type == ItemType::Module);
        assert!(module.is_some(), "Should find module docstring");
        let module = module.unwrap();
        assert!(module.docstring.contains("Module docstring"));
    }

    #[test]
    fn test_extract_function_docstring() {
        let source = r#"
def greet(name: str) -> str:
    """Return a greeting message.

    Args:
        name: The name to greet.

    Returns:
        A greeting string.
    """
    return f"Hello, {name}!"
"#;

        let mut extractor = PyDocExtractor::new();
        let items = extractor.extract_from_source(source, "mymodule").unwrap();

        let func = items.iter().find(|i| i.item_type == ItemType::Function);
        assert!(func.is_some(), "Should find function");
        let func = func.unwrap();
        assert_eq!(func.path, "mymodule.greet");
        assert!(func.docstring.contains("greeting message"));
        assert!(func.signature.as_ref().unwrap().contains("name: str"));
        assert!(func.signature.as_ref().unwrap().contains("-> str"));
    }

    #[test]
    fn test_extract_class_and_methods() {
        let source = r#"
class Calculator:
    """A simple calculator class."""

    def __init__(self, value: int = 0):
        """Initialize with a value."""
        self.value = value

    def add(self, n: int) -> int:
        """Add n to the value."""
        self.value += n
        return self.value
"#;

        let mut extractor = PyDocExtractor::new();
        let items = extractor.extract_from_source(source, "calc").unwrap();

        // Should have: class + 2 methods
        let class = items.iter().find(|i| i.item_type == ItemType::Class);
        assert!(class.is_some(), "Should find class");
        assert_eq!(class.unwrap().path, "calc.Calculator");
        assert!(class.unwrap().docstring.contains("simple calculator"));

        let init = items.iter().find(|i| i.path == "calc.Calculator.__init__");
        assert!(init.is_some(), "Should find __init__");
        assert_eq!(init.unwrap().item_type, ItemType::Method);
        assert_eq!(init.unwrap().parent_class, Some("Calculator".to_string()));

        let add = items.iter().find(|i| i.path == "calc.Calculator.add");
        assert!(add.is_some(), "Should find add method");
        assert!(add.unwrap().signature.as_ref().unwrap().contains("n: int"));
    }

    #[test]
    fn test_extract_decorated_function() {
        let source = r#"
@decorator
def decorated_func():
    """A decorated function."""
    pass

@classmethod
def class_method(cls):
    """A class method."""
    pass
"#;

        let mut extractor = PyDocExtractor::new();
        let items = extractor.extract_from_source(source, "mod").unwrap();

        assert!(
            items.iter().any(|i| i.path == "mod.decorated_func"),
            "Should find decorated function"
        );
        assert!(
            items.iter().any(|i| i.path == "mod.class_method"),
            "Should find class method"
        );
    }

    #[test]
    fn test_items_to_chunks_filters_empty() {
        let items = vec![
            ExtractedPyItem {
                path: "pkg.func1".to_string(),
                item_type: ItemType::Function,
                docstring: "Has docs".to_string(),
                signature: None,
                parent_class: None,
            },
            ExtractedPyItem {
                path: "pkg.func2".to_string(),
                item_type: ItemType::Function,
                docstring: "".to_string(), // Empty docstring
                signature: Some("def func2()".to_string()),
                parent_class: None,
            },
        ];

        let chunks = items_to_chunks(items);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].item_path, "pkg.func1");
    }

    #[test]
    fn test_single_quote_docstrings() {
        let source = r#"
def foo():
    'Single quote docstring.'
    pass

def bar():
    '''Triple single quote docstring.'''
    pass
"#;

        let mut extractor = PyDocExtractor::new();
        let items = extractor.extract_from_source(source, "mod").unwrap();

        let foo = items.iter().find(|i| i.path == "mod.foo");
        assert!(foo.is_some());
        assert!(foo.unwrap().docstring.contains("Single quote"));

        let bar = items.iter().find(|i| i.path == "mod.bar");
        assert!(bar.is_some());
        assert!(bar.unwrap().docstring.contains("Triple single"));
    }

    #[test]
    fn test_nested_class() {
        let source = r#"
class Outer:
    """Outer class."""

    class Inner:
        """Inner class."""

        def inner_method(self):
            """Inner method."""
            pass
"#;

        let mut extractor = PyDocExtractor::new();
        let items = extractor.extract_from_source(source, "mod").unwrap();

        // Note: Our simple implementation doesn't recurse into nested classes,
        // but that's fine for most packages
        assert!(
            items.iter().any(|i| i.path == "mod.Outer"),
            "Should find outer class"
        );
    }
}
