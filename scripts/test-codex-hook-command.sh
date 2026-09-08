#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "${script_dir}/.." && pwd)"
tmp_root="$(mktemp -d)"
cleanup() {
  rm -rf "${tmp_root}"
}
trap cleanup EXIT

fail() {
  printf 'Codex hook command test failed: %s\n' "$1" >&2
  exit 1
}

generated_root="${tmp_root}/generated"
pkl eval -m "${generated_root}" "${repo_root}/config/pkl/generate.pkl" >/dev/null
hooks_json="${generated_root}/config/.codex/hooks.json"
helper="${generated_root}/config/.codex/hooks/run-sce-or-show-install-guidance.sh"

[ -f "${hooks_json}" ] || fail "generated hooks.json is missing"
[ -f "${helper}" ] || fail "generated hook helper is missing"

expected_events='["Interrupt","PostToolUse","PreToolUse","SessionEnd","Stop","SubagentStop","UserPromptSubmit"]'
actual_events="$(jq -c '.hooks | keys | sort' "${hooks_json}")"
[ "${actual_events}" = "${expected_events}" ] || fail "unexpected Codex hook event registrations: ${actual_events}"

jq -e '
  ((.hooks.UserPromptSubmit | length == 1) and (.hooks.UserPromptSubmit[0].hooks | length == 1) and (.hooks.UserPromptSubmit[0] | has("matcher") | not))
  and ((.hooks.Stop | length == 2) and (.hooks.Stop[0].hooks | length == 1) and (.hooks.Stop[1].hooks | length == 1) and (.hooks.Stop[1] | has("matcher") | not))
  and ((.hooks.PreToolUse | length == 2) and (.hooks.PreToolUse[0].matcher == "Bash") and (.hooks.PreToolUse[0].hooks | length == 1) and (.hooks.PreToolUse[1] | has("matcher") | not) and (.hooks.PreToolUse[1].hooks | length == 1))
  and ((.hooks.PostToolUse | length == 2) and (.hooks.PostToolUse[0].matcher == "apply_patch") and (.hooks.PostToolUse[0].hooks | length == 1) and (.hooks.PostToolUse[1] | has("matcher") | not) and (.hooks.PostToolUse[1].hooks | length == 1))
  and ((.hooks.Interrupt | length == 1) and (.hooks.Interrupt[0] | has("matcher") | not))
  and ((.hooks.SubagentStop | length == 1) and (.hooks.SubagentStop[0] | has("matcher") | not))
  and ((.hooks.SessionEnd | length == 1) and (.hooks.SessionEnd[0] | has("matcher") | not))
  and (has("$schema") | not)
' "${hooks_json}" >/dev/null || fail "Codex hook registrations do not match the expected four-plus-mutation-scope contract"

hook_command="$(jq -r '.hooks.UserPromptSubmit[0].hooks[0].command' "${hooks_json}")"
for path in '.hooks.UserPromptSubmit[0]' '.hooks.Stop[0]' '.hooks.PreToolUse[0]' '.hooks.PostToolUse[0]'; do
  event_command="$(jq -r "${path}.hooks[0].command" "${hooks_json}")"
  [ "${event_command}" = "${hook_command}" ] || fail "${path} does not use the shared Codex hook command"
done
case "${hook_command}" in
  *'git rev-parse --show-toplevel'*'2>/dev/null'*'|| exit 0; exec bash '*'$root/.codex/hooks/run-sce-or-show-install-guidance.sh'*' sce hooks codex') ;;
  *) fail "Codex hook command is not root-aware and fail-open: ${hook_command}" ;;
esac
case "${hook_command}" in
  *eval*) fail "Codex hook command uses eval" ;;
esac

mutation_scope_command="$(jq -r '.hooks.PreToolUse[1].hooks[0].command' "${hooks_json}")"
for path in '.hooks.PreToolUse[1]' '.hooks.PostToolUse[1]' '.hooks.Stop[1]' '.hooks.Interrupt[0]' '.hooks.SubagentStop[0]' '.hooks.SessionEnd[0]'; do
  group_command="$(jq -r "${path}.hooks[0].command" "${hooks_json}")"
  [ "${group_command}" = "${mutation_scope_command}" ] || fail "${path} does not route to the shared mutation-scope command"
done
case "${mutation_scope_command}" in
  *'git rev-parse --show-toplevel'*'2>/dev/null'*'|| exit 0; exec bash '*'$root/.codex/hooks/run-sce-or-show-install-guidance.sh'*' sce hooks codex-mutation-scope') ;;
  *) fail "Codex mutation-scope hook command is not root-aware and fail-open: ${mutation_scope_command}" ;;
esac

repo="${tmp_root}/repo with spaces"
mkdir -p "${repo}/a/b/c"
git init -q "${repo}"
mkdir -p "${repo}/.codex/hooks"
cp "${helper}" "${repo}/.codex/hooks/run-sce-or-show-install-guidance.sh"

fake_bin="${tmp_root}/bin"
mkdir -p "${fake_bin}"
{
  printf '#!%s\n' "$(command -v bash)"
  cat <<'EOF'
set -euo pipefail
[ "$#" -eq 2 ] && [ "$1" = hooks ] && [ "$2" = codex ] || exit 2
cat
EOF
} > "${fake_bin}/sce"
chmod +x "${fake_bin}/sce"

sentinel='{"hook_event_name":"UserPromptSubmit","session_id":"sentinel"}'
printf '%s' "${sentinel}" > "${tmp_root}/expected"

run_from() {
  local working_directory="$1"
  local output_path="$2"
  printf '%s' "${sentinel}" |
    (
      cd "${working_directory}"
      PATH="${fake_bin}:${PATH}" bash -c "${hook_command}"
    ) > "${output_path}"
}

run_from "${repo}" "${tmp_root}/root-output"
run_from "${repo}/a/b/c" "${tmp_root}/nested-output"
cmp -s "${tmp_root}/expected" "${tmp_root}/root-output" || fail "root invocation did not preserve stdin"
cmp -s "${tmp_root}/expected" "${tmp_root}/nested-output" || fail "nested invocation did not preserve stdin"

outside="${tmp_root}/outside"
mkdir -p "${outside}"
run_without_git() {
  local output_path="$1"
  printf '%s' "${sentinel}" |
    (
      cd "${outside}"
      PATH="${fake_bin}:${PATH}" bash -c "${hook_command}"
    ) > "${output_path}"
}
run_without_git "${tmp_root}/outside-output"
[ ! -s "${tmp_root}/outside-output" ] || fail "Git-root failure was not silent"

git_bin="$(command -v git)"
bash_bin="$(command -v bash)"
minimal_path="$(dirname "${git_bin}"):$(dirname "${bash_bin}")"
printf '%s' "${sentinel}" |
  (
    cd "${repo}"
    PATH="${minimal_path}" bash -c "${hook_command}"
  ) > "${tmp_root}/missing-sce-output" 2> "${tmp_root}/missing-sce-error"
[ ! -s "${tmp_root}/missing-sce-output" ] || fail "missing-sce path emitted stdout"
grep -F 'sce CLI not found.' "${tmp_root}/missing-sce-error" >/dev/null || fail "missing-sce guidance was not emitted on stderr"

printf 'Codex hook command tests passed.\n'
