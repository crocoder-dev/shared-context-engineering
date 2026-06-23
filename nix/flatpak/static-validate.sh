#!/usr/bin/env bash
set -euo pipefail

APP_ID="dev.crocoder.sce"

usage() {
  printf 'usage: static-validate.sh --repo-root <path>\n' >&2
}

fail_usage() {
  usage
  exit 2
}

repo_root=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --repo-root)
      [[ $# -ge 2 ]] || fail_usage
      repo_root="$2"
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

flatpak_dir="$repo_root/packaging/flatpak"
manifest_path="$flatpak_dir/$APP_ID.yml"
metainfo_path="$flatpak_dir/$APP_ID.metainfo.xml"
cargo_sources_path="$flatpak_dir/cargo-sources.json"

if [[ ! -r "$manifest_path" ]]; then
  printf 'could not read %s\n' "$manifest_path" >&2
  exit 1
fi

manifest="$(<"$manifest_path")"
errors=()

require_contains() {
  local needle="$1"
  local message="$2"

  if [[ "$manifest" != *"$needle"* ]]; then
    errors+=("$message")
  fi
}

require_contains "id: dev.crocoder.sce" "manifest app ID is not dev.crocoder.sce"
require_contains "command: sce" "manifest command is not sce"
require_contains "org.freedesktop.Sdk.Extension.rust-stable" "Rust SDK extension is missing"
require_contains "bash ./scripts/prepare-cli-generated-assets.sh \"\$PWD\"" "generated-asset preparation command is missing"
require_contains "cargo --offline build --release --manifest-path cli/Cargo.toml --bin sce" "offline Cargo source-build command is missing"
require_contains "install -Dm755 cli/target/release/sce /app/bin/sce" "sce install command is missing"
require_contains "install -Dm755 packaging/flatpak/git-host-bridge /app/bin/git" "host git bridge install command is missing"
require_contains "--talk-name=org.freedesktop.Flatpak" "host Flatpak permission is missing"
require_contains "cargo-sources.json" "Cargo source descriptor is missing from manifest"

if ! awk '
  /^[[:space:]]*-[[:space:]]+commit:[[:space:]]+[0-9a-f]{40}[[:space:]]*$/ { state = 1; next }
  state == 1 && /^[[:space:]]+type:[[:space:]]+git[[:space:]]*$/ { state = 2; next }
  state == 2 && /^[[:space:]]+url:[[:space:]]+https:\/\/github\.com\/crocoder-dev\/shared-context-engineering\.git[[:space:]]*$/ { found = 1 }
  { state = 0 }
  END { exit found ? 0 : 1 }
' "$manifest_path"; then
  errors+=("release manifest does not use a pinned git source")
fi

banned_snippets=(
  "nix build .#sce"
  "nix build .#default"
  "github.com/crocoder-dev/shared-context-engineering/releases"
  "release-artifacts"
  "@crocoder-dev/sce"
)

for path in "$flatpak_dir"/*; do
  [[ -f "$path" ]] || continue
  [[ "$(basename "$path")" != "sce-flatpak.sh" ]] || continue

  if [[ ! -r "$path" ]]; then
    printf 'could not read %s\n' "$path" >&2
    exit 1
  fi

  text="$(<"$path")"
  relative_path="${path#"$repo_root/"}"

  for snippet in "${banned_snippets[@]}"; do
    if [[ "$text" == *"$snippet"* ]]; then
      errors+=("$relative_path references disallowed artifact source: $snippet")
    fi
  done
done

if ! jq -e 'type == "array" and length > 0' "$cargo_sources_path" >/dev/null 2>&1; then
  errors+=("cargo-sources.json is empty or not a list")
fi

if ! jq -e 'any(.[]; type == "object" and ((.url // "") | contains("turso")))' "$cargo_sources_path" >/dev/null 2>&1; then
  errors+=("Turso cargo-sources entry is missing")
fi

if ! jq -e 'any(.[]; type == "object" and .type == "archive" and ((.url // "") | contains("static.crates.io")))' "$cargo_sources_path" >/dev/null 2>&1; then
  errors+=("crates.io archive source entries are missing")
fi

if ! metainfo_id="$(xmllint --xpath 'string(/component/id)' "$metainfo_path" 2>/dev/null)"; then
  printf 'could not parse %s\n' "$metainfo_path" >&2
  exit 1
fi

if [[ "$metainfo_id" != "$APP_ID" ]]; then
  errors+=("metainfo ID is not dev.crocoder.sce")
fi

if ! xmllint --xpath '/component/provides/binary[text()="sce"]' "$metainfo_path" >/dev/null 2>&1; then
  errors+=("metainfo does not provide binary sce")
fi

if [[ ${#errors[@]} -gt 0 ]]; then
  for error in "${errors[@]}"; do
    printf 'Flatpak static validation failed: %s\n' "$error" >&2
  done
  exit 1
fi
