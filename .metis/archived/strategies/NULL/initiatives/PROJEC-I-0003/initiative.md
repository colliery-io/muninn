---
id: rlm-tool-environment-repl-tools
level: initiative
title: "RLM Tool Environment: REPL, Tools, and Context Aggregation"
short_code: "PROJEC-I-0003"
created_at: 2026-01-08T02:30:12.269098+00:00
updated_at: 2026-01-09T02:33:32.440734+00:00
parent: PROJEC-V-0001
blocked_by: []
archived: true

tags:
  - "#initiative"
  - "#phase/completed"


exit_criteria_met: false
estimated_complexity: L
strategy_id: NULL
initiative_id: rlm-tool-environment-repl-tools
---

# RLM Tool Environment: REPL, Tools, and Context Aggregation Initiative

*This template includes sections for various types of initiatives. Delete sections that don't apply to your specific use case.*

## Context

The RLM recursive engine (PROJEC-I-0002) needs tools to explore and understand code. This initiative provides the **tool environment** - the REPL for code execution, the tool registry, and the context aggregation layer that combines results from multiple explorations.

**Core Concept:**
The LLM doesn't just read files - it can execute queries, run code, search memory, and spawn sub-explorations. Each tool returns structured results that get aggregated into coherent context.

**Reference:**
- PROJEC-I-0002: RLM Gateway Core (consumes this tool environment)
- PROJEC-I-0001: Code Graph Infrastructure (graph_query tool)
- ADR-001: Memory layers (memory_query tool)
- Metis integration: Task context and ADR lookup

## Goals & Non-Goals

**Goals:**
- Define tool interface that RLM engine can execute
- Implement core exploration tools (file read, graph query, search)
- Implement REPL for safe code execution
- Implement memory tools (query session memory, curated knowledge)
- Build context aggregation layer to combine tool results
- Support Metis integration for task/ADR context
- Expose tools via MCP for Claude Code direct access

**Non-Goals:**
- Tool execution orchestration (handled by PROJEC-I-0002)
- Graph storage/queries implementation (handled by PROJEC-I-0001)
- Memory persistence (handled by ADR-001)
- Arbitrary code execution outside sandbox

## Requirements

### Tool Categories

| Category | Tools | Source |
|----------|-------|--------|
| **File System** | read_file, list_directory, search_files | Built-in |
| **Code Graph** | graph_query, find_callers, find_implementations | PROJEC-I-0001 |
| **Memory** | query_memory, search_knowledge, get_context | ADR-001 |
| **REPL** | execute_code, evaluate_expression | Sandboxed |
| **Metis** | get_task, list_adrs, search_decisions | MCP integration |
| **Meta** | spawn_subquery, summarize, ask_clarification | PROJEC-I-0002 |

### Functional Requirements

**File System Tools:**
- REQ-001: Read file contents with optional line range
- REQ-002: List directory with glob patterns
- REQ-003: Search files by content (ripgrep-style)
- REQ-004: Respect .gitignore and configurable exclusions

**Code Graph Tools:**
- REQ-005: Execute Cypher queries against code graph
- REQ-006: Find callers/callees of a function
- REQ-007: Find implementations of trait/interface
- REQ-008: Get symbol definition with context

**Memory Tools:**
- REQ-009: Query session memory (JSONL)
- REQ-010: Search curated knowledge (markdown)
- REQ-011: Get relevant context for current task

**REPL Tools:**
- REQ-012: Execute shell commands (sandboxed)
- REQ-013: Run language-specific code (Rust tests, Python scripts)
- REQ-014: Capture stdout/stderr with timeout
- REQ-015: Prevent destructive operations (rm -rf, etc.)

**Context Aggregation:**
- REQ-016: Combine results from multiple tool calls
- REQ-017: Deduplicate overlapping content
- REQ-018: Rank results by relevance to query
- REQ-019: Truncate to fit token budget

### Non-Functional Requirements
- NFR-001: Tool execution under 5 seconds (except REPL with explicit timeout)
- NFR-002: REPL sandboxed via seccomp/landlock (Linux) or sandbox-exec (macOS)
- NFR-003: No network access from REPL sandbox
- NFR-004: Memory-safe result handling (bound output size)

## Use Cases

### UC-1: Graph-Guided File Discovery
- **Actor**: RLM engine exploring "how does auth work?"
- **Flow**:
  1. `graph_query`: Find functions with "auth" in name
  2. `find_callers`: Who calls `authenticate()`?
  3. `read_file`: Read the relevant function bodies
  4. Context aggregator combines into coherent answer
- **Result**: Targeted file reads instead of dumping entire codebase

### UC-2: Memory-Augmented Response
- **Actor**: RLM engine answering "why did we choose JWT?"
- **Flow**:
  1. `search_knowledge`: Search curated knowledge for "JWT"
  2. `list_adrs`: Find architecture decision records about auth
  3. `query_memory`: Check session memory for recent discussions
  4. Synthesize answer from multiple memory sources
- **Result**: Answer includes historical context and decisions

