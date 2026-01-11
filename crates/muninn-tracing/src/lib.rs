//! Generic agentic tracing infrastructure.
//!
//! This crate provides the foundation for structured tracing of agentic operations:
//!
//! - **Types**: Generic `Trace`, `Span`, `Event`, and `Timing` structures
//! - **Collector**: Task-local collection via `with_tracing()` and helper functions
//! - **Writer**: JSONL file persistence with daily rotation
//!
//! # Usage
//!
//! ```rust,no_run
//! use muninn_tracing::{with_tracing, start_span, end_span_ok, record_event, TraceWriter, WriterConfig};
//!
//! #[tokio::main]
//! async fn main() {
//!     let writer = TraceWriter::new(WriterConfig::default()).unwrap();
//!
//!     let (result, trace) = with_tracing(async {
//!         start_span("my_operation");
//!
//!         // Do work...
//!         record_event("checkpoint", Some("halfway done"));
//!
//!         end_span_ok();
//!         "done"
//!     }).await;
//!
//!     writer.write(&trace).unwrap();
//! }
//! ```
//!
//! # Domain-Specific Data
//!
//! Attach any serializable data to spans:
//!
//! ```rust,ignore
//! #[derive(Serialize)]
//! struct RouterDecision {
//!     route: String,
//!     confidence: f32,
//! }
//!
//! start_span_with_data("router_decision", RouterDecision {
//!     route: "rlm".to_string(),
//!     confidence: 0.95,
//! });
//! ```

pub mod collector;
pub mod types;
pub mod writer;

// Re-export main types
pub use collector::{
    TraceCollector, add_metadata, current_trace_id, end_span_error, end_span_ok, is_tracing_active,
    record_event, set_timing, start_span, start_span_with_data, with_tracing, with_tracing_id,
};
pub use types::{Event, Span, SpanOutcome, Timing, Trace};
pub use writer::{TraceWriter, WriteError, WriterConfig};
