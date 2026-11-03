#!/usr/bin/env bash
set -euo pipefail

# Packed-package smoke test for the @crocoder-dev/sce npm launcher.
#
# Packs a staged copy of the npm package, serves a locally generated fixture
# release (stub `sce` binary, manifest signed with a throwaway key swapped into
# the staged copy) over loopback HTTPS, installs the tarball into an isolated
# prefix, and runs the installed `sce` bin. No network access is required.
#
# Requires: bash, coreutils, tar, gzip, node/npm (>=18), openssl.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PACKAGE_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
LAUNCHER_ENTRY="package/bin/sce.js"
SERVER_START_TIMEOUT_SECONDS=10
EXPECTED_HELP_OUTPUT="sce fixture help"

usage() {
  cat <<'EOF'
Usage: pack-smoke.sh [--package-dir <path>]

Packs the npm launcher package, installs it into an isolated prefix against a
loopback fixture release, and verifies `sce --help` and `sce version`.
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

[[ -f "$PACKAGE_DIR/package.json" ]] || fail "No package.json found in $PACKAGE_DIR."

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
prefix_dir="$work_dir/prefix"
keys_dir="$work_dir/keys"
mkdir -p "$stage_dir" "$release_dir" "$pack_dir" "$prefix_dir" "$keys_dir"

cp -R "$PACKAGE_DIR/." "$stage_dir/"
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

printf 'pack-smoke: version %s, target %s\n' "$version" "$target_triple"

# Throwaway manifest signing key; only the staged copy trusts it.
openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 -out "$keys_dir/signing.pem" 2>/dev/null
openssl pkey -in "$keys_dir/signing.pem" -pubout -out "$stage_dir/lib/release-manifest-public-key.pem"

# Loopback TLS certificate trusted through NODE_EXTRA_CA_CERTS.
openssl req -x509 -newkey rsa:2048 -nodes -days 1 \
  -subj "/CN=127.0.0.1" \
  -addext "subjectAltName=IP:127.0.0.1" \
  -keyout "$keys_dir/tls-key.pem" -out "$keys_dir/tls-cert.pem" 2>/dev/null

# Fixture release in the canonical archive layout.
archive_root="sce-v${version}-${target_triple}"
archive_name="${archive_root}.tar.gz"
manifest_name="sce-v${version}-release-manifest.json"
archive_src="$work_dir/archive-src"
mkdir -p "$archive_src/$archive_root/bin"
cat >"$archive_src/$archive_root/bin/sce" <<EOF
#!/bin/sh
case "\${1:-}" in
  --help) echo "$EXPECTED_HELP_OUTPUT" ;;
  version) echo "sce $version" ;;
  *) echo "unexpected arguments: \$*" >&2; exit 64 ;;
esac
EOF
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

# Pack and verify the launcher entry mode inside the tarball.
packed_name="$(cd "$pack_dir" && npm pack --silent "$stage_dir")"
tarball="$pack_dir/$packed_name"
launcher_mode="$(tar -tvzf "$tarball" "$LAUNCHER_ENTRY" | cut -d ' ' -f 1)"
[[ -n "$launcher_mode" ]] || fail "Packed tarball is missing $LAUNCHER_ENTRY."
if [[ "${launcher_mode:3:1}" != "x" ]]; then
  fail "Packed $LAUNCHER_ENTRY has mode $launcher_mode; expected an executable mode (e.g. -rwxr-xr-x). Ensure npm/bin/sce.js is committed as Git mode 100755."
fi
printf 'pack-smoke: packed %s mode %s\n' "$LAUNCHER_ENTRY" "$launcher_mode"

# Serve the fixture release over loopback HTTPS.
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

# Install into an isolated prefix with an isolated npm cache/config.
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

sce_bin="$prefix_dir/node_modules/.bin/sce"
[[ -x "$sce_bin" ]] || fail "Installed sce bin $sce_bin is not executable."

help_output="$("$sce_bin" --help)" || fail "sce --help exited non-zero."
[[ "$help_output" == "$EXPECTED_HELP_OUTPUT" ]] ||
  fail "sce --help printed '$help_output'; expected '$EXPECTED_HELP_OUTPUT'."

version_output="$("$sce_bin" version)" || fail "sce version exited non-zero."
[[ "$version_output" == "sce $version" ]] ||
  fail "sce version printed '$version_output'; expected 'sce $version'."

printf 'pack-smoke: OK (sce --help, sce version)\n'
