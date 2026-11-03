#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PACKAGE_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
LAUNCHER_ENTRY="package/bin/sce.js"
SERVER_START_TIMEOUT_SECONDS=10
INSTALLED_PACKAGE_DIR="node_modules/@crocoder-dev/sce"
EXPECTED_RUNTIME_MODE="755"
EXPECTED_HELP_OUTPUT="sce fixture help"
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
EOF
}

fail() {
  printf 'pack-smoke: %s\n' "$1" >&2
  exit 1
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

for tool in node npm openssl tar gzip sha256sum; do
  command -v "$tool" >/dev/null 2>&1 ||
    fail "Required tool '$tool' is not on PATH. Run via: nix shell nixpkgs#nodejs nixpkgs#openssl -c bash $0"
done

if [[ -z "$SOURCE_TARBALL" ]]; then
  [[ -f "$PACKAGE_DIR/package.json" ]] || fail "No package.json found in $PACKAGE_DIR."
fi

work_dir="$(mktemp -d "${TMPDIR:-/tmp}/sce-npm-pack-smoke.XXXXXX")"
server_pid=""
cleanup() {
  if [[ -n "$server_pid" ]]; then
    kill "$server_pid" 2>/dev/null || true
    wait "$server_pid" 2>/dev/null || true
  fi
  rm -rf "$work_dir"
}
trap cleanup EXIT

stage_dir="$work_dir/stage"
release_dir="$work_dir/release"
pack_dir="$work_dir/pack"
prefix_dir="${PREFIX_DIR:-$work_dir/prefix}"
keys_dir="$work_dir/keys"
mkdir -p "$stage_dir" "$release_dir" "$pack_dir" "$prefix_dir" "$keys_dir"

assert_launcher_mode() {
  local tarball="$1"
  local launcher_mode
  launcher_mode="$(tar -tvzf "$tarball" "$LAUNCHER_ENTRY" | cut -d ' ' -f 1)" ||
    fail "Packed tarball $tarball is missing $LAUNCHER_ENTRY."
  [[ -n "$launcher_mode" ]] || fail "Packed tarball $tarball is missing $LAUNCHER_ENTRY."
  if [[ "${launcher_mode:3:1}" != "x" ]]; then
    fail "Packed $LAUNCHER_ENTRY has mode $launcher_mode; expected an executable mode (e.g. -rwxr-xr-x). Ensure npm/bin/sce.js is committed as Git mode 100755."
  fi
  printf 'pack-smoke: packed %s mode %s\n' "$LAUNCHER_ENTRY" "$launcher_mode"
}

if [[ -n "$SOURCE_TARBALL" ]]; then
  assert_launcher_mode "$SOURCE_TARBALL"
  tar -xzf "$SOURCE_TARBALL" -C "$stage_dir" --strip-components=1 --no-same-owner
  [[ -f "$stage_dir/package.json" ]] || fail "Tarball $SOURCE_TARBALL has no package/package.json."
else
  cp -R "$PACKAGE_DIR/." "$stage_dir/"
fi
rm -rf "$stage_dir/runtime" "$stage_dir/node_modules"

version="$(node -p "JSON.parse(require('fs').readFileSync(process.argv[1], 'utf8')).version" "$stage_dir/package.json")"
platform_json="$(
  node --input-type=module -e '
    const { resolveSupportedPlatform } = await import(process.argv[1]);
    const platform = resolveSupportedPlatform();
    if (!platform) process.exit(2);
    console.log(JSON.stringify(platform));
  ' "$stage_dir/lib/platform.js"
)" || fail "Host platform $(node -p 'process.platform + "/" + process.arch') is not supported by the npm package."
target_triple="$(node -p "JSON.parse(process.argv[1]).targetTriple" "$platform_json")"
target_os="$(node -p "JSON.parse(process.argv[1]).os" "$platform_json")"
target_arch="$(node -p "JSON.parse(process.argv[1]).arch" "$platform_json")"

expected_version="${EXPECTED_VERSION:-$version}"

printf 'pack-smoke: version %s, target %s\n' "$version" "$target_triple"

openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 -out "$keys_dir/signing.pem" 2>/dev/null
openssl pkey -in "$keys_dir/signing.pem" -pubout -out "$stage_dir/lib/release-manifest-public-key.pem"

openssl req -x509 -newkey rsa:2048 -nodes -days 1 \
  -subj "/CN=127.0.0.1" \
  -addext "subjectAltName=IP:127.0.0.1" \
  -keyout "$keys_dir/tls-key.pem" -out "$keys_dir/tls-cert.pem" 2>/dev/null

archive_root="sce-v${version}-${target_triple}"
archive_name="${archive_root}.tar.gz"
manifest_name="sce-v${version}-release-manifest.json"
archive_src="$work_dir/archive-src"
mkdir -p "$archive_src/$archive_root/bin"
if [[ -n "$SOURCE_BINARY" ]]; then
  printf 'pack-smoke: serving real binary %s\n' "$SOURCE_BINARY"
  cp "$SOURCE_BINARY" "$archive_src/$archive_root/bin/sce"
