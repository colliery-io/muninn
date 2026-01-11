---
id: agentic-tracing-observability-for
level: initiative
title: "Agentic Tracing: Observability for Router and RLM Decisions"
short_code: "PROJEC-I-0008"
created_at: 2026-01-10T12:45:49.666985+00:00
updated_at: 2026-01-10T13:27:27.411706+00:00
parent: PROJEC-V-0001
blocked_by: []
archived: true

tags:
  - "#initiative"
  - "#phase/completed"


exit_criteria_met: false
estimated_complexity: M
strategy_id: NULL
initiative_id: agentic-tracing-observability-for
---

# Agentic Tracing: Observability for Router and RLM Decisions Initiative

*This template includes sections for various types of initiatives. Delete sections that don't apply to your specific use case.*

## Context

Muninn operates as a proxy between coding agents (like Claude Code) and LLM backends. Requests flow through:
1. **Router** - Decides whether to passthrough or engage RLM
2. **RLM Engine** - Recursive exploration with tool calls
3. **LLM Backend** - Actual model inference (Groq, Anthropic, local)

Currently, this pipeline operates as a black box. We have basic tracing logs but no structured observability for:
- Why the router made its decision
- What prompts/responses flowed through each RLM cycle
- Token usage and latency per step
- Tool call sequences and results

This makes debugging, optimization, and auditing difficult.

## Goals & Non-Goals

**Goals:**
- Emit structured trace events for router decisions (passthrough vs RLM)
- Capture full RLM cycle data: prompts, responses, tool calls, token usage
- Enable post-hoc analysis of agentic behavior
- Support both real-time streaming (tracing spans) and persistent storage (trace files)

**Non-Goals:**
- Real-time dashboards or visualization (future work)
- Distributed tracing across multiple Muninn instances
- PII redaction or compliance features (separate initiative)

## Key Monitoring Points

### 1. Router Decision Point
When a request arrives at the proxy, the router decides the path:
- **Input**: Incoming request (model, messages, headers)
- **Decision**: Passthrough | RLM | Hybrid
- **Rationale**: Why this decision was made (heuristics matched, explicit flag, etc.)
- **Timestamp**: When decision occurred

### 2. RLM Cycle Tracing
Each RLM execution involves multiple turns:
- **Request**: System prompt, user messages, available tools
- **LLM Response**: Model output, stop reason, token usage
- **Tool Calls**: Which tools, arguments, results
- **Depth Tracking**: Current depth, max depth, budget remaining
- **Final Answer**: The synthesized response returned upstream

## Architecture

### Trace Data Model

```rust
/// Top-level trace for a complete request lifecycle
struct RequestTrace {
    trace_id: String,           // Unique ID for this request
    timestamp: DateTime<Utc>,
    router_decision: RouterDecision,
    rlm_trace: Option<RlmTrace>, // Only present if RLM route taken
    total_duration_ms: u64,
}

/// Router's decision - includes full request for analysis
struct RouterDecision {
    // The full incoming request that triggered this decision
    request: CapturedRequest,
    
    // Decision outcome
    route: Route,               // Passthrough | RLM
    rationale: Vec<String>,     // Why this decision (matched heuristics)
    confidence: f32,            // 0.0-1.0
    duration_ms: u64,
}

/// Full request capture for router analysis
struct CapturedRequest {
    model: String,
    system: Option<String>,     // System prompt if present
    messages: Vec<Message>,     // Full message history
    tools: Vec<ToolDefinition>, // Tools if present
    max_tokens: Option<u32>,
    // Muninn-specific headers/flags
    muninn_flags: Option<MuninnFlags>,
}

/// RLM execution trace (if RLM route taken)
struct RlmTrace {
    cycles: Vec<RlmCycle>,      // Each turn in the recursive loop
    final_answer: Option<String>,
    total_tokens: TokenUsage,
    depth_reached: u32,
    tool_calls_total: u32,
}

/// Single cycle in the RLM loop - captures full prompt/response
struct RlmCycle {
    depth: u32,
    
    // Full request sent to LLM
    system_prompt: Option<String>,
    messages: Vec<Message>,     // Full conversation context
    tools_available: Vec<String>, // Tool names available this cycle
    
    // Full response from LLM
    response_text: String,      // Complete model output
    stop_reason: StopReason,
    tokens: TokenUsage,
    
    // Tool executions this cycle
    tool_calls: Vec<ToolTrace>,
    
    // Timing breakdown
    timing: CycleTiming,
}

/// Granular timing for a single RLM cycle
struct CycleTiming {
    total_ms: u64,
    request_build_ms: u64,      // Time to construct request
    backend_latency_ms: u64,    // Time waiting for LLM response
    response_parse_ms: u64,     // Time to parse response
    tool_execution_ms: u64,     // Total time in tool calls
}

/// Individual tool execution with full input/output
struct ToolTrace {
    name: String,
    arguments: serde_json::Value,  // Full arguments
    result: String,                // Full result text
    success: bool,
    error: Option<String>,
    duration_ms: u64,
}
```

