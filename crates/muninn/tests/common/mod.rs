//! Shared helpers for the integration / UAT tests.
//!
//! Cargo's `tests/common/mod.rs` convention: this file isn't compiled
//! as its own test binary. Each integration test that needs these
//! helpers declares `mod common;` and uses the items here directly.
//!
//! ## Provider selection
//!
//! UAT tests run against one LLM provider at a time. The provider is
//! selected by the `MUNINN_UAT_PROVIDER` environment variable (one
//! of `ollama` / `groq` / `anthropic`), defaulting to `ollama` for
//! backwards compatibility. `MUNINN_UAT_MODEL` overrides the model;
//! when unset we pick a sensible default per provider.
//!
//! `angreal test uat --provider <name>` sets these env vars so a
//! single invocation can target a specific backend. Pass
//! `--provider all` to iterate over every provider whose API key
//! is present in the decrypted secrets bundle.

#![allow(dead_code)]

/// The provider selected for this UAT run.
///
/// `MUNINN_UAT_PROVIDER` overrides the default; we fall back to
/// `ollama` so existing test invocations (`cargo test -- --ignored`
/// with `OLLAMA_API_KEY` set) keep working unchanged.
pub fn uat_provider() -> String {
    std::env::var("MUNINN_UAT_PROVIDER").unwrap_or_else(|_| "ollama".to_string())
}

/// Model to use for the selected provider. `MUNINN_UAT_MODEL` lets
/// callers override; otherwise we pick a known-good default per
/// provider. The defaults MUST support tool calling reliably
/// because the RLM exploration loop and the router both depend on
/// it. Empirically:
///   - `qwen/qwen3-32b` on Groq passes all 11/11 UAT tests with the
///     `tool_choice` plumbing fix in place.
///   - `llama-3.3-70b-versatile` intermittently emits XML-style
///     inline function calls (`<function=name>...</function>`) that
///     Groq's API rejects.
///   - `openai/gpt-oss-20b` leaks harmony control tokens
///     (`<|channel|>...`) into tool names on some prompt shapes.
pub fn uat_model() -> String {
    if let Ok(m) = std::env::var("MUNINN_UAT_MODEL") {
        return m;
    }
    match uat_provider().as_str() {
        "groq" => "qwen/qwen3-32b".to_string(),
        "anthropic" => "claude-haiku-4-5-20251001".to_string(),
        // ollama / anything else
        _ => "gemma4:31b".to_string(),
    }
}

/// Name of the environment variable that holds the API key for
/// `provider`. Unknown providers fall back to the ollama key —
/// the caller's skip check will then trip out gracefully.
pub fn provider_env_var(provider: &str) -> &'static str {
    match provider {
        "groq" => "GROQ_API_KEY",
        "anthropic" => "ANTHROPIC_API_KEY",
        _ => "OLLAMA_API_KEY",
    }
}

/// True if the API key required to drive `provider` is present in
/// the process environment.
pub fn provider_credential_present(provider: &str) -> bool {
    std::env::var_os(provider_env_var(provider)).is_some()
}

/// True iff the credentials needed to exercise the selected UAT
/// provider are present. Use this to gate UAT tests behind a
/// clean skip when the harness wasn't given the right secrets.
pub fn uat_credentials_present() -> bool {
    provider_credential_present(&uat_provider())
}

/// TOML fragment that wires `[default]` to the selected provider
/// and model. Drop this verbatim into a `.muninn/config.toml`
/// templated for a UAT test — no extra interpolation needed.
pub fn uat_default_config_fragment() -> String {
    let p = uat_provider();
    let m = uat_model();
    format!(
        "[default]\nprovider = \"{p}\"\nmodel = \"{m}\"\n",
        p = p,
        m = m,
    )
}

/// Optional `[router]` model override per provider. Some providers
/// (notably Groq's `llama-3.3-70b-versatile`) are strict about
/// forced tool calls and occasionally fail validation on the
/// router's `route_decision` schema; a smaller/faster model with
/// looser generation behaves better here AND matches what users
/// would realistically configure (cheap fast router, bigger RLM).
/// Returns `None` when the default model is fine.
pub fn uat_router_model_override() -> Option<String> {
    match uat_provider().as_str() {
        "groq" => Some("llama-3.1-8b-instant".to_string()),
        _ => None,
    }
}

/// TOML fragment for the `[router]` block. Always sets
/// `strategy = "llm"`; adds a model override when the provider
/// needs one (see `uat_router_model_override`).
pub fn uat_router_config_fragment() -> String {
    let mut out = String::from("[router]\nstrategy = \"llm\"\n");
    if let Some(m) = uat_router_model_override() {
        out.push_str(&format!("model = \"{m}\"\n"));
    }
    out
}
