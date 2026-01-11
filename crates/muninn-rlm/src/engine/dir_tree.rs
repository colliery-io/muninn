//! Directory tree generation for context injection.
//!
//! This module provides utilities for generating compact directory trees
//! to include in system prompts for project context.

use std::path::Path;

/// Generate a compact directory tree string for a given path.
///
/// Returns None if the path doesn't exist or can't be read.
pub fn generate_dir_tree(work_dir: &Path) -> Option<String> {
    if !work_dir.exists() {
        return None;
    }

    let mut tree = String::new();
    tree.push_str("## Project Structure\n\n```\n");
    walk_dir(work_dir, &mut tree, 0, 3);
    tree.push_str("```\n");
    Some(tree)
}

fn walk_dir(dir: &Path, output: &mut String, depth: usize, max_depth: usize) {
    if depth > max_depth {
        return;
    }

    let entries: Vec<_> = match std::fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(|e| e.ok())
            .filter(|e| {
                let name = e.file_name();
                let name_str = name.to_string_lossy();
                // Skip hidden files and common noise directories
                !name_str.starts_with('.')
                    && name_str != "target"
                    && name_str != "node_modules"
                    && name_str != "__pycache__"
            })
            .collect(),
        Err(_) => return,
    };

    // Sort: directories first, then alphabetically by name
    let mut sorted: Vec<_> = entries.iter().collect();
    sorted.sort_by_key(|e| (!e.path().is_dir(), e.file_name()));

    for entry in sorted {
        let path = entry.path();
        let name = entry.file_name();
        let indent = "  ".repeat(depth);

        if path.is_dir() {
            output.push_str(&format!("{}{}/\n", indent, name.to_string_lossy()));
            walk_dir(&path, output, depth + 1, max_depth);
        } else {
            output.push_str(&format!("{}{}\n", indent, name.to_string_lossy()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_generate_dir_tree_nonexistent() {
        let result = generate_dir_tree(Path::new("/nonexistent/path/12345"));
        assert!(result.is_none());
    }

    #[test]
    fn test_generate_dir_tree_basic() {
        let temp_dir = TempDir::new().unwrap();
        let base = temp_dir.path();

        // Create some files and directories
        fs::create_dir(base.join("src")).unwrap();
        fs::write(base.join("src/main.rs"), "fn main() {}").unwrap();
        fs::write(base.join("Cargo.toml"), "[package]").unwrap();

        let result = generate_dir_tree(base).unwrap();

        assert!(result.contains("## Project Structure"));
        assert!(result.contains("src/"));
        assert!(result.contains("main.rs"));
        assert!(result.contains("Cargo.toml"));
    }

    #[test]
    fn test_generate_dir_tree_filters_hidden() {
        let temp_dir = TempDir::new().unwrap();
        let base = temp_dir.path();

        fs::write(base.join("visible.txt"), "").unwrap();
        fs::write(base.join(".hidden"), "").unwrap();
        fs::create_dir(base.join(".git")).unwrap();

        let result = generate_dir_tree(base).unwrap();

        assert!(result.contains("visible.txt"));
        assert!(!result.contains(".hidden"));
        assert!(!result.contains(".git"));
    }

    #[test]
    fn test_generate_dir_tree_filters_noise_dirs() {
        let temp_dir = TempDir::new().unwrap();
        let base = temp_dir.path();

        fs::create_dir(base.join("src")).unwrap();
        fs::create_dir(base.join("target")).unwrap();
        fs::create_dir(base.join("node_modules")).unwrap();

        let result = generate_dir_tree(base).unwrap();

        assert!(result.contains("src/"));
        assert!(!result.contains("target/"));
        assert!(!result.contains("node_modules/"));
    }
}
