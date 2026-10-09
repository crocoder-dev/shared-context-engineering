#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
checker="${script_dir}/check-cargo-test-count.sh"
work_dir="$(mktemp -d "${TMPDIR:-/tmp}/test-check-cargo-test-count.XXXXXX")"
trap 'rm -rf "${work_dir}"' EXIT

fake_cargo() {
  local name="$1"
  shift
  local path="${work_dir}/${name}"
  {
    printf '#!%s\n' "${BASH}"
    printf 'cat <<"OUT"\n'
    printf '%s\n' "$@"
    printf 'OUT\n'
    printf 'exit "${FAKE_EXIT:-0}"\n'
  } > "${path}"
  chmod +x "${path}"
  printf '%s' "${path}"
}

expect_status() {
  local expected="$1"
  local label="$2"
  shift 2
  local status=0
  bash "${checker}" "$@" > /dev/null 2>&1 || status=$?
  if [ "${status}" -ne "${expected}" ]; then
    printf 'FAIL %s: expected exit %s, got %s\n' "${label}" "${expected}" "${status}" >&2
    exit 1
  fi
  printf 'ok   %s\n' "${label}" >&2
}

zero="$(fake_cargo zero \
  'running 0 tests' \
  'test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 540 filtered out; finished in 0.00s' \
  'running 0 tests' \
  'test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 12 filtered out; finished in 0.00s')"
three="$(fake_cargo three \
  'test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 540 filtered out; finished in 0.10s' \
  'test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 12 filtered out; finished in 0.10s')"

expect_status 1 "zero executed tests is rejected" 1 "${zero}"
expect_status 1 "below the minimum is rejected" 4 "${three}"
expect_status 0 "sum across binaries meets the minimum" 3 "${three}"
expect_status 2 "missing command is a usage error" 3
expect_status 2 "non-numeric minimum is a usage error" many "${three}"
expect_status 2 "zero minimum is a usage error" 0 "${three}"
FAKE_EXIT=101 expect_status 101 "failing command propagates its status" 1 "${three}"
printf 'all check-cargo-test-count checks passed\n' >&2
