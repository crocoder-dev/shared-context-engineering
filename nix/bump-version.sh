#!/usr/bin/env bash
set -euo pipefail

APP_ID="dev.crocoder.sce"

usage() {
  printf 'usage: bump-version.sh --repo-root <path> --version <version>\n\n' >&2
  printf 'Updates the project version in .version, cli/Cargo.toml, cli/Cargo.lock,\n' >&2
  printf 'npm/package.json, and the first release in packaging/flatpak/%s.metainfo.xml.\n' "$APP_ID" >&2
  printf '\n' >&2
  printf 'Options:\n' >&2
  printf '  --repo-root <path>  Repository root directory\n' >&2
  printf '  --version <version> New version to set\n' >&2
  printf '  --from <version>    Previous version (optional; auto-detected from .version if omitted)\n' >&2
  printf '  --dry-run           Print changes without writing\n' >&2
  printf '  -h, --help          Show this help\n' >&2
}

fail_usage() {
  usage
  exit 2
}

repo_root=""
version=""
from_version=""
dry_run=0

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
    --from)
      [[ $# -ge 2 ]] || fail_usage
      from_version="$2"
      shift 2
      ;;
    --dry-run)
      dry_run=1
      shift
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

escape_sed_regex() {
  # shellcheck disable=SC2016
  sed 's/[.[\*^$()+?{}|\\]/\\&/g'
}

escape_sed_replacement() {
  sed 's/[&\\]/\\&/g'
}

version_path="$repo_root/.version"
cargo_toml_path="$repo_root/cli/Cargo.toml"
cargo_lock_path="$repo_root/cli/Cargo.lock"
npm_package_path="$repo_root/npm/package.json"
metainfo_path="$repo_root/packaging/flatpak/$APP_ID.metainfo.xml"

if [[ -z "$from_version" ]]; then
  from_version="$(sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//' < "$version_path")"
fi

files=(
  "$version_path"
  "$cargo_toml_path"
  "$cargo_lock_path"
  "$npm_package_path"
  "$metainfo_path"
)

for f in "${files[@]}"; do
  if [[ ! -r "$f" ]]; then
    printf 'error: could not read %s\n' "$f" >&2
    exit 1
  fi
  if [[ ! -w "$f" ]] && [[ "$dry_run" -ne 1 ]]; then
    printf 'error: %s is not writable\n' "$f" >&2
    exit 1
  fi
done

from_version_regex="$(printf '%s' "$from_version" | escape_sed_regex)"
version_replacement="$(printf '%s' "$version" | escape_sed_replacement)"
changes=0

replace_once() {
  local path="$1"
  local description="$2"
  local expression="$3"
  local tmp_path
  local rel

  rel="${path#"$repo_root"/}"
  tmp_path="$(mktemp)"
  sed "$expression" "$path" > "$tmp_path"

  if cmp -s "$path" "$tmp_path"; then
    printf '  %s: warning: did not update %s from %s\n' "$rel" "$description" "$from_version"
    rm -f "$tmp_path"
    return
  fi

  printf '  %s: %s: %s -> %s\n' "$rel" "$description" "$from_version" "$version"
  if [[ "$dry_run" -ne 1 ]]; then
    cp "$tmp_path" "$path"
  fi
  rm -f "$tmp_path"
  changes=$((changes + 1))
}

replace_once \
  "$version_path" \
  "checked-in version" \
  "s/^${from_version_regex}\$/${version_replacement}/"

replace_once \
  "$cargo_toml_path" \
  "package version" \
  "0,/^version = /{/^version = \"$from_version_regex\"\$/s//version = \"$version_replacement\"/}"

replace_once \
  "$cargo_lock_path" \
  "shared-context-engineering package version" \
  "/^name = \"shared-context-engineering\"\$/,/^dependencies = \[/s/^version = \"$from_version_regex\"\$/version = \"$version_replacement\"/"

replace_once \
  "$npm_package_path" \
  "package version" \
  "0,/\"version\": /{/\"version\": \"$from_version_regex\"/s//\"version\": \"$version_replacement\"/}"

replace_once \
  "$metainfo_path" \
  "primary Flatpak release version" \
  "0,/<release /{/<release version=\"$from_version_regex\"/s//<release version=\"$version_replacement\"/}"

if [[ "$dry_run" -eq 1 ]]; then
  printf 'Dry run: %d file(s) would change\n' "$changes"
else
  printf 'Bumped %d file(s) to %s\n' "$changes" "$version"
fi
