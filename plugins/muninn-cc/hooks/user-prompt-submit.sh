#!/bin/bash
# muninn-cc UserPromptSubmit hook.
#
# Fires once per user message in Claude Code, before the agent
# starts. Lets muninn pre-inject project context into the turn so
# Claude has the answer from the first token.
#
# NFR-002 contract: every failure path collapses to silent passthrough
# so muninn can never block the user's turn from starting.

set -u

if ! command -v muninn >/dev/null 2>&1; then
    exit 0
fi

# Make sure the daemon is up. `daemon ensure` is idempotent (no-op
# when alive) so the cost is paid once per CC session — well under
# 2s — and only when the daemon isn't already running. Without this,
# the hook would silently passthrough every turn in setups where
# MCP hasn't been invoked yet to start the daemon.
muninn daemon ensure >/dev/null 2>&1 || true

RESPONSE=$(muninn hook submit 2>/dev/null) || exit 0

if [ -n "$RESPONSE" ]; then
    printf '%s' "$RESPONSE"
fi
exit 0
