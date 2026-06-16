#!/usr/bin/env bash
set -euo pipefail

APP_ID="dev.crocoder.sce"
MANIFEST_NAME="${APP_ID}.yml"
METAINFO_NAME="${APP_ID}.metainfo.xml"

usage() {
  cat <<'EOF'
Usage: sce-flatpak <command> [options]

Commands:
  validate                 Run lightweight Flatpak packaging validation
  prepare-local-manifest   Generate a local-checkout Flatpak manifest
  build                    Build the Flatpak from the local checkout

validate options:
  --repo-root <path>       Repository checkout to validate (default: git root or cwd)
  --skip-optional-lint     Do not invoke flatpak-builder-lint even if available

prepare-local-manifest options:
  --repo-root <path>       Repository checkout used as the Flatpak source
  --out-dir <path>         Directory for generated manifest/support files

build options:
  --repo-root <path>       Repository checkout used as the Flatpak source
  --build-dir <path>       flatpak-builder build directory
                           (default: ${TMPDIR:-/tmp}/sce-flatpak-build/dev.crocoder.sce)
  --manifest-out <path>    Directory for generated local manifest/support files
  --install                Forward --install to flatpak-builder
  --user                   Forward --user to flatpak-builder
  --install-deps-from <r>  Forward --install-deps-from=<r> to flatpak-builder
  --no-force-clean         Do not pass --force-clean to flatpak-builder
  -- <args...>             Extra arguments forwarded to flatpak-builder

The generated local manifest replaces the checked-in release git source with a
Flatpak type: dir source pointed at the checkout. It still runs the manifest's
Cargo source build inside Flatpak and does not consume a Nix-built sce binary.
EOF
}

die() {
  printf 'sce-flatpak: %s\n' "$1" >&2
  exit 1
}

resolve_repo_root() {
  local override="${1:-}"

  if [ -n "${override}" ]; then
    [ -d "${override}" ] || die "--repo-root does not point to a directory: ${override}"
    (cd "${override}" && pwd -P)
    return
  fi

  local git_root
  git_root="$(git rev-parse --show-toplevel 2>/dev/null || true)"
  if [ -n "${git_root}" ]; then
    (cd "${git_root}" && pwd -P)
    return
  fi

  if [ -f "flake.nix" ] && [ -d "packaging/flatpak" ]; then
    pwd -P
    return
  fi

  die "could not resolve repository root; run from the repo or pass --repo-root"
}

flatpak_dir_for() {
  local repo_root="$1"
  printf '%s/packaging/flatpak\n' "${repo_root}"
}

require_file() {
  local path="$1"
  [ -f "${path}" ] || die "missing required file: ${path}"
}

require_command() {
  local name="$1"
  local guidance="$2"

  if ! command -v "${name}" >/dev/null 2>&1; then
    die "${name} is required. ${guidance}"
  fi
}

generate_local_manifest() {
  local repo_root="$1"
  local out_dir="$2"
  local flatpak_dir
  flatpak_dir="$(flatpak_dir_for "${repo_root}")"

  require_file "${flatpak_dir}/${MANIFEST_NAME}"
  require_file "${flatpak_dir}/${METAINFO_NAME}"
  require_file "${flatpak_dir}/git-host-bridge"
  require_file "${flatpak_dir}/cargo-sources.json"

  mkdir -p "${out_dir}"
  cp "${flatpak_dir}/${METAINFO_NAME}" "${out_dir}/${METAINFO_NAME}"
  cp "${flatpak_dir}/git-host-bridge" "${out_dir}/git-host-bridge"
  cp "${flatpak_dir}/cargo-sources.json" "${out_dir}/cargo-sources.json"

  python3 - "${repo_root}" "${flatpak_dir}/${MANIFEST_NAME}" "${out_dir}/${MANIFEST_NAME}" <<'PY'
import json
import pathlib
import re
import sys

repo_root = pathlib.Path(sys.argv[1]).resolve()
source_manifest = pathlib.Path(sys.argv[2])
target_manifest = pathlib.Path(sys.argv[3])

text = source_manifest.read_text(encoding="utf-8")
release_source = re.compile(
    r"(?m)^      - type: git\n"
    r"        url: https://github\.com/crocoder-dev/shared-context-engineering\.git\n"
    r"        commit: [0-9a-f]{40}\n"
)
local_source = f"      - type: dir\n        path: {json.dumps(str(repo_root))}\n"
text, count = release_source.subn(local_source, text, count=1)
if count != 1:
    raise SystemExit("could not replace release git source with local dir source")

target_manifest.write_text(text, encoding="utf-8")
PY

  printf '%s/%s\n' "${out_dir}" "${MANIFEST_NAME}"
}

