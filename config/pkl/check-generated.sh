#!/usr/bin/env bash
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"

if [[ -z "${IN_NIX_SHELL:-}" ]]; then
  cat <<'EOF'
This integration check must run in the Nix dev shell.

Run with:
  nix develop -c ./config/pkl/check-generated.sh
EOF
  exit 1
fi

if ! command -v pkl >/dev/null 2>&1; then
  printf 'pkl is not available in PATH. Enter the dev shell with: nix develop\n' >&2
  exit 1
fi

tmp_dir="$(mktemp -d)"
cleanup() {
  rm -rf "$tmp_dir"
}
trap cleanup EXIT

pkl eval -m "$tmp_dir" config/pkl/generate.pkl >/dev/null

paths=(
  ".github/workflows/publish-tiles.yml"
  "config/.opencode/agent"
  "config/.opencode/command"
  "config/.opencode/skills"
  "config/.opencode/lib/drift-collectors.js"
  "config/automated/.opencode/agent"
  "config/automated/.opencode/command"
  "config/automated/.opencode/skills"
  "config/automated/.opencode/lib/drift-collectors.js"
  "config/.claude/agents"
  "config/.claude/commands"
  "config/.claude/skills"
  "config/.claude/lib/drift-collectors.js"
  "config/schema/sce-config.schema.json"
)

stale=0
for path in "${paths[@]}"; do
  if ! git diff --no-index --exit-code -- "$tmp_dir/$path" "$path" >/dev/null; then
    stale=1
    printf 'Generated output drift detected at %s\n' "$path"
    git diff --no-index -- "$tmp_dir/$path" "$path" || true
  fi
done

if [[ "$stale" -ne 0 ]]; then
  cat <<'EOF'
Generated files are stale.

Regenerate with:
  nix develop -c pkl eval -m . config/pkl/generate.pkl
EOF
  exit 1
fi

printf 'Generated outputs are up to date.\n'
