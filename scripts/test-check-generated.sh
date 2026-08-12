#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
check_script="${script_dir}/../config/pkl/check-generated.sh"
producer="${script_dir}/produce-cli-generated-input.sh"
tmp_root="$(mktemp -d)"
cleanup() {
  rm -rf "${tmp_root}"
}
trap cleanup EXIT

test_repo="${tmp_root}/repo"
fake_bin="${tmp_root}/bin"
state_root="${tmp_root}/state"
mkdir -p \
  "${test_repo}/config/pkl/renderers/fixtures" \
  "${test_repo}/scripts" \
  "${fake_bin}" \
  "${state_root}"
cp "${check_script}" "${test_repo}/config/pkl/check-generated.sh"
cp "${producer}" "${test_repo}/scripts/produce-cli-generated-input.sh"
printf 'generator\n' > "${test_repo}/config/pkl/generate.pkl"
printf 'config/pkl\n' > "${test_repo}/config/pkl/generator-inputs.txt"
touch \
  "${test_repo}/config/pkl/renderers/metadata-coverage-check.pkl" \
  "${test_repo}/config/pkl/renderers/generation-contract-check.pkl" \
  "${test_repo}/config/pkl/renderers/fixtures/extra-artifact-check.pkl" \
  "${test_repo}/config/pkl/renderers/fixtures/missing-artifact-check.pkl" \
  "${test_repo}/config/pkl/renderers/fixtures/forbidden-workflow-reference-check.pkl" \
  "${test_repo}/config/pkl/renderers/fixtures/layout-reference-check.pkl" \
  "${test_repo}/config/pkl/renderers/fixtures/package-local-reference-check.pkl" \
  "${test_repo}/config/pkl/renderers/fixtures/validate-forbidden-path-check.pkl" \
  "${test_repo}/config/pkl/renderers/fixtures/commit-forbidden-path-check.pkl" \
  "${test_repo}/config/pkl/renderers/fixtures/atomic-commit-content-check.pkl" \
  "${test_repo}/config/pkl/renderers/fixtures/next-task-report-ownership-check.pkl" \
  "${test_repo}/config/pkl/renderers/fixtures/target-neutral-reference-check.pkl" \
  "${test_repo}/config/pkl/renderers/fixtures/stale-sync-debt-check.pkl" \
  "${test_repo}/config/pkl/renderers/fixtures/validation-repair-check.pkl"

cat > "${fake_bin}/pkl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
: "${FAKE_PKL_STATE:?missing fake Pkl state directory}"
destination=""
module=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    eval) shift ;;
    -m)
      destination="$2"
      shift 2
      ;;
    *)
      module="$1"
      shift
      ;;
  esac
done

case "${module}" in
  *extra-artifact-check.pkl|*missing-artifact-check.pkl)
    printf '%s\n' \
      'generated artifact inventory does not match the exact expected path contract' >&2
    exit 1
    ;;
  *forbidden-workflow-reference-check.pkl)
    printf '%s\n' \
      'generated workflow document contains a forbidden sibling-package reference or unresolved internalization token' >&2
    exit 1
    ;;
  *layout-reference-check.pkl)
    printf '%s\n' 'generated workflow layout citation does not match a heading in its references/output.md' >&2
    exit 1
    ;;
  *package-local-reference-check.pkl)
    printf '%s\n' 'generated package-local reference points to a missing document' >&2
    exit 1
    ;;
  *validate-forbidden-path-check.pkl)
    printf '%s\n' 'sce-validate must not generate sync-report.md or validation-result.md' >&2
    exit 1
    ;;
  *commit-forbidden-path-check.pkl)
    printf '%s\n' 'sce-commit must not generate a legacy commit reference file' >&2
    exit 1
    ;;
  *atomic-commit-content-check.pkl)
    printf '%s\n' 'atomic-commit.md must contain both commit-message rules and the atomic-commit result contract' >&2
    exit 1
    ;;
  *next-task-report-ownership-check.pkl)
    printf '%s\n' 'sce-next-task output.md must not duplicate the context-sync report contract' >&2
    exit 1
    ;;
  *target-neutral-reference-check.pkl)
    printf '%s\n' 'target-neutral package references differ between Pi, Claude, and OpenCode' >&2
    exit 1
    ;;
  *stale-sync-debt-check.pkl)
    printf '%s\n' 'generated file contains the stale synchronization-loss wording' >&2
    exit 1
    ;;
  *validation-repair-check.pkl)
    printf '%s\n' 'final validation must not instruct the agent to repair implementation' >&2
    exit 1
    ;;
esac

if [ -z "${destination}" ]; then
  exit 0
fi

count_file="${FAKE_PKL_STATE}/generation-count"
count=0
if [ -f "${count_file}" ]; then
  count="$(<"${count_file}")"
fi
printf '%s\n' "$((count + 1))" > "${count_file}"

mkdir -p \
  "${destination}/config/.opencode/agent" \
  "${destination}/config/.opencode/command" \
  "${destination}/config/.opencode/skills" \
  "${destination}/config/.opencode/lib" \
  "${destination}/config/.opencode/plugins" \
  "${destination}/config/.claude/commands" \
  "${destination}/config/.claude/skills" \
  "${destination}/config/.claude/hooks" \
  "${destination}/config/.pi/prompts" \
  "${destination}/config/.pi/skills" \
  "${destination}/config/.pi/extensions/sce" \
  "${destination}/config/schema"
printf 'generated\n' > "${destination}/config/.opencode/agent/agent.md"
printf 'generated\n' > "${destination}/config/.opencode/command/command.md"
printf 'generated\n' > "${destination}/config/.opencode/skills/skill.md"
printf 'generated\n' > "${destination}/config/.opencode/lib/bash-policy-presets.json"
printf 'generated\n' > "${destination}/config/.opencode/plugins/sce-bash-policy.ts"
printf 'generated\n' > "${destination}/config/.opencode/plugins/sce-agent-trace.ts"
printf 'generated\n' > "${destination}/config/.opencode/opencode.json"
printf 'generated\n' > "${destination}/config/.claude/commands/command.md"
printf 'generated\n' > "${destination}/config/.claude/skills/skill.md"
printf 'generated\n' > "${destination}/config/.claude/hooks/run-sce-or-show-install-guidance.sh"
printf 'generated\n' > "${destination}/config/.claude/settings.json"
printf 'generated\n' > "${destination}/config/.pi/prompts/prompt.md"
printf 'generated\n' > "${destination}/config/.pi/skills/skill.md"
printf 'generated\n' > "${destination}/config/.pi/extensions/sce/index.ts"
printf 'generated\n' > "${destination}/config/schema/sce-config.schema.json"
EOF
chmod +x \
  "${fake_bin}/pkl" \
  "${test_repo}/config/pkl/check-generated.sh" \
  "${test_repo}/scripts/produce-cli-generated-input.sh"

output="$({
  env \
    PATH="${fake_bin}:${PATH}" \
    IN_NIX_SHELL=1 \
    SCE_REPO_ROOT="${test_repo}" \
    FAKE_PKL_STATE="${state_root}" \
    "${test_repo}/config/pkl/check-generated.sh"
} 2>&1)"

if [ "$(<"${state_root}/generation-count")" -ne 2 ]; then
  printf 'Generated-output check did not delegate exactly one producer run.\n' >&2
  exit 1
fi
if [[ "${output}" != *'Ephemeral Pkl generation passed:'* ]]; then
  printf 'Generated-output check did not report producer inventory success.\n%s\n' \
    "${output}" >&2
  exit 1
fi

printf 'check-generated delegation tests passed.\n'
