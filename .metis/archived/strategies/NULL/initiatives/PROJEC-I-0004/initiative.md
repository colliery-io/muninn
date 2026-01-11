---
id: local-llm-engine-embedded
level: initiative
title: "Local LLM Engine: Embedded Inference with llama-cpp-2"
short_code: "PROJEC-I-0004"
created_at: 2026-01-08T02:37:54.979491+00:00
updated_at: 2026-01-09T01:35:05.375455+00:00
parent: PROJEC-V-0001
blocked_by: []
archived: true

tags:
  - "#initiative"
  - "#phase/completed"


exit_criteria_met: false
estimated_complexity: L
strategy_id: NULL
initiative_id: local-llm-engine-embedded
---

# Local LLM Engine: Embedded Inference with llama-cpp-2 Initiative

*This template includes sections for various types of initiatives. Delete sections that don't apply to your specific use case.*

## Context

Muninn's vision emphasizes **privacy-first** operation. While cloud LLMs (Claude, GPT-4) provide the best quality, local LLM capability enables:

1. **Complete privacy**: No data leaves the machine
2. **Offline operation**: Work without network connectivity
3. **Cost reduction**: Cheap sub-queries in RLM loop use local model
4. **Local embeddings**: Semantic search without API calls

**Technology Choice: llama.cpp via `llama-cpp-2`**

llama.cpp is the most mature and performant local inference engine. Using Rust bindings via `llama-cpp-2` crate:
- Best-in-class performance (heavily optimized C++)
- Excellent GGUF quantization (Q2-Q8, K-quants)
- 50k+ models on HuggingFace
- Cross-platform: Metal (Apple Silicon), CUDA (NVIDIA), CPU fallback
- Embedding mode for semantic search
- Single provider for both generation and embeddings

**Reference:**
- PROJEC-I-0002: RLM Gateway (local LLM as a backend option)
- ADR-001: Embeddings stored in sqlite-vec
- llama-cpp-2: https://github.com/utilityai/llama-cpp-rs
- llama.cpp: https://github.com/ggerganov/llama.cpp

## Goals & Non-Goals

**Goals:**
- Integrate llama.cpp via `llama-cpp-2` for local inference
- Support text generation for RLM sub-queries and offline operation
- Support embedding generation for semantic search (sqlite-vec)
- Auto-detect best backend (Metal → CUDA → CPU)
- Implement model management (download, cache, select)
- Expose as `LocalBackend` implementing `LLMBackend` trait (PROJEC-I-0002)
- Recommend default models for different use cases

**Non-Goals:**
- Training or fine-tuning (inference only)
- Model format conversion (expect GGUF)
- GUI for model management (CLI only)
- Competing with cloud model quality (local is for speed/privacy/cost)

## Requirements

### Functional Requirements

**Inference:**
- REQ-001: Load GGUF models from local filesystem
- REQ-002: Generate text completions with configurable parameters (temperature, top_p, etc.)
- REQ-003: Generate embeddings for text chunks
- REQ-004: Support streaming token generation
- REQ-005: Implement `LLMBackend` trait from PROJEC-I-0002

**Hardware Detection:**
- REQ-006: Auto-detect Metal availability (macOS)
- REQ-007: Auto-detect CUDA availability (Linux/Windows)
- REQ-008: Fall back to CPU if no GPU available
- REQ-009: Report detected backend to user on startup

**Model Management:**
- REQ-010: Download models from HuggingFace Hub
- REQ-011: Cache models in `~/.muninn/models/`
- REQ-012: List available/downloaded models
- REQ-013: Delete cached models
- REQ-014: Validate model files (checksum)

**Configuration:**
- REQ-015: Configure default generation model
- REQ-016: Configure default embedding model
- REQ-017: Configure context window size
- REQ-018: Configure GPU layers (partial offload)

### Non-Functional Requirements
- NFR-001: First token latency < 500ms for 7B model on M1
- NFR-002: Embedding generation > 100 chunks/second
- NFR-003: Memory usage within model's stated requirements
- NFR-004: Graceful degradation if model too large for hardware

### Recommended Models

| Use Case | Model | Size | Quantization |
|----------|-------|------|--------------|
| Sub-queries (fast) | Phi-3-mini | 3.8B | Q4_K_M |
| General (balanced) | Mistral-7B | 7B | Q4_K_M |
| Quality (offline) | Llama-3-8B | 8B | Q5_K_M |
| Embeddings | nomic-embed-text | 137M | F16 |

## Use Cases

### UC-1: Cheap Sub-Query in RLM Loop
- **Actor**: RLM engine (PROJEC-I-0002)
- **Scenario**: Main query uses Claude, spawns sub-query "summarize this function"
- **Flow**: Sub-query routes to local Phi-3-mini instead of Claude
- **Benefit**: 10x cheaper, no API latency, keeps data local

