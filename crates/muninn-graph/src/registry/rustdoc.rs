//! Rustdoc JSON extraction for Rust crates.
//!
//! This module provides functionality to:
//! 1. Run `cargo rustdoc --output-format json` on a crate
//! 2. Parse the resulting JSON documentation
//! 3. Extract documentation items (functions, structs, traits, etc.)
//!
//! # Example
//!
//! ```no_run
//! use muninn_graph::registry::rustdoc::{RustdocExtractor, extract_docs_from_json};
//!
//! // Run rustdoc on a crate
//! let extractor = RustdocExtractor::new();
//! let json_path = extractor.generate_json("/path/to/crate")?;
//!
//! // Parse and extract documentation
//! let items = extract_docs_from_json(&json_path)?;
//! for item in items {
//!     println!("{}: {}", item.path, item.doc_text.unwrap_or_default());
//! }
//! # Ok::<(), muninn_graph::registry::rustdoc::RustdocError>(())
//! ```

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use rustdoc_types::{Crate, Id, Item, ItemEnum, Visibility};

use crate::doc_store::{DocChunkInput, ItemType};

/// Error type for rustdoc operations.
#[derive(Debug, thiserror::Error)]
pub enum RustdocError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Cargo rustdoc failed: {0}")]
    CargoFailed(String),

    #[error("Rustdoc JSON not found at expected path: {0}")]
    JsonNotFound(PathBuf),

    #[error("Could not determine crate name from Cargo.toml")]
    NoCrateName,

    #[error("Cargo.toml not found in {0}")]
    NoCargoToml(PathBuf),
}

pub type Result<T> = std::result::Result<T, RustdocError>;

/// Extracted documentation item from rustdoc JSON.
#[derive(Debug, Clone)]
pub struct ExtractedItem {
    /// Full path of the item (e.g., "tokio::spawn", "std::vec::Vec")
    pub path: String,

    /// Type of the item
    pub item_type: ItemType,

    /// Documentation text (from doc comments)
    pub doc_text: Option<String>,

    /// Function/method signature (if applicable)
    pub signature: Option<String>,

    /// Visibility level
    pub visibility: ItemVisibility,
}

/// Visibility of an extracted item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemVisibility {
    Public,
    Crate,
    Restricted,
    Private,
}

impl From<&Visibility> for ItemVisibility {
    fn from(vis: &Visibility) -> Self {
        match vis {
            Visibility::Public => ItemVisibility::Public,
            Visibility::Default => ItemVisibility::Private,
            Visibility::Crate => ItemVisibility::Crate,
            Visibility::Restricted { .. } => ItemVisibility::Restricted,
        }
    }
}

impl ExtractedItem {
    /// Convert to DocChunkInput for storage.
    pub fn to_doc_chunk(&self) -> DocChunkInput {
        DocChunkInput {
            item_path: self.path.clone(),
            item_type: self.item_type,
            doc_text: self.doc_text.clone().unwrap_or_default(),
            signature: self.signature.clone(),
            embedding: None,
        }
    }
}

/// Rustdoc JSON generator and parser.
///
/// Handles running `cargo rustdoc` and extracting documentation from the
/// resulting JSON file.
pub struct RustdocExtractor {
    /// Additional rustdoc flags
    rustdoc_flags: Vec<String>,
}

impl RustdocExtractor {
    /// Create a new rustdoc extractor with default settings.
    pub fn new() -> Self {
        Self {
            rustdoc_flags: Vec::new(),
        }
    }

    /// Add custom rustdoc flags.
    pub fn with_flags(mut self, flags: Vec<String>) -> Self {
        self.rustdoc_flags = flags;
        self
    }