run_static_checks() {
  local repo_root="$1"
  local flatpak_dir
  flatpak_dir="$(flatpak_dir_for "${repo_root}")"

  python3 - "${repo_root}" <<'PY'
import json
import pathlib
import re
import sys
import xml.etree.ElementTree as ET

APP_ID = "dev.crocoder.sce"
repo_root = pathlib.Path(sys.argv[1])
flatpak_dir = repo_root / "packaging" / "flatpak"
manifest_path = flatpak_dir / f"{APP_ID}.yml"
metainfo_path = flatpak_dir / f"{APP_ID}.metainfo.xml"
cargo_sources_path = flatpak_dir / "cargo-sources.json"

errors = []

def require(condition, message):
    if not condition:
        errors.append(message)

manifest = manifest_path.read_text(encoding="utf-8")
require("id: dev.crocoder.sce" in manifest, "manifest app ID is not dev.crocoder.sce")
require("command: sce" in manifest, "manifest command is not sce")
require("org.freedesktop.Sdk.Extension.rust-stable" in manifest, "Rust SDK extension is missing")
require("bash ./scripts/prepare-cli-generated-assets.sh \"$PWD\"" in manifest, "generated-asset preparation command is missing")
require("cargo --offline build --locked --release --manifest-path cli/Cargo.toml --bin sce" in manifest, "offline locked Cargo source-build command is missing")
require("install -Dm755 cli/target/release/sce /app/bin/sce" in manifest, "sce install command is missing")
require("install -Dm755 packaging/flatpak/git-host-bridge /app/bin/git" in manifest, "host git bridge install command is missing")
require("--talk-name=org.freedesktop.Flatpak" in manifest, "host Flatpak permission is missing")
require("cargo-sources.json" in manifest, "Cargo source descriptor is missing from manifest")

release_source = re.compile(
    r"(?m)^      - type: git\n"
    r"        url: https://github\.com/crocoder-dev/shared-context-engineering\.git\n"
    r"        commit: [0-9a-f]{40}\n"
)
require(release_source.search(manifest) is not None, "release manifest does not use a pinned git source")

banned_snippets = [
    "nix build .#sce",
    "nix build .#default",
    "github.com/crocoder-dev/shared-context-engineering/releases",
    "release-artifacts",
    "@crocoder-dev/sce",
]
for path in sorted(flatpak_dir.iterdir()):
    if not path.is_file():
        continue
    if path.name == "sce-flatpak.sh":
        continue
    text = path.read_text(encoding="utf-8")
    for snippet in banned_snippets:
        require(snippet not in text, f"{path.relative_to(repo_root)} references disallowed artifact source: {snippet}")

cargo_sources = json.loads(cargo_sources_path.read_text(encoding="utf-8"))
require(isinstance(cargo_sources, list) and cargo_sources, "cargo-sources.json is empty or not a list")
require(any(entry.get("type") == "git" and entry.get("url") == "https://github.com/tursodatabase/turso" for entry in cargo_sources if isinstance(entry, dict)), "Turso git source entry is missing")
require(any(entry.get("type") == "archive" and "static.crates.io" in entry.get("url", "") for entry in cargo_sources if isinstance(entry, dict)), "crates.io archive source entries are missing")

root = ET.parse(metainfo_path).getroot()
require(root.findtext("id") == APP_ID, "metainfo ID is not dev.crocoder.sce")
provides = root.find("provides")
require(provides is not None and any(child.tag == "binary" and child.text == "sce" for child in list(provides)), "metainfo does not provide binary sce")

if errors:
    for error in errors:
        print(f"Flatpak static validation failed: {error}", file=sys.stderr)
    raise SystemExit(1)
PY
}

