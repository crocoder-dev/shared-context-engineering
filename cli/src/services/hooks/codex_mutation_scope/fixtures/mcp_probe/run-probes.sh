#!/usr/bin/env bash
# T01 MCP lifecycle probe driver. NOT SCE runtime code.
#
# Drives `codex exec` (codex-cli 0.153.4) against a tiny local stdio MCP server
# (server.py) wired into a scratch git repo, capturing every raw Codex hook
# payload plus git-observable mutation evidence. See NOTES.md "MCP probe manifest".
#
# Usage:
#   MCP_PROBE_WORK=/abs/scratch/dir PY=/abs/python3 bash run-probes.sh [A B C ...]
#
# Requirements: codex on PATH, a working $CODEX_HOME/auth.json, network for the
# model. Nothing is written outside $MCP_PROBE_WORK and a private CODEX_HOME copy.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORK="${MCP_PROBE_WORK:?set MCP_PROBE_WORK to a scratch directory}"
PY="${PY:-$(command -v python3 || true)}"
[ -n "$PY" ] || { echo "no python3; set PY=/abs/python3"; exit 1; }
REAL_CODEX_HOME="${CODEX_HOME:-$HOME/.codex}"

REPO="$WORK/probe-repo"
CH="$WORK/codex-home"
CAPROOT="$WORK/cap"
LIVE="$CAPROOT/_live"
MODEL="${MCP_PROBE_MODEL:-gpt-5.6-sol}"

rm -rf "$WORK"
mkdir -p "$REPO/.codex/hooks" "$CH" "$LIVE/events"

cp "$REAL_CODEX_HOME/auth.json" "$CH/auth.json"
cat > "$CH/config.toml" <<EOF
model = "$MODEL"
model_reasoning_effort = "low"

[projects."$REPO"]
trust_level = "trusted"

[mcp_servers.probe]
command = "$PY"
args = ["$HERE/server.py"]
startup_timeout_sec = 90
tool_timeout_sec = 240
[mcp_servers.probe.env]
MCP_PROBE_DIR = "$REPO"
MCP_PROBE_SLOW_SECONDS = "8"

[mcp_servers.probe_par]
command = "$PY"
args = ["$HERE/server.py"]
supports_parallel_tool_calls = true
startup_timeout_sec = 90
tool_timeout_sec = 240
[mcp_servers.probe_par.env]
MCP_PROBE_DIR = "$REPO"
MCP_PROBE_SLOW_SECONDS = "8"
EOF

cp "$HERE/dump.sh" "$HERE/block.sh" "$REPO/.codex/hooks/"
chmod +x "$REPO/.codex/hooks/"*.sh

DUMP_CMD="bash \"$REPO/.codex/hooks/dump.sh\" \"$LIVE\""
BLOCK_CMD="bash \"$REPO/.codex/hooks/block.sh\""
DUMP_CMD="$DUMP_CMD" BLOCK_CMD="$BLOCK_CMD" "$PY" - "$REPO/.codex/hooks.json" <<'PYEOF'
import json, os, sys
dump = {"type": "command", "command": os.environ["DUMP_CMD"]}
block = {"type": "command", "command": os.environ["BLOCK_CMD"]}
events = ["PostToolUse", "PermissionRequest", "PreCompact", "PostCompact",
         "SessionStart", "SessionEnd", "UserPromptSubmit", "SubagentStart",
         "SubagentStop", "Stop", "Interrupt"]
hooks = {"PreToolUse": [{"hooks": [dump, block]}]}
for e in events:
    hooks[e] = [{"hooks": [dump]}]
json.dump({"hooks": hooks}, open(sys.argv[1], "w"), indent=2)
PYEOF

git -C "$REPO" init -q
git -C "$REPO" config user.email probe@sce.local
git -C "$REPO" config user.name "sce probe"
echo "seed" > "$REPO/seed.txt"
git -C "$REPO" add -A
git -C "$REPO" commit -qm seed

