#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: nix run .#sync-opencode-config [-- --help]

Deterministic flake entrypoint for opencode config sync workflow.

Current scope:
- Regenerate generated config outputs in a staging workspace.
- Replace repository config/ only after successful staged regeneration.
- Replace repository-root .opencode/ from staged config/.opencode/.
- Replace repository-root .mcp.json from staged config/.mcp.json when present.
- Exclude runtime artifacts during root sync (for example node_modules/).
EOF
}

case "${1:-}" in
  -h|--help)
    usage
    exit 0
    ;;
esac

repo_root="$(git rev-parse --show-toplevel 2>/dev/null || true)"
if [ -z "${repo_root}" ]; then
  repo_root="$(pwd)"
fi

live_config="${repo_root}/config"
live_opencode="${repo_root}/.opencode"
live_project_mcp="${repo_root}/.mcp.json"
generator_path="${live_config}/pkl/generate.pkl"

if [ ! -d "${live_config}" ]; then
  cat >&2 <<EOF
Could not locate live config directory at:
  ${live_config}
EOF
  exit 1
fi

if [ ! -f "${generator_path}" ]; then
  cat >&2 <<EOF
Could not locate config generator at:
  ${generator_path}
Run this command from the repository (or inside a git worktree rooted there).
EOF
  exit 1
fi

stage_root="$(mktemp -d "${TMPDIR:-/tmp}/sync-opencode-config.XXXXXX")"
stage_config="${stage_root}/config"
stage_opencode="${stage_config}/.opencode"
stage_project_mcp="${stage_config}/.mcp.json"
stage_generator_path="${stage_config}/pkl/generate.pkl"
backup_config="${repo_root}/.config-pre-sync-backup"
backup_opencode="${repo_root}/.opencode-pre-sync-backup"
backup_project_mcp="${repo_root}/.mcp.json-pre-sync-backup"
config_swap_complete=0
opencode_swap_complete=0
project_mcp_swap_complete=0
project_mcp_preexisting=0

exclude_runtime_artifacts=(
  node_modules
)

cleanup() {
  if [ "${project_mcp_swap_complete}" -ne 1 ]; then
    if [ -f "${backup_project_mcp}" ]; then
      rm -f "${live_project_mcp}"
      mv "${backup_project_mcp}" "${live_project_mcp}"
    elif [ "${project_mcp_preexisting}" -ne 1 ]; then
      rm -f "${live_project_mcp}"
    fi
  fi
  if [ "${opencode_swap_complete}" -ne 1 ] && [ -d "${backup_opencode}" ]; then
    rm -rf "${live_opencode}"
    mv "${backup_opencode}" "${live_opencode}"
  fi
  if [ "${config_swap_complete}" -ne 1 ] && [ -d "${backup_config}" ]; then
    rm -rf "${live_config}"
    mv "${backup_config}" "${live_config}"
  fi
  rm -rf "${stage_root}" "${backup_config}" "${backup_opencode}"
  rm -f "${backup_project_mcp}"
}
trap cleanup EXIT

echo "==> Preparing staged config workspace"
cp -R "${live_config}" "${stage_config}"

echo "==> Regenerating generated-owned config outputs in staging"
pkl eval -m "${stage_root}" "${stage_generator_path}"

if [ ! -d "${stage_config}/.opencode" ] || [ ! -d "${stage_config}/.claude" ]; then
  cat >&2 <<EOF
Staged regeneration is incomplete; refusing to replace live config/.
Missing expected generated directories under:
  ${stage_config}
EOF
  exit 1
fi

if [ ! -d "${stage_opencode}" ]; then
  cat >&2 <<EOF
Staged regeneration is missing config/.opencode; refusing to replace root .opencode/.
Missing directory:
  ${stage_opencode}
EOF
  exit 1
fi

if [ ! -f "${stage_project_mcp}" ]; then
  cat >&2 <<EOF
Staged regeneration is missing config/.mcp.json; refusing to replace root .mcp.json.
Missing file:
  ${stage_project_mcp}
EOF
  exit 1
fi

if [ -e "${backup_config}" ]; then
  rm -rf "${backup_config}"
fi

echo "==> Replacing live config/ from staged output"
mv "${live_config}" "${backup_config}"
cp -R "${stage_config}" "${live_config}"
rm -rf "${backup_config}"
config_swap_complete=1

if [ -e "${backup_opencode}" ]; then
  rm -rf "${backup_opencode}"
fi

if [ -e "${backup_project_mcp}" ]; then
  rm -f "${backup_project_mcp}"
fi

if [ -e "${live_opencode}" ]; then
  echo "==> Replacing repository-root .opencode/ from staged config/.opencode/"
  mv "${live_opencode}" "${backup_opencode}"
else
  echo "==> Creating repository-root .opencode/ from staged config/.opencode/"
fi

rm -rf "${live_opencode}"
mkdir -p "${live_opencode}"

rsync_excludes=()
diff_excludes=()
for entry in "${exclude_runtime_artifacts[@]}"; do
  rsync_excludes+=(--exclude "${entry}/")
  diff_excludes+=(-x "${entry}")
done

rsync -a "${rsync_excludes[@]}" "${stage_opencode}/" "${live_opencode}/"

if ! diff -rq "${diff_excludes[@]}" "${stage_opencode}" "${live_opencode}" >/dev/null; then
  cat >&2 <<EOF
Root .opencode replacement verification failed.
Source and target trees differ after copy.
EOF
  exit 1
fi

rm -rf "${backup_opencode}"
opencode_swap_complete=1

if [ -f "${live_project_mcp}" ]; then
  echo "==> Replacing repository-root .mcp.json from staged config/.mcp.json"
  project_mcp_preexisting=1
  mv "${live_project_mcp}" "${backup_project_mcp}"
else
  echo "==> Creating repository-root .mcp.json from staged config/.mcp.json"
fi

cp "${stage_project_mcp}" "${live_project_mcp}"

if ! diff -q "${stage_project_mcp}" "${live_project_mcp}" >/dev/null; then
  cat >&2 <<EOF
Root .mcp.json replacement verification failed.
Source and target files differ after copy.
EOF
  exit 1
fi

rm -f "${backup_project_mcp}"
project_mcp_swap_complete=1

cat <<'EOF'
Done: repository config/ has been regenerated in staging and replaced.
Done: repository-root .opencode/ has been replaced from staged config/.opencode/.
Done: repository-root .mcp.json has been replaced from staged config/.mcp.json.
EOF
