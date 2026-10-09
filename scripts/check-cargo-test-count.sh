#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'USAGE'
Usage: scripts/check-cargo-test-count.sh <minimum-executed-tests> <command> [args...]

Runs <command> (normally a `cargo test` invocation), then fails unless it exits
successfully and the summed `N passed` counts across every `test result:` line
are at least <minimum-executed-tests>. A filtered run that matches zero tests
exits 0 under Cargo, so it is never accepted as a pass here.

Example:
  scripts/check-cargo-test-count.sh 5 nix develop -c ./scripts/run-cli-cargo.sh \
    test --manifest-path cli/Cargo.toml telemetry_hook_export_gated
USAGE
}

if [ "$#" -lt 2 ]; then
  usage
  exit 2
fi

minimum="$1"
shift

if ! [[ "${minimum}" =~ ^[1-9][0-9]*$ ]]; then
  printf 'check-cargo-test-count: minimum must be a positive integer, got %q\n' "${minimum}" >&2
  exit 2
fi

output_file="$(mktemp "${TMPDIR:-/tmp}/check-cargo-test-count.XXXXXX")"
cleanup() {
  rm -f "${output_file}"
}
trap cleanup EXIT

command_status=0
"$@" 2>&1 | tee "${output_file}" || command_status="${PIPESTATUS[0]}"

if [ "${command_status}" -ne 0 ]; then
  printf 'check-cargo-test-count: command failed with exit status %s\n' "${command_status}" >&2
  exit "${command_status}"
fi

executed=0
result_pattern='^test result: [A-Za-z]+\. ([0-9]+) passed;'
while IFS= read -r line; do
  if [[ "${line}" =~ ${result_pattern} ]]; then
    executed=$((executed + BASH_REMATCH[1]))
  fi
done < "${output_file}"

if [ "${executed}" -lt "${minimum}" ]; then
  printf 'check-cargo-test-count: executed %s test(s), expected at least %s; a filter matching zero tests is not a pass.\n' \
    "${executed}" "${minimum}" >&2
  exit 1
fi

printf 'check-cargo-test-count: executed %s test(s) (minimum %s)\n' "${executed}" "${minimum}" >&2
