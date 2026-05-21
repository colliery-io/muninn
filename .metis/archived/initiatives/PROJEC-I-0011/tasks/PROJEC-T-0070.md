---
id: add-hook-decision-config-and
level: task
title: "Add [hook_decision] config and implement muninn hook decide subcommand"
short_code: "PROJEC-T-0070"
created_at: 2026-05-19T16:41:32.341540+00:00
updated_at: 2026-05-20T16:23:28.280523+00:00
parent: PROJEC-I-0011
blocked_by: []
archived: true

tags:
  - "#task"
  - "#phase/completed"


exit_criteria_met: false
initiative_id: PROJEC-I-0011
---

# Add [hook_decision] config and implement muninn hook decide subcommand

## Parent Initiative

[[hook-mcp-integration-layer-for-claude-code]] (PROJEC-I-0011)

## Objective

Two coupled changes:

1. Extend the tiered config from PROJEC-T-0063 with an optional `[hook_decision]` section that inherits provider/model from `[default]` unless overridden. Lets users pin a smaller/faster model for the hook when NFR-001 bites.
2. Implement the `muninn hook decide` subcommand the plugin (PROJEC-T-0069) shells out to. Reads the CC hook input, calls the decision model with the schema in [[hook-mcp-integration-layer-for-claude-code]], parses the response, emits CC's hook response.

## Acceptance Criteria

## Acceptance Criteria

## Acceptance Criteria

## Acceptance Criteria

### Config
- [ ] `[hook_decision]` section in `Config`, optional `provider` and `model` overrides.
- [ ] `Config::resolved_hook_decision() -> ResolvedLlmConfig` returns post-inheritance values.
- [ ] `Config::validate()` treats it the same way as router/rlm.
- [ ] Tests cover inheritance + override.

### Decision subcommand
- [ ] `muninn hook decide` reads CC hook input from stdin, parses tool name + args.
- [ ] Builds the decision-model prompt per initiative spec (tool_name, tool_args, recent history, top-k memory hints from daemon).
- [ ] Calls the decision model via the resolved `[hook_decision]` config.
- [ ] Parses the JSON response (`{decision, rewrite?, augment_hint?}`); parse failure → passthrough.
- [ ] Hard timeout: 500ms wall clock; timeout → passthrough.
- [ ] Emits the CC hook response: passthrough = allow original; augment = allow original + `additionalContext` from PROJEC-T-0071; rewrite = block original + return synthetic tool result from the appropriate engine method.
- [ ] Logs decision + latency to muninn's trace facility.

## Dependencies

- PROJEC-T-0063 (tiered config baseline)
- PROJEC-T-0066 (daemon, for memory hints)
- PROJEC-T-0069 (plugin that calls this)
- PROJEC-T-0071 (augmentation block — referenced from the augment branch)

## Implementation Notes

- Load-bearing latency path. Every millisecond matters. Allocation-light prompt construction; reuse daemon IPC connection if practical.
- Provider JSON mode preferred where available; fall back to plain text + tolerant parser only if necessary.
- Treat the decision model as untrusted: validate output structure, clamp `rewrite.tool` to the known whitelist, ignore unknown decisions.

## Status Updates

### 2026-05-20 — Implementation landed (augment relays hints; rewrite still degrades to passthrough)

**Config (`crates/muninn/src/config.rs`):**
- New `HookDecisionConfig { provider: Option<String>, model: Option<String> }` parallel to `RouterConfig` / `RlmConfig`. Inherits both fields from `[default]` when unset.
- `Config::resolved_hook_decision() -> ResolvedLlmConfig` accessor.
- `validate()` extended: hook_decision counts toward the provider allowlist and the credential checks for groq / anthropic / ollama-cloud, so a `[hook_decision]` override that demands a key surfaces a clear error.
- 2 new tests: `test_hook_decision_inherits_from_default`, `test_hook_decision_override_beats_default`.

