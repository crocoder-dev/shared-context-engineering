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

pkl eval config/pkl/renderers/metadata-coverage-check.pkl >/dev/null
pkl eval -m "$tmp_dir" config/pkl/generate.pkl >/dev/null

# Compare complete generated directories so nested skill references and stale
# files are covered without maintaining a separate per-document path list.
paths=(
  "config/.opencode/agent"
  "config/.opencode/command"
  "config/.opencode/skills"
  "config/.opencode/lib"
  "config/.opencode/plugins"
  "config/.opencode/opencode.json"
  "config/.claude/commands"
  "config/.claude/skills"
  "config/.claude/hooks"
  "config/.claude/settings.json"
  "config/.pi/prompts"
  "config/.pi/skills"
  "config/.pi/extensions"
  "config/schema/sce-config.schema.json"
)

# These removed surfaces must stay absent from both generated previews and the
# committed tree. Obsolete files inside retained directories are caught by the
# complete-directory comparisons above.
forbidden_paths=(
  "config/automated/.opencode"
  "config/.claude/agents"
)

stale=0
for path in "${paths[@]}"; do
  if [[ ! -e "$tmp_dir/$path" ]]; then
    stale=1
    printf 'Generator did not emit required output at %s\n' "$path"
    continue
  fi

  if [[ ! -e "$path" ]]; then
    stale=1
    printf 'Required generated output is missing at %s\n' "$path"
    continue
  fi

  if ! git diff --no-index --exit-code -- "$tmp_dir/$path" "$path" >/dev/null; then
    stale=1
    printf 'Generated output drift detected at %s\n' "$path"
    git diff --no-index -- "$tmp_dir/$path" "$path" || true
  fi
done

for path in "${forbidden_paths[@]}"; do
  if [[ -e "$tmp_dir/$path" ]]; then
    stale=1
    printf 'Generator emitted removed output at %s\n' "$path"
  fi
  if [[ -e "$path" ]]; then
    stale=1
    printf 'Removed generated output still exists at %s\n' "$path"
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
