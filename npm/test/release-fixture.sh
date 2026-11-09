#!/usr/bin/env bash
set -euo pipefail

FIXTURE_SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
FIXTURE_LAUNCHER_ENTRY="package/bin/sce.js"
FIXTURE_SERVER_START_TIMEOUT_SECONDS=10
FIXTURE_INSTALLED_PACKAGE_DIR="node_modules/@crocoder-dev/sce"
FIXTURE_EXPECTED_RUNTIME_MODE="755"
FIXTURE_STUB_HELP_OUTPUT="sce fixture help"
FIXTURE_ENV_FILE_NAME="fixture.env"
FIXTURE_LOG_PREFIX="${FIXTURE_LOG_PREFIX:-release-fixture}"
FIXTURE_SERVER_PID=""

fixture_log() {
  printf '%s: %s\n' "$FIXTURE_LOG_PREFIX" "$1"
}

fixture_fail() {
  printf '%s: %s\n' "$FIXTURE_LOG_PREFIX" "$1" >&2
  exit 1
}

fixture_require_tools() {
  local tool
  for tool in node npm openssl tar gzip sha256sum; do
    command -v "$tool" >/dev/null 2>&1 ||
      fixture_fail "Required tool '$tool' is not on PATH. Run via: nix shell nixpkgs#nodejs nixpkgs#openssl -c bash $0"
  done
}

fixture_assert_launcher_mode() {
  local tarball="$1"
  local launcher_mode
  launcher_mode="$(tar -tvzf "$tarball" "$FIXTURE_LAUNCHER_ENTRY" | cut -d ' ' -f 1)" ||
    fixture_fail "Packed tarball $tarball is missing $FIXTURE_LAUNCHER_ENTRY."
  [[ -n "$launcher_mode" ]] || fixture_fail "Packed tarball $tarball is missing $FIXTURE_LAUNCHER_ENTRY."
  if [[ "${launcher_mode:3:1}" != "x" ]]; then
    fixture_fail "Packed $FIXTURE_LAUNCHER_ENTRY has mode $launcher_mode; expected an executable mode (e.g. -rwxr-xr-x). Ensure npm/bin/sce.js is committed as Git mode 100755."
  fi
  fixture_log "packed $FIXTURE_LAUNCHER_ENTRY mode $launcher_mode"
}

fixture_stage_package() {
  local stage_dir="$1"
  local package_dir="$2"
  local tarball="$3"

  if [[ -n "$tarball" ]]; then
    fixture_assert_launcher_mode "$tarball"
    tar -xzf "$tarball" -C "$stage_dir" --strip-components=1 --no-same-owner
    [[ -f "$stage_dir/package.json" ]] || fixture_fail "Tarball $tarball has no package/package.json."
  else
    [[ -f "$package_dir/package.json" ]] || fixture_fail "No package.json found in $package_dir."
    cp -R "$package_dir/." "$stage_dir/"
  fi
  rm -rf "$stage_dir/runtime" "$stage_dir/node_modules"
}

fixture_resolve_platform() {
  local stage_dir="$1"
  local platform_json

  FIXTURE_VERSION="$(node -p "JSON.parse(require('fs').readFileSync(process.argv[1], 'utf8')).version" "$stage_dir/package.json")"
  platform_json="$(
    node --input-type=module -e '
      const { resolveSupportedPlatform } = await import(process.argv[1]);
      const platform = resolveSupportedPlatform();
      if (!platform) process.exit(2);
      console.log(JSON.stringify(platform));
    ' "$stage_dir/lib/platform.js"
  )" || fixture_fail "Host platform $(node -p 'process.platform + "/" + process.arch') is not supported by the npm package."
  FIXTURE_TARGET_TRIPLE="$(node -p "JSON.parse(process.argv[1]).targetTriple" "$platform_json")"
  FIXTURE_TARGET_OS="$(node -p "JSON.parse(process.argv[1]).os" "$platform_json")"
  FIXTURE_TARGET_ARCH="$(node -p "JSON.parse(process.argv[1]).arch" "$platform_json")"
}

fixture_generate_keys() {
  local stage_dir="$1"
  local keys_dir="$2"

  openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 -out "$keys_dir/signing.pem" 2>/dev/null
  openssl pkey -in "$keys_dir/signing.pem" -pubout -out "$stage_dir/lib/release-manifest-public-key.pem"

  fixture_generate_tls_cert "$keys_dir"
}

fixture_generate_tls_cert() {
  local keys_dir="$1"

  openssl req -x509 -newkey rsa:2048 -nodes -days 1 \
    -subj "/CN=127.0.0.1" \
    -addext "subjectAltName=IP:127.0.0.1" \
    -keyout "$keys_dir/tls-key.pem" -out "$keys_dir/tls-cert.pem" 2>/dev/null
}

