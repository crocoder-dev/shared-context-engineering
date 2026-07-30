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

forbidden_source_artifacts=(
  "config/pkl/rendered"
)

for path in "${forbidden_source_artifacts[@]}"; do
  if [[ -e "$path" ]]; then
    printf 'Accidental repository-local Pkl evaluation artifact exists at %s\n' "$path" >&2
    exit 1
  fi
done

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

producer="$repo_root/scripts/produce-cli-generated-input.sh"
if [[ ! -x "$producer" ]]; then
  printf 'CLI generated-input producer is missing or not executable: %s\n' \
    "$producer" >&2
  exit 1
fi

generated_input_root="$tmp_dir/generated-input"

pkl eval config/pkl/renderers/metadata-coverage-check.pkl >/dev/null
pkl eval config/pkl/renderers/generation-contract-check.pkl >/dev/null

expect_pkl_fixture_failure() {
  local fixture_path="$1"
  local expected_diagnostic="$2"
  local diagnostic

  if diagnostic="$(pkl eval "$fixture_path" 2>&1 >/dev/null)"; then
    printf 'Negative Pkl fixture unexpectedly passed: %s\n' "$fixture_path" >&2
    exit 1
  fi

  if [[ "$diagnostic" != *"$expected_diagnostic"* ]]; then
    printf 'Negative Pkl fixture failed without the expected diagnostic: %s\n%s\n' \
      "$fixture_path" "$diagnostic" >&2
    exit 1
  fi
}

expect_pkl_fixture_failure \
  "config/pkl/renderers/fixtures/extra-artifact-check.pkl" \
  "generated artifact inventory does not match the exact expected path contract"
expect_pkl_fixture_failure \
  "config/pkl/renderers/fixtures/missing-artifact-check.pkl" \
  "generated artifact inventory does not match the exact expected path contract"
expect_pkl_fixture_failure \
  "config/pkl/renderers/fixtures/forbidden-workflow-reference-check.pkl" \
  "generated workflow document contains a forbidden sibling-package reference or unresolved internalization token"
expect_pkl_fixture_failure \
  "config/pkl/renderers/fixtures/blank-line-run-check.pkl" \
  "generated workflow document contains two or more consecutive blank lines"
expect_pkl_fixture_failure \
  "config/pkl/renderers/fixtures/output-dedup-check.pkl" \
  "generated SKILL.md reproduces a references/output.md fenced layout verbatim"

"$producer" "$repo_root" "$generated_input_root"
generated_root="$generated_input_root/pkl-generated"

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
  if [[ ! -e "$generated_root/$path" ]]; then
    printf 'Generator did not emit required output at %s\n' "$path" >&2
    exit 1
  fi
done

forbidden_outputs=(
  "config/automated/.opencode"
  "config/.claude/agents"
)

for path in "${forbidden_outputs[@]}"; do
  if [[ -e "$generated_root/$path" ]]; then
    printf 'Generator emitted removed output at %s\n' "$path" >&2
    exit 1
  fi
done

inventory_path="$generated_input_root/SHA256SUMS"
check_inventory="$tmp_dir/SHA256SUMS"
while read -r checksum path; do
  printf '%s  %s\n' "$checksum" "${path#pkl-generated/}"
done < "$inventory_path" > "$check_inventory"
inventory_digest="$(sha256sum "$check_inventory" | cut -d ' ' -f 1)"
inventory_count="$(wc -l < "$check_inventory" | tr -d ' ')"
printf 'Ephemeral Pkl generation passed: %s files, inventory sha256 %s.\n' \
  "$inventory_count" "$inventory_digest"
