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

generator="${repo_root}/config/pkl/generate.pkl"
config_lib="${repo_root}/config/lib"

if [ ! -f "${generator}" ] || [ ! -d "${config_lib}" ]; then
  cat >&2 <<EOF
Missing canonical Pkl inputs required for CLI crate packaging.
Expected:
  ${generator}
  ${config_lib}
EOF
  exit 1
fi

if ! command -v pkl >/dev/null 2>&1; then
  cat >&2 <<'EOF'
pkl is required to prepare the CLI crate fallback payload.
Run this script from the repository Nix dev shell:
  nix develop -c ./scripts/prepare-cli-generated-assets.sh [repo-root] [output-dir]
EOF
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

generate_payload() {
  destination="$1"
  mkdir -p "${destination}/pkl-generated" "${destination}/static/schema"
  (
    cd "${repo_root}"
    pkl eval -m "${destination}/pkl-generated" "${generator}" >/dev/null
  )
  cp -R "${repo_root}/cli/assets/hooks" "${destination}/static/hooks"
  cp -R "${repo_root}/cli/migrations" "${destination}/static/migrations"
  cp "${repo_root}/config/schema/agent-trace.schema.json" \
    "${destination}/static/schema/agent-trace.schema.json"
}

generate_payload "${tmp_root}/first"
generate_payload "${tmp_root}/second"

if ! diff -qr "${tmp_root}/first" "${tmp_root}/second" >/dev/null; then
  printf 'CLI crate fallback generation is not deterministic.\n' >&2
  diff -r "${tmp_root}/first" "${tmp_root}/second" >&2 || true
  exit 1
fi

rm -rf "${target_root}"
mv "${tmp_root}/first" "${target_root}"

(
  cd "${target_root}"
  find pkl-generated static -type f -print \
    | LC_ALL=C sort \
    | while IFS= read -r path; do
        sha256sum "${path}"
      done > SHA256SUMS
)

printf 'Prepared deterministic packaging-only fallback at %s\n' "${target_root}"