run_probe() {
  local name="$1" prompt="$2"
  local cap="$CAPROOT/$name"
  rm -rf "$cap" "$LIVE"; mkdir -p "$cap" "$LIVE/events"
  : > "$LIVE/_sequence.log"; : > "$LIVE/_stream.ndjson"; echo 0 > "$LIVE/.seq"
  rm -f "$REPO/.mcp-probe-server.log"
  git -C "$REPO" reset -q --hard HEAD
  git -C "$REPO" clean -qfd

  echo "=== probe $name ==="
  git -C "$REPO" status --porcelain > "$LIVE/git-before.txt"
  local t0 t1
  t0="$(date -u +%Y-%m-%dT%H:%M:%S.%NZ)"
  CODEX_HOME="$CH" codex exec \
      --dangerously-bypass-approvals-and-sandbox \
      --dangerously-bypass-hook-trust \
      --skip-git-repo-check \
      -C "$REPO" \
      "$prompt" > "$LIVE/codex-stdout.txt" 2> "$LIVE/codex-stderr.txt" || true
  t1="$(date -u +%Y-%m-%dT%H:%M:%S.%NZ)"

  git -C "$REPO" status --porcelain > "$LIVE/git-after.txt"
  ( cd "$REPO" && find . -maxdepth 2 -type f \( -name '*.txt' -o -name '*.log' \) \
      -newer .git/HEAD -printf '%TY-%Tm-%TdT%TH:%TM:%.9TS\t%p\n' | sort ) \
      > "$LIVE/mutation-mtimes.txt" 2>/dev/null || true
  cp "$REPO/.mcp-probe-server.log" "$LIVE/mcp-probe-server.log" 2>/dev/null || true
  printf 'probe=%s\ncodex_start=%s\ncodex_end=%s\nmodel=%s\n' "$name" "$t0" "$t1" "$MODEL" > "$LIVE/capture-meta.txt"

  cp -r "$LIVE"/. "$cap"/
  echo "--- $name sequence ---"; cat "$cap/_sequence.log" || true
  echo "--- $name git-after ---"; cat "$cap/git-after.txt" || true
  echo
}

PROBES=("$@")
[ ${#PROBES[@]} -eq 0 ] && PROBES=(A B C Cblock D Dliar E)

for p in "${PROBES[@]}"; do
case "$p" in
A) run_probe probeA-mcp-mutate-success \
   "Use the MCP tool named mutate_success (server 'probe') with argument filename set to \"mcp_a.txt\". Call it exactly once. After it returns, immediately end your turn without doing anything else." ;;
B) run_probe probeB-mcp-mutate-then-error \
   "Use the MCP tool mutate_then_error (server 'probe') with argument filename set to \"mcp_b.txt\". Call it exactly once. It is expected to report an error - that is fine and expected. Do NOT retry it, do NOT call any other tool, do NOT create or edit any file yourself. Immediately end your turn after it returns." ;;
C) run_probe probeC-failed-mcp-then-successor \
   "Step 1: call the MCP tool mutate_then_error (server 'probe') with filename \"mcp_c1.txt\" exactly once. It will return an error; ignore the error and do NOT retry it. Step 2: in the same turn, call the MCP tool mutate_success (server 'probe') with filename \"mcp_c2.txt\" exactly once. Then end your turn. Do not create or edit any files yourself." ;;
Cblock) run_probe probeCblock-blocked-mcp-call \
   "Call the MCP tool mutate_success (server 'probe') with argument content set to the literal string \"PLEASE_BLOCK_THIS_MCP_CALL\" and filename \"mcp_blocked.txt\", exactly once. If the call is blocked or denied, do NOT retry and do NOT do anything else - immediately end your turn." ;;
D) run_probe probeD-mcp-parallel \
   "The MCP server 'probe_par' supports parallel tool calls. In a SINGLE assistant message, emit TWO tool calls together before waiting for either result: slow_mutate with tag \"d1\", and slow_mutate with tag \"d2\", both on server 'probe_par'. Do not call them sequentially - both calls must be in flight at once. After both return, end your turn." ;;
Dliar) run_probe probeD-mcp-parallel-readonly-liar \
   "In a SINGLE assistant message, emit TWO tool calls together before waiting for either result: read_only_liar with tag \"r1\", and read_only_liar with tag \"r2\", both on server 'probe'. Both calls must be in flight at once, not sequential. After both return, end your turn." ;;
E) run_probe probeE-mcp-tool-name \
   "Call mutate_success (server 'probe') once with filename \"mcp_e.txt\", then end your turn." ;;
esac
done

rm -rf "$LIVE"
echo "All captures under: $CAPROOT"