else
  cat >"$archive_src/$archive_root/bin/sce" <<EOF
#!/bin/sh
case "\${1:-}" in
  --help) echo "$EXPECTED_HELP_OUTPUT" ;;
  version) echo "sce $version" ;;
  *) echo "unexpected arguments: \$*" >&2; exit 64 ;;
esac
EOF
fi
chmod 0755 "$archive_src/$archive_root/bin/sce"
printf 'fixture license\n' >"$archive_src/$archive_root/LICENSE"
printf 'fixture readme\n' >"$archive_src/$archive_root/README.md"
tar -C "$archive_src" --owner=0 --group=0 --numeric-owner -czf "$release_dir/$archive_name" "$archive_root"

checksum="$(sha256sum "$release_dir/$archive_name" | cut -d ' ' -f 1)"
# shellcheck disable=SC2016 # JavaScript template literal, not a shell expansion.
node -e '
  const [version, archive, checksum, target_triple, os, arch, output] = process.argv.slice(1);
  const manifest = {
    version,
    binary: "sce",
    artifacts: [
      {
        version,
        binary: "sce",
        archive,
        checksum_file: `${archive}.sha256`,
        checksum_sha256: checksum,
        target_triple,
        os,
        arch,
      },
    ],
  };
  require("fs").writeFileSync(output, `${JSON.stringify(manifest, null, 2)}\n`);
' "$version" "$archive_name" "$checksum" "$target_triple" "$target_os" "$target_arch" "$release_dir/$manifest_name"
openssl dgst -sha256 -sign "$keys_dir/signing.pem" "$release_dir/$manifest_name" |
  openssl base64 -A >"$release_dir/$manifest_name.sig"

packed_name="$(cd "$pack_dir" && npm pack --silent "$stage_dir")"
tarball="$pack_dir/$packed_name"
assert_launcher_mode "$tarball"

port_file="$work_dir/server-port"
node "$SCRIPT_DIR/fixture-release-server.mjs" \
  --root "$release_dir" \
  --cert "$keys_dir/tls-cert.pem" \
  --key "$keys_dir/tls-key.pem" \
  --port-file "$port_file" &
server_pid=$!

for ((attempt = 0; attempt < SERVER_START_TIMEOUT_SECONDS * 10; attempt++)); do
  [[ -s "$port_file" ]] && break
  kill -0 "$server_pid" 2>/dev/null || fail "Fixture release server exited before listening."
  sleep 0.1
done
[[ -s "$port_file" ]] || fail "Fixture release server did not start within ${SERVER_START_TIMEOUT_SECONDS}s."
port="$(tr -d '\n' <"$port_file")"

SCE_NPM_RELEASE_BASE_URL="https://127.0.0.1:${port}" \
  NODE_EXTRA_CA_CERTS="$keys_dir/tls-cert.pem" \
  npm_config_cache="$work_dir/npm-cache" \
  npm_config_userconfig="$work_dir/npmrc" \
  npm install \
  --prefix "$prefix_dir" \
  --offline \
  --no-audit \
  --no-fund \
  --foreground-scripts \
  "$tarball" ||
  fail "npm install of the packed tarball failed."

installed_dir="$prefix_dir/$INSTALLED_PACKAGE_DIR"
runtime_bin="$installed_dir/runtime/sce"
[[ -f "$runtime_bin" ]] || fail "Installer did not produce $runtime_bin."
runtime_mode="$(node -p "(require('fs').statSync(process.argv[1]).mode & 0o777).toString(8)" "$runtime_bin")"
[[ "$runtime_mode" == "$EXPECTED_RUNTIME_MODE" ]] ||
  fail "Installed $runtime_bin has mode $runtime_mode; expected $EXPECTED_RUNTIME_MODE."
printf 'pack-smoke: installed runtime/sce mode %s\n' "$runtime_mode"

sce_bin="$prefix_dir/node_modules/.bin/sce"
[[ -x "$sce_bin" ]] || fail "Installed sce bin $sce_bin is not executable."
sce_bin_target="$(realpath "$sce_bin")"
expected_bin_target="$(realpath "$installed_dir/bin/sce.js")"
[[ "$sce_bin_target" == "$expected_bin_target" ]] ||
  fail "Installed sce bin resolves to $sce_bin_target; expected $expected_bin_target."
printf 'pack-smoke: .bin/sce resolves to %s\n' "$sce_bin_target"

sce_cmd=("$sce_bin")
if [[ ! -x /usr/bin/env ]]; then
  printf 'pack-smoke: /usr/bin/env unavailable; running installed bin through node\n'
  sce_cmd=(node "$sce_bin")
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

[[ "$help_output" == "$EXPECTED_HELP_OUTPUT" ]] ||
  fail "sce --help printed '$help_output'; expected '$EXPECTED_HELP_OUTPUT'."

version_output="$("${sce_cmd[@]}" version)" || fail "sce version exited non-zero."
[[ "$version_output" == "sce $expected_version" ]] ||
  fail "sce version printed '$version_output'; expected 'sce $expected_version'."

printf 'pack-smoke: OK (sce --help, sce version)\n'