validate_generated_local_manifest() {
  local repo_root="$1"
  local local_manifest="$2"

  python3 - "${repo_root}" "${local_manifest}" <<'PY'
import json
import pathlib
import sys

repo_root = pathlib.Path(sys.argv[1]).resolve()
manifest_path = pathlib.Path(sys.argv[2])
manifest = manifest_path.read_text(encoding="utf-8")
expected_path = f"        path: {json.dumps(str(repo_root))}"

errors = []
if "      - type: dir\n" not in manifest:
    errors.append("local manifest does not contain a Flatpak type: dir source")
if expected_path not in manifest:
    errors.append("local manifest does not point at the requested checkout path")
if "nix build .#sce" in manifest or "nix build .#default" in manifest:
    errors.append("local manifest references a Nix-built sce binary")
if "cargo --offline build --locked --release --manifest-path cli/Cargo.toml --bin sce" not in manifest:
    errors.append("local manifest no longer runs the Flatpak Cargo source build")

if errors:
    for error in errors:
        print(f"Flatpak local-manifest validation failed: {error}", file=sys.stderr)
    raise SystemExit(1)
PY
}

cmd_validate() {
  local repo_root_override=""
  local skip_optional_lint=0

  while [ $# -gt 0 ]; do
    case "$1" in
      --repo-root)
        repo_root_override="${2:-}"
        [ -n "${repo_root_override}" ] || die "--repo-root requires a path"
        shift 2
        ;;
      --skip-optional-lint)
        skip_optional_lint=1
        shift
        ;;
      --help|-h)
        usage
        exit 0
        ;;
      *)
        die "unknown validate argument: $1"
        ;;
    esac
  done

  local repo_root
  repo_root="$(resolve_repo_root "${repo_root_override}")"
  local flatpak_dir
  flatpak_dir="$(flatpak_dir_for "${repo_root}")"

  require_file "${flatpak_dir}/${MANIFEST_NAME}"
  require_file "${flatpak_dir}/${METAINFO_NAME}"

  run_static_checks "${repo_root}"

  local tmp_dir
  tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/sce-flatpak-validate.XXXXXX")"
  cleanup() {
    if [ -n "${tmp_dir:-}" ]; then
      rm -rf "${tmp_dir}"
    fi
  }
  trap cleanup EXIT

  local local_manifest
  local_manifest="$(generate_local_manifest "${repo_root}" "${tmp_dir}")"
  validate_generated_local_manifest "${repo_root}" "${local_manifest}"

  require_command "appstreamcli" "Use 'nix run .#flatpak-validate' or enter 'nix develop'."
  appstreamcli validate --pedantic --no-net "${flatpak_dir}/${METAINFO_NAME}"

  if [ "${skip_optional_lint}" -eq 0 ]; then
    if command -v flatpak-builder-lint >/dev/null 2>&1; then
      flatpak-builder-lint manifest "${flatpak_dir}/${MANIFEST_NAME}"
      flatpak-builder-lint appstream "${flatpak_dir}/${METAINFO_NAME}"
    else
      printf 'flatpak-builder-lint not found; optional Flathub lint skipped.\n'
    fi
  fi

  rm -rf "${tmp_dir}"
  trap - EXIT

  printf 'Flatpak validation passed for %s.\n' "${MANIFEST_NAME}"
  printf 'Generated local manifest check passed for checkout source %s.\n' "${repo_root}"
}

