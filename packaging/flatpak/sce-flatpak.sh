#!/usr/bin/env bash
set -euo pipefail

APP_ID="dev.crocoder.sce"
MANIFEST_NAME="${APP_ID}.yml"
METAINFO_NAME="${APP_ID}.metainfo.xml"
FLATHUB_REMOTE_NAME="flathub"
FLATHUB_REMOTE_URL="https://flathub.org/repo/flathub.flatpakrepo"

usage() {
  cat <<'EOF'
Usage: sce-flatpak <command> [options]

Commands:
  validate                 Run lightweight Flatpak packaging validation
  prepare-local-manifest   Generate a local-checkout Flatpak manifest
  release-package          Package Flatpak source-manifest release assets
  release-bundle           Build and bundle Flatpak release assets

validate options:
  --repo-root <path>       Repository checkout to validate (default: git root or cwd)
  --skip-optional-lint     Do not invoke flatpak-builder-lint even if available

prepare-local-manifest options:
  --repo-root <path>       Repository checkout used as the Flatpak source
  --out-dir <path>         Directory for generated manifest/support files

release-package options:
  --version <semver>       Release version to package; must match checked-in metadata
  --out-dir <path>         Directory for release tarball, checksum, and JSON metadata
  --repo-root <path>       Repository checkout to package (default: git root or cwd)

release-bundle options:
  --version <semver>       Release version to bundle; must match checked-in metadata
  --arch <arch>            Target architecture (default: host arch via uname -m)
  --out-dir <path>         Directory for bundle, checksum, and JSON metadata
  --repo-root <path>       Repository checkout to build from (default: git root or cwd)
EOF
}

die() { printf 'sce-flatpak: %s\n' "$1" >&2; exit 1; }

resolve_repo_root() {
  local override="${1:-}"
  if [ -n "${override}" ]; then
    [ -d "${override}" ] || die "--repo-root does not point to a directory: ${override}"
    (cd "${override}" && pwd -P); return
  fi
  local git_root
  git_root="$(git rev-parse --show-toplevel 2>/dev/null || true)"
  if [ -n "${git_root}" ]; then (cd "${git_root}" && pwd -P); return; fi
  if [ -f "flake.nix" ] && [ -d "packaging/flatpak" ]; then pwd -P; return; fi
  die "could not resolve repository root; run from the repo or pass --repo-root"
}

flatpak_dir_for() { printf '%s/packaging/flatpak\n' "$1"; }
require_file() { [ -f "$1" ] || die "missing required file: $1"; }
require_command() {
  command -v "$1" >/dev/null 2>&1 || die "$1 is required. $2"
}

ensure_flatpak_user_remote() {
  printf 'Ensuring Flatpak user remote %s is configured for SDK/runtime dependencies.\n' "${FLATHUB_REMOTE_NAME}"
  flatpak --user remote-add --if-not-exists --from "${FLATHUB_REMOTE_NAME}" "${FLATHUB_REMOTE_URL}"
}

substitute_placeholder() {
  # Streams a template, replacing every occurrence of $1 with literal $2.
  awk -v ph="$1" -v val="$2" \
    'BEGIN { len = length(ph) }
     {
       out = ""; line = $0
       while ((i = index(line, ph)) > 0) {
         out = out substr(line, 1, i - 1) val
         line = substr(line, i + len)
       }
       print out line
     }' "$3"
}

