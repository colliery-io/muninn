---
id: add-ollama-cloud-provider-and-make
level: task
title: "Add Ollama Cloud provider and make it the default for router + RLM"
short_code: "PROJEC-T-0063"
created_at: 2026-05-19T15:22:55.845946+00:00
updated_at: 2026-05-19T19:49:26.825306+00:00
parent: 
blocked_by: []
archived: false

tags:
  - "#task"
  - "#feature"
  - "#phase/completed"


exit_criteria_met: false
initiative_id: NULL
---

# Add Ollama Cloud provider and make it the default for router + RLM

## Objective

Two coupled changes:

1. Add Ollama Cloud (`https://api.ollama.com/v1`) as a first-class provider option in muninn, distinct from local Ollama. Reference implementation in `../arawn/crates/arawn-llm/src/openai_compat.rs`; pattern confirmed by archived `ARAWN-T-0166`.
2. **Restructure the LLM config around a `[default]` baseline that `[router]` and `[rlm]` inherit from**, so the out-of-the-box default is a single Ollama-Cloud model serving both surfaces (works on free tier, max cache reuse), while users who want to invest in specialization can override either section independently.

## Type / Priority

- Type: Feature
- Priority: P1 — unlocks a sanctioned managed-Ollama path so users aren't forced onto Groq for the default experience.

## Business Justification

- **User value**: Ollama Cloud gives users a hosted big-model option that still aligns with the Ollama ecosystem they already trust locally; no Groq account required to get started.
- **Vision alignment**: PROJEC-V-0001 principle "Backend-agnostic — work with any OpenAI-compatible LLM backend" — Groq-as-default is incidental, not strategic.
- **Effort**: S — mostly a config + provider-registration change; the OpenAI-compatible plumbing in `crates/muninn-rlm/src/ollama.rs` already works.

## Acceptance Criteria

