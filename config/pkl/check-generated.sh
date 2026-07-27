#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="${SCE_REPO_ROOT:-$(cd "${script_dir}/../.." && pwd)}"
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

legacy_paths=(
  "config/.opencode"
  "config/.claude"
  "config/.pi"
  "config/schema/sce-config.schema.json"
  "cli/assets/generated"
)

for path in "${legacy_paths[@]}"; do
  if [[ -e "$path" ]]; then
    printf 'Removed generated output still exists at %s\n' "$path" >&2
    exit 1
  fi
done

tmp_dir="$(mktemp -d)"
cleanup() {
  rm -rf "$tmp_dir"
}
trap cleanup EXIT

first_root="$tmp_dir/first"
second_root="$tmp_dir/second"
mkdir -p "$first_root" "$second_root"

pkl eval config/pkl/renderers/metadata-coverage-check.pkl >/dev/null
pkl eval -m "$first_root" config/pkl/generate.pkl >/dev/null
pkl eval -m "$second_root" config/pkl/generate.pkl >/dev/null

required_paths=(
  "config/.opencode/agent"
  "config/.opencode/command"
  "config/.opencode/skills"
  "config/.opencode/lib/bash-policy-presets.json"
  "config/.opencode/plugins/sce-bash-policy.ts"
  "config/.opencode/plugins/sce-agent-trace.ts"
  "config/.opencode/opencode.json"
  "config/.claude/commands"
  "config/.claude/skills"
  "config/.claude/hooks/run-sce-or-show-install-guidance.sh"
  "config/.claude/settings.json"
  "config/.pi/prompts"
  "config/.pi/skills"
  "config/.pi/extensions/sce/index.ts"
  "config/schema/sce-config.schema.json"
)

for path in "${required_paths[@]}"; do
  if [[ ! -e "$first_root/$path" ]]; then
    printf 'Generator did not emit required output at %s\n' "$path" >&2
    exit 1
  fi
done

forbidden_outputs=(
  "config/automated/.opencode"
  "config/.claude/agents"
)

for path in "${forbidden_outputs[@]}"; do
  if [[ -e "$first_root/$path" ]]; then
    printf 'Generator emitted removed output at %s\n' "$path" >&2
    exit 1
  fi
done

write_inventory() {
  local generated_root="$1"
  local inventory_path="$2"

  (
    cd "$generated_root"
    find config -type f -print \
      | LC_ALL=C sort \
      | while IFS= read -r path; do
          sha256sum "$path"
        done
  ) > "$inventory_path"
}

first_inventory="$tmp_dir/first.SHA256SUMS"
second_inventory="$tmp_dir/second.SHA256SUMS"
write_inventory "$first_root" "$first_inventory"
write_inventory "$second_root" "$second_inventory"

if ! diff -u "$first_inventory" "$second_inventory"; then
  printf 'Pkl generation is not deterministic.\n' >&2
  exit 1
fi

inventory_digest="$(sha256sum "$first_inventory" | cut -d ' ' -f 1)"
inventory_count="$(wc -l < "$first_inventory" | tr -d ' ')"
printf 'Ephemeral Pkl generation passed: %s files, inventory sha256 %s.\n' \
  "$inventory_count" "$inventory_digest"
