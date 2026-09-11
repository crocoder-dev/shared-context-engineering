#!/usr/bin/env bash
set -euo pipefail

sce_pre_tool_use_deny() {
  printf '%s' '{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"SCE could not establish mutation attribution for this tool execution."}}'
}

if [ "${SCE_CODEX_PRE_TOOL_USE_FAIL_CLOSED:-0}" = "1" ]; then
  if ! command -v sce >/dev/null 2>&1; then
    echo "sce CLI not found. Install it from https://sce.crocoder.dev/docs/getting-started#install-cli" >&2
    sce_pre_tool_use_deny
    exit 0
  fi
  if ! adapter_output="$("$@")"; then
    echo "SCE mutation-scope adapter failed; denying the tracked tool to preserve fail-closed PreToolUse." >&2
    sce_pre_tool_use_deny
    exit 0
  fi
  printf '%s' "$adapter_output"
  exit 0
fi

if ! command -v sce >/dev/null 2>&1; then
  echo "sce CLI not found. Install it from https://sce.crocoder.dev/docs/getting-started#install-cli" >&2
  exit 0
fi

exec "$@"