### Storage Strategy

1. **JSONL File**: Append-only trace log in `.muninn/traces/YYYY-MM-DD.jsonl`
2. **Always-on by default**: Every request traced, disable via config if needed
3. **Tracing Spans**: Integrate with `tracing` crate for real-time log output

### Scope Clarification

- **Router traces**: Capture full incoming request + decision rationale
- **RLM traces**: Capture full prompts/responses for each cycle
- **Passthrough**: Only capture router decision, NOT upstream LLM responses (we're tuning Muninn, not Anthropic)

## Detailed Design

### Module Structure

Separate crate for reusability:

```
crates/muninn-tracing/
├── Cargo.toml
├── src/
│   ├── lib.rs          # Module exports + task-local storage
│   ├── types.rs        # Trace data structures (Serialize/Deserialize)
│   ├── collector.rs    # TraceCollector - aggregates trace data
│   └── writer.rs       # Sync writes to .muninn/traces/YYYY-MM-DD.jsonl
```

### Collector Pattern

Uses `tokio::task_local!` for request-scoped storage:

```rust
tokio::task_local! {
    static TRACE_COLLECTOR: RefCell<TraceCollector>;
}

// Scoped to request lifecycle
pub async fn with_tracing<F, T>(f: F) -> (T, RequestTrace)
where
    F: Future<Output = T>,
{
    TRACE_COLLECTOR.scope(RefCell::new(TraceCollector::new()), async {
        let result = f.await;
        let trace = TRACE_COLLECTOR.with(|tc| tc.borrow_mut().finalize());
        (result, trace)
    }).await
}

// Instrumentation points call this
pub fn record_router_decision(decision: RouterDecision) {
    TRACE_COLLECTOR.with(|tc| tc.borrow_mut().record_router_decision(decision));
}
```

### Integration Points

1. **Router (router.rs)**
   - Emit `RouterDecision` trace on every request
   - Include matched heuristics and confidence score

2. **RLM Engine (engine.rs)**
   - Create `RlmTrace` on RLM execution start
   - Add `RlmCycle` on each recursive turn
   - Record tool calls with timing

3. **Proxy (proxy.rs)**
   - Wrap request handling with trace context
   - Write completed trace on response

### API Surface

```rust
/// Collector passed through the request lifecycle
pub struct TraceCollector {
    trace_id: String,
    start_time: Instant,
    router_decision: Option<RouterDecision>,
    rlm_trace: Option<RlmTrace>,
}

impl TraceCollector {
    pub fn new() -> Self;
    pub fn record_router_decision(&mut self, decision: RouterDecision);
    pub fn start_rlm_cycle(&mut self, depth: u32);
    pub fn record_tool_call(&mut self, tool: ToolTrace);
    pub fn end_rlm_cycle(&mut self, response: LlmResponse);
    pub fn finalize(self) -> RequestTrace;
}
```

## Testing Strategy

- Unit tests for trace data structures serialization
- Integration test: mock request through proxy, verify trace file output
- Verify trace collector doesn't add significant latency (<1ms overhead)

## Alternatives Considered

1. **OpenTelemetry Integration**
   - Pro: Industry standard, rich ecosystem
   - Con: Heavy dependency, overkill for single-instance use
   - Decision: Defer to future; use simple JSONL for now

2. **SQLite Trace Storage**
   - Pro: Queryable, structured
   - Con: Additional complexity, schema migrations
   - Decision: Start with JSONL, can add SQLite later if needed

3. **Emit Only (No Storage)**
   - Pro: Simplest implementation
   - Con: Loses historical data for analysis
   - Decision: Need persistent storage for post-hoc debugging

## Implementation Plan

### Task 1: Define Trace Data Types
Create `tracing/types.rs` with all trace structures (RequestTrace, RouterDecision, RlmTrace, RlmCycle, ToolTrace). Derive Serialize/Deserialize.

### Task 2: Implement TraceCollector
Create `tracing/collector.rs` with the collector that aggregates trace data through the request lifecycle.

### Task 3: Implement TraceWriter
Create `tracing/writer.rs` to write completed traces to `.muninn/traces/YYYY-MM-DD.jsonl`.

### Task 4: Integrate with Router
Update `router.rs` to emit RouterDecision traces with matched heuristics.

### Task 5: Integrate with RLM Engine
Update `engine.rs` to record each cycle, tool call, and final answer.

### Task 6: Integrate with Proxy
Update `proxy.rs` to create TraceCollector at request start and write trace on completion.

### Task 7: Add CLI for Trace Inspection
Add `muninn traces` subcommand to list/view recent traces.