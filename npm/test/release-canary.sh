#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/release-fixture.sh"
FIXTURE_LOG_PREFIX="release-canary"

REPOSITORY_SLUG="crocoder-dev/shared-context-engineering"
NPM_PACKAGE_NAME="@crocoder-dev/sce"
REPOSITORY_PUBLIC_KEY="$SCRIPT_DIR/../lib/release-manifest-public-key.pem"
PACKED_PUBLIC_KEY_ENTRY="package/lib/release-manifest-public-key.pem"
PACKED_PACKAGE_JSON_ENTRY="package/package.json"
VERSION_PATTERN='^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$'
DOWNLOAD_TIMEOUT_MS=60000
DOWNLOAD_ATTEMPTS=3
DOWNLOAD_RETRY_SECONDS=5
REGISTRY_ATTEMPTS=10
REGISTRY_RETRY_SECONDS=30

MODE=""
RELEASE_DIR=""
SOURCE_TARBALL=""
GITHUB_TAG=""
REGISTRY_VERSION=""
EXPECTED_VERSION=""
EXPECTED_SHA256=""

usage() {
  cat <<'EOF'
Usage: release-canary.sh --expected-version <version> [--expected-sha256 <hex>]
                         ( --release-dir <dir> --tarball <tgz>
                         | --github-release <tag>
                         | --registry-version <version> [--tarball <tgz>] )

Installs the exact, unmodified sce-v<version>-npm.tgz with its bundled
production public key and verifies it. Nothing is re-signed or repacked.

--release-dir serves the real release manifest, signature, and archives from
<dir> over loopback HTTPS and installs <tgz> against them (pre-release gate).

--github-release downloads sce-v<version>-npm.tgz and the signed manifest from
the published GitHub Release and installs with no installer overrides.

--registry-version compares the exact registry tarball and its dist.integrity
with the GitHub Release tgz (or --tarball), with bounded retries, then installs
that exact version from the registry with no installer overrides.

Every mode asserts the tarball SHA-256 is unchanged by the run (and equals
--expected-sha256 when given), the bundled public key is byte-identical to the
checked-in npm/lib/release-manifest-public-key.pem, the release manifest
signature verifies against it, the installed launcher, .bin/sce, and
runtime/sce modes, and that sce reports --expected-version.
EOF
}

fail() {
  fixture_fail "$1"
}

require_version() {
  local label="$1"
  local value="$2"

  [[ "$value" =~ $VERSION_PATTERN ]] ||
    fail "$label '$value' is not an exact version; dist-tags and ranges are not resolved."
}

set_mode() {
  [[ -z "$MODE" ]] || fail "Choose exactly one of --release-dir, --github-release, or --registry-version."
  MODE="$1"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --release-dir)
      [[ -n "${2:-}" ]] || fail "Missing value for --release-dir."
      [[ -d "$2" ]] || fail "Release directory $2 does not exist."
      set_mode local
      RELEASE_DIR="$(cd "$2" && pwd)"
      shift 2
      ;;
    --tarball)
      [[ -n "${2:-}" ]] || fail "Missing value for --tarball."
      [[ -f "$2" ]] || fail "Tarball $2 does not exist."
      SOURCE_TARBALL="$(cd "$(dirname "$2")" && pwd)/$(basename "$2")"
      shift 2
      ;;
    --github-release)
      [[ -n "${2:-}" ]] || fail "Missing value for --github-release."
      set_mode github
      GITHUB_TAG="$2"
      shift 2
      ;;
    --registry-version)
      [[ -n "${2:-}" ]] || fail "Missing value for --registry-version."
      set_mode registry
      REGISTRY_VERSION="$2"
      shift 2
      ;;
    --expected-version)
      [[ -n "${2:-}" ]] || fail "Missing value for --expected-version."
      EXPECTED_VERSION="$2"
      shift 2
      ;;
    --expected-sha256)
      [[ -n "${2:-}" ]] || fail "Missing value for --expected-sha256."
      EXPECTED_SHA256="$2"
      shift 2
      ;;
    --help | -h)
      usage
      exit 0
      ;;
    *)
      usage >&2
      fail "Unknown argument: $1"
      ;;
  esac
done

