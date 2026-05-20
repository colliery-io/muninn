#!/bin/bash
# muninn-cc UserPromptSubmit hook.
#
# Fires once per user message in Claude Code, before the agent
# starts. Lets muninn pre-inject project context (related memory,
# graph hits, code references) into the turn so Claude has it from
# the first token.
#
# Same NFR-002 contract as the PreToolUse hook: every failure path
# collapses to silent passthrough so muninn can never block the
# user's turn from starting.

set -u

if ! command -v muninn >/dev/null 2>&1; then
    exit 0
fi

RESPONSE=$(muninn hook submit 2>/dev/null) || exit 0

if [ -n "$RESPONSE" ]; then
    printf '%s' "$RESPONSE"
fi
exit 0