generate_local_manifest() {
  local repo_root="$1" out_dir="$2"
  local flatpak_dir; flatpak_dir="$(flatpak_dir_for "${repo_root}")"
  require_file "${flatpak_dir}/${METAINFO_NAME}"
  require_file "${flatpak_dir}/git-host-bridge"
  require_file "${flatpak_dir}/cargo-sources.json"
  [ -n "${SCE_FLATPAK_LOCAL_MANIFEST_TEMPLATE:-}" ] && [ -n "${SCE_FLATPAK_LOCAL_PATH_PLACEHOLDER:-}" ] \
    || die "generate_local_manifest: Nix-emitted local manifest template not available; run via 'nix run .#sce-flatpak'"
  mkdir -p "${out_dir}"
  cp "${flatpak_dir}/${METAINFO_NAME}" "${out_dir}/${METAINFO_NAME}"
  cp "${flatpak_dir}/git-host-bridge" "${out_dir}/git-host-bridge"
  cp "${flatpak_dir}/cargo-sources.json" "${out_dir}/cargo-sources.json"
  local abs_repo_root; abs_repo_root="$(cd "${repo_root}" && pwd -P)"
  substitute_placeholder "${SCE_FLATPAK_LOCAL_PATH_PLACEHOLDER}" "${abs_repo_root}" \
    "${SCE_FLATPAK_LOCAL_MANIFEST_TEMPLATE}" > "${out_dir}/${MANIFEST_NAME}"
  printf '%s/%s\n' "${out_dir}" "${MANIFEST_NAME}"
}

run_static_checks() {
  [ -n "${SCE_FLATPAK_STATIC_CHECK:-}" ] \
    || die "run_static_checks: Nix-built static validator not available; run via 'nix run .#sce-flatpak'"
  "${SCE_FLATPAK_STATIC_CHECK}" --repo-root "$1"
}

validate_generated_local_manifest() {
  [ -n "${SCE_FLATPAK_LOCAL_MANIFEST_CHECK:-}" ] \
    || die "validate_generated_local_manifest: Nix-built local-manifest validator not available; run via 'nix run .#sce-flatpak'"
  "${SCE_FLATPAK_LOCAL_MANIFEST_CHECK}" --repo-root "$1" --manifest-path "$2"
}

resolve_release_commit() {
  local repo_root="$1" release_commit
  release_commit="$(git -C "${repo_root}" rev-parse --verify "HEAD^{commit}" 2>/dev/null)" \
    || die "could not resolve release commit from repository checkout: ${repo_root}"
  [[ "${release_commit}" =~ ^[0-9a-f]{40}$ ]] \
    || die "resolved release commit is not a full 40-character git SHA: ${release_commit}"
  printf '%s\n' "${release_commit}"
}

validate_release_version_parity() {
  [ -n "${SCE_FLATPAK_VERSION_PARITY_CHECK:-}" ] \
    || die "validate_release_version_parity: Nix-built version-parity validator not available; run via 'nix run .#sce-flatpak'"
  "${SCE_FLATPAK_VERSION_PARITY_CHECK}" --repo-root "$1" --version "$2"
}

generate_release_manifest() {
  local release_commit="$1" out_dir="$2"
  [[ "${release_commit}" =~ ^[0-9a-f]{40}$ ]] \
    || die "release commit must be a full 40-character lowercase git SHA"
  [ -n "${SCE_FLATPAK_COMMIT_MANIFEST_TEMPLATE:-}" ] && [ -n "${SCE_FLATPAK_COMMIT_PLACEHOLDER:-}" ] \
    || die "generate_release_manifest: Nix-emitted commit manifest template not available; run via 'nix run .#sce-flatpak'"
  mkdir -p "${out_dir}"
  substitute_placeholder "${SCE_FLATPAK_COMMIT_PLACEHOLDER}" "${release_commit}" \
    "${SCE_FLATPAK_COMMIT_MANIFEST_TEMPLATE}" > "${out_dir}/${MANIFEST_NAME}"
  printf '%s/%s\n' "${out_dir}" "${MANIFEST_NAME}"
}

# Constrained inputs (semver, hex SHA, fixed filenames, x86_64/aarch64) — no JSON escaping needed.
emit_source_manifest_metadata() {
  local path="$1" version="$2" release_commit="$3" package_name="$4" checksum_name="$5" checksum="$6"
  cat > "${path}" <<EOF
{
  "asset_type": "flatpak-source-manifest",
  "app_id": "${APP_ID}",
  "version": "${version}",
  "release_commit": "${release_commit}",
  "manifest_name": "${MANIFEST_NAME}",
  "package_file": "${package_name}",
  "checksum_file": "${checksum_name}",
  "checksum_sha256": "${checksum}",
  "packaged_support_files": [
    "${METAINFO_NAME}",
    "cargo-sources.json",
    "git-host-bridge"
  ],
  "packaged_files": [
    "${MANIFEST_NAME}",
    "${METAINFO_NAME}",
    "cargo-sources.json",
    "git-host-bridge"
  ]
}
EOF
}