[[ -n "$MODE" ]] || {
  usage >&2
  fail "Choose exactly one of --release-dir, --github-release, or --registry-version."
}
[[ -n "$EXPECTED_VERSION" ]] || fail "Missing --expected-version."
require_version "--expected-version" "$EXPECTED_VERSION"
case "$MODE" in
  local)
    [[ -n "$SOURCE_TARBALL" ]] || fail "--release-dir requires --tarball."
    ;;
  github)
    [[ -z "$SOURCE_TARBALL" ]] || fail "--github-release downloads the tarball; do not pass --tarball."
    [[ "$GITHUB_TAG" == "v$EXPECTED_VERSION" ]] ||
      fail "GitHub release tag '$GITHUB_TAG' does not match --expected-version (expected 'v$EXPECTED_VERSION')."
    ;;
  registry)
    require_version "--registry-version" "$REGISTRY_VERSION"
    [[ "$REGISTRY_VERSION" == "$EXPECTED_VERSION" ]] ||
      fail "--registry-version '$REGISTRY_VERSION' does not match --expected-version '$EXPECTED_VERSION'."
    ;;
esac
if [[ -n "$EXPECTED_SHA256" && ! "$EXPECTED_SHA256" =~ ^[0-9a-f]{64}$ ]]; then
  fail "--expected-sha256 must be 64 lowercase hex characters."
fi

fixture_require_tools
[[ -f "$REPOSITORY_PUBLIC_KEY" ]] || fail "Checked-in public key $REPOSITORY_PUBLIC_KEY does not exist."

work_dir="$(mktemp -d "${TMPDIR:-/tmp}/sce-npm-release-canary.XXXXXX")"
cleanup() {
  fixture_serve_stop
  rm -rf "$work_dir"
}
trap cleanup EXIT

download_dir="$work_dir/download"
prefix_dir="$work_dir/prefix"
keys_dir="$work_dir/keys"
mkdir -p "$download_dir" "$prefix_dir" "$keys_dir" "$work_dir/home" "$work_dir/tmp"

tarball_name="sce-v${EXPECTED_VERSION}-npm.tgz"
manifest_name="sce-v${EXPECTED_VERSION}-release-manifest.json"
release_base_url="https://github.com/${REPOSITORY_SLUG}/releases/download/v${EXPECTED_VERSION}"

clean_env=(
  env -i
  "PATH=$PATH"
  "HOME=$work_dir/home"
  "TMPDIR=$work_dir/tmp"
  "npm_config_cache=$work_dir/npm-cache"
  "npm_config_userconfig=$work_dir/npmrc"
)

sha256_of() {
  sha256sum "$1" | cut -d ' ' -f 1
}

sha512_integrity_of() {
  printf 'sha512-%s' "$(openssl dgst -sha512 -binary "$1" | openssl base64 -A)"
}

retry() {
  local attempts="$1"
  local seconds="$2"
  local attempt
  shift 2

  for ((attempt = 1; attempt <= attempts; attempt++)); do
    "$@" && return 0
    if ((attempt < attempts)); then
      fixture_log "attempt $attempt/$attempts failed; retrying in ${seconds}s"
      sleep "$seconds"
    fi
  done
  return 1
}

download() {
  local url="$1"
  local output="$2"

  "${clean_env[@]}" node --input-type=module -e '
    import { writeFileSync } from "node:fs";
    const [url, output, timeoutMs] = process.argv.slice(1);
    const response = await fetch(url, { signal: AbortSignal.timeout(Number(timeoutMs)) });
    if (!response.ok) {
      console.error("GET " + url + " returned HTTP " + response.status);
      process.exit(1);
    }
    writeFileSync(output, Buffer.from(await response.arrayBuffer()));
  ' "$url" "$output" "$DOWNLOAD_TIMEOUT_MS"
}

download_release_asset() {
  local name="$1"

  retry "$DOWNLOAD_ATTEMPTS" "$DOWNLOAD_RETRY_SECONDS" download "$release_base_url/$name" "$download_dir/$name" ||
    fail "Could not download $release_base_url/$name after $DOWNLOAD_ATTEMPTS attempts."
  fixture_log "downloaded $release_base_url/$name"
}

