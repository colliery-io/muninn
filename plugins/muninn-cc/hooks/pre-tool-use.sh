#!/bin/bash
# muninn-cc PreToolUse hook.
#
# Claude Code pipes a JSON hook-input payload to this script's stdin when an
# agent is about to run `Grep`, `Read`, or `Glob`. The script delegates the
# augment / passthrough / rewrite decision to `muninn hook decide`
# (PROJEC-T-0070) and relays the response.
#
# NFR-002: the hook MUST NEVER block the user's tool call. Every failure
# path here falls through to a silent passthrough (exit 0, no stdout) so
# Claude Code runs the original tool unchanged.

set -u

# 1. If the muninn binary isn't installed or isn't on PATH, drop straight
#    through. Common case during initial setup; not an error.
if ! command -v muninn >/dev/null 2>&1; then
    exit 0
fi

# 2. Pipe Claude Code's hook-input through `muninn hook decide`. The
#    subcommand inherits our stdin directly, so we don't have to buffer or
#    re-encode anything.
#
#    Any of these conditions fall through to silent passthrough:
#      - subcommand doesn't exist yet (e.g. before PROJEC-T-0070 ships)
#      - subcommand exits non-zero
#      - subcommand times out (Claude Code enforces the hooks.json `timeout`)
#      - subcommand writes garbage to stdout
#
#    The `|| exit 0` ensures we never propagate a non-zero exit upward.
RESPONSE=$(muninn hook decide 2>/dev/null) || exit 0

# 3. Relay any non-empty stdout to Claude Code. An empty response is also
#    valid — it signals "allow original tool unchanged", which is what we
#    want for the `passthrough` decision.
if [ -n "$RESPONSE" ]; then
    printf '%s' "$RESPONSE"
fi
exit 0