    /// Generate rustdoc JSON for a crate.
    ///
    /// # Arguments
    ///
    /// * `crate_path` - Path to the crate directory (containing Cargo.toml)
    ///
    /// # Returns
    ///
    /// Path to the generated JSON file.
    ///
    /// # Note
    ///
    /// This requires nightly Rust. The method will try `cargo +nightly rustdoc` first,
    /// then fall back to `cargo rustdoc` (which works if nightly is the default).
    pub fn generate_json(&self, crate_path: impl AsRef<Path>) -> Result<PathBuf> {
        let crate_path = crate_path.as_ref();

        // Verify Cargo.toml exists
        let cargo_toml = crate_path.join("Cargo.toml");
        if !cargo_toml.exists() {
            return Err(RustdocError::NoCargoToml(crate_path.to_path_buf()));
        }

        // Get crate name from Cargo.toml
        let crate_name = self.get_crate_name(&cargo_toml)?;

        // Try with +nightly first, then fall back to default toolchain
        let result = self.try_generate_json_with_toolchain(crate_path, Some("nightly"));

        if result.is_ok() {
            return self.find_json_output(crate_path, &crate_name);
        }

        // Fall back to default toolchain (in case nightly is the default)
        let result = self.try_generate_json_with_toolchain(crate_path, None);

        if let Err(e) = result {
            // Provide a more helpful error message
            let stderr = match &e {
                RustdocError::CargoFailed(msg) => msg.clone(),
                _ => e.to_string(),
            };

            if stderr.contains("nightly") || stderr.contains("option `Z`") {
                return Err(RustdocError::CargoFailed(
                    "Rustdoc JSON output requires nightly Rust. Install with: rustup install nightly".to_string()
                ));
            }
            return Err(e);
        }

        self.find_json_output(crate_path, &crate_name)
    }

    /// Try to generate rustdoc JSON with a specific toolchain.
    fn try_generate_json_with_toolchain(
        &self,
        crate_path: &Path,
        toolchain: Option<&str>,
    ) -> Result<()> {
        let mut cmd = Command::new("cargo");
        cmd.current_dir(crate_path);

        // Add toolchain specifier if provided (e.g., +nightly)
        if let Some(tc) = toolchain {
            cmd.arg(format!("+{}", tc));
        }

        cmd.arg("rustdoc")
            .arg("--")
            .arg("--output-format")
            .arg("json")
            .arg("-Z")
            .arg("unstable-options");

        // Add custom flags
        for flag in &self.rustdoc_flags {
            cmd.arg(flag);
        }

        // Run cargo rustdoc
        let output = cmd.output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(RustdocError::CargoFailed(stderr.to_string()));
        }

        Ok(())
    }

    /// Find the generated JSON output file.
    fn find_json_output(&self, crate_path: &Path, crate_name: &str) -> Result<PathBuf> {
        // It's typically at target/doc/{crate_name}.json
        let json_path = crate_path
            .join("target")
            .join("doc")
            .join(format!("{}.json", crate_name.replace('-', "_")));

        if !json_path.exists() {
            return Err(RustdocError::JsonNotFound(json_path));
        }

        Ok(json_path)
    }

    /// Extract crate name from Cargo.toml.
    fn get_crate_name(&self, cargo_toml: &Path) -> Result<String> {
        let content = fs::read_to_string(cargo_toml)?;

        // Simple TOML parsing for [package] name
        // A proper implementation would use the `toml` crate
        for line in content.lines() {
            let line = line.trim();
            if line.starts_with("name") && line.contains('=') {
                if let Some(name) = line.split('=').nth(1) {
                    let name = name.trim().trim_matches('"').trim_matches('\'');
                    return Ok(name.to_string());
                }
            }
        }

        Err(RustdocError::NoCrateName)
    }
}

impl Default for RustdocExtractor {
    fn default() -> Self {
        Self::new()
    }
}

/// Extract documentation items from a rustdoc JSON file.
///
/// # Arguments
///
/// * `json_path` - Path to the rustdoc JSON file
///
/// # Returns
///
/// Vector of extracted documentation items.
pub fn extract_docs_from_json(json_path: impl AsRef<Path>) -> Result<Vec<ExtractedItem>> {
    let json_content = fs::read_to_string(json_path)?;
    let krate: Crate = serde_json::from_str(&json_content)?;

    extract_docs_from_crate(&krate)
}

/// Extract documentation items from a parsed rustdoc Crate.
pub fn extract_docs_from_crate(krate: &Crate) -> Result<Vec<ExtractedItem>> {
    let mut items = Vec::new();
    let mut path_cache: HashMap<&Id, Vec<String>> = HashMap::new();

    // Process all items in the index
    for (id, item) in &krate.index {
        // Get the full path for this item
        let path = build_item_path(krate, id, &mut path_cache);

        // For impl blocks, always try to extract nested items (methods)
        // even if the impl itself isn't "public" (impls don't have traditional visibility)
        if matches!(&item.inner, ItemEnum::Impl(_)) {
            extract_nested_items(krate, item, &path, &mut items, &mut path_cache);
            continue;
        }

        // Skip private items (unless they have public re-exports)
        if !is_public_item(item) {
            continue;
        }

        // Extract documentation based on item type
        if let Some(extracted) = extract_item(krate, item, &path, &mut path_cache) {
            items.push(extracted);
        }

        // Extract nested items (struct fields, enum variants, trait methods)
        extract_nested_items(krate, item, &path, &mut items, &mut path_cache);
    }

    Ok(items)
}