### UC-3: REPL Verification
- **Actor**: RLM engine checking "does this regex match?"
- **Flow**:
  1. `execute_code`: Run Python snippet to test regex
  2. Capture output showing match results
  3. Include verified output in response
- **Result**: Concrete execution instead of speculation

### UC-4: Context Aggregation Under Budget
- **Actor**: RLM with 10k token budget remaining
- **Flow**:
  1. Multiple tools return 20k tokens of results
  2. Aggregator ranks by relevance to original query
  3. Deduplicates overlapping file content
  4. Truncates to 10k tokens, preserving most relevant
- **Result**: Best context fits within budget

## Architecture

### Overview

```
┌─────────────────────────────────────────────────────────────────────┐
│                    Tool Environment                                  │
├─────────────────────────────────────────────────────────────────────┤
│  ┌───────────────────────────────────────────────────────────────┐  │
│  │                    Context Aggregator                         │  │
│  │   (combines, dedupes, ranks, truncates tool results)          │  │
│  └───────────────────────────────────────────────────────────────┘  │
│                              ▲                                       │
│          ┌───────────────────┼───────────────────┐                  │
│          │                   │                   │                  │
│  ┌───────┴───────┐  ┌───────┴───────┐  ┌───────┴───────┐          │
│  │  Tool Registry │  │  Tool Registry │  │  Tool Registry │          │
│  │   (File/Graph) │  │    (Memory)    │  │     (REPL)     │          │
│  └───────┬───────┘  └───────┬───────┘  └───────┬───────┘          │
│          │                   │                   │                  │
│  ┌───────┴───────┐  ┌───────┴───────┐  ┌───────┴───────┐          │
│  │   read_file   │  │ query_memory  │  │ execute_code  │          │
│  │  graph_query  │  │search_knowledge│  │   (sandbox)   │          │
│  │ search_files  │  │  get_context  │  │               │          │
│  └───────────────┘  └───────────────┘  └───────────────┘          │
└─────────────────────────────────────────────────────────────────────┘
         │                     │                     │
         ▼                     ▼                     ▼
    ┌─────────┐          ┌─────────┐          ┌─────────┐
    │  File   │          │ .muninn │          │ Sandbox │
    │ System  │          │ memory/ │          │ Process │
    └─────────┘          └─────────┘          └─────────┘
         │
         ▼
    ┌─────────┐
    │ graph.db│
    │(graphqlite)
    └─────────┘
```

### Components

**1. Tool Registry**
- Central registration of all available tools
- Tool metadata (name, description, parameters, schema)
- Tool discovery for LLM (generates tool definitions)

**2. Individual Tools**
- Implement `Tool` trait
- Validate inputs, execute, return structured result
- Handle errors gracefully with informative messages

**3. REPL Sandbox**
- Process isolation (fork + sandbox)
- Resource limits (CPU, memory, time)
- Blocked syscalls (network, destructive fs ops)
- Captured output with size limits

**4. Context Aggregator**
- Receives results from multiple tool executions
- Scores relevance to original query
- Deduplicates (same file read twice)
- Truncates to fit token budget
- Formats for LLM consumption

## Detailed Design

### Tool Trait

```rust
/// A tool that can be executed by the RLM engine
#[async_trait]
pub trait Tool: Send + Sync {
    /// Unique tool name
    fn name(&self) -> &str;
    
    /// Human-readable description for LLM
    fn description(&self) -> &str;
    
    /// JSON Schema for parameters
    fn parameters_schema(&self) -> serde_json::Value;
    
    /// Execute the tool
    async fn execute(&self, params: serde_json::Value) -> Result<ToolResult>;
}

pub struct ToolResult {
    /// Structured content (can be text, json, file contents, etc.)
    pub content: ToolContent,
    /// Metadata for aggregation (relevance hints, source info)
    pub metadata: ToolMetadata,
}

pub enum ToolContent {
    Text(String),
    Json(serde_json::Value),
    FileContent { path: String, content: String, language: Option<String> },
    Error { message: String, recoverable: bool },
}
```

### Tool Definitions

**read_file:**
```json
{
  "name": "read_file",
  "description": "Read contents of a file",
  "parameters": {
    "type": "object",
    "properties": {
      "path": { "type": "string", "description": "File path relative to repo root" },
      "start_line": { "type": "integer", "description": "Optional start line (1-indexed)" },
      "end_line": { "type": "integer", "description": "Optional end line (inclusive)" }
    },
    "required": ["path"]
  }
}
```

**graph_query:**
```json
{
  "name": "graph_query",
  "description": "Execute Cypher query against code graph",
  "parameters": {
    "type": "object",
    "properties": {
      "query": { "type": "string", "description": "Cypher query" },
      "limit": { "type": "integer", "description": "Max results", "default": 20 }
    },
    "required": ["query"]
  }
}
```

**execute_code:**
```json
{
  "name": "execute_code",
  "description": "Execute code in sandboxed environment",
  "parameters": {
    "type": "object",
    "properties": {
      "language": { "type": "string", "enum": ["python", "bash", "rust"] },
      "code": { "type": "string", "description": "Code to execute" },
      "timeout_seconds": { "type": "integer", "default": 30 }
    },
    "required": ["language", "code"]
  }
}
```

