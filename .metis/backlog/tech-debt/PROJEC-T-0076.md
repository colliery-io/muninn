---
id: clear-pre-existing-clippy-debt-in
level: task
title: "Clear pre-existing clippy debt in muninn-graph and muninn so angreal ci is green"
short_code: "PROJEC-T-0076"
created_at: 2026-05-19T19:45:26.074942+00:00
updated_at: 2026-05-19T19:45:26.074942+00:00
parent: 
blocked_by: []
archived: false

tags:
  - "#task"
  - "#phase/backlog"
  - "#tech-debt"


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
