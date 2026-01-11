---
id: rlm-gateway-core-proxy-layer-and
level: initiative
title: "RLM Gateway Core: Proxy Layer and Recursive Completion Engine"
short_code: "PROJEC-I-0002"
created_at: 2026-01-08T02:30:12.213780+00:00
updated_at: 2026-01-08T22:29:32.531958+00:00
parent: PROJEC-V-0001
blocked_by: []
archived: true

tags:
  - "#initiative"
  - "#phase/completed"


exit_criteria_met: false
estimated_complexity: L
strategy_id: NULL
initiative_id: rlm-gateway-core-proxy-layer-and
---

# RLM Gateway Core: Proxy Layer and Recursive Completion Engine Initiative

*This template includes sections for various types of initiatives. Delete sections that don't apply to your specific use case.*

## Context

The Recursive Language Model (RLM) pattern transforms how LLMs interact with codebases. Instead of stuffing context into a single prompt, RLM treats **context tokens as compute** - the LLM can recursively explore, execute code, and spawn sub-queries to gather exactly the information it needs.

**Core Insight from rlmgw:**
```
Traditional: llm.completion(giant_context + question) → answer
RLM:         rlm.completion(question) → [explore, execute, sub-query]* → answer
```

This initiative implements the gateway/proxy layer that makes RLM transparent to clients. A Claude Code session or API client calls the gateway, which orchestrates recursive exploration behind the scenes.

**Reference:**
- rlmgw repository: Original RLM implementation concept
- Vision (PROJEC-V-0001): Muninn's privacy-first recursive context gateway
- PROJEC-I-0001: Code Graph Infrastructure (provides structural queries)
- PROJEC-I-0003: Tool Environment (provides REPL and tools)

## Goals & Non-Goals

**Goals:**
- Implement transparent proxy layer for LLM API calls (Anthropic API compatible)
- Build recursive completion engine that orchestrates exploration loops
- Support sub-query spawning with budget management
- Enable streaming responses with incremental context discovery
- Provide backend-agnostic design (Claude, OpenAI, local models)
- Integrate with Claude Code via MCP protocol

**Non-Goals:**
- Tool implementation details (covered by PROJEC-I-0003)
- Code graph queries (covered by PROJEC-I-0001)
- Memory persistence (covered by ADR-001)
- Cloud/SaaS deployment (CLI-first per vision)

## Requirements

### Functional Requirements

**Proxy Layer:**
- REQ-001: Accept Anthropic Messages API format (`/v1/messages`)
- REQ-002: Forward requests to configured LLM backend
- REQ-003: Intercept tool_use responses to enable recursive exploration
- REQ-004: Support streaming (`stream: true`) with SSE
- REQ-005: Pass through non-RLM requests transparently

**Recursive Engine:**
- REQ-006: Execute recursive exploration loop until termination condition
- REQ-007: Spawn sub-queries with isolated context
- REQ-008: Aggregate results from sub-queries into parent context
- REQ-009: Track exploration depth and enforce limits
- REQ-010: Support cancellation of in-progress exploration

**Budget Management:**
- REQ-011: Track token usage across recursive calls
- REQ-012: Enforce configurable token budget per request
- REQ-013: Enforce configurable time budget per request
- REQ-014: Enforce maximum recursion depth

**Backend Support:**
- REQ-015: Support Anthropic API as primary backend
- REQ-016: Support OpenAI-compatible APIs
- REQ-017: Support local models via OpenAI-compatible interface (ollama, llama.cpp)

### Non-Functional Requirements
- NFR-001: Sub-100ms overhead for proxy pass-through (non-recursive)
- NFR-002: Support concurrent recursive explorations
- NFR-003: Graceful degradation when backend is unavailable
- NFR-004: All data stays local (no telemetry, no external calls except to configured LLM)

## Use Cases

### UC-1: Claude Code Session via Muninn Proxy
- **Actor**: Developer using Claude Code
- **Scenario**:
  1. Developer configures Claude Code to use Muninn proxy endpoint
  2. Developer asks: "How does the authentication flow work?"
  3. Muninn intercepts request, invokes RLM loop
  4. RLM explores code graph, reads relevant files, spawns sub-queries
  5. Aggregated context returned to Claude Code transparently
- **Expected Outcome**: Claude Code receives enriched response without knowing about recursive exploration

### UC-2: Direct API Call with Recursive Exploration
- **Actor**: Script/application using Anthropic API
- **Scenario**:
  1. Application calls `POST /v1/messages` to Muninn proxy
  2. Request includes `muninn: { recursive: true, budget: { tokens: 50000 } }`
  3. Muninn executes recursive exploration within budget
  4. Returns final response in standard Anthropic format
- **Expected Outcome**: Application receives response; exploration details available in metadata

### UC-3: Streaming Response with Incremental Discovery
- **Actor**: Interactive CLI tool
- **Scenario**:
  1. Tool calls Muninn with `stream: true`
  2. Muninn streams partial responses as exploration progresses
  3. Tool displays real-time progress: "Exploring auth module... Found 3 relevant functions..."
  4. Final answer streams when exploration completes
- **Expected Outcome**: User sees exploration progress in real-time

