#!/usr/bin/env bash
# T01 MCP-probe dump hook. NOT SCE runtime code.
# Reads the raw Codex hook payload from STDIN, records it verbatim, and exits 0
# with empty stdout (a neutral no-op) so it never influences Codex behaviour.
#
# The capture directory is passed as $1 (absolute path, baked into the
# .codex/hooks.json command by run-probes.sh) because Codex runs hook commands
# with a cleared environment (see codex-rs/hooks/src/engine/command_runner.rs
# build_command: env_clear()).
set -u
CAP="${1:?capture dir arg missing}"
mkdir -p "$CAP/events" 2>/dev/null || true

payload="$(cat)"
ts="$(date -u +%Y-%m-%dT%H:%M:%S.%NZ)"

event="$(printf '%s' "$payload" | sed -n 's/.*"hook_event_name":"\([^"]*\)".*/\1/p')"
[ -z "$event" ] && event="unknown"
tuid="$(printf '%s' "$payload" | sed -n 's/.*"tool_use_id":"\([^"]*\)".*/\1/p')"
tname="$(printf '%s' "$payload" | sed -n 's/.*"tool_name":"\([^"]*\)".*/\1/p')"

# Concurrent hook deliveries (parallel MCP calls) run this script at the same
# time, so serialise the shared counter/log writes with a lock and give each
# delivery a collision-proof filename (nanosecond timestamp + pid).
uniq="$(date -u +%Y%m%dT%H%M%S.%N)-$$"
( flock 9
  n="$(cat "$CAP/.seq" 2>/dev/null || echo 0)"
  n=$((n + 1))
  echo "$n" > "$CAP/.seq"
  printf '%s\t%s\tevent=%s\ttool_name=%s\ttool_use_id=%s\n' \
    "$n" "$ts" "$event" "$tname" "$tuid" >> "$CAP/_sequence.log"
  printf '%s\n' "$payload" >> "$CAP/_stream.ndjson"
) 9>"$CAP/.lock"

slot="${uniq}-${event}"
[ -n "$tname" ] && slot="${slot}-${tname}"
printf '%s' "$payload" > "$CAP/events/${slot}.json"

exit 0
