//! Task-local trace collector.
//!
//! Provides infrastructure for collecting trace data scoped to async tasks.
//! Consumers create spans and events; the collector aggregates them into a Trace.

use std::cell::RefCell;
use std::mem;
use std::time::Instant;

use crate::types::{Span, Timing, Trace};

tokio::task_local! {
    static CURRENT_COLLECTOR: RefCell<TraceCollector>;
}

/// Collects trace data during a request/operation lifecycle.
#[derive(Debug)]
pub struct TraceCollector {
    trace: Trace,
    start_instant: Instant,
    span_stack: Vec<Span>,
}

impl TraceCollector {
    /// Create a new collector with a random trace ID.
    pub fn new() -> Self {
        Self {
            trace: Trace::new_random(),
            start_instant: Instant::now(),
            span_stack: Vec::new(),
        }
    }

    /// Create a new collector with a specific trace ID.
    pub fn with_trace_id(trace_id: impl Into<String>) -> Self {
        Self {
            trace: Trace::new(trace_id),
            start_instant: Instant::now(),
            span_stack: Vec::new(),
        }
    }

    /// Add metadata to the trace.
    pub fn add_metadata(&mut self, key: impl Into<String>, value: impl serde::Serialize) {
        if let Ok(v) = serde_json::to_value(value) {
            self.trace.metadata.insert(key.into(), v);
        }
    }

    /// Start a new span. Must be paired with `end_span()`.
    pub fn start_span(&mut self, name: impl Into<String>) {
        self.span_stack.push(Span::new(name));
    }

    /// Start a span with attached data.
    pub fn start_span_with_data(&mut self, name: impl Into<String>, data: impl serde::Serialize) {
        self.span_stack.push(Span::new(name).with_data(data));
    }

    /// Record an event in the current span.
    pub fn record_event(&mut self, name: impl Into<String>, data: Option<impl serde::Serialize>) {
        if let Some(span) = self.span_stack.last_mut() {
            span.record_event(name, data);
        }
    }

    /// Set timing breakdown for the current span.
    pub fn set_current_timing(&mut self, timing: Timing) {
        if let Some(span) = self.span_stack.last_mut() {
            span.set_timing(timing);
        }
    }

    /// End the current span successfully.
    pub fn end_span_ok(&mut self) {
        if let Some(mut span) = self.span_stack.pop() {
            span.complete_ok();
            self.attach_span(span);
        }
    }

    /// End the current span with an error.
    pub fn end_span_error(&mut self, message: impl Into<String>) {
        if let Some(mut span) = self.span_stack.pop() {
            span.complete_error(message);
            self.attach_span(span);
        }
    }

    /// Add a complete span directly.
    pub fn add_span(&mut self, span: Span) {
        self.attach_span(span);
    }

    fn attach_span(&mut self, span: Span) {
        // If there's a parent span on the stack, add as child; otherwise add to trace
        if let Some(parent) = self.span_stack.last_mut() {
            parent.add_child(span);
        } else {
            self.trace.add_span(span);
        }
    }

    /// Finalize the trace and return it.
    pub fn finalize(mut self) -> Trace {
        // Close any unclosed spans
        while let Some(mut span) = self.span_stack.pop() {
            span.complete_error("span not explicitly closed");
            self.attach_span(span);
        }

        self.trace.complete();
        self.trace
    }

    /// Get the trace ID.
    pub fn trace_id(&self) -> &str {
        &self.trace.trace_id
    }

    /// Get elapsed time since trace started.
    pub fn elapsed_ms(&self) -> u64 {
        self.start_instant.elapsed().as_millis() as u64
    }
}

impl Default for TraceCollector {
    fn default() -> Self {
        Self::new()
    }
}

/// Execute an async operation with tracing enabled.
///
/// Returns both the operation result and the completed trace.
pub async fn with_tracing<F, T>(f: F) -> (T, Trace)
where
    F: std::future::Future<Output = T>,
{
    CURRENT_COLLECTOR
        .scope(RefCell::new(TraceCollector::new()), async {
            let result = f.await;
            let trace = CURRENT_COLLECTOR.with(|tc| {
                let collector = mem::take(&mut *tc.borrow_mut());
                collector.finalize()
            });
            (result, trace)
        })
        .await
}

/// Execute an async operation with tracing, using a specific trace ID.
pub async fn with_tracing_id<F, T>(trace_id: impl Into<String>, f: F) -> (T, Trace)
where
    F: std::future::Future<Output = T>,
{
    let collector = TraceCollector::with_trace_id(trace_id);
    CURRENT_COLLECTOR
        .scope(RefCell::new(collector), async {
            let result = f.await;
            let trace = CURRENT_COLLECTOR.with(|tc| {
                let collector = mem::take(&mut *tc.borrow_mut());
                collector.finalize()
            });
            (result, trace)
        })
        .await
}

/// Check if tracing is active in the current task.
pub fn is_tracing_active() -> bool {
    CURRENT_COLLECTOR.try_with(|_| ()).is_ok()
}

/// Add metadata to the current trace (no-op if tracing not active).
pub fn add_metadata(key: impl Into<String>, value: impl serde::Serialize) {
    let _ = CURRENT_COLLECTOR.try_with(|tc| tc.borrow_mut().add_metadata(key, value));
}

/// Start a new span in the current trace (no-op if tracing not active).
pub fn start_span(name: impl Into<String>) {
    let _ = CURRENT_COLLECTOR.try_with(|tc| tc.borrow_mut().start_span(name));
}

/// Start a span with data in the current trace (no-op if tracing not active).
pub fn start_span_with_data(name: impl Into<String>, data: impl serde::Serialize) {
    let _ = CURRENT_COLLECTOR.try_with(|tc| tc.borrow_mut().start_span_with_data(name, data));
}

/// Record an event in the current span (no-op if tracing not active).
pub fn record_event(name: impl Into<String>, data: Option<impl serde::Serialize>) {
    let _ = CURRENT_COLLECTOR.try_with(|tc| tc.borrow_mut().record_event(name, data));
}

/// Set timing for the current span (no-op if tracing not active).
pub fn set_timing(timing: Timing) {
    let _ = CURRENT_COLLECTOR.try_with(|tc| tc.borrow_mut().set_current_timing(timing));
}

/// End the current span successfully (no-op if tracing not active).
pub fn end_span_ok() {
    let _ = CURRENT_COLLECTOR.try_with(|tc| tc.borrow_mut().end_span_ok());
}

/// End the current span with an error (no-op if tracing not active).
pub fn end_span_error(message: impl Into<String>) {
    let _ = CURRENT_COLLECTOR.try_with(|tc| tc.borrow_mut().end_span_error(message));
}

/// Get the current trace ID (returns None if tracing not active).
pub fn current_trace_id() -> Option<String> {
    CURRENT_COLLECTOR
        .try_with(|tc| tc.borrow().trace_id().to_string())
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_with_tracing() {
        let (result, trace) = with_tracing(async {
            start_span("outer");
            record_event("something_happened", Some("details"));
            start_span("inner");
            end_span_ok();
            end_span_ok();
            42
        })
        .await;

        assert_eq!(result, 42);
        assert_eq!(trace.spans.len(), 1);
        assert_eq!(trace.spans[0].name, "outer");
        assert_eq!(trace.spans[0].children.len(), 1);
        assert_eq!(trace.spans[0].children[0].name, "inner");
    }

    #[tokio::test]
    async fn test_no_tracing_context() {
        // These should be no-ops, not panics
        start_span("orphan");
        record_event("orphan_event", None::<()>);
        end_span_ok();

        assert!(!is_tracing_active());
    }
}