/// Check if an item is publicly visible.
fn is_public_item(item: &Item) -> bool {
    matches!(item.visibility, Visibility::Public)
}

/// Build the full path for an item (e.g., "crate::module::function").
fn build_item_path<'a>(
    krate: &'a Crate,
    id: &'a Id,
    cache: &mut HashMap<&'a Id, Vec<String>>,
) -> String {
    if let Some(cached) = cache.get(id) {
        return cached.join("::");
    }

    // Look up the item's path from the paths table
    if let Some(item_summary) = krate.paths.get(id) {
        let path = item_summary.path.clone();
        cache.insert(id, path.clone());
        return path.join("::");
    }

    // Fallback: use the item name
    if let Some(item) = krate.index.get(id) {
        if let Some(name) = &item.name {
            return name.clone();
        }
    }

    // Unknown path
    format!("unknown::{}", id.0)
}

/// Extract documentation from a single item.
fn extract_item<'a>(
    krate: &'a Crate,
    item: &Item,
    path: &str,
    cache: &mut HashMap<&'a Id, Vec<String>>,
) -> Option<ExtractedItem> {
    let (item_type, signature) = match &item.inner {
        ItemEnum::Module(_) => (ItemType::Module, None),
        ItemEnum::Struct(s) => {
            let sig = format_struct_signature(s, item.name.as_deref());
            (ItemType::Struct, Some(sig))
        }
        ItemEnum::Enum(e) => {
            let sig = format_enum_signature(e, item.name.as_deref());
            (ItemType::Enum, Some(sig))
        }
        ItemEnum::Trait(t) => {
            let sig = format_trait_signature(t, item.name.as_deref());
            (ItemType::Trait, Some(sig))
        }
        ItemEnum::Function(f) => {
            let sig = format_function_signature(krate, f, item.name.as_deref(), cache);
            return Some(ExtractedItem {
                path: path.to_string(),
                item_type: ItemType::Function,
                doc_text: item.docs.clone(),
                signature: Some(sig),
                visibility: (&item.visibility).into(),
            });
        }
        ItemEnum::TypeAlias(t) => {
            let sig = format_type_alias_signature(krate, t, item.name.as_deref(), cache);
            (ItemType::Type, Some(sig))
        }
        ItemEnum::Constant { type_: ty, const_: c } => {
            let sig = format_constant_signature(krate, ty, c, item.name.as_deref(), cache);
            (ItemType::Constant, Some(sig))
        }
        ItemEnum::Impl(_) => {
            // For impls, we don't create a top-level item
            // The impl's items (methods) are extracted via extract_nested_items
            return None;
        }
        _ => return None, // Skip other item types (ExternCrate, Use, etc.)
    };

    // Only include items with documentation or signature
    if item.docs.is_none() && signature.is_none() && !matches!(item_type, ItemType::Module) {
        return None;
    }

    Some(ExtractedItem {
        path: path.to_string(),
        item_type,
        doc_text: item.docs.clone(),
        signature,
        visibility: (&item.visibility).into(),
    })
}

