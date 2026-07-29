#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
helper="${script_dir}/prepare-cli-generated-assets.sh"
producer="${script_dir}/produce-cli-generated-input.sh"
tmp_root="$(mktemp -d)"
cleanup() {
  rm -rf "${tmp_root}"
}
trap cleanup EXIT

test_repo="${tmp_root}/repo"
fake_bin="${tmp_root}/bin"
state_root="${tmp_root}/state"
mkdir -p \
  "${test_repo}/scripts" \
  "${test_repo}/config/pkl" \
  "${test_repo}/config/schema" \
  "${test_repo}/cli/assets/hooks" \
  "${test_repo}/cli/migrations" \
  "${fake_bin}" \
  "${state_root}"
cp "${helper}" "${test_repo}/scripts/prepare-cli-generated-assets.sh"
cp "${producer}" "${test_repo}/scripts/produce-cli-generated-input.sh"
printf 'generator\n' > "${test_repo}/config/pkl/generate.pkl"
printf 'config/pkl\n' > "${test_repo}/config/pkl/generator-inputs.txt"
printf 'schema\n' > "${test_repo}/config/schema/agent-trace.schema.json"
printf 'hook\n' > "${test_repo}/cli/assets/hooks/hook.sh"
printf 'migration\n' > "${test_repo}/cli/migrations/001.sql"

cat > "${fake_bin}/pkl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
: "${FAKE_PKL_STATE:?missing fake Pkl state directory}"
destination=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    eval) shift ;;
    -m)
      destination="$2"
      shift 2
      ;;
    *) shift ;;
  esac
done
count_file="${FAKE_PKL_STATE}/generation-count"
count=0
if [ -f "${count_file}" ]; then
  count="$(<"${count_file}")"
fi
printf '%s\n' "$((count + 1))" > "${count_file}"
mkdir -p "${destination}/config/.opencode"
printf 'generated\n' > "${destination}/config/.opencode/generated.txt"
EOF
chmod +x \
  "${fake_bin}/pkl" \
  "${test_repo}/scripts/prepare-cli-generated-assets.sh" \
  "${test_repo}/scripts/produce-cli-generated-input.sh"

run_helper() {
  env \
    PATH="${fake_bin}:${PATH}" \
    FAKE_PKL_STATE="${state_root}" \
    "${test_repo}/scripts/prepare-cli-generated-assets.sh" \
    "${test_repo}" "$1" >/dev/null
}

first_root="${tmp_root}/first"
second_root="${tmp_root}/second"
run_helper "${first_root}"
run_helper "${second_root}"

if ! diff -r "${first_root}" "${second_root}" >/dev/null; then
  printf 'Producer-backed package fallbacks differed across runs.\n' >&2
  exit 1
fi
if [ "$(<"${state_root}/generation-count")" -ne 4 ]; then
  printf 'Packaging helper did not delegate one producer run per fallback.\n' >&2
  exit 1
fi
test ! -e "${first_root}/INPUTS.SHA256SUMS"
test -f "${first_root}/pkl-generated/config/.opencode/generated.txt"
test -f "${first_root}/static/hooks/hook.sh"
test -f "${first_root}/static/migrations/001.sql"
test -f "${first_root}/static/schema/agent-trace.schema.json"
(
  cd "${first_root}"
  sha256sum -c SHA256SUMS >/dev/null
)
if [ "$(wc -l < "${first_root}/SHA256SUMS" | tr -d ' ')" -ne 4 ]; then
  printf 'Combined package fallback inventory has the wrong entry count.\n' >&2
  exit 1
fi

printf 'prepare-cli-generated-assets delegation tests passed.\n'