fixture_build_release() {
  local release_dir="$1"
  local keys_dir="$2"
  local scratch_dir="$3"
  local binary="$4"
  local version="$FIXTURE_VERSION"
  local archive_root="sce-v${version}-${FIXTURE_TARGET_TRIPLE}"
  local archive_name="${archive_root}.tar.gz"
  local manifest_name="sce-v${version}-release-manifest.json"
  local archive_src="$scratch_dir/archive-src"
  local checksum

  mkdir -p "$archive_src/$archive_root/bin"
  if [[ -n "$binary" ]]; then
    fixture_log "serving real binary $binary"
    cp "$binary" "$archive_src/$archive_root/bin/sce"
  else
    cat >"$archive_src/$archive_root/bin/sce" <<EOF
#!/bin/sh
case "\${1:-}" in
  --help) echo "$FIXTURE_STUB_HELP_OUTPUT" ;;
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
  ' "$version" "$archive_name" "$checksum" "$FIXTURE_TARGET_TRIPLE" "$FIXTURE_TARGET_OS" "$FIXTURE_TARGET_ARCH" "$release_dir/$manifest_name"
  openssl dgst -sha256 -sign "$keys_dir/signing.pem" "$release_dir/$manifest_name" |
    openssl base64 -A >"$release_dir/$manifest_name.sig"
}

fixture_pack() {
  local stage_dir="$1"
  local pack_dir="$2"
  local packed_name

  packed_name="$(cd "$pack_dir" && npm pack --silent "$stage_dir")"
  FIXTURE_TARBALL="$pack_dir/$packed_name"
  fixture_assert_launcher_mode "$FIXTURE_TARBALL"
}

fixture_serve_start() {
  local release_dir="$1"
  local keys_dir="$2"
  local port_file="$3"
  local attempt

  node "$FIXTURE_SCRIPT_DIR/fixture-release-server.mjs" \
    --root "$release_dir" \
    --cert "$keys_dir/tls-cert.pem" \
    --key "$keys_dir/tls-key.pem" \
    --port-file "$port_file" &
  FIXTURE_SERVER_PID=$!

  for ((attempt = 0; attempt < FIXTURE_SERVER_START_TIMEOUT_SECONDS * 10; attempt++)); do
    [[ -s "$port_file" ]] && break
    kill -0 "$FIXTURE_SERVER_PID" 2>/dev/null || fixture_fail "Fixture release server exited before listening."
    sleep 0.1
  done
  [[ -s "$port_file" ]] || fixture_fail "Fixture release server did not start within ${FIXTURE_SERVER_START_TIMEOUT_SECONDS}s."
  # shellcheck disable=SC2034 # Consumed by sourcing callers.
  FIXTURE_PORT="$(tr -d '\n' <"$port_file")"
}

fixture_serve_stop() {
  if [[ -n "$FIXTURE_SERVER_PID" ]]; then
    kill "$FIXTURE_SERVER_PID" 2>/dev/null || true
    wait "$FIXTURE_SERVER_PID" 2>/dev/null || true
    FIXTURE_SERVER_PID=""
  fi
}

fixture_npm_install() {
  local prefix_dir="$1"
  local tarball="$2"
  local port="$3"
  local tls_cert="$4"
  local npm_state_dir="$5"

  SCE_NPM_RELEASE_BASE_URL="https://127.0.0.1:${port}" \
    NODE_EXTRA_CA_CERTS="$tls_cert" \
    npm_config_cache="$npm_state_dir/npm-cache" \
    npm_config_userconfig="$npm_state_dir/npmrc" \
    npm install \
    --prefix "$prefix_dir" \
    --offline \
    --no-audit \
    --no-fund \
    --foreground-scripts \
    "$tarball" ||
    fixture_fail "npm install of the packed tarball failed."
}

fixture_assert_install_layout() {
  local prefix_dir="$1"
  local installed_dir="$prefix_dir/$FIXTURE_INSTALLED_PACKAGE_DIR"
  local runtime_bin="$installed_dir/runtime/sce"
  local runtime_mode sce_bin_target expected_bin_target

  [[ -f "$runtime_bin" ]] || fixture_fail "Installer did not produce $runtime_bin."
  runtime_mode="$(node -p "(require('fs').statSync(process.argv[1]).mode & 0o777).toString(8)" "$runtime_bin")"
  [[ "$runtime_mode" == "$FIXTURE_EXPECTED_RUNTIME_MODE" ]] ||
    fixture_fail "Installed $runtime_bin has mode $runtime_mode; expected $FIXTURE_EXPECTED_RUNTIME_MODE."
  fixture_log "installed runtime/sce mode $runtime_mode"

  FIXTURE_SCE_BIN="$prefix_dir/node_modules/.bin/sce"
  [[ -x "$FIXTURE_SCE_BIN" ]] || fixture_fail "Installed sce bin $FIXTURE_SCE_BIN is not executable."
  sce_bin_target="$(realpath "$FIXTURE_SCE_BIN")"
  expected_bin_target="$(realpath "$installed_dir/bin/sce.js")"
  [[ "$sce_bin_target" == "$expected_bin_target" ]] ||
    fixture_fail "Installed sce bin resolves to $sce_bin_target; expected $expected_bin_target."
  fixture_log ".bin/sce resolves to $sce_bin_target"
}

