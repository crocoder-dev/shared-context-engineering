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

for command in pkl cargo sha256sum; do
  if ! command -v "${command}" >/dev/null 2>&1; then
    printf '%s is required to generate CLI assets and run Cargo.\n' "${command}" >&2
    exit 1
  fi
done

generator="${repo_root}/config/pkl/generate.pkl"
canonical_inputs=(
  "config/pkl"
  "config/lib/agent-trace-plugin/opencode-sce-agent-trace-plugin.ts"
  "config/lib/bash-policy-plugin/opencode-bash-policy-plugin.ts"
  "config/lib/pi-plugin/sce-pi-extension.ts"
)

for relative_path in "${canonical_inputs[@]}"; do
  if [ ! -e "${repo_root}/${relative_path}" ]; then
    printf 'Missing canonical CLI generator input: %s\n' "${repo_root}/${relative_path}" >&2
    exit 1
  fi
done

tmp_root="$(mktemp -d "${TMPDIR:-/tmp}/sce-cli-generated-input.XXXXXX")"
cleanup() {
  rm -rf "${tmp_root}"
}
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

generated_input_root="${tmp_root}/generated-input"
comparison_root="${tmp_root}/comparison"
mkdir -p \
  "${generated_input_root}/pkl-generated" \
  "${comparison_root}/pkl-generated"

generate_payload() {
  local destination="$1"
  (
    cd "${repo_root}"
    pkl eval -m "${destination}" "${generator}" >/dev/null
  )
}

generate_payload "${generated_input_root}/pkl-generated"
generate_payload "${comparison_root}/pkl-generated"

if ! diff -qr \
  "${generated_input_root}/pkl-generated" \
  "${comparison_root}/pkl-generated" >/dev/null; then
  printf 'Canonical Pkl generation is not deterministic.\n' >&2
  diff -r \
    "${generated_input_root}/pkl-generated" \
    "${comparison_root}/pkl-generated" >&2 || true
  exit 1
fi

write_payload_inventory() {
  (
    cd "${generated_input_root}"
    find pkl-generated -type f -print \
      | LC_ALL=C sort \
      | while IFS= read -r path; do
          sha256sum "${path}"
        done
  ) > "${generated_input_root}/SHA256SUMS"
}

write_canonical_input_inventory() {
  (
    cd "${repo_root}"
    {
      find config/pkl -type f -print
      printf '%s\n' \
        config/lib/agent-trace-plugin/opencode-sce-agent-trace-plugin.ts \
        config/lib/bash-policy-plugin/opencode-bash-policy-plugin.ts \
        config/lib/pi-plugin/sce-pi-extension.ts
    } \
      | LC_ALL=C sort \
      | while IFS= read -r path; do
          sha256sum "${path}"
        done
  ) > "${generated_input_root}/INPUTS.SHA256SUMS"
}

write_payload_inventory
write_canonical_input_inventory
rm -rf "${comparison_root}"

printf 'Prepared temporary CLI generated-input handoff at %s\n' "${generated_input_root}"
(
  cd "${repo_root}"
  SCE_CLI_GENERATED_INPUT_DIR="${generated_input_root}" cargo "$@"
)
