#!/usr/bin/env bash
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"

tmp_dir="$(mktemp -d)"
cleanup() {
  rm -rf "$tmp_dir"
}
trap cleanup EXIT

nix develop -c pkl eval -m "$tmp_dir" config/pkl/generate.pkl >/dev/null

paths=(
  "config/.opencode/agent"
  "config/.opencode/command"
  "config/.opencode/skills"
  "config/.opencode/lib/drift-collectors.js"
  "config/.claude/agents"
  "config/.claude/commands"
  "config/.claude/skills"
  "config/.claude/lib/drift-collectors.js"
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
