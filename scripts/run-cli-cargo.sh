#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "${script_dir}/.." && pwd)"

if [ "$#" -eq 0 ]; then
  cat >&2 <<'EOF'
Usage: ./scripts/run-cli-cargo.sh <cargo-arguments...>

Examples:
  ./scripts/run-cli-cargo.sh build --manifest-path cli/Cargo.toml
  ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup
  ./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings
  ./scripts/run-cli-cargo.sh install --path cli --locked
EOF
  exit 2
fi

if ! command -v cargo >/dev/null 2>&1; then
  printf 'cargo is required to run the CLI build.\n' >&2
  exit 1
fi

producer="${script_dir}/produce-cli-generated-input.sh"
if [ ! -x "${producer}" ]; then
  printf 'CLI generated-input producer is missing or not executable: %s\n' \
    "${producer}" >&2
  exit 1
fi

tmp_root="$(mktemp -d "${TMPDIR:-/tmp}/sce-cli-generated-input.XXXXXX")"
cleanup() {
  rm -rf "${tmp_root}"
}
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

generated_input_root="${tmp_root}/generated-input"
"${producer}" "${repo_root}" "${generated_input_root}"

printf 'Prepared temporary CLI generated-input handoff at %s\n' "${generated_input_root}"
(
  cd "${repo_root}"
  SCE_CLI_GENERATED_INPUT_DIR="${generated_input_root}" cargo "$@"
)
