#!/usr/bin/env bash
set -euo pipefail

FORBIDDEN_REFERENCE="/nix/store/"

usage() {
  cat >&2 <<'EOF'
usage: native-portability-audit.sh --binary <path> [--platform auto|linux|macos]

Fails when a native release binary contains forbidden /nix/store runtime
references. macOS inspection reads dynamic library install names with otool -L;
Linux inspection reads ELF dynamic metadata with readelf and scans binary strings.
EOF
}

fail_usage() {
  usage
  exit 2
}

binary_path=""
platform="auto"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --binary)
      [[ $# -ge 2 ]] || fail_usage
      binary_path="$2"
      shift 2
      ;;
    --platform)
      [[ $# -ge 2 ]] || fail_usage
      platform="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      fail_usage
      ;;
  esac
done

[[ -n "$binary_path" ]] || fail_usage

if [[ ! -r "$binary_path" ]]; then
  printf 'Native binary portability audit failed: cannot read binary at %s\n' "$binary_path" >&2
  exit 1
fi

if [[ "$platform" == "auto" ]]; then
  case "$(uname -s)" in
    Linux)
      platform="linux"
      ;;
    Darwin)
      platform="macos"
      ;;
    *)
      printf 'Native binary portability audit failed: unsupported host platform %s; pass --platform linux or --platform macos\n' "$(uname -s)" >&2
      exit 1
      ;;
  esac
fi

tmp_dir="$(mktemp -d)"
cleanup() {
  rm -rf "$tmp_dir"
}
trap cleanup EXIT

references_file="$tmp_dir/references.txt"
: > "$references_file"

append_forbidden_lines() {
  local source_file="$1"

  if [[ -s "$source_file" ]]; then
    grep -F "$FORBIDDEN_REFERENCE" "$source_file" >> "$references_file" || true
  fi
}

append_linux_forbidden_string_lines() {
  local source_file="$1"

  if [[ -s "$source_file" ]]; then
    grep -Eo '/nix/store/[^[:space:]]+\.so([^[:space:]]*)?' "$source_file" >> "$references_file" || true
  fi
}

audit_macos() {
  local otool_bin="${NATIVE_PORTABILITY_AUDIT_OTOOL:-otool}"
  local otool_output="$tmp_dir/otool.txt"

  if ! command -v "$otool_bin" >/dev/null 2>&1; then
    printf 'Native binary portability audit failed: otool is required for macOS install-name inspection\n' >&2
    exit 1
  fi

  if ! "$otool_bin" -L "$binary_path" > "$otool_output" 2> "$tmp_dir/otool.err"; then
    printf 'Native binary portability audit failed: otool -L could not inspect %s\n' "$binary_path" >&2
    cat "$tmp_dir/otool.err" >&2
    exit 1
  fi

  append_forbidden_lines "$otool_output"
}

audit_linux() {
  local readelf_bin="${NATIVE_PORTABILITY_AUDIT_READELF:-readelf}"
  local strings_bin="${NATIVE_PORTABILITY_AUDIT_STRINGS:-strings}"

  if command -v "$readelf_bin" >/dev/null 2>&1; then
    "$readelf_bin" -d "$binary_path" > "$tmp_dir/readelf-dynamic.txt" 2> "$tmp_dir/readelf.err" || true
    append_forbidden_lines "$tmp_dir/readelf-dynamic.txt"
  fi

  if ! command -v "$strings_bin" >/dev/null 2>&1; then
    printf 'Native binary portability audit failed: strings is required for Linux runtime-reference inspection\n' >&2
    exit 1
  fi

  if ! "$strings_bin" -a "$binary_path" > "$tmp_dir/strings.txt" 2> "$tmp_dir/strings.err"; then
    printf 'Native binary portability audit failed: strings could not inspect %s\n' "$binary_path" >&2
    cat "$tmp_dir/strings.err" >&2
    exit 1
  fi

  append_linux_forbidden_string_lines "$tmp_dir/strings.txt"
}

case "$platform" in
  linux)
    audit_linux
    ;;
  macos|darwin)
    platform="macos"
    audit_macos
    ;;
  *)
    printf 'Native binary portability audit failed: unsupported audit platform %s\n' "$platform" >&2
    exit 2
    ;;
esac

if [[ -s "$references_file" ]]; then
  printf 'Native binary portability audit failed: %s contains forbidden %s runtime references:\n' "$binary_path" "$FORBIDDEN_REFERENCE" >&2
  sort -u "$references_file" | while IFS= read -r reference; do
    printf '  %s\n' "$reference" >&2
  done
  exit 1
fi

printf 'Native binary portability audit passed: %s has no forbidden %s runtime references for %s\n' "$binary_path" "$FORBIDDEN_REFERENCE" "$platform"
