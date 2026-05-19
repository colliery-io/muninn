---
id: add-hook-decision-config-and
level: task
title: "Add [hook_decision] config and implement muninn hook decide subcommand"
short_code: "PROJEC-T-0070"
created_at: 2026-05-19T16:41:32.341540+00:00
updated_at: 2026-05-19T16:41:32.341540+00:00
parent: PROJEC-I-0011
blocked_by: []
archived: false

tags:
  - "#task"
  - "#phase/todo"


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

*To be added during implementation.*