### UC-2: Fully Offline Operation
- **Actor**: Developer on airplane
- **Scenario**: No network, needs to understand codebase
- **Flow**: All RLM queries use local Mistral-7B
- **Benefit**: Full functionality without connectivity

### UC-3: Semantic Code Search
- **Actor**: RLM tool environment (PROJEC-I-0003)
- **Scenario**: "Find functions related to authentication"
- **Flow**: Generate embedding for query, search sqlite-vec
- **Benefit**: Semantic search without API calls

### UC-4: Model Management
- **Actor**: Developer setting up Muninn
- **Scenario**: First run, needs to download models
- **Flow**: `muninn models pull mistral-7b-q4` downloads from HuggingFace
- **Benefit**: Simple model acquisition

## Architecture

### Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                    RLM Gateway (PROJEC-I-0002)                  │
│                                                                 │
│   ┌─────────────┐  ┌─────────────┐  ┌─────────────────────┐   │
│   │  Anthropic  │  │   OpenAI    │  │    LocalBackend     │   │
│   │   Backend   │  │   Backend   │  │  (this initiative)  │   │
│   └─────────────┘  └─────────────┘  └──────────┬──────────┘   │
└────────────────────────────────────────────────┼───────────────┘
                                                 │
                                                 ▼
┌─────────────────────────────────────────────────────────────────┐
│                     Local LLM Engine                            │
├─────────────────────────────────────────────────────────────────┤
│  ┌─────────────────┐  ┌─────────────────┐  ┌────────────────┐  │
│  │  Model Manager  │  │ Hardware Detect │  │   Inference    │  │
│  │  (download,     │  │ (Metal/CUDA/CPU)│  │    Engine      │  │
│  │   cache, list)  │  │                 │  │                │  │
│  └────────┬────────┘  └────────┬────────┘  └───────┬────────┘  │
│           │                    │                    │           │
│           └────────────────────┼────────────────────┘           │
│                                │                                │
│                                ▼                                │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │                    llama-cpp-2 Bindings                  │  │
│  └──────────────────────────────────────────────────────────┘  │
│                                │                                │
└────────────────────────────────┼────────────────────────────────┘
                                 │
                                 ▼
                    ┌────────────────────────┐
                    │      llama.cpp         │
                    │  (C++ inference lib)   │
                    └────────────────────────┘
                                 │
              ┌──────────────────┼──────────────────┐
              ▼                  ▼                  ▼
         ┌─────────┐       ┌─────────┐       ┌─────────┐
         │  Metal  │       │  CUDA   │       │   CPU   │
         │ (macOS) │       │(NVIDIA) │       │(fallback)│
         └─────────┘       └─────────┘       └─────────┘
```

### Components

**1. LocalBackend**
- Implements `LLMBackend` trait from PROJEC-I-0002
- Routes completion requests to llama.cpp
- Translates between Anthropic message format and llama.cpp format

**2. Model Manager**
- Downloads GGUF models from HuggingFace Hub
- Caches in `~/.muninn/models/`
- Tracks model metadata (size, quant, capabilities)
- Validates checksums

**3. Hardware Detector**
- Probes for Metal (macOS)
- Probes for CUDA (checks for libcuda)
- Reports available backends and memory
- Selects optimal backend automatically

**4. Inference Engine**
- Wraps llama-cpp-2 for generation
- Wraps llama-cpp-2 for embeddings
- Manages model loading/unloading
- Handles context window management

## Detailed Design

### LocalBackend Implementation

```rust
use llama_cpp_2::model::{LlamaModel, AddBos, Special};
use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;

pub struct LocalBackend {
    model: LlamaModel,
    backend: LlamaBackend,
    config: LocalConfig,
}

impl LocalBackend {
    pub fn new(model_path: &Path, config: LocalConfig) -> Result<Self> {
        // Initialize llama.cpp backend (auto-detects Metal/CUDA/CPU)
        let backend = LlamaBackend::init()?;
        
        let model_params = LlamaModelParams::default()
            .with_n_gpu_layers(config.gpu_layers);
        
        let model = LlamaModel::load_from_file(&backend, model_path, &model_params)?;
        
        Ok(Self { model, backend, config })
    }
}

#[async_trait]
impl LLMBackend for LocalBackend {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse> {
        // Create context for this request
        let ctx_params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(self.config.context_size).unwrap());
        
        let mut ctx = self.model.new_context(&self.backend, ctx_params)?;
        
        // Convert messages to prompt
        let prompt = self.format_prompt(&request.messages);
        
        // Tokenize
        let tokens = self.model.str_to_token(&prompt, AddBos::Always)?;
        
