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

tracked_matcher='^(Bash|apply_patch)$'

jq -e --arg m "${tracked_matcher}" '
  ((.hooks.UserPromptSubmit | length == 1) and (.hooks.UserPromptSubmit[0].hooks | length == 1) and (.hooks.UserPromptSubmit[0] | has("matcher") | not))
  and ((.hooks.Stop | length == 2) and (.hooks.Stop[0].hooks | length == 1) and (.hooks.Stop[1].hooks | length == 1) and (.hooks.Stop[1] | has("matcher") | not))
  and ((.hooks.PreToolUse | length == 2) and (.hooks.PreToolUse[0].matcher == "Bash") and (.hooks.PreToolUse[0].hooks | length == 1) and (.hooks.PreToolUse[1].matcher == $m) and (.hooks.PreToolUse[1].hooks | length == 1))
  and ((.hooks.PostToolUse | length == 2) and (.hooks.PostToolUse[0].matcher == "apply_patch") and (.hooks.PostToolUse[0].hooks | length == 1) and (.hooks.PostToolUse[1].matcher == $m) and (.hooks.PostToolUse[1].hooks | length == 1))
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

mutation_scope_command="$(jq -r '.hooks.PostToolUse[1].hooks[0].command' "${hooks_json}")"
for path in '.hooks.PostToolUse[1]' '.hooks.Stop[1]' '.hooks.Interrupt[0]' '.hooks.SubagentStop[0]' '.hooks.SessionEnd[0]'; do
  group_command="$(jq -r "${path}.hooks[0].command" "${hooks_json}")"
  [ "${group_command}" = "${mutation_scope_command}" ] || fail "${path} does not route to the shared mutation-scope command"
done
case "${mutation_scope_command}" in
  *'git rev-parse --show-toplevel'*'2>/dev/null'*'|| exit 0; exec bash '*'$root/.codex/hooks/run-sce-or-show-install-guidance.sh'*' sce hooks codex-mutation-scope') ;;
  *) fail "Codex mutation-scope hook command is not root-aware and fail-open: ${mutation_scope_command}" ;;
esac

pre_tool_use_command="$(jq -r '.hooks.PreToolUse[1].hooks[0].command' "${hooks_json}")"
[ "${pre_tool_use_command}" != "${mutation_scope_command}" ] || fail "mutation-scope PreToolUse must use the dedicated fail-closed bootstrap"
case "${pre_tool_use_command}" in
  *'SCE_CODEX_PRE_TOOL_USE_FAIL_CLOSED=1 exec bash '*'$root/.codex/hooks/run-sce-or-show-install-guidance.sh'*' sce hooks codex-mutation-scope') ;;
  *) fail "mutation-scope PreToolUse bootstrap is not the fail-closed helper form: ${pre_tool_use_command}" ;;
esac
case "${pre_tool_use_command}" in
  *'"permissionDecision":"deny"'*) ;;
  *) fail "mutation-scope PreToolUse bootstrap does not carry the D8 deny contract" ;;
esac
case "${pre_tool_use_command}" in
  *eval*) fail "mutation-scope PreToolUse bootstrap uses eval" ;;
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

deny_json='{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"SCE could not establish mutation attribution for this tool execution."}}'
tracked_stdin='{"hook_event_name":"PreToolUse","tool_name":"Bash","session_id":"s","tool_use_id":"exec-1"}'

assert_deny() {
  local label="$1"
  local out_path="$2"
  local exit_code="$3"
  [ "${exit_code}" = "0" ] || fail "${label}: expected exit 0, got ${exit_code}"
  [ "$(cat "${out_path}")" = "${deny_json}" ] || fail "${label}: stdout is not the stable D8 deny JSON: $(cat "${out_path}")"
  jq -e '.hookSpecificOutput.hookEventName == "PreToolUse" and .hookSpecificOutput.permissionDecision == "deny" and (.hookSpecificOutput.permissionDecisionReason | length > 0)' \
    "${out_path}" >/dev/null || fail "${label}: deny JSON does not parse as the PreToolUse deny contract"
}

set +e
printf '%s' "${tracked_stdin}" |
  (
    cd "${repo}"
    PATH="${minimal_path}" bash -c "${pre_tool_use_command}"
  ) > "${tmp_root}/pre-missing-sce-out" 2> "${tmp_root}/pre-missing-sce-err"
pre_missing_sce_exit=$?
set -e
assert_deny "sce missing" "${tmp_root}/pre-missing-sce-out" "${pre_missing_sce_exit}"
grep -F 'sce CLI not found.' "${tmp_root}/pre-missing-sce-err" >/dev/null || fail "sce missing: install guidance was not emitted on stderr"

