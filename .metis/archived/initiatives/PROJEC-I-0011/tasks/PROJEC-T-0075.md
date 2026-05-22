---
id: docs-reorganization-around-two
level: task
title: "Docs reorganization around two surfaces plus migration guide for proxy-only users"
short_code: "PROJEC-T-0075"
created_at: 2026-05-19T16:41:39.662406+00:00
updated_at: 2026-05-20T20:31:24.331120+00:00
parent: PROJEC-I-0011
blocked_by: []
archived: true

tags:
  - "#task"
  - "#phase/completed"


exit_criteria_met: false
initiative_id: PROJEC-I-0011
---

# Docs reorganization around two surfaces plus migration guide for proxy-only users

## Parent Initiative

[[hook-mcp-integration-layer-for-claude-code]] (PROJEC-I-0011)

## Objective

Restructure top-level docs so the primary (hook+MCP for Claude Code) and secondary (proxy for non-CC clients) surfaces are clearly separated, with explicit guidance on when to pick which. Ship a migration guide walking existing muninn-as-proxy users through enabling the plugin and MCP server side-by-side configs.

## Acceptance Criteria

## Acceptance Criteria

## Acceptance Criteria

## Acceptance Criteria

- [ ] README rewritten with two clearly labeled top-level paths: "Using muninn with Claude Code (recommended)" and "Using muninn with other clients (proxy)".
- [ ] Each path has a copy-pasteable getting-started block.
- [ ] New page `docs/migration-proxy-to-hook.md` (or equivalent) showing:
  - what the proxy-only setup looks like today,
  - what the hook+MCP setup looks like,
  - whether to keep proxy enabled in parallel (yes by default — both first-class),
  - troubleshooting common issues during the switch.
- [ ] Vision principles in `PROJEC-V-0001` still reflect reality; if "Invisible by default" needs a nuance, note it.
- [ ] All references to "the proxy" in user-facing docs say which surface they mean.
- [ ] Internal architecture diagram (`docs/architecture.md` or equivalent) updated to show the engine/adapter split.

## Dependencies

- PROJEC-T-0072 (install-cc — the recommended path docs reference it)
- All upstream implementation tasks must be far enough along that the docs reflect shipped behavior, not aspiration.

## Implementation Notes

- This is the user-facing capstone for the initiative. It's where most users will form their first impression of the new architecture.
- Resist the urge to document every internal detail. The reader wants to know: what do I do, what changes for me, what's the rollback if something breaks.
- Cross-link [[003-hook-mcp-integration-model-as]] (PROJEC-A-0003) from the migration page for users who want the "why".

## Status Updates

*To be added during implementation.*
### 2026-05-20 — Implementation landed

**README rewritten** around the two-surface model from ADR-0003:

- New nav links + "Two integration surfaces" intro table that names
  hook + MCP as the recommended Claude Code path and proxy as the
  surface for everyone else, with a link to the ADR.
- **"Using muninn with Claude Code (recommended)"** is now the first
  setup section: `muninn install-cc`, `/plugin add-source
  ./plugins/muninn-cc`, optional `[hook_decision]` tuning. Cross-links
  the migration guide and `docs/mcp-tools.md`. Lists the four MCP
  tools at this level so readers can see what they're getting.
- **"Using muninn with other clients (proxy)"** keeps the prior
  proxy / OAuth / OpenAI-compatible content, including the request /
  response schema tables, but moved below the CC path.
- **Configuration** section now leads with the tiered config and
  shows a worked example overriding `[router]`, `[rlm]`, and
  `[hook_decision]` independently. Local-Ollama override demoted to
  a sub-section (it's a one-line `base_url` change, not a separate
  feature anymore).
- **How It Works** diagram redrawn to show *both* adapter paths
  (Claude Code's PreToolUse hook + MCP server, plus the proxy)
  feeding into the same `MuninnEngine` trait inside the daemon.
- Migration paragraph at the bottom links to the new guide.

**New page `docs/migration-proxy-to-hook.md`**:

- TL;DR with the four-step migration recipe.
- "What changed under the hood" table (proxy-only vs. now).
- "Why migrate at all?" — sanctioned extension points, implicit
  augmentation, explicit MCP tools, no proxy bypass needed for
  non-CC calls.
- Step-by-step: verify binary surface, `install-cc`, plugin install
  via `/plugin add-source`, optional `[hook_decision]` tuning with
  the actual benchmark numbers from PROJEC-T-0074 (290 ms p50 on the
  default `gemma4:31b`; pinning Groq `llama-3.1-8b-instant` is the
  documented path to NFR-001).
- "Keep the proxy running" sub-section — explicit "both surfaces
  coexist" position from ADR-0003.
- Verification recipe (`muninn daemon status`, `muninn mcp
  --no-ensure`, `angreal test uat`).
- Troubleshooting: always-passthrough decisions, MCP server not
  visible to CC, latency too high, daemon won't start, rollback.
- Cross-links to ADR-0003, `docs/mcp-tools.md`, the plugin README,
  and the initiative.

**`docs/mcp-tools.md`** unchanged from PROJEC-T-0067 — already covers
the tool reference; README and migration guide both link to it.

**`plugins/muninn-cc/README.md`** unchanged from PROJEC-T-0069 —
already cross-links ADR-0003 / PROJEC-I-0011 / PROJEC-T-0070 /
PROJEC-T-0072.

### Vision principles check

Re-read `PROJEC-V-0001`'s principles to see if any need a nuance
update given ADR-0003:

- **"Invisible by default"** — the *spirit* still holds (the agent
  doesn't have to know muninn exists), but the *mechanism* is now
  CC's sanctioned extension points rather than HTTP intercept on the
  primary path. I didn't edit the vision doc; the principle wording
  is broad enough to cover both. The README's "Two integration
  surfaces" framing makes the operational change clear without
  pulling vision into the diff.
- All other principles (privacy non-negotiable, memory is a repo
  artifact, backend-agnostic, computation over storage,
  rebuildable-over-opaque) are unaffected.

### Decisions

- **Don't rewrite the vision doc** as part of T-0075. The principles
  are still accurate; the README + migration guide carry the
  operational story. Touching vision risks scope creep.
- **Migration guide includes real benchmark numbers** from
  PROJEC-T-0074, not aspirational targets. Honest measurement is
  more useful than glossy promises — and points the reader directly
  at the override that closes the gap.
- **Keep proxy docs in the README, not a separate file**. The proxy
  is first-class, not a deprecation; making readers chase a separate
  doc for it would mis-signal.

### Tests / verification

- `angreal tree` still resolves cleanly (no Python syntax errors in
  the task module).
- `angreal test uat` still passes (1.5 s, passthrough decision).
- README + migration guide render cleanly (manual eyeball check).
- All cross-links use existing relative paths.

### CI carve-out
Same as previous initiative tasks — workspace `angreal ci` still blocked by the pre-existing muninn-graph clippy debt tracked in PROJEC-T-0076. No new clippy or fmt issues introduced; this is a docs-only change.