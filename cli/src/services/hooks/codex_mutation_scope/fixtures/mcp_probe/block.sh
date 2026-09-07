#!/usr/bin/env bash
# T01 MCP-probe PreToolUse block hook. NOT SCE runtime code.
# Second PreToolUse handler (after dump.sh). Only blocks when the payload
# contains the marker "PLEASE_BLOCK_THIS_MCP_CALL"; otherwise a neutral no-op.
# Used to prove whether a blocked MCP call still emits PostToolUse.
set -u
payload="$(cat)"
if printf '%s' "$payload" | grep -q 'PLEASE_BLOCK_THIS_MCP_CALL'; then
  printf '%s' '{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"blocked by MCP probe"}}'
fi
exit 0