### UC-4: Sub-Query for Deep Analysis
- **Actor**: RLM engine (internal)
- **Scenario**:
  1. Parent query asks about error handling
  2. RLM identifies need to understand `Result` type usage
  3. Spawns sub-query: "What error types are used in this module?"
  4. Sub-query explores, returns findings
  5. Parent query incorporates sub-query results
- **Expected Outcome**: Complex questions answered through decomposition

## Architecture

### Overview

```
                                    ┌─────────────────────┐
                                    │   LLM Backend       │
                                    │ (Anthropic/OpenAI)  │
                                    └──────────▲──────────┘
                                               │
┌──────────────┐    ┌──────────────────────────┴───────────────────────────┐
│ Claude Code  │───▶│                    Muninn Gateway                    │
│   or API     │◀───│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐  │
│   Client     │    │  │   Proxy     │  │  Recursive  │  │   Budget    │  │
└──────────────┘    │  │   Layer     │──│   Engine    │──│   Manager   │  │
                    │  └─────────────┘  └──────┬──────┘  └─────────────┘  │
                    └──────────────────────────┼───────────────────────────┘
                                               │
                                               ▼
                    ┌──────────────────────────────────────────────────────┐
                    │              Tool Environment (PROJEC-I-0003)        │
                    │  ┌─────────┐  ┌─────────┐  ┌─────────┐  ┌─────────┐ │
                    │  │  REPL   │  │  Graph  │  │  Files  │  │ Memory  │ │
                    │  │ Execute │  │  Query  │  │  Read   │  │  Query  │ │
                    │  └─────────┘  └─────────┘  └─────────┘  └─────────┘ │
                    └──────────────────────────────────────────────────────┘
```

### Components

**1. Proxy Layer**
- HTTP server accepting Anthropic Messages API format
- Request validation and normalization
- Response transformation (add Muninn metadata)
- Streaming support via SSE

**2. Recursive Engine**
- Core loop: receive response → check for tool_use → execute tools → continue or terminate
- Sub-query spawner with context isolation
- Result aggregator combining sub-query findings
- Termination detection (answer ready, budget exhausted, max depth)

**3. Budget Manager**
- Token counter (input + output across all recursive calls)
- Time tracker with configurable timeout
- Depth tracker with configurable max
- Cost estimation for paid APIs

**4. Backend Abstraction**
- `LLMBackend` trait for provider abstraction
- `AnthropicBackend`: Native Anthropic API
- `OpenAIBackend`: OpenAI-compatible (including local models)
- Request/response translation between formats

### Sequence: Recursive Exploration

```
Client          Proxy           Engine          Tools           Backend
  │               │               │               │               │
  │──request────▶│               │               │               │
  │               │──init────────▶│               │               │
  │               │               │──────────────────────────────▶│
  │               │               │◀─────────tool_use─────────────│
  │               │               │──execute────▶│               │
  │               │               │◀───result────│               │
  │               │               │──────────────────────────────▶│
  │               │               │◀─────────tool_use─────────────│
  │               │               │──execute────▶│               │
  │               │               │◀───result────│               │
  │               │               │──────────────────────────────▶│
  │               │               │◀─────────end_turn─────────────│
  │               │◀──response────│               │               │
  │◀──response────│               │               │               │
```

## Detailed Design

### Core Traits

```rust
/// Backend abstraction for LLM providers
#[async_trait]
pub trait LLMBackend: Send + Sync {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse>;
    async fn complete_stream(&self, request: CompletionRequest) -> Result<ResponseStream>;
    fn name(&self) -> &str;
}

/// Tool execution environment
#[async_trait]
pub trait ToolEnvironment: Send + Sync {
    async fn execute_tool(&self, tool_use: &ToolUse) -> Result<ToolResult>;
    fn available_tools(&self) -> Vec<ToolDefinition>;
}
```

### Request Flow

```rust
pub struct RecursiveEngine {
    backend: Arc<dyn LLMBackend>,
    tools: Arc<dyn ToolEnvironment>,
    budget: BudgetManager,
}

impl RecursiveEngine {
    pub async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse> {
        let mut context = ExplorationContext::new(request);
        
        loop {
            // Check budget
            self.budget.check(&context)?;
            
            // Call LLM
            let response = self.backend.complete(context.build_request()).await?;
            
            // Check for tool use
            match response.stop_reason {
                StopReason::EndTurn => {
                    return Ok(context.finalize(response));
                }
                StopReason::ToolUse => {
                    for tool_use in response.tool_uses() {
                        let result = self.tools.execute_tool(&tool_use).await?;
                        context.add_tool_result(tool_use.id, result);
                    }
                }
                StopReason::MaxTokens => {
                    // Continue with truncation handling
                    context.handle_truncation();
                }
            }
            
            context.increment_depth();
        }
    }
}
```

### Budget Configuration

