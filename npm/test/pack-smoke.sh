#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=npm/test/release-fixture.sh
source "$SCRIPT_DIR/release-fixture.sh"
FIXTURE_LOG_PREFIX="pack-smoke"

PACKAGE_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
SOURCE_TARBALL=""
SOURCE_BINARY=""
EXPECTED_VERSION=""
PREFIX_DIR=""

usage() {
  cat <<'EOF'
Usage: pack-smoke.sh [--package-dir <path> | --tarball <path>]
                     [--binary <path>] [--expected-version <version>]
                     [--prefix <dir>]

Packs the npm launcher package, installs it into an isolated prefix against a
loopback fixture release, and verifies `sce --help` and `sce version`.

With --tarball, the launcher mode is asserted on the given packed tarball and
its package contents are used instead of a package directory.

With --binary, the given executable (e.g. the Nix-built sce-release binary) is
served as bin/sce in the release archive instead of the fixture stub. The
installed CLI must exit 0 for `sce --help` and report the expected version via
`sce version --format json`.

--expected-version overrides the version asserted from `sce version`
(default: the staged package.json version).

--prefix installs into the given directory and keeps it after the run instead
of using a temporary prefix.

The release fixture is built by release-fixture.sh with a throwaway signing
key, so this smoke does not prove the exact release artifact or production key.
EOF
}

fail() {
  fixture_fail "$1"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --package-dir)
      [[ -n "${2:-}" ]] || fail "Missing value for --package-dir."
      PACKAGE_DIR="$(cd "$2" && pwd)"
      shift 2
      ;;
    --tarball)
      [[ -n "${2:-}" ]] || fail "Missing value for --tarball."
      [[ -f "$2" ]] || fail "Tarball $2 does not exist."
      SOURCE_TARBALL="$(cd "$(dirname "$2")" && pwd)/$(basename "$2")"
      shift 2
      ;;
    --binary)
      [[ -n "${2:-}" ]] || fail "Missing value for --binary."
      [[ -f "$2" ]] || fail "Binary $2 does not exist."
      SOURCE_BINARY="$(cd "$(dirname "$2")" && pwd)/$(basename "$2")"
      shift 2
      ;;
    --expected-version)
      [[ -n "${2:-}" ]] || fail "Missing value for --expected-version."
      EXPECTED_VERSION="$2"
      shift 2
      ;;
    --prefix)
      [[ -n "${2:-}" ]] || fail "Missing value for --prefix."
      mkdir -p "$2"
      PREFIX_DIR="$(cd "$2" && pwd)"
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

fixture_require_tools

if [[ -z "$SOURCE_TARBALL" ]]; then
  [[ -f "$PACKAGE_DIR/package.json" ]] || fail "No package.json found in $PACKAGE_DIR."
fi

work_dir="$(mktemp -d "${TMPDIR:-/tmp}/sce-npm-pack-smoke.XXXXXX")"
cleanup() {
  fixture_serve_stop
  rm -rf "$work_dir"
}
trap cleanup EXIT

stage_dir="$work_dir/stage"
release_dir="$work_dir/release"
pack_dir="$work_dir/pack"
prefix_dir="${PREFIX_DIR:-$work_dir/prefix}"
keys_dir="$work_dir/keys"
mkdir -p "$stage_dir" "$release_dir" "$pack_dir" "$prefix_dir" "$keys_dir"

fixture_stage_package "$stage_dir" "$PACKAGE_DIR" "$SOURCE_TARBALL"
fixture_resolve_platform "$stage_dir"
version="$FIXTURE_VERSION"
expected_version="${EXPECTED_VERSION:-$version}"

printf 'pack-smoke: version %s, target %s\n' "$version" "$FIXTURE_TARGET_TRIPLE"

fixture_generate_keys "$stage_dir" "$keys_dir"
fixture_build_release "$release_dir" "$keys_dir" "$work_dir" "$SOURCE_BINARY"
fixture_pack "$stage_dir" "$pack_dir"
fixture_serve_start "$release_dir" "$keys_dir" "$work_dir/server-port"
fixture_npm_install "$prefix_dir" "$FIXTURE_TARBALL" "$FIXTURE_PORT" "$keys_dir/tls-cert.pem" "$work_dir"
fixture_assert_install_layout "$prefix_dir"

sce_cmd=("$FIXTURE_SCE_BIN")
if [[ ! -x /usr/bin/env ]]; then
  printf 'pack-smoke: /usr/bin/env unavailable; running installed bin through node\n'
  sce_cmd=(node "$FIXTURE_SCE_BIN")
fi

help_output="$("${sce_cmd[@]}" --help)" || fail "sce --help exited non-zero."

if [[ -n "$SOURCE_BINARY" ]]; then
  version_json="$("${sce_cmd[@]}" version --format json)" ||
    fail "sce version --format json exited non-zero."
  reported_version="$(node -p "JSON.parse(process.argv[1]).version" "$version_json")" ||
    fail "sce version --format json printed invalid JSON: $version_json"
  [[ "$reported_version" == "$expected_version" ]] ||
    fail "sce version reported '$reported_version'; expected '$expected_version'."
  printf 'pack-smoke: real binary reported version %s\n' "$reported_version"
  printf 'pack-smoke: OK (real binary: sce --help, sce version --format json)\n'
  exit 0
fi

[[ "$help_output" == "$FIXTURE_STUB_HELP_OUTPUT" ]] ||
  fail "sce --help printed '$help_output'; expected '$FIXTURE_STUB_HELP_OUTPUT'."

version_output="$("${sce_cmd[@]}" version)" || fail "sce version exited non-zero."
[[ "$version_output" == "sce $expected_version" ]] ||
  fail "sce version printed '$version_output'; expected 'sce $expected_version'."

printf 'pack-smoke: OK (sce --help, sce version)\n'
