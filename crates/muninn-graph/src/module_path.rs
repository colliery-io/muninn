//! Derive a symbol's module path from its file path.
//!
//! Used during graph construction to populate `Symbol::qualified_name`
//! so that scoped calls like `muninn_rlm::daemon::socket_path_for_repo`
//! can be resolved against an exact-match index rather than guessed
//! by short name.
//!
//! ## What it handles
//!
//! For Rust files under the conventional `crates/<name>/src/...`
//! layout, this derives the file-level module prefix:
//!
//!   * `crates/muninn-rlm/src/lib.rs` → `muninn_rlm`
//!   * `crates/muninn-rlm/src/router.rs` → `muninn_rlm::router`
//!   * `crates/muninn-rlm/src/lang/mod.rs` → `muninn_rlm::lang`
//!   * `crates/muninn-rlm/src/lang/rust.rs` → `muninn_rlm::lang::rust`
//!
//! Crate-name hyphens are converted to underscores (Rust convention).
//!
//! ## What it doesn't handle (v1 limitations)
//!
//! * **Enclosing `mod foo {}` blocks.** A function nested inside a
//!   `mod foo {}` inside `router.rs` gets the file-level prefix
//!   `muninn_rlm::router`, not `muninn_rlm::router::foo`. Tree-sitter
//!   could give us this with a separate query pass; deferred.
//! * **`impl` receivers.** `impl Foo { fn bar() {} }` produces a
//!   symbol with name `bar`, qualified to `muninn_rlm::router::bar`
//!   rather than `muninn_rlm::router::Foo::bar`.
//! * **`use` aliases.** Callees written after a `use` get only
//!   their visible prefix in the call expression; this module only
//!   provides definitions' canonical paths. Resolution code can
//!   still fall back to short-name lookup for non-canonical references.
//! * **Non-`crates/` layouts.** Returns `None` when the file path
//!   doesn't match the `crates/<name>/` convention. For repos with
//!   a different layout, qualified resolution simply isn't available
//!   and the short-name fallback handles things.

/// Derive the module prefix for a Rust file at `file_path`.
/// Returns `None` if the path doesn't match the `crates/<name>/`
/// workspace convention.
pub fn derive_rust_module_prefix(file_path: &str) -> Option<String> {
    let (crate_name, after_crate) = split_at_crate_segment(file_path)?;
    let normalized_crate = crate_name.replace('-', "_");

    // After the crate name we may have `src/...` or just `...`.
    // Strip a leading `src` segment if present.
    let mut segments = after_crate.split('/').peekable();
    if segments.peek() == Some(&"src") {
        segments.next();
    }

    let module_parts: Vec<String> = segments
        .filter_map(|seg| {
            if let Some(stem) = seg.strip_suffix(".rs") {
                // `lib.rs` is the crate root; `mod.rs` represents
                // the directory itself — neither contributes a
                // module segment of its own.
                if stem == "lib" || stem == "mod" {
                    None
                } else {
                    Some(stem.to_string())
                }
            } else {
                // A directory segment in the middle of the path.
                // (Empty segments from leading/trailing slashes are
                // dropped by `split`.)
                if seg.is_empty() {
                    None
                } else {
                    Some(seg.to_string())
                }
            }
        })
        .collect();

    if module_parts.is_empty() {
        Some(normalized_crate)
    } else {
        Some(format!("{normalized_crate}::{}", module_parts.join("::")))
    }
}

/// Find the `crates/<name>/` segment and return `(name, rest)`.
fn split_at_crate_segment(path: &str) -> Option<(&str, &str)> {
    let needle = "crates/";
    let start = path.find(needle)?;
    let after = &path[start + needle.len()..];
    let slash = after.find('/')?;
    Some((&after[..slash], &after[slash + 1..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lib_rs_is_crate_root() {
        assert_eq!(
            derive_rust_module_prefix("/repo/crates/muninn-rlm/src/lib.rs"),
            Some("muninn_rlm".to_string())
        );
    }

    #[test]
    fn file_under_src_becomes_module() {
        assert_eq!(
            derive_rust_module_prefix("/repo/crates/muninn-rlm/src/router.rs"),
            Some("muninn_rlm::router".to_string())
        );
    }

    #[test]
    fn mod_rs_is_directory_module() {
        assert_eq!(
            derive_rust_module_prefix("/repo/crates/muninn-rlm/src/lang/mod.rs"),
            Some("muninn_rlm::lang".to_string())
        );
    }

    #[test]
    fn nested_file_is_nested_module() {
        assert_eq!(
            derive_rust_module_prefix("/repo/crates/muninn-rlm/src/lang/rust.rs"),
            Some("muninn_rlm::lang::rust".to_string())
        );
    }

    #[test]
    fn hyphens_in_crate_become_underscores() {
        assert_eq!(
            derive_rust_module_prefix("/repo/crates/some-crate-name/src/lib.rs"),
            Some("some_crate_name".to_string())
        );
    }

    #[test]
    fn returns_none_outside_crates_convention() {
        assert_eq!(derive_rust_module_prefix("/repo/src/main.rs"), None);
        assert_eq!(derive_rust_module_prefix("relative/path/foo.rs"), None);
    }

    #[test]
    fn handles_tests_directory_alongside_src() {
        // Integration test under crates/<name>/tests/foo.rs.
        // We treat it as the crate's `tests::foo` module — the
        // tests dir isn't really a Rust module path, but for graph
        // resolution this still avoids name collisions.
        assert_eq!(
            derive_rust_module_prefix("/repo/crates/muninn-rlm/tests/integration.rs"),
            Some("muninn_rlm::tests::integration".to_string())
        );
    }
}
