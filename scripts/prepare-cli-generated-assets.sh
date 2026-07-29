#!/usr/bin/env bash
set -euo pipefail

repo_root="${1:-}"
if [ -z "${repo_root}" ]; then
  repo_root="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
fi
repo_root="$(cd "${repo_root}" && pwd)"

target_root="${2:-${repo_root}/cli/package-fallback}"
case "${target_root}" in
  /*) ;;
  *) target_root="$(pwd)/${target_root}" ;;
esac

producer="${repo_root}/scripts/produce-cli-generated-input.sh"
if [ ! -x "${producer}" ]; then
  printf 'CLI generated-input producer is missing or not executable: %s\n' \
    "${producer}" >&2
  exit 1
fi

for path in \
  "${repo_root}/cli/assets/hooks" \
  "${repo_root}/cli/migrations" \
  "${repo_root}/config/schema/agent-trace.schema.json"; do
  if [ ! -e "${path}" ]; then
    printf 'Missing static input required for CLI crate packaging: %s\n' "${path}" >&2
    exit 1
  fi
done

tmp_root="$(mktemp -d)"
cleanup() {
  rm -rf "${tmp_root}"
}
trap cleanup EXIT

generated_input_root="${tmp_root}/generated-input"
staged_root="${tmp_root}/package-fallback"
"${producer}" "${repo_root}" "${generated_input_root}"

mkdir -p "${staged_root}/static/schema"
mv "${generated_input_root}/pkl-generated" "${staged_root}/pkl-generated"
cp -R "${repo_root}/cli/assets/hooks" "${staged_root}/static/hooks"
cp -R "${repo_root}/cli/migrations" "${staged_root}/static/migrations"
cp "${repo_root}/config/schema/agent-trace.schema.json" \
  "${staged_root}/static/schema/agent-trace.schema.json"

cp "${generated_input_root}/SHA256SUMS" "${staged_root}/SHA256SUMS"
(
  cd "${staged_root}"
  find static -type f -print \
    | LC_ALL=C sort \
    | while IFS= read -r path; do
        sha256sum "${path}"
      done >> SHA256SUMS
)

rm -rf "${target_root}"
mv "${staged_root}" "${target_root}"

printf 'Prepared deterministic packaging-only fallback at %s\n' "${target_root}"