```rust
pub struct BudgetConfig {
    /// Maximum total tokens (input + output) across all recursive calls
    pub max_tokens: Option<u64>,
    /// Maximum wall-clock time for entire exploration
    pub max_duration: Option<Duration>,
    /// Maximum recursion depth
    pub max_depth: Option<u32>,
    /// Maximum number of tool executions
    pub max_tool_calls: Option<u32>,
}

impl Default for BudgetConfig {
    fn default() -> Self {
        Self {
            max_tokens: Some(100_000),
            max_duration: Some(Duration::from_secs(300)),
            max_depth: Some(10),
            max_tool_calls: Some(50),
        }
    }
}
```

### Sub-Query Spawning

```rust
pub struct SubQuery {
    /// The question for the sub-query
    pub question: String,
    /// Subset of tools available to sub-query
    pub allowed_tools: Vec<String>,
    /// Budget allocation for this sub-query
    pub budget: BudgetConfig,
    /// Whether results should be summarized before return
    pub summarize: bool,
}

impl RecursiveEngine {
    async fn spawn_subquery(&self, subquery: SubQuery) -> Result<String> {
        // Create isolated context
        let sub_engine = self.with_budget(subquery.budget);
        
        // Execute sub-query
        let response = sub_engine.complete(CompletionRequest {
            messages: vec![Message::user(&subquery.question)],
            tools: self.tools.filter(&subquery.allowed_tools),
            ..Default::default()
        }).await?;
        
        // Optionally summarize
        if subquery.summarize {
            self.summarize(&response.content).await
        } else {
            Ok(response.content)
        }
    }
}
```

### Anthropic API Compatibility

The proxy accepts standard Anthropic Messages API format with optional Muninn extensions:

```json
{
  "model": "claude-sonnet-4-20250514",
  "max_tokens": 4096,
  "messages": [{"role": "user", "content": "How does auth work?"}],
  "muninn": {
    "recursive": true,
    "budget": {
      "max_tokens": 50000,
      "max_depth": 5
    }
  }
}
```

Response includes exploration metadata:

```json
{
  "content": [{"type": "text", "text": "..."}],
  "stop_reason": "end_turn",
  "muninn": {
    "exploration": {
      "depth_reached": 3,
      "tokens_used": 24500,
      "tool_calls": 12,
      "duration_ms": 8500
    }
  }
}
```

## Testing Strategy

### Unit Testing
- Mock LLM backend returning scripted responses
- Budget manager edge cases (exactly at limit, over limit)
- Request/response transformation validation

### Integration Testing
- End-to-end with real LLM backend (Anthropic API)
- Streaming response handling
- Concurrent request handling

### Mock Backend for Development
```rust
pub struct MockBackend {
    responses: Vec<CompletionResponse>,
}

impl LLMBackend for MockBackend {
    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse> {
        // Return scripted responses for deterministic testing
    }
}
```

## Alternatives Considered

### 1. Direct rlmgw Port
**Rejected**: rlmgw is Python-based. Muninn targets Rust for performance and to align with the code graph infrastructure (PROJEC-I-0001). Clean-room implementation allows Rust-native async and better integration.

### 2. LangChain/LlamaIndex Integration
**Rejected**: These frameworks add significant dependencies and abstractions. Muninn needs a minimal, focused implementation that can be embedded in CLI tools without heavy runtimes.

### 3. MCP-Only (No Proxy)
**Considered**: Could expose RLM purely through MCP tools. However, transparent proxy enables drop-in replacement for existing Claude Code workflows and supports non-MCP clients.

### 4. Single-Threaded Exploration
**Rejected**: Parallel sub-queries can significantly speed up complex explorations. The async design supports concurrent tool execution and sub-queries.

## Implementation Plan

### Phase 1: Core Types and Traits
- [ ] Define `CompletionRequest` / `CompletionResponse` types (Anthropic-compatible)
- [ ] Define `LLMBackend` trait
- [ ] Define `ToolEnvironment` trait  
- [ ] Implement `MockBackend` for testing

### Phase 2: Anthropic Backend
- [ ] Implement `AnthropicBackend` with reqwest
- [ ] Handle authentication (API key from env/config)
- [ ] Implement streaming response handling
- [ ] Error handling and retries

### Phase 3: Recursive Engine
- [ ] Implement `ExplorationContext` for tracking state
- [ ] Implement core recursive loop
- [ ] Tool use detection and execution dispatch
- [ ] Termination condition handling

### Phase 4: Budget Management
- [ ] Token counting (estimate from messages)
- [ ] Time tracking
- [ ] Depth tracking
- [ ] Budget exceeded error handling

### Phase 5: Proxy Layer
- [ ] HTTP server with axum
- [ ] `/v1/messages` endpoint
- [ ] Request validation
- [ ] Muninn extension parsing
- [ ] Response metadata injection

### Phase 6: Sub-Query Support
- [ ] `spawn_subquery` tool implementation
- [ ] Context isolation for sub-queries
- [ ] Result aggregation
- [ ] Budget partitioning

### Phase 7: OpenAI Backend (Stretch)
- [ ] Implement `OpenAIBackend`
- [ ] Request/response format translation
- [ ] Support for local models (ollama)

### Dependencies
- `tokio` - async runtime
- `axum` - HTTP server
- `reqwest` - HTTP client
- `serde` / `serde_json` - serialization
- `tracing` - logging
- `async-trait` - async trait support