**Subcommand (`muninn hook decide`, in `crates/muninn/src/main.rs`):**
- New `Commands::Hook { command: HookCommand }` with `Decide` variant.
- `run_hook_decide` reads CC's PreToolUse hook input from stdin, builds the decision-model prompt per the initiative spec (tool_name + truncated tool_input), constructs a backend from the resolved `[hook_decision]` config via the existing `create_backend_from_config`, calls `backend.complete(...)` with `max_tokens = 64`, and parses the JSON response.
- **Hard wall-clock timeout: 500 ms** enforced via `tokio::time::timeout`. Timeout → passthrough.
- **Stdout discipline**: when invoked, the binary routes tracing to stderr (`init_logging_stderr_only`) so stdout stays clean for the hook response.
- **Decision dispatch**:
  - `passthrough` → empty stdout (CC sentinel for "allow original").
  - `augment` with `augment_hint` → emits `{"hookSpecificOutput": {"hookEventName":"PreToolUse","additionalContext":"Muninn hint: …"}}`. The full augmentation retrieval (PROJEC-T-0071) hasn't shipped yet; until then we relay just the model's hint as best-effort signal.
  - `augment` without hint → passthrough.
  - `rewrite` → **passthrough for now** (NFR-002 degrade). Wiring rewrite to a real engine call is bundled with PROJEC-T-0071.
  - Unknown discriminant → passthrough.
- **Tolerant parsing**: `extract_json_block` strips ```` ```json …``` ```` fences and tolerates models that wrap JSON in a rationale paragraph by hunting for the outer `{ … }`. `DecisionPayload` accepts extra fields without erroring.
- **NFR-002 contract preserved end-to-end**: every error path in `decide_inner` (stdin read, hook-input parse, backend construction, backend call, model output empty, JSON parse fail, …) returns an `Err` that the outer `run_hook_decide` translates into a passthrough. The function returns `Ok(())` unconditionally so the binary never exits non-zero from the hook path.

**System prompt**: short, strict-JSON, biased toward passthrough so the model only augments when it can articulate a concrete reason.

### Tests
- 8 new unit tests in `main.rs::hook_tests` covering `extract_json_block` (bare object / json fence / plain fence / prose-prefixed / no-braces), `DecisionPayload` parsing (passthrough / augment+hint / extra-fields tolerance), and `HookInput` parsing (full CC payload + minimal-field tolerance).
- muninn binary unit tests: **28/28 pass** (was 18, +10 — 8 hook tests + 2 hook_decision config tests).
- Workspace tests: all 15 suites still green, no regressions.
- `cargo clippy -p muninn -p muninn-core -p muninn-rlm --no-deps -- -D warnings`: only the pre-existing `main.rs:1242` print_literal warning (tracked in PROJEC-T-0076).
- `cargo fmt --check`: clean on touched crates.

### Decisions

- **Augment without retrieval (yet)**: Until PROJEC-T-0071 lands the full markdown context block, an `augment` decision just relays the decision model's `augment_hint` as a one-line `Muninn hint: …`. It's a deliberate degrade — better than passthrough on cases the model flags as interesting, less than the eventual ≤2 KB block.
- **Rewrite degrade**: Rewriting a tool call to short-circuit Grep/Read/Glob needs an engine round-trip to `search_code` / `query_graph` and a stable contract with CC for how to surface the synthetic result. That coupling lives more naturally in PROJEC-T-0071 (augmentation retrieval), so this iteration just degrades rewrite to passthrough. NFR-002 honored.
- **No daemon-side recall_memory yet**: The initiative design called for top-k memory hints in the prompt. Until the daemon's `recall_memory` is wired to a real store (planned with PROJEC-T-0071's retrieval work), the prompt is just `tool_name` + truncated `tool_input`. The decision model still has enough to make reasonable passthrough/augment calls on heuristic grounds; the memory-augmented prompt is a future improvement.

### Deferred / explicit non-scope

- Full augmentation block (PROJEC-T-0071): markdown context, ≤2 KB cap, retrieval via `recall_memory` + `query_graph`.
- Rewrite path (PROJEC-T-0071): synthetic tool results via engine methods.
- Decision latency benchmarks vs. NFR-001 (PROJEC-T-0074).
- Failure-mode integration tests (PROJEC-T-0073): the unit tests here cover parsing logic; the daemon-down / model-down / timeout integration coverage is the dedicated step.

### CI carve-out
Same as previous initiative tasks — workspace `angreal ci` still blocked by the pre-existing muninn-graph clippy debt tracked in PROJEC-T-0076. No new clippy or fmt issues introduced.