### Context Aggregator

```rust
pub struct ContextAggregator {
    /// Maximum tokens in aggregated result
    max_tokens: usize,
    /// Embedding model for relevance scoring (optional)
    embedder: Option<Arc<dyn Embedder>>,
}

impl ContextAggregator {
    pub fn aggregate(
        &self,
        results: Vec<ToolResult>,
        query: &str,
        budget: usize,
    ) -> AggregatedContext {
        // 1. Score each result for relevance
        let scored: Vec<_> = results.into_iter()
            .map(|r| (self.score_relevance(&r, query), r))
            .collect();
        
        // 2. Sort by relevance
        let mut sorted = scored;
        sorted.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
        
        // 3. Deduplicate (same file path, overlapping line ranges)
        let deduped = self.deduplicate(sorted);
        
        // 4. Truncate to budget
        self.truncate_to_budget(deduped, budget)
    }
}
```

### REPL Sandbox (Linux)

```rust
pub struct SandboxConfig {
    /// Max execution time
    pub timeout: Duration,
    /// Max memory (bytes)
    pub memory_limit: usize,
    /// Max output size (bytes)
    pub output_limit: usize,
    /// Allowed paths (read-only)
    pub allowed_paths: Vec<PathBuf>,
}

impl Sandbox {
    pub async fn execute(&self, code: &str, language: Language) -> Result<ExecutionResult> {
        // Fork process
        // Apply seccomp filter (block network, dangerous syscalls)
        // Apply resource limits (rlimit)
        // Execute in isolated namespace
        // Capture stdout/stderr with size limit
        // Kill on timeout
    }
}
```

## Testing Strategy

### Unit Testing
- Each tool tested in isolation with mock dependencies
- Context aggregator tested with synthetic results
- Sandbox tested with known-safe and known-dangerous code

### Integration Testing
- Tool chain: graph_query → read_file → aggregate
- Memory tools with real .muninn/memory/ structure
- REPL sandbox escape attempts (should all fail)

### Security Testing
- REPL sandbox: attempt network access (must fail)
- REPL sandbox: attempt file write outside allowed paths (must fail)
- REPL sandbox: attempt resource exhaustion (must be limited)
- Path traversal attempts in read_file (must fail)

## Alternatives Considered

### 1. WebAssembly Sandbox for REPL
**Considered**: WASM provides strong isolation. However, WASM support for arbitrary Python/Rust code is limited. Native sandbox (seccomp/landlock) provides better language support.

### 2. Container-Based Isolation
**Rejected**: Docker/podman adds significant overhead and complexity for CLI tool. Process-level sandboxing is lighter weight and sufficient.

### 3. No REPL (Read-Only Tools Only)
**Rejected**: Code execution enables verification and exploration that read-only tools cannot provide. Sandbox makes it safe enough.

### 4. LLM-Based Relevance Scoring
**Considered**: Using the LLM to score relevance. Adds latency and cost. Simple embedding-based or keyword scoring is faster and often sufficient.

## Implementation Plan

### Phase 1: Tool Framework
- [ ] Define `Tool` trait and `ToolResult` types
- [ ] Implement `ToolRegistry` with registration/discovery
- [ ] Generate Anthropic tool definitions from registry
- [ ] Unit tests for framework

### Phase 2: File System Tools
- [ ] Implement `read_file` tool
- [ ] Implement `list_directory` tool
- [ ] Implement `search_files` tool (ripgrep wrapper)
- [ ] Path validation and .gitignore respect

### Phase 3: Code Graph Tools
- [ ] Implement `graph_query` tool (wraps PROJEC-I-0001)
- [ ] Implement `find_callers` convenience tool
- [ ] Implement `find_implementations` convenience tool
- [ ] Implement `get_symbol` tool

### Phase 4: Memory Tools
- [ ] Implement `query_memory` tool (JSONL search)
- [ ] Implement `search_knowledge` tool (markdown search)
- [ ] Implement `get_context` tool (relevant to current task)

### Phase 5: REPL Sandbox
- [ ] Process isolation with fork
- [ ] seccomp filter implementation (Linux)
- [ ] sandbox-exec wrapper (macOS)
- [ ] Resource limits (time, memory, output)
- [ ] `execute_code` tool implementation

### Phase 6: Context Aggregator
- [ ] Basic aggregation (concat with separators)
- [ ] Deduplication logic
- [ ] Relevance scoring (keyword-based initially)
- [ ] Truncation to budget

### Phase 7: MCP Integration
- [ ] Expose tools as MCP tools
- [ ] Tool execution via MCP protocol
- [ ] Integration with Claude Code

### Dependencies
- `grep` crate or ripgrep binding for search
- `seccomp` crate for Linux sandboxing
- `nix` crate for process control
- PROJEC-I-0001 for graph queries
- ADR-001 memory layer implementation