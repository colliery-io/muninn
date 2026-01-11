//! Language-specific extractors.
//!
//! Each language module provides extraction logic for symbols, imports,
//! and call relationships from parsed syntax trees.

pub mod python;
pub mod rust;

pub use python::PythonExtractor;
pub use rust::RustExtractor;
