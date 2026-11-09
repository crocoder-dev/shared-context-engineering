#!/usr/bin/env bash
set -u
payload="$(cat)"
if printf '%s' "$payload" | grep -q 'PLEASE_BLOCK_THIS_MCP_CALL'; then
  printf '%s' '{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"blocked by MCP probe"}}'
fi
exit 0