set +e
printf '%s' "${tracked_stdin}" |
  (
    cd "${outside}"
    PATH="${fake_bin}:${PATH}" bash -c "${pre_tool_use_command}"
  ) > "${tmp_root}/pre-no-git-out" 2>/dev/null
pre_no_git_exit=$?
set -e
assert_deny "git root failure" "${tmp_root}/pre-no-git-out" "${pre_no_git_exit}"

helperless_repo="${tmp_root}/helperless"
mkdir -p "${helperless_repo}"
git init -q "${helperless_repo}"
set +e
printf '%s' "${tracked_stdin}" |
  (
    cd "${helperless_repo}"
    PATH="${fake_bin}:${PATH}" bash -c "${pre_tool_use_command}"
  ) > "${tmp_root}/pre-no-helper-out" 2>/dev/null
pre_no_helper_exit=$?
set -e
assert_deny "helper missing" "${tmp_root}/pre-no-helper-out" "${pre_no_helper_exit}"

mutation_bin="${tmp_root}/mutation-bin"
mkdir -p "${mutation_bin}"
{
  printf '#!%s\n' "$(command -v bash)"
  cat <<'EOF'
set -euo pipefail
[ "$1" = hooks ] && [ "$2" = codex-mutation-scope ] || exit 2
cat >/dev/null
exit 9
EOF
} > "${mutation_bin}/sce"
chmod +x "${mutation_bin}/sce"
set +e
printf '%s' "${tracked_stdin}" |
  (
    cd "${repo}"
    PATH="${mutation_bin}:${minimal_path}" bash -c "${pre_tool_use_command}"
  ) > "${tmp_root}/pre-adapter-fail-out" 2>/dev/null
pre_adapter_fail_exit=$?
set -e
assert_deny "adapter non-zero exit" "${tmp_root}/pre-adapter-fail-out" "${pre_adapter_fail_exit}"

neutral_bin="${tmp_root}/neutral-bin"
mkdir -p "${neutral_bin}"
seen_stdin="${tmp_root}/adapter-stdin-seen"
{
  printf '#!%s\n' "$(command -v bash)"
  printf 'set -euo pipefail\n'
  printf '[ "$1" = hooks ] && [ "$2" = codex-mutation-scope ] || exit 2\n'
  printf 'cat > %q\n' "${seen_stdin}"
  printf 'exit 0\n'
} > "${neutral_bin}/sce"
chmod +x "${neutral_bin}/sce"
set +e
printf '%s' "${tracked_stdin}" |
  (
    cd "${repo}"
    PATH="${neutral_bin}:${minimal_path}" bash -c "${pre_tool_use_command}"
  ) > "${tmp_root}/pre-neutral-out" 2>/dev/null
pre_neutral_exit=$?
set -e
[ "${pre_neutral_exit}" = "0" ] || fail "neutral adapter: expected exit 0, got ${pre_neutral_exit}"
[ ! -s "${tmp_root}/pre-neutral-out" ] || fail "neutral adapter: a neutral response must be forwarded as empty stdout"
[ "$(cat "${seen_stdin}")" = "${tracked_stdin}" ] || fail "neutral adapter: stdin was not forwarded byte-for-byte"

forward_bin="${tmp_root}/forward-bin"
mkdir -p "${forward_bin}"
recovery_json='{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"recovery barrier"}}'
{
  printf '#!%s\n' "$(command -v bash)"
  printf 'set -euo pipefail\n'
  printf '[ "$1" = hooks ] && [ "$2" = codex-mutation-scope ] || exit 2\n'
  printf 'cat >/dev/null\n'
  printf 'printf %%s %q\n' "${recovery_json}"
  printf 'exit 0\n'
} > "${forward_bin}/sce"
chmod +x "${forward_bin}/sce"
set +e
printf '%s' "${tracked_stdin}" |
  (
    cd "${repo}"
    PATH="${forward_bin}:${minimal_path}" bash -c "${pre_tool_use_command}"
  ) > "${tmp_root}/pre-forward-out" 2>/dev/null
pre_forward_exit=$?
set -e
[ "${pre_forward_exit}" = "0" ] || fail "adapter forward: expected exit 0, got ${pre_forward_exit}"
[ "$(cat "${tmp_root}/pre-forward-out")" = "${recovery_json}" ] || fail "adapter forward: a successful adapter response was not forwarded unchanged"

printf 'Codex hook command tests passed.\n'