        // Generate
        let mut output = String::new();
        // ... generation loop with sampling
        
        Ok(CompletionResponse {
            content: vec![ContentBlock::Text { text: output }],
            stop_reason: StopReason::EndTurn,
            usage: Usage { input_tokens, output_tokens },
        })
    }
    
    fn name(&self) -> &str {
        "local"
    }
}
```

### Embedding Generation

```rust
impl LocalBackend {
    pub fn embed(&self, text: &str) -> Result<Vec<f32>> {
        // llama.cpp supports embedding mode
        let ctx_params = LlamaContextParams::default()
            .with_embeddings(true);
        
        let ctx = self.model.new_context(&self.backend, ctx_params)?;
        
        let tokens = self.model.str_to_token(text, AddBos::Always)?;
        ctx.decode(&tokens)?;
        
        // Extract embeddings from context
        let embeddings = ctx.embeddings()?;
        Ok(embeddings.to_vec())
    }
    
    pub fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        // Batch embedding for efficiency
        texts.iter().map(|t| self.embed(t)).collect()
    }
}
```

### Hardware Detection

```rust
pub struct HardwareInfo {
    pub backend: BackendType,
    pub gpu_name: Option<String>,
    pub vram_mb: Option<u64>,
    pub recommended_layers: u32,
}

pub enum BackendType {
    Metal,
    Cuda,
    Cpu,
}

pub fn detect_hardware() -> HardwareInfo {
    // Try Metal first (macOS)
    #[cfg(target_os = "macos")]
    if metal_available() {
        return HardwareInfo {
            backend: BackendType::Metal,
            gpu_name: Some(get_metal_device_name()),
            vram_mb: Some(get_metal_memory()),
            recommended_layers: 999, // Metal handles all layers well
        };
    }
    
    // Try CUDA (Linux/Windows with NVIDIA)
    if cuda_available() {
        let vram = get_cuda_memory();
        return HardwareInfo {
            backend: BackendType::Cuda,
            gpu_name: Some(get_cuda_device_name()),
            vram_mb: Some(vram),
            recommended_layers: estimate_layers_for_vram(vram),
        };
    }
    
    // Fallback to CPU
    HardwareInfo {
        backend: BackendType::Cpu,
        gpu_name: None,
        vram_mb: None,
        recommended_layers: 0,
    }
}
```

### Model Manager

```rust
pub struct ModelManager {
    cache_dir: PathBuf,  // ~/.muninn/models/
    registry: ModelRegistry,
}

impl ModelManager {
    pub async fn pull(&self, model_id: &str) -> Result<PathBuf> {
        // Resolve model ID to HuggingFace URL
        let spec = self.registry.resolve(model_id)?;
        
        let dest = self.cache_dir.join(&spec.filename);
        if dest.exists() {
            // Verify checksum
            if verify_checksum(&dest, &spec.sha256)? {
                return Ok(dest);
            }
        }
        
        // Download with progress
        download_with_progress(&spec.url, &dest).await?;
        
        // Verify
        verify_checksum(&dest, &spec.sha256)?;
        
        Ok(dest)
    }
    
    pub fn list(&self) -> Result<Vec<CachedModel>> {
        // List all models in cache_dir
    }
    
    pub fn delete(&self, model_id: &str) -> Result<()> {
        // Remove model from cache
    }
}
```

### Configuration

```toml
# ~/.muninn/config.toml

[local]
# Default model for generation
generation_model = "mistral-7b-q4_k_m"

# Default model for embeddings  
embedding_model = "nomic-embed-text"

# Context window size
context_size = 4096

# GPU layers (0 = CPU only, -1 = auto, N = specific count)
gpu_layers = -1