emit_bundle_metadata() {
  local path="$1" version="$2" arch="$3" bundle_name="$4" checksum_name="$5" checksum="$6"
  cat > "${path}" <<EOF
{
  "asset_type": "flatpak-bundle",
  "app_id": "${APP_ID}",
  "version": "${version}",
  "architecture": "${arch}",
  "bundle_file": "${bundle_name}",
  "checksum_file": "${checksum_name}",
  "checksum_sha256": "${checksum}"
}
EOF
}

cmd_validate() {
  local repo_root_override="" skip_optional_lint=0
  while [ $# -gt 0 ]; do
    case "$1" in
      --repo-root) repo_root_override="${2:-}"; [ -n "${repo_root_override}" ] || die "--repo-root requires a path"; shift 2 ;;
      --skip-optional-lint) skip_optional_lint=1; shift ;;
      --help|-h) usage; exit 0 ;;
      *) die "unknown validate argument: $1" ;;
    esac
  done

  local repo_root; repo_root="$(resolve_repo_root "${repo_root_override}")"
  local flatpak_dir; flatpak_dir="$(flatpak_dir_for "${repo_root}")"
  require_file "${flatpak_dir}/${MANIFEST_NAME}"
  require_file "${flatpak_dir}/${METAINFO_NAME}"

  run_static_checks "${repo_root}"

  local tmp_dir; tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/sce-flatpak-validate.XXXXXX")"
  trap '[ -n "${tmp_dir:-}" ] && rm -rf "${tmp_dir}"' EXIT

  local local_manifest; local_manifest="$(generate_local_manifest "${repo_root}" "${tmp_dir}")"
  validate_generated_local_manifest "${repo_root}" "${local_manifest}"

  require_command "appstreamcli" "Enter 'nix develop' or install appstream."
  appstreamcli validate --pedantic --no-net "${flatpak_dir}/${METAINFO_NAME}"

  if [ "${skip_optional_lint}" -eq 0 ]; then
    if command -v flatpak-builder-lint >/dev/null 2>&1; then
      flatpak-builder-lint manifest "${flatpak_dir}/${MANIFEST_NAME}"
      flatpak-builder-lint appstream "${flatpak_dir}/${METAINFO_NAME}"
    else
      printf 'flatpak-builder-lint not found; optional Flathub lint skipped.\n'
    fi
  fi

  rm -rf "${tmp_dir}"; trap - EXIT
  printf 'Flatpak validation passed for %s.\n' "${MANIFEST_NAME}"
  printf 'Generated local manifest check passed for checkout source %s.\n' "${repo_root}"
}

cmd_prepare_local_manifest() {
  local repo_root_override="" out_dir=""
  while [ $# -gt 0 ]; do
    case "$1" in
      --repo-root) repo_root_override="${2:-}"; [ -n "${repo_root_override}" ] || die "--repo-root requires a path"; shift 2 ;;
      --out-dir) out_dir="${2:-}"; [ -n "${out_dir}" ] || die "--out-dir requires a path"; shift 2 ;;
      --help|-h) usage; exit 0 ;;
      *) die "unknown prepare-local-manifest argument: $1" ;;
    esac
  done

  local repo_root; repo_root="$(resolve_repo_root "${repo_root_override}")"
  [ -n "${out_dir}" ] || out_dir="$(mktemp -d "${TMPDIR:-/tmp}/sce-flatpak-manifest.XXXXXX")"
  local local_manifest; local_manifest="$(generate_local_manifest "${repo_root}" "${out_dir}")"
  validate_generated_local_manifest "${repo_root}" "${local_manifest}"
  printf '%s\n' "${local_manifest}"
}