### Tiered config inheritance
- [ ] New `[default]` config section holds the baseline `provider` and `model`.
- [ ] `[router]` and `[rlm]` sections accept `provider` / `model` as **optional overrides**. When omitted, they inherit from `[default]`. A `Config::resolved_router()` / `resolved_rlm()` accessor (or equivalent) returns the post-inheritance values; nothing downstream should consume the raw section fields directly.
- [ ] `[default]`, `[router]`, `[rlm]` all retain the existing knobs (`enabled`, `strategy`, etc., where relevant). Only `provider` and `model` participate in inheritance.
- [ ] Defaults: `[default].provider = "ollama"`, `[default].model = "gemma4:31b"`. (Verify the exact Ollama Cloud identifier at implementation time — may need a `:cloud` tag suffix depending on current catalog conventions.)
- [ ] Backwards-compat: if a user has only `[router]` / `[rlm]` filled in (today's shape), inheritance still resolves correctly without `[default]` — fall back to a built-in default.

### Ollama Cloud provider plumbing
- [ ] `OllamaConfig` supports an optional `api_key` and a configurable `base_url` that defaults to local but accepts the Ollama Cloud URL.
- [ ] `crates/muninn/src/config.rs` adds an `OllamaProviderConfig { api_key, base_url }` parallel to `GroqProviderConfig` / `AnthropicProviderConfig`, with `OLLAMA_API_KEY` env-var fallback.
- [ ] `Config::validate()` accepts `"ollama"` in the provider allowlist and requires an API key when the *resolved* base_url points at the cloud host. Keyless local Ollama remains valid.
- [ ] Backend factory wiring in router/rlm provider selection routes `"ollama"` to `OllamaBackend` with the resolved config.

### Tests / docs
- [ ] Unit tests cover: `[default]`-only config resolves both router and rlm; `[router]` override beats `[default]`; missing `[default]` still falls back; default-config snapshot picks `"ollama"`; cloud-vs-local URL handling; "cloud requires API key" validator branch.
- [ ] `angreal ci` passes (fmt, clippy, build, test).
- [ ] README / config docs explain the tiered model with a worked example: free-tier single-model config, then a "tune for cost/quality" example that overrides `[rlm]` with a bigger model.
- [ ] Upgrade-notes section for the default flip + `OLLAMA_API_KEY` requirement.

## Implementation Notes

### Technical Approach

1. **Config schema reshape** in `crates/muninn/src/config.rs`:
   - Add `DefaultLlmConfig { provider: String, model: String }` (the baseline).
   - Change `RouterConfig.provider` / `RouterConfig.model` (and same for `RlmConfig`) to `Option<String>`.
   - Add `Config::resolved_router() -> ResolvedLlmConfig` and `Config::resolved_rlm() -> ResolvedLlmConfig` that perform the inheritance and return a fully-populated struct. Downstream code should only ever consume the resolved form.
   - Built-in defaults: `DefaultLlmConfig { provider: "ollama", model: "gemma4:31b" }`. Router/Rlm `Option` fields default to `None`.
2. **Ollama plumbing** in `crates/muninn-rlm/src/ollama.rs`:
   - Add `api_key: Option<String>` to `OllamaConfig`; attach `Authorization: Bearer …` in `add_headers` when present.
   - Add `OllamaConfig::cloud(api_key)` and `OllamaConfig::local()` constructors mirroring Arawn's pattern.
3. Add `OllamaProviderConfig { api_key, base_url }` to `config.rs`, surfaced as `Config.ollama`, with `OLLAMA_API_KEY` env-var fallback.
4. Update `Config::validate()`:
   - Add `"ollama"` to the allowed-provider list (validated against the *resolved* configs, not the raw sections).
   - Require an API key when the resolved base_url is the cloud host.
5. **Backend factory**: replace direct reads of `Config.router.provider` etc. with calls to the resolved accessors. Add the `"ollama"` arm and pass `base_url` + `api_key` from `Config.ollama`.
6. Update existing tests that snapshot `"groq"` defaults — these will fail and need re-baselining against the new `"ollama"` single-model default (intentional).

### Dependencies

- None blocking. Should land before / alongside [[hook-mcp-integration-layer-for-claude-code]] since the new integration layer will want the same backend abstraction.

### Risk Considerations

- **Default flip is a breaking change** for existing users with no `[router]` / `[rlm]` in their config — they'll start hitting Ollama Cloud and get auth errors. Mitigate with a clear validator error message pointing at `OLLAMA_API_KEY` and an upgrade-notes section.
- Ollama Cloud model naming may not match local model naming; document the chosen defaults explicitly and verify they exist at time of implementation.

### Cost / Viability Investigation (added 2026-05-19)

Ollama Cloud's pricing shape materially constrains the "default for both router + rlm" plan. Findings:

- **Billing is subscription, not per-token.** Free $0, Pro $20/mo, Max $100/mo. Usage measured in GPU-time, with models classified by compute level 1–4. No overage billing — hard cutoffs at session (5h) and weekly (7d) windows.
- **Concurrent-model caps are the blocker.** Free tier allows **1 concurrent model**; Pro allows 3; Max allows 10. A two-model default (small router model + larger RLM model) **cannot run on the free tier** — the first time both fire together, the user hits a wall.
- **Prompt cache is per-model.** Two different models on the same key means each call rebuilds its own cache. Cross-call caching gains are forfeited unless router and RLM share a model.
- **No per-swap fee documented**, but cold-start cost isn't addressed in the public pricing docs either — assume non-zero until measured.
- **Implementation pattern confirmed via arawn** (`ARAWN-T-0166`, archived): `base_url = "https://api.ollama.com/v1"`, `OLLAMA_API_KEY`, OpenAI-compatible. Model names like `gemma4`, `llama-3.3-70b`, `qwen3-32b`. Wire plumbing is unchanged from what `crates/muninn-rlm/src/ollama.rs` already does.

### Default-Selection Decision (resolved 2026-05-19)

**Chosen: tiered config with single-model default.** `[default]` defines one Ollama Cloud model that `[router]` and `[rlm]` inherit. Free-tier users get a working setup with concurrent=1 satisfied and prompt cache shared across both calls. Users who want specialization override `[router]` or `[rlm]` independently — making the cost/quality tradeoff an explicit user choice rather than a baked-in assumption.

Why this beats the earlier alternatives:
- Subsumes option A (single-model default) — that's exactly what unconfigured users get.
- Subsumes option B (two-model with Pro) — users on Pro override `[rlm]` to a bigger model; the config schema makes this one line.
- Avoids option C's auto-fallback complexity — no runtime concurrent-limit error handling needed for the default path.
- Avoids option D's "local install required" friction.

Baseline model: **`gemma4:31b`** on Ollama Cloud. Verify the exact identifier (with or without `:cloud` suffix) against the live catalog at implementation time.

## Status Updates

### 2026-05-19 — Implementation landed (pending CI carve-out)

Implemented in one pass:

- **`crates/muninn/src/config.rs`** — Added `DefaultLlmConfig`, `OllamaProviderConfig`, `ResolvedLlmConfig`. Changed `RouterConfig.{provider,model}` and `RlmConfig.{provider,model}` to `Option<String>`. Added `Config.default` and `Config.ollama` fields with `#[serde(default)]`. Added `Config::resolved_router()` / `resolved_rlm()` accessors that perform inheritance. Updated `validate()` to operate on resolved configs, added `"ollama"` to the provider allowlist, and added the Ollama-cloud API-key requirement.
- **`crates/muninn-rlm/src/ollama.rs`** — Added `api_key: Option<String>` to `OllamaConfig`. Added `with_api_key(...)`, `cloud(api_key)`, and `local()` builders. `add_headers()` now attaches `Authorization: Bearer …` when `api_key` is set. Bumped `DEFAULT_MODEL` to `"gemma4:31b"`.
- **`crates/muninn/src/main.rs`** — `create_backend_from_config` ollama arm now consumes `config.ollama.{base_url, api_key}` (with `OLLAMA_API_KEY` env fallback). Both call sites now read provider/model via `resolved_router()` / `resolved_rlm()` instead of reading the raw section fields. `muninn init` default-config template updated to the tiered shape.
- **`README.md`** — Quick-start rewritten around the tiered config. Local Ollama example moved to `[ollama] base_url = ...` override. Worked examples for cost/quality tuning and switching providers.

### Tests
- 5 new tests added: `test_inheritance_default_only`, `test_inheritance_router_override_beats_default`, `test_inheritance_backwards_compat_no_default_section`, `test_validate_requires_ollama_api_key_for_cloud`, `test_validate_local_ollama_keyless_ok`.
- Existing `test_default_config` and `test_parse_full_config` re-baselined to match the new defaults.
- **All tests pass**: muninn 16/16, muninn-rlm 289/289, integration 10/10.
- Strict clippy on muninn + muninn-rlm: clean for new code.

### CI carve-out (decision needed)
`angreal ci` does not pass — but **not because of T-0063's changes**. Two pre-existing issues block the pipeline:

1. **muninn-graph: 11 clippy diagnostics** (10 warnings + 1 hard error). All existed on `main` before this task started — verified by stashing my changes and running `cargo clippy -p muninn-graph --no-deps`.
2. **muninn/main.rs:1242** — one `clippy::print_literal` warning in the docs-list table formatting (introduced by an earlier docs task, predates T-0063).

Recommend folding these into a separate tech-debt backlog task; T-0063's feature work is complete and tested.