# Model cache directory
cache_dir = "~/.muninn/models"
```

## Testing Strategy

### Unit Testing
- Hardware detection mocking (test all three paths)
- Model manager with mock HTTP responses
- Prompt formatting for different model types

### Integration Testing
- End-to-end generation with small test model (TinyLlama)
- Embedding generation and similarity verification
- Model download/cache/delete cycle

### Performance Testing
- First token latency benchmarks
- Tokens/second throughput
- Memory usage tracking
- Compare CPU vs GPU performance

## Alternatives Considered

### 1. Candle (Pure Rust)
**What**: HuggingFace's pure Rust ML framework for inference.

**Pros:**
- No C/C++ dependencies, pure Rust stack
- Easier cross-compilation
- More hackable/extensible

**Cons:**
- Less mature than llama.cpp
- Fewer models available (limited GGUF support)
- Performance gaps, especially for quantization
- Smaller community, fewer optimizations

**Decision**: Rejected. While pure Rust is appealing, llama.cpp's performance and model ecosystem are significantly better. The C++ build dependency is acceptable.

### 2. Ollama Subprocess
**What**: Shell out to Ollama binary for inference.

**Pros:**
- Zero integration effort
- Ollama handles model management
- Already popular with developers

**Cons:**
- External dependency (user must install Ollama)
- Less control over inference parameters
- IPC overhead for every call
- Can't embed directly for embedding generation
- Harder to bundle/distribute

**Decision**: Rejected. Muninn should be self-contained without requiring external services.

### 3. Hybrid: llama.cpp for Generation + Candle for Embeddings
**What**: Use llama-cpp-2 for text generation but Candle for embedding models (which are simpler).

**Pros:**
- Best of both worlds: llama.cpp performance for generation
- Pure Rust for simpler embedding models

**Cons:**
- Two inference backends to maintain
- More complex codebase
- Inconsistent model loading/caching

**Decision**: Rejected. User preference for single provider. llama.cpp handles both generation and embeddings well.

### 4. ONNX Runtime
**What**: Use ONNX format with ort crate for inference.

**Pros:**
- Model-agnostic format
- Good tooling

**Cons:**
- Fewer LLM models in ONNX format
- Less optimized for autoregressive generation
- Conversion required from HuggingFace models

**Decision**: Rejected. GGUF ecosystem is much richer for LLMs.

### Final Choice: llama-cpp-2
Selected for:
- Best-in-class inference performance
- Excellent quantization (Q2-Q8, K-quants)
- Massive model ecosystem (50k+ GGUF models on HuggingFace)
- Single provider for generation and embeddings
- Cross-platform GPU support (Metal, CUDA)
- Active development and community

## Implementation Plan

### Phase 1: Foundation
**Goal**: Basic inference working with llama-cpp-2

**Tasks:**
1. Set up Cargo workspace with llama-cpp-2 dependency
2. Implement hardware detection (Metal/CUDA/CPU probe)
3. Create basic model loading from GGUF file
4. Implement simple text generation (no streaming)
5. Write unit tests with TinyLlama model

**Exit Criteria:**
- Can load a GGUF model and generate text on macOS (Metal)
- Hardware detection reports correct backend
- Tests pass in CI

### Phase 2: Embedding Support
**Goal**: Embedding generation for semantic search

**Tasks:**
1. Implement embedding mode context creation
2. Add `embed()` and `embed_batch()` methods
3. Test with nomic-embed-text model
4. Benchmark embedding throughput
5. Integrate with sqlite-vec storage (from PROJEC-I-0001)

**Exit Criteria:**
- Can generate embeddings at >100 chunks/second
- Embeddings work with sqlite-vec similarity search
- Tests verify embedding quality (cosine similarity sanity checks)

### Phase 3: Model Management
**Goal**: Download and manage models from HuggingFace

**Tasks:**
1. Define model registry with recommended models
2. Implement HuggingFace download with progress reporting
3. Create model cache directory structure (~/.muninn/models/)
4. Add checksum verification
5. Implement list/delete commands
6. Add CLI commands: `muninn models pull`, `list`, `delete`

**Exit Criteria:**
- Can download Mistral-7B-Q4 from HuggingFace
- Models cached and reusable across sessions
- CLI provides model management

### Phase 4: LLMBackend Integration
**Goal**: Integrate with RLM Gateway (PROJEC-I-0002)

**Tasks:**
1. Implement `LLMBackend` trait for `LocalBackend`
2. Add prompt formatting for different model types (Llama, Mistral, Phi)
3. Implement streaming token generation
4. Add configuration for default models
5. Test routing between cloud and local backends

**Exit Criteria:**
- LocalBackend works as drop-in replacement in RLM gateway
- Can switch between Claude and local model via config
- Streaming works for responsive UX

### Phase 5: Polish & Performance
**Goal**: Production-ready quality

**Tasks:**
1. Performance benchmarking (latency, throughput)
2. Memory profiling and optimization
3. Graceful handling of OOM conditions
4. Documentation and examples
5. GPU layer auto-tuning based on VRAM

**Exit Criteria:**
- First token latency <500ms for 7B model on M1
- Documented recommended models table
- Handles edge cases gracefully

### Dependencies
- **PROJEC-I-0001** (Code Graph): For sqlite-vec integration in Phase 2
- **PROJEC-I-0002** (RLM Gateway): For LLMBackend trait definition in Phase 4

### Risks & Mitigations

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| llama-cpp-2 API changes | Medium | Medium | Pin to specific version, test before upgrading |
| Build complexity on Windows | Medium | Low | Document CUDA toolkit requirements, provide CPU fallback |
| Model quality insufficient | Low | High | Test multiple models, allow user to choose |
| Memory issues on 8GB machines | Medium | Medium | Recommend smaller models, implement offloading |