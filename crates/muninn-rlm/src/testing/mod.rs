//! Testing utilities for muninn-rlm.
//!
//! This module provides mock implementations and test fixtures for testing
//! LLM-based functionality without making real API calls.
//!
//! # Components
//!
//! - [`fixtures`]: Common test data and request/response builders
//! - [`mock_backend`]: Enhanced mock LLM backend with request capture
//! - [`mock_server`]: HTTP mock server for integration tests

pub mod fixtures;
pub mod mock_backend;
pub mod mock_server;

pub use fixtures::*;
pub use mock_backend::MockLLMBackend;
pub use mock_server::MockLLMServer;
