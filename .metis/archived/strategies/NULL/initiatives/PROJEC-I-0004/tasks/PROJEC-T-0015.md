---
id: set-up-llama-cpp-2-integration
level: task
title: "Set up llama-cpp-2 integration with hardware detection (Metal/CUDA/CPU)"
short_code: "PROJEC-T-0015"
created_at: 2026-01-09T01:06:10.656395+00:00
updated_at: 2026-01-09T01:18:17.274101+00:00
parent: PROJEC-I-0004
blocked_by: []
archived: true

tags:
  - "#task"
  - "#phase/completed"


exit_criteria_met: false
strategy_id: NULL
initiative_id: PROJEC-I-0004
---

# Set up llama-cpp-2 integration with hardware detection (Metal/CUDA/CPU)

## Parent Initiative

[[PROJEC-I-0004]]

## Objective

Integrate the llama-cpp-2 Rust bindings into muninn-llm crate with hardware detection for Metal (macOS), CUDA (NVIDIA), and CPU fallback.

## Acceptance Criteria

## Acceptance Criteria

## Acceptance Criteria

- [x] Add llama-cpp-2 dependency with sampler feature
- [x] Feature flags for metal and cuda backends  
- [x] Hardware detection module (Metal/CUDA/CPU)
- [x] Error types for LLM operations
- [x] Core inference engine with model loading
- [x] Text generation with sampling parameters
- [x] Unit tests for config and params
- [x] Build and tests pass on macOS with Metal

## Implementation Notes

### Files Created

- `crates/muninn-llm/Cargo.toml` - Dependencies with metal/cuda feature flags
- `crates/muninn-llm/src/lib.rs` - Module exports
- `crates/muninn-llm/src/error.rs` - Error types (LlmError enum)
- `crates/muninn-llm/src/hardware.rs` - Hardware detection (BackendType, HardwareInfo)
- `crates/muninn-llm/src/inference.rs` - Inference engine (InferenceConfig, GenerationParams, InferenceEngine)

### Key Types

- `BackendType`: Metal | Cuda | Cpu
- `HardwareInfo`: Detected hardware with VRAM, system RAM, recommended GPU layers
- `InferenceConfig`: GPU layers, context size, threads, batch size, seed
- `GenerationParams`: max_tokens, temperature, top_p, top_k, stop_sequences
- `InferenceEngine`: Wraps llama.cpp for model loading and text generation

### Test Results

13 tests passed:
- 6 hardware detection tests
- 5 inference config/params tests  
- 1 error display test
- 1 integration harness test

## Status Updates

**2026-01-09**: Task completed. muninn-llm builds with Metal support on macOS. All 13 tests pass.