assert_tarball_identity() {
  local tarball="$1"
  local packed_name packed_version packed_launcher_mode

  packed_name="$(tar -xzOf "$tarball" "$PACKED_PACKAGE_JSON_ENTRY" | node -p 'JSON.parse(require("fs").readFileSync(0, "utf8")).name')" ||
    fail "Tarball $tarball has no readable $PACKED_PACKAGE_JSON_ENTRY."
  packed_version="$(tar -xzOf "$tarball" "$PACKED_PACKAGE_JSON_ENTRY" | node -p 'JSON.parse(require("fs").readFileSync(0, "utf8")).version')"
  [[ "$packed_name" == "$NPM_PACKAGE_NAME" ]] ||
    fail "Packed package name '$packed_name' does not match '$NPM_PACKAGE_NAME'."
  [[ "$packed_version" == "$EXPECTED_VERSION" ]] ||
    fail "Packed package version '$packed_version' does not match --expected-version '$EXPECTED_VERSION'."
  fixture_log "packed package $packed_name@$packed_version"

  packed_launcher_mode="$(tar -tvzf "$tarball" "$FIXTURE_LAUNCHER_ENTRY" | cut -d ' ' -f 1)"
  [[ -n "$packed_launcher_mode" ]] || fail "Tarball $tarball is missing $FIXTURE_LAUNCHER_ENTRY."
  fixture_log "packed $FIXTURE_LAUNCHER_ENTRY mode $packed_launcher_mode"
}

assert_bundled_public_key() {
  local tarball="$1"
  local bundled_key="$work_dir/bundled-public-key.pem"
  local bundled_sha repository_sha

  tar -xzOf "$tarball" "$PACKED_PUBLIC_KEY_ENTRY" >"$bundled_key" ||
    fail "Tarball $tarball is missing $PACKED_PUBLIC_KEY_ENTRY."
  bundled_sha="$(sha256_of "$bundled_key")"
  repository_sha="$(sha256_of "$REPOSITORY_PUBLIC_KEY")"
  [[ "$bundled_sha" == "$repository_sha" ]] ||
    fail "Bundled $PACKED_PUBLIC_KEY_ENTRY (sha256 $bundled_sha) is not byte-identical to the checked-in npm/lib/release-manifest-public-key.pem (sha256 $repository_sha)."
  fixture_log "bundled public key byte-identical to checked-in key (sha256 $bundled_sha)"
}

assert_manifest_signature() {
  local manifest="$1"
  local signature="$2"
  local manifest_version

  [[ -f "$manifest" ]] || fail "Release manifest $manifest does not exist."
  [[ -f "$signature" ]] || fail "Release manifest signature $signature does not exist."
  openssl base64 -d -A -in "$signature" -out "$work_dir/manifest.sig.bin" ||
    fail "Release manifest signature $signature is not valid base64."
  openssl dgst -sha256 -verify "$REPOSITORY_PUBLIC_KEY" -signature "$work_dir/manifest.sig.bin" "$manifest" >/dev/null ||
    fail "Release manifest $manifest signature does not verify against the checked-in public key."
  manifest_version="$(node -p "JSON.parse(require('fs').readFileSync(process.argv[1], 'utf8')).version" "$manifest")"
  [[ "$manifest_version" == "$EXPECTED_VERSION" ]] ||
    fail "Release manifest version '$manifest_version' does not match --expected-version '$EXPECTED_VERSION'."
  fixture_log "release manifest $(basename "$manifest") verified with the checked-in public key (sha256 $(sha256_of "$manifest"))"
}

fetch_registry_tarball() {
  local registry_dir="$work_dir/registry"
  local packed_name

  rm -rf "$registry_dir"
  mkdir -p "$registry_dir"
  registry_integrity="$("${clean_env[@]}" npm view "$NPM_PACKAGE_NAME@$REGISTRY_VERSION" dist.integrity)" || return 1
  [[ -n "$registry_integrity" ]] || return 1
  packed_name="$(cd "$registry_dir" && "${clean_env[@]}" npm pack --silent "$NPM_PACKAGE_NAME@$REGISTRY_VERSION")" || return 1
  registry_tarball="$registry_dir/$packed_name"
  [[ -f "$registry_tarball" ]]
}

install_remote() {
  local spec="$1"

  "${clean_env[@]}" npm install \
    --prefix "$prefix_dir" \
    --no-audit \
    --no-fund \
    --foreground-scripts \
    "$spec" ||
    fail "npm install of $spec failed."
}

case "$MODE" in
  local)
    tarball="$SOURCE_TARBALL"
    manifest="$RELEASE_DIR/$manifest_name"
    ;;
  github | registry)
    download_release_asset "$manifest_name"
    download_release_asset "$manifest_name.sig"
    manifest="$download_dir/$manifest_name"
    if [[ -n "$SOURCE_TARBALL" ]]; then
      tarball="$SOURCE_TARBALL"
    else
      download_release_asset "$tarball_name"
      tarball="$download_dir/$tarball_name"
    fi
    ;;
