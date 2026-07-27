#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
helper="${script_dir}/run-cli-cargo.sh"
tmp_root="$(mktemp -d)"
cleanup() {
  rm -rf "${tmp_root}"
}
trap cleanup EXIT

test_repo="${tmp_root}/repo"
fake_bin="${tmp_root}/bin"
state_root="${tmp_root}/state"
tmp_dir="${tmp_root}/tmp"
mkdir -p \
  "${test_repo}/scripts" \
  "${test_repo}/config/pkl" \
  "${test_repo}/config/lib/agent-trace-plugin" \
  "${test_repo}/config/lib/bash-policy-plugin" \
  "${test_repo}/config/lib/pi-plugin" \
  "${fake_bin}" \
  "${state_root}" \
  "${tmp_dir}"
cp "${helper}" "${test_repo}/scripts/run-cli-cargo.sh"

printf 'generator-v1\n' > "${test_repo}/config/pkl/generate.pkl"
printf 'agent-trace\n' > "${test_repo}/config/lib/agent-trace-plugin/opencode-sce-agent-trace-plugin.ts"
printf 'bash-policy\n' > "${test_repo}/config/lib/bash-policy-plugin/opencode-bash-policy-plugin.ts"
printf 'pi-extension\n' > "${test_repo}/config/lib/pi-plugin/sce-pi-extension.ts"

cat > "${fake_bin}/pkl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
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
mkdir -p "${destination}/config/.opencode"
sha256sum "${generator}" > "${destination}/config/.opencode/generator.sha256"
EOF

cat > "${fake_bin}/cargo" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
: "${SCE_CLI_GENERATED_INPUT_DIR:?missing generated-input handoff}"
: "${FAKE_CARGO_STATE:?missing test state directory}"
mkdir -p "${FAKE_CARGO_STATE}"
printf '%s\n' "$@" > "${FAKE_CARGO_STATE}/arguments"
(
  cd "${SCE_CLI_GENERATED_INPUT_DIR}"
  sha256sum -c SHA256SUMS >/dev/null
)
(
  cd "${PWD}"
  sha256sum -c "${SCE_CLI_GENERATED_INPUT_DIR}/INPUTS.SHA256SUMS" >/dev/null
)
cp "${SCE_CLI_GENERATED_INPUT_DIR}/SHA256SUMS" "${FAKE_CARGO_STATE}/SHA256SUMS"
cp "${SCE_CLI_GENERATED_INPUT_DIR}/INPUTS.SHA256SUMS" "${FAKE_CARGO_STATE}/INPUTS.SHA256SUMS"
exit "${FAKE_CARGO_EXIT_CODE:-0}"
EOF
chmod +x "${fake_bin}/pkl" "${fake_bin}/cargo" "${test_repo}/scripts/run-cli-cargo.sh"

run_helper() {
  env \
    PATH="${fake_bin}:${PATH}" \
    TMPDIR="${tmp_dir}" \
    FAKE_CARGO_STATE="$1" \
    FAKE_CARGO_EXIT_CODE="${2:-0}" \
    "${test_repo}/scripts/run-cli-cargo.sh" "${@:3}"
}

assert_tmp_dir_empty() {
  local entries=("${tmp_dir}"/*)
  if [ -e "${entries[0]}" ]; then
    printf 'Temporary generated-input directories were not cleaned up.\n' >&2
    exit 1
  fi
}

first_state="${state_root}/first"
run_helper "${first_state}" 0 test --manifest-path cli/Cargo.toml setup
expected_arguments=$'test\n--manifest-path\ncli/Cargo.toml\nsetup'
if [ "$(cat "${first_state}/arguments")" != "${expected_arguments}" ]; then
  printf 'Cargo arguments were not forwarded unchanged.\n' >&2
  exit 1
fi
assert_tmp_dir_empty

printf 'generator-v2\n' > "${test_repo}/config/pkl/generate.pkl"
second_state="${state_root}/second"
run_helper "${second_state}" 0 build --manifest-path cli/Cargo.toml
if cmp -s "${first_state}/SHA256SUMS" "${second_state}/SHA256SUMS"; then
  printf 'Canonical input changes did not regenerate the payload.\n' >&2
  exit 1
fi
if cmp -s "${first_state}/INPUTS.SHA256SUMS" "${second_state}/INPUTS.SHA256SUMS"; then
  printf 'Canonical input changes did not refresh the input inventory.\n' >&2
  exit 1
fi
assert_tmp_dir_empty

set +e
run_helper "${state_root}/failure" 42 clippy --manifest-path cli/Cargo.toml --all-targets
failure_status=$?
set -e
if [ "${failure_status}" -ne 42 ]; then
  printf 'Cargo failure status was not preserved: expected 42, got %s.\n' "${failure_status}" >&2
  exit 1
fi
assert_tmp_dir_empty

printf 'run-cli-cargo helper tests passed.\n'
