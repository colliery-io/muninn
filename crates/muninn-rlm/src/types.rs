//! LLM type re-exports.
//!
//! The concrete definitions live in [`muninn_core::llm`] so the engine
//! trait can name them. This module is a thin re-export shim that keeps
//! existing `crate::types::Foo` paths working inside `muninn-rlm` without
//! touching every call site.
//!
//! Prefer `muninn_core::llm::Foo` (or the top-level `muninn_core::Foo`
//! re-exports) for new code; this shim exists for in-crate backwards
//! compatibility during the engine-boundary refactor (PROJEC-T-0065).

pub use muninn_core::llm::*;