cmd_release_package() {
  local repo_root_override="" version="" out_dir=""
  while [ $# -gt 0 ]; do
    case "$1" in
      --repo-root) repo_root_override="${2:-}"; [ -n "${repo_root_override}" ] || die "--repo-root requires a path"; shift 2 ;;
      --version) version="${2:-}"; [ -n "${version}" ] || die "--version requires a semver value"; shift 2 ;;
      --out-dir) out_dir="${2:-}"; [ -n "${out_dir}" ] || die "--out-dir requires a path"; shift 2 ;;
      --help|-h) usage; exit 0 ;;
      *) die "unknown release-package argument: $1" ;;
    esac
  done
  [ -n "${version}" ] && [ -n "${out_dir}" ] || { usage >&2; exit 1; }

  local repo_root; repo_root="$(resolve_repo_root "${repo_root_override}")"
  local flatpak_dir; flatpak_dir="$(flatpak_dir_for "${repo_root}")"
  require_file "${flatpak_dir}/${MANIFEST_NAME}"
  require_file "${flatpak_dir}/${METAINFO_NAME}"
  require_file "${flatpak_dir}/git-host-bridge"
  require_file "${flatpak_dir}/cargo-sources.json"

  validate_release_version_parity "${repo_root}" "${version}"
  run_static_checks "${repo_root}"

  local release_commit; release_commit="$(resolve_release_commit "${repo_root}")"

  mkdir -p "${out_dir}"
  local tmp_dir; tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/sce-flatpak-release.XXXXXX")"
  trap '[ -n "${tmp_dir:-}" ] && rm -rf "${tmp_dir}"' EXIT

  local package_root="sce-v${version}-flatpak-manifest"
  local package_name="${package_root}.tar.gz"
  local checksum_name="${package_name}.sha256"
  local metadata_name="sce-v${version}-flatpak.json"
  local stage_dir="${tmp_dir}/${package_root}"
  local package_path="${out_dir}/${package_name}"
  local checksum_path="${out_dir}/${checksum_name}"
  local metadata_path="${out_dir}/${metadata_name}"

  mkdir -p "${stage_dir}"
  generate_release_manifest "${release_commit}" "${stage_dir}" >/dev/null
  cp "${flatpak_dir}/${METAINFO_NAME}" "${stage_dir}/${METAINFO_NAME}"
  cp "${flatpak_dir}/cargo-sources.json" "${stage_dir}/cargo-sources.json"
  cp "${flatpak_dir}/git-host-bridge" "${stage_dir}/git-host-bridge"
  chmod 0644 "${stage_dir}/${MANIFEST_NAME}" "${stage_dir}/${METAINFO_NAME}" "${stage_dir}/cargo-sources.json"
  chmod 0755 "${stage_dir}/git-host-bridge"

  tar --sort=name --mtime='UTC 1970-01-01' --owner=0 --group=0 --numeric-owner \
    -C "${tmp_dir}" -cf - "${package_root}" | gzip -n > "${package_path}"

  local checksum; checksum="$(sha256sum "${package_path}" | cut -d ' ' -f 1)"
  printf '%s  %s\n' "${checksum}" "${package_name}" > "${checksum_path}"

  emit_source_manifest_metadata \
    "${metadata_path}" "${version}" "${release_commit}" \
    "${package_name}" "${checksum_name}" "${checksum}"

  rm -rf "${tmp_dir}"; trap - EXIT
  printf 'Built Flatpak source-manifest release assets:\n'
  printf '  %s\n' "${package_path}" "${checksum_path}" "${metadata_path}"
}

