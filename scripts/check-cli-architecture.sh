#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="${CLI_ARCH_CHECK_ROOT:-$(cd "${script_dir}/.." && pwd)}"

domain_root="${repo_root}/cli/src/domain"
application_root="${repo_root}/cli/src/application"

domain_forbidden=(
  clap
  turso
  reqwest
  inquire
  keyring_core
  std::fs
  std::env
  std::process
  crate::adapters
  crate::application
  crate::composition
  crate::services
)

application_forbidden=(
  clap
  turso
  reqwest
  inquire
  keyring_core
  std::fs
  std::process
  crate::adapters
  crate::composition
  crate::services
)

violations_found=0

check_layer() {
  local layer_name="$1"
  local layer_root="$2"
  shift 2
  local forbidden=("$@")

  if [ ! -d "${layer_root}" ]; then
    return 0
  fi

  local file
  while IFS= read -r -d '' file; do
    local line_num=0
    local line
    while IFS= read -r line || [ -n "${line}" ]; do
      line_num=$((line_num + 1))

      # Best-effort: skip lines that are entirely a comment.
      if [[ "${line}" =~ ^[[:space:]]*// ]]; then
        continue
      fi

      local token
      for token in "${forbidden[@]}"; do
        local pattern="(^|[^A-Za-z0-9_:])${token}([^A-Za-z0-9_]|\$)"
        if [[ "${line}" =~ ${pattern} ]]; then
          printf '%s:%d: forbidden dependency in %s layer: %s\n' \
            "${file}" "${line_num}" "${layer_name}" "${token}" >&2
          violations_found=1
        fi
      done
    done < "${file}"
  done < <(find "${layer_root}" -type f -name '*.rs' -print0)
}

check_layer domain "${domain_root}" "${domain_forbidden[@]}"
check_layer application "${application_root}" "${application_forbidden[@]}"

if [ "${violations_found}" -ne 0 ]; then
  exit 1
fi

printf 'cli-architecture check passed: no forbidden dependencies in domain or application layers.\n'