/// Extract nested items like struct fields, enum variants, and impl methods.
fn extract_nested_items<'a>(
    krate: &'a Crate,
    item: &Item,
    parent_path: &str,
    items: &mut Vec<ExtractedItem>,
    cache: &mut HashMap<&'a Id, Vec<String>>,
) {
    match &item.inner {
        ItemEnum::Struct(s) => {
            // Extract struct fields
            match &s.kind {
                rustdoc_types::StructKind::Plain { fields, .. } => {
                    for field_id in fields {
                        if let Some(field_item) = krate.index.get(field_id) {
                            if let Some(name) = &field_item.name {
                                let field_path = format!("{}::{}", parent_path, name);
                                if let Some(docs) = &field_item.docs {
                                    items.push(ExtractedItem {
                                        path: field_path,
                                        item_type: ItemType::Constant, // Use Constant for fields
                                        doc_text: Some(docs.clone()),
                                        signature: None,
                                        visibility: (&field_item.visibility).into(),
                                    });
                                }
                            }
                        }
                    }
                }
                _ => {} // Skip tuple and unit structs
            }
        }
        ItemEnum::Enum(e) => {
            // Extract enum variants
            for variant_id in &e.variants {
                if let Some(variant_item) = krate.index.get(variant_id) {
                    if let Some(name) = &variant_item.name {
                        let variant_path = format!("{}::{}", parent_path, name);
                        if variant_item.docs.is_some() {
                            items.push(ExtractedItem {
                                path: variant_path,
                                item_type: ItemType::Constant, // Use Constant for variants
                                doc_text: variant_item.docs.clone(),
                                signature: None,
                                visibility: (&variant_item.visibility).into(),
                            });
                        }
                    }
                }
            }
        }
        ItemEnum::Trait(t) => {
            // Extract trait methods
            for method_id in &t.items {
                if let Some(method_item) = krate.index.get(method_id) {
                    if let ItemEnum::Function(f) = &method_item.inner {
                        if let Some(name) = &method_item.name {
                            let method_path = format!("{}::{}", parent_path, name);
                            let sig = format_function_signature(krate, f, Some(name), cache);
                            items.push(ExtractedItem {
                                path: method_path,
                                item_type: ItemType::Method,
                                doc_text: method_item.docs.clone(),
                                signature: Some(sig),
                                visibility: (&method_item.visibility).into(),
                            });
                        }
                    }
                }
            }
        }
        ItemEnum::Impl(i) => {
            // Extract impl methods
            // Build the impl path (e.g., "Type" or "<Type as Trait>")
            let impl_path = build_impl_path(krate, i, cache);

            for method_id in &i.items {
                if let Some(method_item) = krate.index.get(method_id) {
                    // Only extract public methods
                    if !is_public_item(method_item) {
                        continue;
                    }

                    if let ItemEnum::Function(f) = &method_item.inner {
                        if let Some(name) = &method_item.name {
                            let method_path = format!("{}::{}", impl_path, name);
                            let sig = format_function_signature(krate, f, Some(name), cache);
                            items.push(ExtractedItem {
                                path: method_path,
                                item_type: ItemType::Method,
                                doc_text: method_item.docs.clone(),
                                signature: Some(sig),
                                visibility: (&method_item.visibility).into(),
                            });
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

/// Build the path for an impl block.
fn build_impl_path<'a>(
    krate: &'a Crate,
    impl_: &rustdoc_types::Impl,
    cache: &mut HashMap<&'a Id, Vec<String>>,
) -> String {
    let type_path = format_type(krate, &impl_.for_, cache);

    if let Some(trait_) = &impl_.trait_ {
        let trait_path = format_path(&trait_.path);
        format!("<{} as {}>", type_path, trait_path)
    } else {
        type_path
    }
}

/// Format a function signature for display.
fn format_function_signature<'a>(
    krate: &'a Crate,
    f: &rustdoc_types::Function,
    name: Option<&str>,
    cache: &mut HashMap<&'a Id, Vec<String>>,
) -> String {
    let name = name.unwrap_or("unknown");

    // Build generics
    let generics = if f.generics.params.is_empty() {
        String::new()
    } else {
        format!(
            "<{}>",
            f.generics
                .params
                .iter()
                .map(|p| p.name.clone())
                .collect::<Vec<_>>()
                .join(", ")
        )
    };

    // Build parameter list with types
    let inputs: Vec<String> = f
        .sig
        .inputs
        .iter()
        .map(|(param_name, ty)| {
            let type_str = format_type(krate, ty, cache);
            if param_name == "self" {
                param_name.clone()
            } else {
                format!("{}: {}", param_name, type_str)
            }
        })
        .collect();

    // Build return type
    let output = if let Some(ty) = &f.sig.output {
        format!(" -> {}", format_type(krate, ty, cache))
    } else {
        String::new()
    };

    format!("fn {}{}({}){}", name, generics, inputs.join(", "), output)
}

/// Format a struct signature.
fn format_struct_signature(s: &rustdoc_types::Struct, name: Option<&str>) -> String {
    let name = name.unwrap_or("unknown");
    let generics = format_generics(&s.generics);

    match &s.kind {
        rustdoc_types::StructKind::Unit => format!("struct {}{}", name, generics),
        rustdoc_types::StructKind::Tuple(_) => format!("struct {}{}(...)", name, generics),
        rustdoc_types::StructKind::Plain { .. } => format!("struct {}{} {{ ... }}", name, generics),
    }
}

/// Format an enum signature.
fn format_enum_signature(e: &rustdoc_types::Enum, name: Option<&str>) -> String {
    let name = name.unwrap_or("unknown");
    let generics = format_generics(&e.generics);
    format!("enum {}{} {{ ... }}", name, generics)
}

/// Format a trait signature.
fn format_trait_signature(t: &rustdoc_types::Trait, name: Option<&str>) -> String {
    let name = name.unwrap_or("unknown");
    let generics = format_generics(&t.generics);
    format!("trait {}{} {{ ... }}", name, generics)
}

/// Format a type alias signature.
fn format_type_alias_signature<'a>(
    krate: &'a Crate,
    t: &rustdoc_types::TypeAlias,
    name: Option<&str>,
    cache: &mut HashMap<&'a Id, Vec<String>>,
) -> String {
    let name = name.unwrap_or("unknown");
    let generics = format_generics(&t.generics);
    let type_str = format_type(krate, &t.type_, cache);
    format!("type {}{} = {}", name, generics, type_str)
}

/// Format a constant signature.
fn format_constant_signature<'a>(
    krate: &'a Crate,
    ty: &rustdoc_types::Type,
    _const_: &rustdoc_types::Constant,
    name: Option<&str>,
    cache: &mut HashMap<&'a Id, Vec<String>>,
) -> String {
    let name = name.unwrap_or("unknown");
    let type_str = format_type(krate, ty, cache);
    format!("const {}: {}", name, type_str)
}

/// Format generics for display.
fn format_generics(generics: &rustdoc_types::Generics) -> String {
    if generics.params.is_empty() {
        String::new()
    } else {
        format!(
            "<{}>",
            generics
                .params
                .iter()
                .map(|p| p.name.clone())
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

/// Format a type for display.
fn format_type<'a>(
    krate: &'a Crate,
    ty: &rustdoc_types::Type,
    cache: &mut HashMap<&'a Id, Vec<String>>,
) -> String {
    use rustdoc_types::Type;

    match ty {
        Type::ResolvedPath(path) => {
            let base = format_path(&path.path);
            if let Some(args) = &path.args {
                if let rustdoc_types::GenericArgs::AngleBracketed { args, .. } = args.as_ref() {
                    if !args.is_empty() {
                        let args_str: Vec<String> = args
                            .iter()
                            .filter_map(|arg| match arg {
                                rustdoc_types::GenericArg::Type(t) => {
                                    Some(format_type(krate, t, cache))
                                }
                                rustdoc_types::GenericArg::Lifetime(l) => Some(l.clone()),
                                rustdoc_types::GenericArg::Const(c) => Some(c.value.clone().unwrap_or_else(|| c.expr.clone())),
                                _ => None,
                            })
                            .collect();
                        if !args_str.is_empty() {
                            return format!("{}<{}>", base, args_str.join(", "));
                        }
                    }
                }
            }
            base
        }
        Type::DynTrait(dt) => {
            let traits: Vec<String> = dt
                .traits
                .iter()
                .map(|pb| format_path(&pb.trait_.path))
                .collect();
            format!("dyn {}", traits.join(" + "))
        }
        Type::Generic(name) => name.clone(),
        Type::Primitive(name) => name.clone(),
        Type::FunctionPointer(fp) => {
            let inputs: Vec<String> = fp
                .sig
                .inputs
                .iter()
                .map(|(_, ty)| format_type(krate, ty, cache))
                .collect();
            let output = if let Some(ty) = &fp.sig.output {
                format!(" -> {}", format_type(krate, ty, cache))
            } else {
                String::new()
            };
            format!("fn({}){}", inputs.join(", "), output)
        }
        Type::Tuple(types) => {
            if types.is_empty() {
                "()".to_string()
            } else {
                let inner: Vec<String> = types
                    .iter()
                    .map(|t| format_type(krate, t, cache))
                    .collect();
                format!("({})", inner.join(", "))
            }
        }
        Type::Slice(ty) => format!("[{}]", format_type(krate, ty, cache)),
        Type::Array { type_, len } => {
            format!("[{}; {}]", format_type(krate, type_, cache), len)
        }
        Type::ImplTrait(bounds) => {
            let bounds_str: Vec<String> = bounds
                .iter()
                .filter_map(|b| match b {
                    rustdoc_types::GenericBound::TraitBound { trait_, .. } => {
                        Some(format_path(&trait_.path))
                    }
                    rustdoc_types::GenericBound::Outlives(l) => Some(l.clone()),
                    _ => None,
                })
                .collect();
            format!("impl {}", bounds_str.join(" + "))
        }
        Type::Infer => "_".to_string(),
        Type::RawPointer { is_mutable, type_ } => {
            let mutability = if *is_mutable { "mut" } else { "const" };
            format!("*{} {}", mutability, format_type(krate, type_, cache))
        }
        Type::BorrowedRef {
            lifetime,
            is_mutable,
            type_,
        } => {
            let lifetime_str = lifetime
                .as_ref()
                .map(|l| format!("{} ", l))
                .unwrap_or_default();
            let mutability = if *is_mutable { "mut " } else { "" };
            format!(
                "&{}{}{}",
                lifetime_str,
                mutability,
                format_type(krate, type_, cache)
            )
        }
        Type::QualifiedPath {
            self_type, trait_, ..
        } => {
            let self_str = format_type(krate, self_type, cache);
            if let Some(trait_) = trait_ {
                let trait_str = format_path(&trait_.path);
                format!("<{} as {}>::...", self_str, trait_str)
            } else {
                format!("{}::...", self_str)
            }
        }
        Type::Pat { type_, .. } => format_type(krate, type_, cache),
    }
}

/// Format a path for display.
fn format_path(path: &str) -> String {
    path.to_string()
}

/// Convert extracted items to DocChunkInput for storage.
pub fn items_to_chunks(items: Vec<ExtractedItem>) -> Vec<DocChunkInput> {
    items
        .into_iter()
        .filter(|item| item.doc_text.is_some() || item.signature.is_some())
        .map(|item| item.to_doc_chunk())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extractor_creation() {
        let extractor = RustdocExtractor::new();
        // Just verify it doesn't panic
        drop(extractor);
    }

    #[test]
    fn test_item_visibility_conversion() {
        assert_eq!(
            ItemVisibility::from(&Visibility::Public),
            ItemVisibility::Public
        );
        assert_eq!(
            ItemVisibility::from(&Visibility::Default),
            ItemVisibility::Private
        );
        assert_eq!(
            ItemVisibility::from(&Visibility::Crate),
            ItemVisibility::Crate
        );
    }

    #[test]
    fn test_extracted_item_to_chunk() {
        let item = ExtractedItem {
            path: "my_crate::my_func".to_string(),
            item_type: ItemType::Function,
            doc_text: Some("This is a function.".to_string()),
            signature: Some("fn my_func(x: i32) -> bool".to_string()),
            visibility: ItemVisibility::Public,
        };

        let chunk = item.to_doc_chunk();
        assert_eq!(chunk.item_path, "my_crate::my_func");
        assert_eq!(chunk.item_type, ItemType::Function);
        assert_eq!(chunk.doc_text, "This is a function.");
        assert_eq!(chunk.signature, Some("fn my_func(x: i32) -> bool".to_string()));
    }

    #[test]
    fn test_items_to_chunks_filters_empty() {
        let items = vec![
            ExtractedItem {
                path: "with_docs".to_string(),
                item_type: ItemType::Function,
                doc_text: Some("Has docs".to_string()),
                signature: None,
                visibility: ItemVisibility::Public,
            },
            ExtractedItem {
                path: "no_docs".to_string(),
                item_type: ItemType::Function,
                doc_text: None,
                signature: None,
                visibility: ItemVisibility::Public,
            },
            ExtractedItem {
                path: "with_sig".to_string(),
                item_type: ItemType::Function,
                doc_text: None,
                signature: Some("fn with_sig()".to_string()),
                visibility: ItemVisibility::Public,
            },
        ];

        let chunks = items_to_chunks(items);
        assert_eq!(chunks.len(), 2);
        assert!(chunks.iter().any(|c| c.item_path == "with_docs"));
        assert!(chunks.iter().any(|c| c.item_path == "with_sig"));
    }

    #[test]
    #[ignore] // Requires network, cargo/rustdoc, and nightly Rust
    fn test_full_pipeline_with_crate() {
        use crate::registry::CratesIoClient;

        // Check if nightly is available
        let nightly_check = Command::new("cargo")
            .args(["+nightly", "--version"])
            .output();

        if nightly_check.is_err() || !nightly_check.unwrap().status.success() {
            eprintln!("Skipping test: nightly Rust not available. Install with: rustup install nightly");
            return;
        }

        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        let client = CratesIoClient::new();

        // Download a small, well-documented crate
        let (crate_path, _version) = client
            .download_latest("cfg-if", temp_dir.path())
            .expect("Failed to download crate");

        // Generate rustdoc JSON
        let extractor = RustdocExtractor::new();
        let json_path = match extractor.generate_json(&crate_path) {
            Ok(path) => path,
            Err(RustdocError::CargoFailed(msg)) if msg.contains("nightly") => {
                eprintln!("Skipping test: {}", msg);
                return;
            }
            Err(e) => panic!("Failed to generate rustdoc JSON: {}", e),
        };

        // Extract documentation
        let items = extract_docs_from_json(&json_path).expect("Failed to extract docs");

        // Verify we got some items
        assert!(!items.is_empty(), "Should extract at least some items");

        // Check that we have a module or macro (cfg-if mainly exports a macro)
        let has_macro_or_module = items
            .iter()
            .any(|i| matches!(i.item_type, ItemType::Module));
        assert!(has_macro_or_module, "Should have module items");
    }

    #[test]
    #[ignore] // Requires cargo/rustdoc installed
    fn test_generate_json_no_cargo_toml() {
        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        let extractor = RustdocExtractor::new();

        let result = extractor.generate_json(temp_dir.path());
        assert!(matches!(result, Err(RustdocError::NoCargoToml(_))));
    }

    #[test]
    #[ignore] // Requires network, cargo/rustdoc, and nightly Rust
    fn test_extract_structs_and_methods() {
        use crate::registry::CratesIoClient;

        // Check if nightly is available
        let nightly_check = Command::new("cargo")
            .args(["+nightly", "--version"])
            .output();

        if nightly_check.is_err() || !nightly_check.unwrap().status.success() {
            eprintln!("Skipping test: nightly Rust not available");
            return;
        }

        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        let client = CratesIoClient::new();

        // Download once_cell - it has well-documented structs with methods
        let (crate_path, _version) = client
            .download_latest("once_cell", temp_dir.path())
            .expect("Failed to download crate");

        let extractor = RustdocExtractor::new();
        let json_path = match extractor.generate_json(&crate_path) {
            Ok(path) => path,
            Err(RustdocError::CargoFailed(msg)) if msg.contains("nightly") => {
                eprintln!("Skipping test: {}", msg);
                return;
            }
            Err(e) => panic!("Failed to generate rustdoc JSON: {}", e),
        };

        let items = extract_docs_from_json(&json_path).expect("Failed to extract docs");

        // Verify we extracted various item types
        let item_types: Vec<_> = items.iter().map(|i| &i.item_type).collect();

        // Should have structs (OnceCell, Lazy, etc.)
        assert!(
            item_types.iter().any(|t| **t == ItemType::Struct),
            "Should have struct items"
        );

        // Should have methods on those structs
        assert!(
            item_types.iter().any(|t| **t == ItemType::Method),
            "Should have method items"
        );

        // Verify some signatures include type information
        let functions_with_types = items
            .iter()
            .filter(|i| i.signature.is_some())
            .filter(|i| {
                i.signature
                    .as_ref()
                    .map(|s| s.contains(": ") || s.contains("->"))
                    .unwrap_or(false)
            })
            .count();

        assert!(
            functions_with_types > 0,
            "Should have signatures with type annotations"
        );

        // Print some stats for debugging
        eprintln!("Extracted {} total items", items.len());
        eprintln!(
            "  Structs: {}",
            items
                .iter()
                .filter(|i| i.item_type == ItemType::Struct)
                .count()
        );
        eprintln!(
            "  Methods: {}",
            items
                .iter()
                .filter(|i| i.item_type == ItemType::Method)
                .count()
        );
        eprintln!(
            "  Functions: {}",
            items
                .iter()
                .filter(|i| i.item_type == ItemType::Function)
                .count()
        );
        eprintln!(
            "  Items with docs: {}",
            items.iter().filter(|i| i.doc_text.is_some()).count()
        );
        eprintln!(
            "  Items with signatures: {}",
            items.iter().filter(|i| i.signature.is_some()).count()
        );
    }
}
