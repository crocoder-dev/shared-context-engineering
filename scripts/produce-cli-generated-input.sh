#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -ne 2 ]; then
  cat >&2 <<'EOF'
Usage: ./scripts/produce-cli-generated-input.sh <repo-root> <output-dir>
EOF
  exit 2
fi

repo_root="$(cd "$1" && pwd)"
output_root="$2"
case "${output_root}" in
  /*) ;;
  *) output_root="$(pwd)/${output_root}" ;;
esac

generator="${repo_root}/config/pkl/generate.pkl"
input_declaration="${repo_root}/config/pkl/generator-inputs.txt"

for command in pkl sha256sum diff find sort mktemp; do
  if ! command -v "${command}" >/dev/null 2>&1; then
    printf '%s is required to produce CLI generated inputs.\n' "${command}" >&2
    exit 1
  fi
done

if [ ! -f "${generator}" ]; then
  printf 'Missing canonical CLI generator: %s\n' "${generator}" >&2
  exit 1
fi
if [ ! -f "${input_declaration}" ]; then
  printf 'Missing canonical CLI generator input declaration: %s\n' "${input_declaration}" >&2
  exit 1
fi
if [ -e "${output_root}" ]; then
  printf 'Generated-input output already exists: %s\n' "${output_root}" >&2
  exit 1
fi

output_parent="$(dirname "${output_root}")"
mkdir -p "${output_parent}"
tmp_root="$(mktemp -d "${output_parent}/.sce-cli-generated-input.XXXXXX")"
cleanup() {
  rm -rf "${tmp_root}"
}
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

first_root="${tmp_root}/first"
second_root="${tmp_root}/second"
mkdir -p "${first_root}/pkl-generated" "${second_root}/pkl-generated"

collect_input_paths() {
  local declaration_entry
  while IFS= read -r declaration_entry || [ -n "${declaration_entry}" ]; do
    case "${declaration_entry}" in
      ''|'#'*) continue ;;
      /*|..|../*|*/../*|*/..)
        printf 'Invalid canonical CLI generator input path: %s\n' \
          "${declaration_entry}" >&2
        return 1
        ;;
    esac

    if [ -d "${repo_root}/${declaration_entry}" ]; then
      (
        cd "${repo_root}"
        find "${declaration_entry}" -type f -print
      )
    elif [ -f "${repo_root}/${declaration_entry}" ]; then
      printf '%s\n' "${declaration_entry}"
    else
      printf 'Missing canonical CLI generator input: %s\n' \
        "${repo_root}/${declaration_entry}" >&2
      return 1
    fi
  done < "${input_declaration}"
}

write_input_inventory() {
  local destination="$1"
  local paths_file="${tmp_root}/input-paths"

  collect_input_paths | LC_ALL=C sort -u > "${paths_file}"
  (
    cd "${repo_root}"
    while IFS= read -r path; do
      sha256sum "${path}"
    done < "${paths_file}"
  ) > "${destination}"
}

generate_payload() {
  local destination="$1"
  (
    cd "${repo_root}"
    pkl eval -m "${destination}" "${generator}" >/dev/null
  )
}

write_payload_inventory() {
  local generated_input_root="$1"
  (
    cd "${generated_input_root}"
    find pkl-generated -type f -print \
      | LC_ALL=C sort \
      | while IFS= read -r path; do
          sha256sum "${path}"
        done
  ) > "${generated_input_root}/SHA256SUMS"
}

initial_input_inventory="${tmp_root}/initial.INPUTS.SHA256SUMS"
final_input_inventory="${first_root}/INPUTS.SHA256SUMS"
write_input_inventory "${initial_input_inventory}"

generate_payload "${first_root}/pkl-generated"
generate_payload "${second_root}/pkl-generated"

if ! diff -qr \
  "${first_root}/pkl-generated" \
  "${second_root}/pkl-generated" >/dev/null; then
  printf 'Canonical Pkl generation is not deterministic.\n' >&2
  diff -r \
    "${first_root}/pkl-generated" \
    "${second_root}/pkl-generated" >&2 || true
  exit 1
fi

write_input_inventory "${final_input_inventory}"
if ! diff -u "${initial_input_inventory}" "${final_input_inventory}" >&2; then
  printf 'Canonical CLI generator inputs changed during generation.\n' >&2
  exit 1
fi

write_payload_inventory "${first_root}"
rm -rf "${second_root}"
mv "${first_root}" "${output_root}"
