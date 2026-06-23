#!/usr/bin/env bash
set -euo pipefail

usage() {
  printf 'usage: local-manifest-validate.sh --repo-root <path> --manifest-path <path>\n' >&2
}

fail_usage() {
  usage
  exit 2
}

repo_root=""
manifest_path=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --repo-root)
      [[ $# -ge 2 ]] || fail_usage
      repo_root="$2"
      shift 2
      ;;
    --manifest-path)
      [[ $# -ge 2 ]] || fail_usage
      manifest_path="$2"
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

[[ -n "$repo_root" ]] || fail_usage
[[ -n "$manifest_path" ]] || fail_usage

repo_root="$(realpath "$repo_root")"
manifest="$(<"$manifest_path")"
expected_path="path: $repo_root"

errors=()

if [[ "$manifest" != *"type: dir"* ]]; then
  errors+=("local manifest does not contain a Flatpak type: dir source")
fi

if [[ "$manifest" != *"$expected_path"* ]]; then
  errors+=("local manifest does not point at the requested checkout path")
fi

if [[ "$manifest" == *"nix build .#sce"* || "$manifest" == *"nix build .#default"* ]]; then
  errors+=("local manifest references a Nix-built sce binary")
fi

if [[ "$manifest" != *"cargo --offline build --release --manifest-path cli/Cargo.toml --bin sce"* ]]; then
  errors+=("local manifest no longer runs the Flatpak Cargo source build")
fi

if [[ ${#errors[@]} -gt 0 ]]; then
  for error in "${errors[@]}"; do
    printf 'Flatpak local-manifest validation failed: %s\n' "$error" >&2
  done
  exit 1
fi