fixture_usage() {
  cat <<'EOF'
Usage: release-fixture.sh prepare <dir> [--package-dir <path> | --tarball <path>]
                                        [--binary <path>]
       release-fixture.sh serve <dir> [--port-file <file>]

prepare writes a hermetic fixture release into <dir>: the repacked package
tarball (throwaway public key), release/ (archive, signed manifest), keys/
(throwaway signing key, loopback TLS cert and key), and fixture.env with the
version, target triple, and absolute paths for later consumers.

serve runs the loopback HTTPS fixture server against <dir>/release in the
foreground and writes its port to --port-file (default <dir>/server-port).
EOF
}

fixture_cmd_prepare() {
  local fixture_dir="${1:-}"
  local package_dir tarball="" binary=""

  [[ -n "$fixture_dir" ]] || fixture_fail "Missing <dir> for prepare."
  shift
  package_dir="$(cd "$FIXTURE_SCRIPT_DIR/.." && pwd)"
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --package-dir)
        [[ -n "${2:-}" ]] || fixture_fail "Missing value for --package-dir."
        package_dir="$(cd "$2" && pwd)"
        shift 2
        ;;
      --tarball)
        [[ -n "${2:-}" ]] || fixture_fail "Missing value for --tarball."
        [[ -f "$2" ]] || fixture_fail "Tarball $2 does not exist."
        tarball="$(cd "$(dirname "$2")" && pwd)/$(basename "$2")"
        shift 2
        ;;
      --binary)
        [[ -n "${2:-}" ]] || fixture_fail "Missing value for --binary."
        [[ -f "$2" ]] || fixture_fail "Binary $2 does not exist."
        binary="$(cd "$(dirname "$2")" && pwd)/$(basename "$2")"
        shift 2
        ;;
      *)
        fixture_usage >&2
        fixture_fail "Unknown argument: $1"
        ;;
    esac
  done

  fixture_require_tools
  mkdir -p "$fixture_dir"
  fixture_dir="$(cd "$fixture_dir" && pwd)"
  [[ ! -e "$fixture_dir/$FIXTURE_ENV_FILE_NAME" ]] ||
    fixture_fail "Fixture directory $fixture_dir is already prepared."

  local stage_dir="$fixture_dir/stage"
  local release_dir="$fixture_dir/release"
  local pack_dir="$fixture_dir/pack"
  local keys_dir="$fixture_dir/keys"
  mkdir -p "$stage_dir" "$release_dir" "$pack_dir" "$keys_dir"

  fixture_stage_package "$stage_dir" "$package_dir" "$tarball"
  fixture_resolve_platform "$stage_dir"
  fixture_log "version $FIXTURE_VERSION, target $FIXTURE_TARGET_TRIPLE"
  fixture_generate_keys "$stage_dir" "$keys_dir"
  fixture_build_release "$release_dir" "$keys_dir" "$fixture_dir" "$binary"
  fixture_pack "$stage_dir" "$pack_dir"

  {
    printf 'FIXTURE_VERSION=%q\n' "$FIXTURE_VERSION"
    printf 'FIXTURE_TARGET_TRIPLE=%q\n' "$FIXTURE_TARGET_TRIPLE"
    printf 'FIXTURE_TARBALL=%q\n' "$FIXTURE_TARBALL"
    printf 'FIXTURE_RELEASE_DIR=%q\n' "$release_dir"
    printf 'FIXTURE_TLS_CERT=%q\n' "$keys_dir/tls-cert.pem"
    printf 'FIXTURE_TLS_KEY=%q\n' "$keys_dir/tls-key.pem"
  } >"$fixture_dir/$FIXTURE_ENV_FILE_NAME"
  fixture_log "prepared $fixture_dir"
}

fixture_cmd_serve() {
  local fixture_dir="${1:-}"
  local port_file=""

  [[ -n "$fixture_dir" ]] || fixture_fail "Missing <dir> for serve."
  shift
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --port-file)
        [[ -n "${2:-}" ]] || fixture_fail "Missing value for --port-file."
        port_file="$2"
        shift 2
        ;;
      *)
        fixture_usage >&2
        fixture_fail "Unknown argument: $1"
        ;;
    esac
  done

  [[ -f "$fixture_dir/$FIXTURE_ENV_FILE_NAME" ]] ||
    fixture_fail "Fixture directory $fixture_dir is not prepared; run prepare first."
  command -v node >/dev/null 2>&1 || fixture_fail "Required tool 'node' is not on PATH."
  exec node "$FIXTURE_SCRIPT_DIR/fixture-release-server.mjs" \
    --root "$fixture_dir/release" \
    --cert "$fixture_dir/keys/tls-cert.pem" \
    --key "$fixture_dir/keys/tls-key.pem" \
    --port-file "${port_file:-$fixture_dir/server-port}"
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  case "${1:-}" in
    prepare)
      shift
      fixture_cmd_prepare "$@"
      ;;
    serve)
      shift
      fixture_cmd_serve "$@"
      ;;
    --help | -h)
      fixture_usage
      ;;
    *)
      fixture_usage >&2
      fixture_fail "Unknown command: ${1:-<none>}"
      ;;
  esac
fi