cmd_prepare_local_manifest() {
  local repo_root_override=""
  local out_dir=""

  while [ $# -gt 0 ]; do
    case "$1" in
      --repo-root)
        repo_root_override="${2:-}"
        [ -n "${repo_root_override}" ] || die "--repo-root requires a path"
        shift 2
        ;;
      --out-dir)
        out_dir="${2:-}"
        [ -n "${out_dir}" ] || die "--out-dir requires a path"
        shift 2
        ;;
      --help|-h)
        usage
        exit 0
        ;;
      *)
        die "unknown prepare-local-manifest argument: $1"
        ;;
    esac
  done

  local repo_root
  repo_root="$(resolve_repo_root "${repo_root_override}")"
  if [ -z "${out_dir}" ]; then
    out_dir="$(mktemp -d "${TMPDIR:-/tmp}/sce-flatpak-manifest.XXXXXX")"
  fi

  local local_manifest
  local_manifest="$(generate_local_manifest "${repo_root}" "${out_dir}")"
  validate_generated_local_manifest "${repo_root}" "${local_manifest}"
  printf '%s\n' "${local_manifest}"
}

cmd_build() {
  local repo_root_override=""
  local build_dir="${TMPDIR:-/tmp}/sce-flatpak-build/${APP_ID}"
  local manifest_out=""
  local force_clean=1
  local install=0
  local user=0
  local install_deps_from=""
  local extra_args=()

  while [ $# -gt 0 ]; do
    case "$1" in
      --repo-root)
        repo_root_override="${2:-}"
        [ -n "${repo_root_override}" ] || die "--repo-root requires a path"
        shift 2
        ;;
      --build-dir)
        build_dir="${2:-}"
        [ -n "${build_dir}" ] || die "--build-dir requires a path"
        shift 2
        ;;
      --manifest-out)
        manifest_out="${2:-}"
        [ -n "${manifest_out}" ] || die "--manifest-out requires a path"
        shift 2
        ;;
      --install)
        install=1
        shift
        ;;
      --user)
        user=1
        shift
        ;;
      --install-deps-from)
        install_deps_from="${2:-}"
        [ -n "${install_deps_from}" ] || die "--install-deps-from requires a remote name"
        shift 2
        ;;
      --no-force-clean)
        force_clean=0
        shift
        ;;
      --help|-h)
        usage
        exit 0
        ;;
      --)
        shift
        extra_args+=("$@")
        break
        ;;
      *)
        die "unknown build argument: $1"
        ;;
    esac
  done

  require_command "flatpak-builder" "Use 'nix run .#flatpak-build' or enter 'nix develop'."

  local repo_root
  repo_root="$(resolve_repo_root "${repo_root_override}")"
  if [ -z "${manifest_out}" ]; then
    manifest_out="$(mktemp -d "${TMPDIR:-/tmp}/sce-flatpak-manifest.XXXXXX")"
  fi

  local local_manifest
  local_manifest="$(generate_local_manifest "${repo_root}" "${manifest_out}")"
  validate_generated_local_manifest "${repo_root}" "${local_manifest}"

  local builder_args=()
  if [ "${force_clean}" -eq 1 ]; then
    builder_args+=(--force-clean)
  fi
  if [ "${install}" -eq 1 ]; then
    builder_args+=(--install)
  fi
  if [ "${user}" -eq 1 ]; then
    builder_args+=(--user)
  fi
  if [ -n "${install_deps_from}" ]; then
    builder_args+=("--install-deps-from=${install_deps_from}")
  fi
  builder_args+=("${extra_args[@]}")
  builder_args+=("${build_dir}" "${local_manifest}")

  printf 'Building %s from local checkout source: %s\n' "${APP_ID}" "${repo_root}"
  printf 'Generated local manifest: %s\n' "${local_manifest}"
  printf 'Build directory: %s\n' "${build_dir}"

  exec flatpak-builder "${builder_args[@]}"
}

main() {
  local command="${1:-}"
  if [ -z "${command}" ]; then
    usage
    exit 1
  fi
  shift

  case "${command}" in
    validate)
      cmd_validate "$@"
      ;;
    prepare-local-manifest)
      cmd_prepare_local_manifest "$@"
      ;;
    build)
      cmd_build "$@"
      ;;
    --help|-h|help)
      usage
      ;;
    *)
      die "unknown command: ${command}"
      ;;
  esac
}

main "$@"
