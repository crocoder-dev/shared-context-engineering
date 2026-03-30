#!/usr/bin/env bash
set -euo pipefail

repo_root="${1:-}"
if [ -z "${repo_root}" ]; then
  repo_root="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
fi

source_opencode="${repo_root}/config/.opencode"
source_claude="${repo_root}/config/.claude"
source_schema="${repo_root}/config/schema/sce-config.schema.json"
target_root="${repo_root}/cli/assets/generated/config"

if [ ! -d "${source_opencode}" ] || [ ! -d "${source_claude}" ] || [ ! -f "${source_schema}" ]; then
  cat >&2 <<EOF
Missing generated config inputs required for CLI crate asset preparation.
Expected:
  ${source_opencode}
  ${source_claude}
  ${source_schema}
EOF
  exit 1
fi

rm -rf "${repo_root}/cli/assets/generated"
mkdir -p "${target_root}"
cp -R "${source_opencode}" "${target_root}/opencode"
cp -R "${source_claude}" "${target_root}/claude"
mkdir -p "${target_root}/schema"
cp "${source_schema}" "${target_root}/schema/sce-config.schema.json"

printf 'Prepared cli/assets/generated from config/ inputs.\n'
