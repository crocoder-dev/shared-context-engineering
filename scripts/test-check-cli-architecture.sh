#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
check_script="${script_dir}/check-cli-architecture.sh"

tmp_root="$(mktemp -d)"
cleanup() {
  rm -rf "${tmp_root}"
}
trap cleanup EXIT

assertions=0
failures=0

run_check() {
  local root="$1"
  CLI_ARCH_CHECK_ROOT="${root}" "${check_script}"
}

assert_reject() {
  local name="$1" root="$2" expected_substring="$3"
  local output=""
  local status=0
  output="$(run_check "${root}" 2>&1)" || status=$?
  assertions=$((assertions + 1))

  if [ "${status}" -eq 0 ]; then
    printf 'FAIL (%s): expected non-zero exit, got 0. Output:\n%s\n' \
      "${name}" "${output}" >&2
    failures=$((failures + 1))
    return
  fi
  if [[ "${output}" != *"${expected_substring}"* ]]; then
    printf 'FAIL (%s): expected output to contain %q. Output:\n%s\n' \
      "${name}" "${expected_substring}" "${output}" >&2
    failures=$((failures + 1))
    return
  fi
  printf 'PASS (%s)\n' "${name}"
}

assert_accept() {
  local name="$1" root="$2"
  local output=""
  local status=0
  output="$(run_check "${root}" 2>&1)" || status=$?
  assertions=$((assertions + 1))

  if [ "${status}" -ne 0 ]; then
    printf 'FAIL (%s): expected zero exit, got %d. Output:\n%s\n' \
      "${name}" "${status}" "${output}" >&2
    failures=$((failures + 1))
    return
  fi
  printf 'PASS (%s)\n' "${name}"
}

# --- Reject fixtures ---

reject_domain_adapters="${tmp_root}/reject-domain-adapters"
mkdir -p "${reject_domain_adapters}/cli/src/domain"
printf '//! domain layer\nuse crate::adapters;\n' \
  > "${reject_domain_adapters}/cli/src/domain/mod.rs"
assert_reject 'domain importing crate::adapters' "${reject_domain_adapters}" \
  'forbidden dependency in domain layer: crate::adapters'

reject_domain_stdfs="${tmp_root}/reject-domain-stdfs"
mkdir -p "${reject_domain_stdfs}/cli/src/domain"
printf '//! domain layer\nuse std::fs;\n' \
  > "${reject_domain_stdfs}/cli/src/domain/mod.rs"
assert_reject 'domain using std::fs' "${reject_domain_stdfs}" \
  'forbidden dependency in domain layer: std::fs'

reject_application_services="${tmp_root}/reject-application-services"
mkdir -p "${reject_application_services}/cli/src/application"
printf '//! application layer\nuse crate::services;\n' \
  > "${reject_application_services}/cli/src/application/mod.rs"
assert_reject 'application importing crate::services' "${reject_application_services}" \
  'forbidden dependency in application layer: crate::services'

reject_application_turso="${tmp_root}/reject-application-turso"
mkdir -p "${reject_application_turso}/cli/src/application"
printf '//! application layer\nuse turso::Connection;\n' \
  > "${reject_application_turso}/cli/src/application/mod.rs"
assert_reject 'application importing turso' "${reject_application_turso}" \
  'forbidden dependency in application layer: turso'

# --- Accept fixtures ---

accept_domain_pathbuf="${tmp_root}/accept-domain-pathbuf"
mkdir -p "${accept_domain_pathbuf}/cli/src/domain"
printf '//! domain layer\nuse std::path::PathBuf;\n\npub struct Example(PathBuf);\n' \
  > "${accept_domain_pathbuf}/cli/src/domain/mod.rs"
assert_accept 'domain using std::path::PathBuf' "${accept_domain_pathbuf}"

accept_application_domain="${tmp_root}/accept-application-domain"
mkdir -p "${accept_application_domain}/cli/src/application"
printf '//! application layer\nuse crate::domain;\n' \
  > "${accept_application_domain}/cli/src/application/mod.rs"
assert_accept 'application importing crate::domain' "${accept_application_domain}"

accept_adapter_application="${tmp_root}/accept-adapter-application"
mkdir -p "${accept_adapter_application}/cli/src/adapters"
printf '//! adapters layer\nuse crate::application;\n' \
  > "${accept_adapter_application}/cli/src/adapters/mod.rs"
assert_accept 'adapter importing crate::application' "${accept_adapter_application}"

accept_composition_app="${tmp_root}/accept-composition-app"
mkdir -p "${accept_composition_app}/cli/src"
printf '//! composition root\nuse crate::app;\n\npub(crate) fn run() {\n    crate::app::run(std::env::args());\n}\n' \
  > "${accept_composition_app}/cli/src/composition.rs"
assert_accept 'composition delegating to crate::app' "${accept_composition_app}"

if [ "${failures}" -ne 0 ]; then
  printf '%d of %d assertions failed.\n' "${failures}" "${assertions}" >&2
  exit 1
fi

printf 'test-check-cli-architecture: all %d assertions passed.\n' "${assertions}"
