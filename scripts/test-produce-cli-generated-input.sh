#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
producer="${script_dir}/produce-cli-generated-input.sh"
tmp_root="$(mktemp -d)"
cleanup() {
  rm -rf "${tmp_root}"
}
trap cleanup EXIT

fake_bin="${tmp_root}/bin"
mkdir -p "${fake_bin}"
cat > "${fake_bin}/pkl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
: "${FAKE_PKL_STATE:?missing fake Pkl state directory}"
destination=""
generator=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    eval) shift ;;
    -m)
      destination="$2"
      shift 2
      ;;
    *)
      generator="$1"
      shift
      ;;
  esac
done

mkdir -p "${FAKE_PKL_STATE}"
count_file="${FAKE_PKL_STATE}/count"
count=0
if [ -f "${count_file}" ]; then
  count="$(<"${count_file}")"
fi
count=$((count + 1))
printf '%s\n' "${count}" > "${count_file}"

case "${FAKE_PKL_MODE:-success}" in
  failure)
    exit 37
    ;;
  mutation)
    if [ "${count}" -eq 1 ]; then
      printf 'mutated\n' >> "${FAKE_PKL_MUTATION_TARGET:?missing mutation target}"
    fi
    ;;
esac

mkdir -p "${destination}/config/.opencode"
case "${FAKE_PKL_MODE:-success}" in
  drift)
    printf 'generation-%s\n' "${count}" \
      > "${destination}/config/.opencode/generated.txt"
    ;;
  *)
    sha256sum "${generator}" \
      > "${destination}/config/.opencode/generated.txt"
    ;;
esac
EOF
chmod +x "${fake_bin}/pkl"

create_repo() {
  local repo="$1"
  mkdir -p \
    "${repo}/config/pkl" \
    "${repo}/config/lib/agent-trace-plugin" \
    "${repo}/config/lib/bash-policy-plugin" \
    "${repo}/config/lib/pi-plugin"
  printf 'generator\n' > "${repo}/config/pkl/generate.pkl"
  cat > "${repo}/config/pkl/generator-inputs.txt" <<'EOF'
# Test canonical inputs.
config/pkl
config/lib/agent-trace-plugin/opencode-sce-agent-trace-plugin.ts
config/lib/bash-policy-plugin/opencode-bash-policy-plugin.ts
config/lib/pi-plugin/sce-pi-extension.ts
EOF
  printf 'agent-trace\n' \
    > "${repo}/config/lib/agent-trace-plugin/opencode-sce-agent-trace-plugin.ts"
  printf 'bash-policy\n' \
    > "${repo}/config/lib/bash-policy-plugin/opencode-bash-policy-plugin.ts"
  printf 'pi-extension\n' \
    > "${repo}/config/lib/pi-plugin/sce-pi-extension.ts"
}

assert_no_staging_dirs() {
  local parent="$1"
  local staging_dirs=("${parent}"/.sce-cli-generated-input.*)
  if [ -e "${staging_dirs[0]}" ]; then
    printf 'Producer temporary state was not cleaned up under %s.\n' "${parent}" >&2
    exit 1
  fi
}

run_producer() {
  local repo="$1"
  local output="$2"
  local state="$3"
  local mode="$4"
  env \
    PATH="${fake_bin}:${PATH}" \
    FAKE_PKL_STATE="${state}" \
    FAKE_PKL_MODE="${mode}" \
    FAKE_PKL_MUTATION_TARGET="${repo}/config/lib/pi-plugin/sce-pi-extension.ts" \
    "${producer}" "${repo}" "${output}"
}

success_root="${tmp_root}/success"
success_repo="${success_root}/repo"
success_output="${success_root}/generated-input"
create_repo "${success_repo}"
run_producer "${success_repo}" "${success_output}" \
  "${success_root}/state" success
test "$(<"${success_root}/state/count")" -eq 2
test -f "${success_output}/pkl-generated/config/.opencode/generated.txt"
(
  cd "${success_output}"
  sha256sum -c SHA256SUMS >/dev/null
)
(
  cd "${success_repo}"
  sha256sum -c "${success_output}/INPUTS.SHA256SUMS" >/dev/null
)
assert_no_staging_dirs "${success_root}"

missing_root="${tmp_root}/missing"
missing_repo="${missing_root}/repo"
create_repo "${missing_repo}"
rm "${missing_repo}/config/lib/pi-plugin/sce-pi-extension.ts"
if run_producer "${missing_repo}" "${missing_root}/generated-input" \
  "${missing_root}/state" success >"${missing_root}/log" 2>&1; then
  printf 'Producer accepted a missing declared input.\n' >&2
  exit 1
fi
if ! grep -Fq 'Missing canonical CLI generator input' "${missing_root}/log"; then
  printf 'Missing-input failure did not report the expected diagnostic.\n' >&2
  exit 1
fi
test ! -e "${missing_root}/generated-input"
assert_no_staging_dirs "${missing_root}"

mutation_root="${tmp_root}/mutation"
mutation_repo="${mutation_root}/repo"
create_repo "${mutation_repo}"
if run_producer "${mutation_repo}" "${mutation_root}/generated-input" \
  "${mutation_root}/state" mutation >"${mutation_root}/log" 2>&1; then
  printf 'Producer accepted canonical input mutation during generation.\n' >&2
  exit 1
fi
if ! grep -Fq 'Canonical CLI generator inputs changed during generation' \
  "${mutation_root}/log"; then
  printf 'Input-mutation failure did not report the expected diagnostic.\n' >&2
  exit 1
fi
test ! -e "${mutation_root}/generated-input"
assert_no_staging_dirs "${mutation_root}"

drift_root="${tmp_root}/drift"
drift_repo="${drift_root}/repo"
create_repo "${drift_repo}"
if run_producer "${drift_repo}" "${drift_root}/generated-input" \
  "${drift_root}/state" drift >"${drift_root}/log" 2>&1; then
  printf 'Producer accepted nondeterministic generated output.\n' >&2
  exit 1
fi
if ! grep -Fq 'Canonical Pkl generation is not deterministic' \
  "${drift_root}/log"; then
  printf 'Generation-drift failure did not report the expected diagnostic.\n' >&2
  exit 1
fi
test ! -e "${drift_root}/generated-input"
assert_no_staging_dirs "${drift_root}"

failure_root="${tmp_root}/failure"
failure_repo="${failure_root}/repo"
create_repo "${failure_repo}"
set +e
run_producer "${failure_repo}" "${failure_root}/generated-input" \
  "${failure_root}/state" failure >"${failure_root}/log" 2>&1
failure_status=$?
set -e
if [ "${failure_status}" -ne 37 ]; then
  printf 'Producer did not preserve the Pkl failure status: %s.\n' \
    "${failure_status}" >&2
  exit 1
fi
test ! -e "${failure_root}/generated-input"
assert_no_staging_dirs "${failure_root}"

printf 'CLI generated-input producer tests passed.\n'
