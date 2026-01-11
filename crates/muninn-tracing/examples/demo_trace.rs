//! Demo of the tracing system simulating a proxy -> router -> RLM flow.

use muninn_tracing::{
    TraceWriter, WriterConfig, end_span_ok, record_event, start_span, start_span_with_data,
    with_tracing,
};
use serde::Serialize;
use std::time::Duration;

// Domain-specific trace data (similar to what proxy.rs defines)
#[derive(Serialize)]
struct ProxyRequestData {
    model: String,
    streaming: bool,
    message_count: usize,
}

#[derive(Serialize)]
struct RouterDecisionData {
    strategy: String,
    decision: String,
    reason: Option<String>,
    decision_time_ms: u64,
}

#[derive(Serialize)]
struct RlmIterationData {
    depth: u32,
    llm_latency_ms: u64,
    input_tokens: u32,
    output_tokens: u32,
}

#[derive(Serialize)]
struct ToolExecutionData {
    tool_name: String,
    success: bool,
    execution_time_ms: u64,
}

async fn simulate_tool_call(name: &str) {
    let data = ToolExecutionData {
        tool_name: name.to_string(),
        success: true,
        execution_time_ms: 15,
    };
    start_span_with_data("tool_execution", &data);
    // Simulate some work
    std::thread::sleep(Duration::from_millis(5));
    end_span_ok();
}

async fn simulate_rlm_iteration(depth: u32) {
    let data = RlmIterationData {
        depth,
        llm_latency_ms: 250,
        input_tokens: 1500,
        output_tokens: 200,
    };
    start_span_with_data("rlm_iteration", &data);

    // Simulate tool calls
    simulate_tool_call("read_file").await;
    simulate_tool_call("search_files").await;

    end_span_ok();
}

async fn simulate_request() -> String {
    // Proxy request span
    let request_data = ProxyRequestData {
        model: "qwen/qwen3-32b".to_string(),
        streaming: false,
        message_count: 3,
    };
    start_span_with_data("proxy_request", &request_data);

    // Router decision
    let router_data = RouterDecisionData {
        strategy: "Llm".to_string(),
        decision: "rlm".to_string(),
        reason: Some("Exploration keyword: 'understand'".to_string()),
        decision_time_ms: 1,
    };
    start_span_with_data("router_decision", &router_data);
    end_span_ok();

    // RLM cycle
    start_span("rlm_cycle");

    // Simulate 2 iterations
    simulate_rlm_iteration(0).await;
    simulate_rlm_iteration(1).await;

    record_event(
        "rlm_completion",
        Some(&serde_json::json!({
            "termination_reason": "final_answer_tool",
            "depth_reached": 2,
            "tool_calls": 4,
            "tokens_used": 3400
        })),
    );
    end_span_ok();

    // End proxy request
    record_event(
        "proxy_completion",
        Some(&serde_json::json!({
            "handling": "rlm",
            "success": true,
            "total_time_ms": 520
        })),
    );
    end_span_ok();

    "The function is defined in src/engine.rs at line 42".to_string()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create a trace writer
    let config = WriterConfig::new(".muninn/traces");
    let writer = TraceWriter::new(config)?;

    // Run the simulated request with tracing
    let (result, trace) = with_tracing(simulate_request()).await;

    // Write the trace
    writer.write(&trace)?;

    // Print summary
    println!("Request result: {}", result);
    println!("\nTrace ID: {}", trace.trace_id);
    println!("Duration: {}ms", trace.duration_ms.unwrap_or(0));
    println!("Spans: {}", trace.spans.len());

    // Print the trace as JSON for inspection
    println!("\n--- Full Trace JSON ---");
    println!("{}", serde_json::to_string_pretty(&trace)?);

    println!(
        "\n--- Trace written to {} ---",
        writer.current_file_path().display()
    );

    Ok(())
}
