#!/usr/bin/env bash
set -euo pipefail

APP_ID="dev.crocoder.sce"

usage() {
  printf 'usage: bump-version.sh --repo-root <path> --version <version>\n\n' >&2
  printf 'Updates .version, cli/Cargo.toml, cli/Cargo.lock, npm/package.json,\n' >&2
  printf 'and packaging/flatpak/%s.metainfo.xml to match --version.\n' "$APP_ID" >&2
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

changes=0

for f in "${files[@]}"; do
  rel="${f#"$repo_root"/}"
  if grep -qF "$from_version" "$f"; then
    printf '  %s: %s -> %s\n' "$rel" "$from_version" "$version"
    if [[ "$dry_run" -ne 1 ]]; then
      sed -i "s/\\b$from_version\\b/$version/g" "$f"
    fi
    changes=$((changes + 1))
  else
    printf '  %s: warning: did not find %s, skipping\n' "$rel" "$from_version"
  fi
done

if [[ "$dry_run" -eq 1 ]]; then
  printf 'Dry run: %d file(s) would change\n' "$changes"
else
  printf 'Bumped %d file(s) to %s\n' "$changes" "$version"
fi