cmd_release_bundle() {
  local repo_root_override="" version="" arch="" out_dir=""
  while [ $# -gt 0 ]; do
    case "$1" in
      --repo-root) repo_root_override="${2:-}"; [ -n "${repo_root_override}" ] || die "--repo-root requires a path"; shift 2 ;;
      --version) version="${2:-}"; [ -n "${version}" ] || die "--version requires a semver value"; shift 2 ;;
      --arch) arch="${2:-}"; [ -n "${arch}" ] || die "--arch requires an architecture value"; shift 2 ;;
      --out-dir) out_dir="${2:-}"; [ -n "${out_dir}" ] || die "--out-dir requires a path"; shift 2 ;;
      --help|-h) usage; exit 0 ;;
      *) die "unknown release-bundle argument: $1" ;;
    esac
  done
  [ -n "${version}" ] && [ -n "${out_dir}" ] || { usage >&2; exit 1; }

  local repo_root; repo_root="$(resolve_repo_root "${repo_root_override}")"
  local flatpak_dir; flatpak_dir="$(flatpak_dir_for "${repo_root}")"
  require_file "${flatpak_dir}/${MANIFEST_NAME}"
  require_file "${flatpak_dir}/${METAINFO_NAME}"
  require_file "${flatpak_dir}/git-host-bridge"
  require_file "${flatpak_dir}/cargo-sources.json"

  validate_release_version_parity "${repo_root}" "${version}"
  run_static_checks "${repo_root}"

  [ -n "${arch}" ] || arch="$(uname -m)"
  case "${arch}" in
    x86_64|aarch64) ;;
    *) die "unsupported architecture: ${arch} (supported: x86_64, aarch64)" ;;
  esac

  require_command "flatpak-builder" "Enter 'nix develop' or install flatpak-builder."
  require_command "flatpak" "Enter 'nix develop' or install flatpak."
  ensure_flatpak_user_remote

  mkdir -p "${out_dir}"
  local tmp_dir; tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/sce-flatpak-release-bundle.XXXXXX")"
  trap '[ -n "${tmp_dir:-}" ] && rm -rf "${tmp_dir}"' EXIT

  local local_manifest; local_manifest="$(generate_local_manifest "${repo_root}" "${tmp_dir}/manifest")"
  validate_generated_local_manifest "${repo_root}" "${local_manifest}"

  local build_dir="${tmp_dir}/build"
  local export_repo="${tmp_dir}/repo"
  local bundle_name="sce-v${version}-${arch}.flatpak"
  local bundle_path="${out_dir}/${bundle_name}"
  local checksum_name="${bundle_name}.sha256"
  local checksum_path="${out_dir}/${checksum_name}"
  local metadata_name="sce-v${version}-${arch}.json"
  local metadata_path="${out_dir}/${metadata_name}"

  printf 'Building %s for %s from local checkout source: %s\n' "${APP_ID}" "${arch}" "${repo_root}"
  flatpak-builder --force-clean --user \
    --install-deps-from="${FLATHUB_REMOTE_NAME}" \
    --repo="${export_repo}" --arch="${arch}" \
    "${build_dir}" "${local_manifest}"

  printf 'Creating Flatpak bundle: %s\n' "${bundle_path}"
  flatpak build-bundle --arch="${arch}" "${export_repo}" "${bundle_path}" "${APP_ID}"

  local checksum; checksum="$(sha256sum "${bundle_path}" | cut -d ' ' -f 1)"
  printf '%s  %s\n' "${checksum}" "${bundle_name}" > "${checksum_path}"

  emit_bundle_metadata \
    "${metadata_path}" "${version}" "${arch}" \
    "${bundle_name}" "${checksum_name}" "${checksum}"

  rm -rf "${tmp_dir}"; trap - EXIT
  printf 'Built Flatpak bundle release assets:\n'
  printf '  %s\n' "${bundle_path}" "${checksum_path}" "${metadata_path}"
}

main() {
  local command="${1:-}"
  [ -n "${command}" ] || { usage; exit 1; }
  shift
  case "${command}" in
    validate) cmd_validate "$@" ;;
    prepare-local-manifest) cmd_prepare_local_manifest "$@" ;;
    release-package) cmd_release_package "$@" ;;
    release-bundle) cmd_release_bundle "$@" ;;
    --help|-h|help) usage ;;
    *) die "unknown command: ${command}" ;;
  esac
}

main "$@"