esac

tarball_sha_before="$(sha256_of "$tarball")"
fixture_log "tarball $(basename "$tarball") sha256 $tarball_sha_before"
if [[ -n "$EXPECTED_SHA256" ]]; then
  [[ "$tarball_sha_before" == "$EXPECTED_SHA256" ]] ||
    fail "Tarball sha256 $tarball_sha_before does not match --expected-sha256 $EXPECTED_SHA256."
fi

assert_tarball_identity "$tarball"
assert_bundled_public_key "$tarball"
assert_manifest_signature "$manifest" "$manifest.sig"

case "$MODE" in
  local)
    fixture_generate_tls_cert "$keys_dir"
    fixture_serve_start "$RELEASE_DIR" "$keys_dir" "$work_dir/server-port"
    fixture_npm_install "$prefix_dir" "$tarball" "$FIXTURE_PORT" "$keys_dir/tls-cert.pem" "$work_dir"
    fixture_serve_stop
    ;;
  github)
    install_remote "$tarball"
    ;;
  registry)
    registry_integrity=""
    registry_tarball=""
    retry "$REGISTRY_ATTEMPTS" "$REGISTRY_RETRY_SECONDS" fetch_registry_tarball ||
      fail "Registry did not serve $NPM_PACKAGE_NAME@$REGISTRY_VERSION after $REGISTRY_ATTEMPTS attempts."
    github_integrity="$(sha512_integrity_of "$tarball")"
    [[ "$registry_integrity" == "$github_integrity" ]] ||
      fail "Registry dist.integrity $registry_integrity does not match the GitHub tarball integrity $github_integrity."
    [[ "$(sha512_integrity_of "$registry_tarball")" == "$github_integrity" ]] ||
      fail "Registry tarball bytes for $NPM_PACKAGE_NAME@$REGISTRY_VERSION differ from the GitHub tarball."
    fixture_log "registry tarball byte-identical to the GitHub tarball ($registry_integrity)"
    install_remote "$NPM_PACKAGE_NAME@$REGISTRY_VERSION"
    locked_integrity="$(node -p "JSON.parse(require('fs').readFileSync(process.argv[1], 'utf8')).packages['node_modules/$NPM_PACKAGE_NAME'].integrity" "$prefix_dir/package-lock.json")"
    [[ "$locked_integrity" == "$registry_integrity" ]] ||
      fail "Installed $NPM_PACKAGE_NAME integrity $locked_integrity does not match the registry dist.integrity $registry_integrity."
    fixture_log "installed $NPM_PACKAGE_NAME@$REGISTRY_VERSION from the registry with integrity $locked_integrity"
    ;;
esac

fixture_assert_install_layout "$prefix_dir"
installed_launcher="$prefix_dir/$FIXTURE_INSTALLED_PACKAGE_DIR/bin/sce.js"
[[ -x "$installed_launcher" ]] || fail "Installed launcher $installed_launcher is not executable."
fixture_log "installed bin/sce.js mode $(node -p "(require('fs').statSync(process.argv[1]).mode & 0o777).toString(8)" "$installed_launcher")"

"${clean_env[@]}" "$FIXTURE_SCE_BIN" --help >/dev/null || fail "sce --help exited non-zero."
version_json="$("${clean_env[@]}" "$FIXTURE_SCE_BIN" version --format json)" ||
  fail "sce version --format json exited non-zero."
reported_version="$(node -p "JSON.parse(process.argv[1]).version" "$version_json")" ||
  fail "sce version --format json printed invalid JSON: $version_json"
[[ "$reported_version" == "$EXPECTED_VERSION" ]] ||
  fail "sce version reported '$reported_version'; expected '$EXPECTED_VERSION'."
fixture_log "sce reported version $reported_version"

tarball_sha_after="$(sha256_of "$tarball")"
[[ "$tarball_sha_after" == "$tarball_sha_before" ]] ||
  fail "Tarball sha256 changed during the run: $tarball_sha_before before, $tarball_sha_after after."
fixture_log "tarball sha256 unchanged after the run ($tarball_sha_after)"

printf 'release-canary: OK (%s mode: %s@%s, tarball sha256 %s)\n' "$MODE" "$NPM_PACKAGE_NAME" "$EXPECTED_VERSION" "$tarball_sha_after"
