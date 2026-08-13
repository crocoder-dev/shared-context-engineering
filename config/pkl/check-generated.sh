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
expect_pkl_fixture_failure \
  "config/pkl/renderers/fixtures/unscoped-skill-prohibition-check.pkl" \
  "generated workflow skill contains an unscoped skill prohibition"
expect_pkl_fixture_failure \
  "config/pkl/renderers/fixtures/missing-helper-skill-rule-check.pkl" \
  "generated workflow skill is missing the required helper-skill composition rule"
expect_pkl_fixture_failure \
  "config/pkl/renderers/fixtures/opencode-arbitrary-sce-permission-check.pkl" \
  "OpenCode skill permissions must reject arbitrary SCE permissions"
expect_pkl_fixture_failure \
  "config/pkl/renderers/fixtures/opencode-skill-permission-order-check.pkl" \
  "OpenCode skill permissions must preserve the wildcard, deny, and explicit-allow order"
expect_pkl_fixture_failure \
  "config/pkl/renderers/fixtures/opencode-missing-skill-artifact-check.pkl" \
  "OpenCode skill permission names a missing generated workflow artifact"
expect_pkl_fixture_failure \
  "config/pkl/renderers/fixtures/layout-reference-check.pkl" \
  "generated workflow layout citation does not match a heading in \`config/.opencode/skills/sce-next-task/references/output.md\`"
expect_pkl_fixture_failure \
  "config/pkl/renderers/fixtures/wrong-file-layout-reference-check.pkl" \
  "generated workflow layout citation does not match a heading in \`config/.opencode/skills/sce-next-task/references/wrong-file.md\`"
pkl eval config/pkl/renderers/fixtures/correct-file-layout-reference-check.pkl >/dev/null
expect_pkl_fixture_failure \
  "config/pkl/renderers/fixtures/package-local-reference-check.pkl" \
  "generated package-local reference points to a missing document"
expect_pkl_fixture_failure \
  "config/pkl/renderers/fixtures/validate-forbidden-path-check.pkl" \
  "sce-validate must not generate context-sync.md, sync-report.md, or validation-result.md"
expect_pkl_fixture_failure \
  "config/pkl/renderers/fixtures/commit-forbidden-path-check.pkl" \
  "sce-commit must not generate a commit-contract reference file"
expect_pkl_fixture_failure \
  "config/pkl/renderers/fixtures/atomic-commit-content-check.pkl" \
  "atomic-commit.md must delegate message rules to commit-message-style.md and omit the removed result contract"
expect_pkl_fixture_failure \
  "config/pkl/renderers/fixtures/next-task-report-ownership-check.pkl" \
  "sce-next-task output.md must not duplicate the context-sync report contract"
expect_pkl_fixture_failure \
  "config/pkl/renderers/fixtures/target-neutral-reference-check.pkl" \
  "target-neutral package references differ between Pi, Claude, and OpenCode"
expect_pkl_fixture_failure \
  "config/pkl/renderers/fixtures/stale-sync-debt-check.pkl" \
  "generated file contains the stale synchronization-loss wording"
expect_pkl_fixture_failure \
  "config/pkl/renderers/fixtures/next-task-sync-debt-recovery-check.pkl" \
  "plan-review reference must state sync-debt recovery and legacy-migration-failure behavior"
expect_pkl_fixture_failure \
  "config/pkl/renderers/fixtures/handoff-identity-fields-check.pkl" \
  "persisted handoff must carry Plan path, Task ID, and Task title, and context-sync validation must require them from the handoff itself"
expect_pkl_fixture_failure \
  "config/pkl/renderers/fixtures/sync-debt-recovery-branch-check.pkl" \
  "sce-next-task SKILL.md sync-debt recovery branch must cite references/context-sync.md before invoking the Task context synchronization phase"
expect_pkl_fixture_failure \
  "config/pkl/renderers/fixtures/plan-review-all-tasks-scope-check.pkl" \
  "sce-next-task plan-review reference must state the synchronization-debt scan covers every completed task, with no earlier-completed-task position-relative wording remaining"
expect_pkl_fixture_failure \
  "config/pkl/renderers/fixtures/sync-debt-blocked-routing-check.pkl" \
  "sce-next-task SKILL.md sync-debt recovery blocked outcome must route to the Context synchronization blocked layout, not Review blocked"
expect_pkl_fixture_failure \
  "config/pkl/renderers/fixtures/validation-repair-check.pkl" \
  "final validation must not instruct the agent to repair implementation"
expect_pkl_fixture_failure \
  "config/pkl/renderers/fixtures/validate-decision-sync-boundary-check.pkl" \
  "generated sce-validate document must not contain a sce-decision reference or plan-context-sync wording"

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
