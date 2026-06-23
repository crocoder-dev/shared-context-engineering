#!/usr/bin/env bash
set -euo pipefail

APP_ID="dev.crocoder.sce"

usage() {
  printf 'usage: version-parity.sh --repo-root <path> --version <semver>\n' >&2
}

fail_usage() {
  usage
  exit 2
}

trim_whitespace() {
  sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//'
}

read_trimmed_file() {
  local path="$1"

  if [[ ! -r "$path" ]]; then
    printf 'could not read %s\n' "$path" >&2
    exit 1
  fi

  trim_whitespace < "$path"
}

repo_root=""
version=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --repo-root)
      [[ $# -ge 2 ]] || fail_usage
      repo_root="$2"
      shift 2
      ;;
    --version)
      [[ $# -ge 2 ]] || fail_usage
      version="$2"
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
[[ -n "$version" ]] || fail_usage

version_path="$repo_root/.version"
cargo_toml_path="$repo_root/cli/Cargo.toml"
npm_package_path="$repo_root/npm/package.json"
metainfo_path="$repo_root/packaging/flatpak/$APP_ID.metainfo.xml"

checked_in_version="$(read_trimmed_file "$version_path")"

if [[ ! -r "$cargo_toml_path" ]]; then
  printf 'could not read %s\n' "$cargo_toml_path" >&2
  exit 1
fi

cargo_version="$(sed -n 's/^version = "\([^"]*\)"$/\1/p' "$cargo_toml_path" | sed -n '1p')"

if [[ ! -r "$npm_package_path" ]]; then
  printf 'could not parse %s\n' "$npm_package_path" >&2
  exit 1
fi

if ! npm_version="$(jq -r '.version // ""' "$npm_package_path")"; then
  printf 'could not parse %s\n' "$npm_package_path" >&2
  exit 1
fi

if [[ ! -r "$metainfo_path" ]]; then
  printf 'could not parse %s\n' "$metainfo_path" >&2
  exit 1
fi

if ! flatpak_version="$(xmllint --xpath 'string((/component/releases/release/@version)[1])' "$metainfo_path" 2>/dev/null)"; then
  printf 'could not parse %s\n' "$metainfo_path" >&2
  exit 1
fi

errors=()

if [[ -z "$cargo_version" ]]; then
  errors+=("cli/Cargo.toml package version is missing")
fi

if [[ -z "$flatpak_version" ]]; then
  errors+=("Flatpak metainfo release metadata is missing")
fi

if [[ "$version" != "$checked_in_version" ]]; then
  errors+=("requested release version $version does not match .version $checked_in_version")
fi

if [[ "$version" != "$cargo_version" ]]; then
  errors+=("cli/Cargo.toml version $cargo_version does not match release version $version")
fi

if [[ "$version" != "$npm_version" ]]; then
  errors+=("npm/package.json version $npm_version does not match release version $version")
fi

if [[ "$version" != "$flatpak_version" ]]; then
  errors+=("Flatpak metainfo release version $flatpak_version does not match release version $version")
fi

if [[ ${#errors[@]} -gt 0 ]]; then
  for error in "${errors[@]}"; do
    printf 'Flatpak release version validation failed: %s\n' "$error" >&2
  done
  exit 1
fi
