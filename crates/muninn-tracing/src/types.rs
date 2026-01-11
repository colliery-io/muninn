//! Generic trace data types.
//!
//! These types provide the foundation for structured tracing. Consumers
//! define their domain-specific data and attach it as serializable payloads.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A complete trace representing one logical operation (e.g., one request lifecycle).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trace {
    /// Unique identifier for this trace.
    pub trace_id: String,

    /// When the trace started.
    pub started_at: DateTime<Utc>,

    /// When the trace completed.
    pub ended_at: Option<DateTime<Utc>>,

    /// Total duration in milliseconds.
    pub duration_ms: Option<u64>,

    /// Top-level spans in this trace.
    pub spans: Vec<Span>,

    /// Trace-level metadata (e.g., request info, environment).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, serde_json::Value>,
}

/// A named, timed operation within a trace.
///
/// Spans can be nested (via `children`) and contain events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Span {
    /// Unique identifier for this span within the trace.
    pub span_id: String,

    /// Human-readable name (e.g., "router_decision", "rlm_cycle", "tool_call").
    pub name: String,

    /// When the span started.
    pub started_at: DateTime<Utc>,

    /// When the span completed.
    pub ended_at: Option<DateTime<Utc>>,

    /// Timing breakdown for this span.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timing: Option<Timing>,

    /// Domain-specific data attached to this span.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,

    /// Events that occurred during this span.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<Event>,

    /// Nested child spans.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Span>,

    /// Outcome of the span.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<SpanOutcome>,
}

/// Outcome of a span's execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status")]
pub enum SpanOutcome {
    /// Span completed successfully.
    #[serde(rename = "ok")]
    Ok,

    /// Span completed with an error.
    #[serde(rename = "error")]
    Error { message: String },
}

/// A point-in-time occurrence within a span.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    /// Event name/type (e.g., "request_received", "tool_result").
    pub name: String,

    /// When the event occurred.
    pub timestamp: DateTime<Utc>,

    /// Event-specific data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

/// Timing breakdown for granular performance analysis.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Timing {
    /// Total time in milliseconds.
    pub total_ms: u64,

    /// Named timing segments (e.g., "backend_latency", "parse_time").
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub segments: HashMap<String, u64>,
}

impl Trace {
    /// Create a new trace with the given ID.
    pub fn new(trace_id: impl Into<String>) -> Self {
        Self {
            trace_id: trace_id.into(),
            started_at: Utc::now(),
            ended_at: None,
            duration_ms: None,
            spans: Vec::new(),
            metadata: HashMap::new(),
        }
    }

    /// Generate a new trace with a random UUID.
    pub fn new_random() -> Self {
        Self::new(uuid::Uuid::new_v4().to_string())
    }

    /// Add metadata to the trace.
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Serialize) -> Self {
        if let Ok(v) = serde_json::to_value(value) {
            self.metadata.insert(key.into(), v);
        }
        self
    }

    /// Mark the trace as complete.
    pub fn complete(&mut self) {
        let now = Utc::now();
        self.ended_at = Some(now);
        self.duration_ms = Some((now - self.started_at).num_milliseconds().max(0) as u64);
    }

    /// Add a span to the trace.
    pub fn add_span(&mut self, span: Span) {
        self.spans.push(span);
    }
}

impl Span {
    /// Create a new span.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            span_id: uuid::Uuid::new_v4().to_string(),
            name: name.into(),
            started_at: Utc::now(),
            ended_at: None,
            timing: None,
            data: None,
            events: Vec::new(),
            children: Vec::new(),
            outcome: None,
        }
    }

    /// Attach domain-specific data to the span.
    pub fn with_data(mut self, data: impl Serialize) -> Self {
        self.data = serde_json::to_value(data).ok();
        self
    }

    /// Mark the span as complete with success.
    pub fn complete_ok(&mut self) {
        self.ended_at = Some(Utc::now());
        self.outcome = Some(SpanOutcome::Ok);
        self.calculate_timing();
    }

    /// Mark the span as complete with an error.
    pub fn complete_error(&mut self, message: impl Into<String>) {
        self.ended_at = Some(Utc::now());
        self.outcome = Some(SpanOutcome::Error {
            message: message.into(),
        });
        self.calculate_timing();
    }

    /// Add an event to the span.
    pub fn add_event(&mut self, event: Event) {
        self.events.push(event);
    }

    /// Record an event with the given name and optional data.
    pub fn record_event(&mut self, name: impl Into<String>, data: Option<impl Serialize>) {
        self.events.push(Event {
            name: name.into(),
            timestamp: Utc::now(),
            data: data.and_then(|d| serde_json::to_value(d).ok()),
        });
    }

    /// Add a child span.
    pub fn add_child(&mut self, child: Span) {
        self.children.push(child);
    }

    /// Set timing breakdown.
    pub fn set_timing(&mut self, timing: Timing) {
        self.timing = Some(timing);
    }

    fn calculate_timing(&mut self) {
        if let Some(ended) = self.ended_at {
            let total = (ended - self.started_at).num_milliseconds().max(0) as u64;
            if let Some(ref mut timing) = self.timing {
                timing.total_ms = total;
            } else {
                self.timing = Some(Timing {
                    total_ms: total,
                    segments: HashMap::new(),
                });
            }
        }
    }
}

impl Timing {
    /// Create a new timing with total milliseconds.
    pub fn new(total_ms: u64) -> Self {
        Self {
            total_ms,
            segments: HashMap::new(),
        }
    }

    /// Add a named timing segment.
    pub fn with_segment(mut self, name: impl Into<String>, ms: u64) -> Self {
        self.segments.insert(name.into(), ms);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trace_creation() {
        let trace = Trace::new_random();
        assert!(!trace.trace_id.is_empty());
        assert!(trace.ended_at.is_none());
    }

    #[test]
    fn test_span_with_data() {
        #[derive(Serialize)]
        struct MyData {
            value: i32,
        }

        let span = Span::new("test_span").with_data(MyData { value: 42 });
        assert!(span.data.is_some());
    }

    #[test]
    fn test_trace_serialization() {
        let mut trace = Trace::new("test-123");
        trace.metadata.insert(
            "request_id".to_string(),
            serde_json::Value::String("abc".to_string()),
        );

        let mut span = Span::new("operation");
        span.record_event("started", None::<()>);
        span.complete_ok();
        trace.add_span(span);
        trace.complete();

        let json = serde_json::to_string_pretty(&trace).unwrap();
        assert!(json.contains("test-123"));
        assert!(json.contains("operation"));
    }
}
