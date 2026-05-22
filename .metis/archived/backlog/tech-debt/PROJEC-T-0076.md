---
id: clear-pre-existing-clippy-debt-in
level: task
title: "Clear pre-existing clippy debt in muninn-graph and muninn so angreal ci is green"
short_code: "PROJEC-T-0076"
created_at: 2026-05-19T19:45:26.074942+00:00
updated_at: 2026-05-20T20:40:55.181250+00:00
parent: 
blocked_by: []
archived: true

tags:
  - "#task"
  - "#tech-debt"
  - "#phase/completed"


exit_criteria_met: false
initiative_id: NULL
---

# Clear pre-existing clippy debt in muninn-graph and muninn so angreal ci is green

## Objective

`angreal ci` fails on `cargo clippy -- -D warnings` due to issues that predate PROJEC-T-0063. Surface area is small but spread across two crates. Land a one-shot cleanup so future feature PRs can satisfy "angreal ci passes" without scope creep.

## Type / Priority

- Type: Tech debt
- Priority: P2 — not a user-facing issue, but every feature task is currently forced to either fix unrelated lints or carve out the ci AC. Closing this restores clean signal.

## Acceptance Criteria

## Acceptance Criteria

## Acceptance Criteria

## Acceptance Criteria

## Acceptance Criteria

- [ ] `cargo clippy --workspace --no-deps -- -D warnings` exits 0 on a clean checkout.
- [ ] `angreal ci` passes end-to-end (fmt, clippy, build, test).
- [ ] No behavior change. Pure lint-driven refactors.
- [ ] Commit message lists each fix one-liner so it's clear nothing snuck in.

## Known Issues (as of 2026-05-19)

### muninn-graph (10 warnings + 1 error)

Verified by stashing PROJEC-T-0063 changes and running `cargo clippy -p muninn-graph --no-deps -- -D warnings`. Per-issue list (from the clippy output):

- `method from_str can be confused for FromStr::from_str` — at least 2 sites (registry parsing helpers).
- `called unwrap_err on result after checking its variant with is_err`.
- `this impl can be derived` — manual `Default` impl that should use `#[derive(Default)]`.
- `stripping a prefix manually` — 3 sites; use `strip_prefix` instead.
- `this loop never actually loops` — likely an `Iterator::next` pattern that should just be the call itself.
- `you seem to be trying to use match for destructuring a single pattern. Consider using if let`.
- `parameter is only used in recursion` — 2 sites; either reuse via mutable state or accept the lint with an `#[allow]` and document why.

### muninn

- `crates/muninn/src/main.rs:1242` — `clippy::print_literal`: `"INDEXED AT"` as a format arg when it should be inlined into the format string.

## Implementation Notes

- Treat this as a single PR. Don't bundle other changes.
- For each lint, prefer the *suggested fix* clippy emits unless it changes behavior. If you have to `#[allow(...)]` something (e.g. recursion-only parameter pattern), add a one-line comment explaining why.
- After the cleanup, re-run `angreal ci` locally to confirm exit 0 before opening the PR.

## Dependencies

None. Can land at any time. Should land soon so PROJEC-T-0063's CI carve-out is closed.

## Status Updates

*To be added during implementation.*
### 2026-05-20 — Implementation landed; angreal ci now green

Walked through every clippy diagnostic the pre-existing audit listed plus one straggler that surfaced when CI ran the tests-as-clippy-targets. Each fix is a small, behavior-preserving edit.

**muninn-graph (10 lints):**

1. `doc_store.rs:43` Ecosystem::from_str — would conflict with `FromStr::from_str` but the inherent method returns `Option<Self>` (vs. trait's `Result<Self, Err>`), so add `#[allow(clippy::should_implement_trait)]` with a comment explaining the intent.
2. `doc_store.rs:88` ItemType::from_str — same fix as (1).
3. `doc_store.rs:535` `result.is_err()` followed by `result.unwrap_err()` → `if let Err(e) = result`. Pure refactor; no allocation/control-flow change.
4. `registry/indexer.rs:73` manual `impl Default for IndexerConfig` whose body is `Default::default()` → `#[derive(Default)]` on the struct, drop the impl.
5. `registry/llmstxt.rs:118` manual `trimmed[2..]` slice after `starts_with("# ")` → `if let Some(rest) = trimmed.strip_prefix("# ")`. Also collapse with the existing `&& !trimmed.starts_with("## ")` guard.
6. `registry/llmstxt.rs:129` ditto for `"## "`.
7. `registry/llmstxt.rs:139` ditto for `"> "`.
8. `registry/llmstxt.rs:208` `rest.starts_with(':') { rest[1..].trim().to_string() }` → `rest.strip_prefix(':').map(|s| s.trim().to_string())`. Removes one branch.
9. `registry/pydoc.rs:469` `loop { ...; break; }` (one-iteration loop) → straight-line code that reads the first statement under the block cursor. Same behavior, less reader confusion.
10. `registry/rustdoc.rs:424` single-pattern `match` (`StructKind::Plain { fields, .. } => { ... } _ => {}`) → `if let StructKind::Plain { fields, .. } = ...`.
11. `registry/rustdoc.rs:653,655` `format_type`'s `krate` + `cache` parameters are only used inside the recursive call. Both are required by the recursion; `#[allow(clippy::only_used_in_recursion)]` on the fn with a comment explaining why.

**muninn (1 lint):**

12. `main.rs:1242` (now 1348) — `println!("{:<20} {:<10} {:<10} {}", "LIBRARY", "VERSION", "ECOSYSTEM", "INDEXED AT")` had a bare `{}` for the literal "INDEXED AT". Inline the literal into the format string and drop it from the argument list.

**Test-time straggler:**

13. `muninn-core/src/daemon.rs:852` `let _ = server_task.abort();` — `JoinHandle::abort()` returns `()`, so the `let _ = ` was redundant. Drop the binding.

### Verification

- `cargo clippy --workspace --no-deps -- -D warnings` → exit 0.
- `angreal ci` → **PASS**. All four stages clean (fmt, clippy, build, test).
- `cargo test --workspace` → all 17 test suites green; no regressions from the refactors.

### Decisions

- **Two `#[allow(...)]` annotations** rather than full rewrites: `should_implement_trait` on the two `from_str` methods (semantics differ from the standard trait — they're intentionally `Option`-returning) and `only_used_in_recursion` on `format_type` (the parameters are genuinely needed by the recursive call). Both carry comments explaining why so future readers don't second-guess.
- **No behavior changes anywhere.** Every fix was either a syntactic rewrite (`if let`, `strip_prefix`, `#[derive]`) or an allow-with-comment. Workspace tests are the safety net — they all still pass.

### CI carve-out
**CLOSED.** This was the last initiative blocker on `angreal ci`. The "same as previous initiative tasks" line that's been at the bottom of every T-006x status update no longer applies — future feature work can simply say "ci is green."