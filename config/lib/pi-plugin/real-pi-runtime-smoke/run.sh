#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "${script_dir}/../../../.." && pwd)"

fail() {
  printf 'FAIL: %s\n' "$1" >&2
  exit 1
}

(cd "${repo_root}" && nix develop --command bash -c "./scripts/run-cli-cargo.sh build --manifest-path cli/Cargo.toml")

sce_bin="${repo_root}/cli/target/debug/sce"
if [ ! -x "${sce_bin}" ]; then
  fail "expected built sce binary at ${sce_bin}"
fi
cli_bin_dir="${repo_root}/cli/target/debug"

scratch="$(mktemp -d "${TMPDIR:-/tmp}/sce-real-pi-runtime-smoke.XXXXXX")"
agent_dir="${scratch}/agentdir"
mkdir -p "${agent_dir}"
cat > "${agent_dir}/settings.json" <<'EOF'
{
  "defaultProvider": "sce-test-provider",
  "defaultModel": "sce-test-model"
}
EOF

export XDG_STATE_HOME="${scratch}/state"

(
  cd "${scratch}"
  git init --quiet
  git config user.email smoke@example.com
  git config user.name "Smoke Test"
  echo seed > seed.txt
  git add seed.txt
  git commit --quiet -m seed
  git remote add origin https://example.com/sce-real-pi-runtime-smoke.git
)

mkdir -p "${scratch}/.sce"
cat > "${scratch}/.sce/config.json" <<'EOF'
{
  "agent_trace": {
    "auto_sync": false
  }
}
EOF

(cd "${scratch}" && "${sce_bin}" setup --pi --non-interactive)

mkdir -p "${scratch}/.pi/extensions/test-provider"
cp "${script_dir}/provider-extension.ts" "${scratch}/.pi/extensions/test-provider/index.ts"

export SCE_CLI_BIN_DIR="${cli_bin_dir}"
(
  cd "${repo_root}"
  nix develop --command bash -c '
    set -euo pipefail
    export PATH="${SCE_CLI_BIN_DIR}:${PATH}"
    resolved="$(command -v sce)" || {
      printf "FAIL: no sce resolved on PATH inside the Pi driver environment\n" >&2
      exit 1
    }
    resolved_real="$(realpath "${resolved}")"
    expected_real="$(realpath "${SCE_CLI_BIN_DIR}/sce")"
    if [ "${resolved_real}" != "${expected_real}" ]; then
      printf "FAIL: sce on PATH resolved to %s (real path %s), expected the branch-built binary at %s\n" \
        "${resolved}" "${resolved_real}" "${expected_real}" >&2
      exit 1
    fi
    printf "sce on PATH resolved correctly to %s\n" "${resolved_real}"
    node "$1" "$2" "$3"
  ' bash "${script_dir}/driver.mjs" "${scratch}" "${agent_dir}"
)

printf '\nscratch repo: %s\n' "${scratch}"

doctor_json="$(cd "${scratch}" && "${sce_bin}" doctor --format json)" || fail "sce doctor did not succeed"

db_path="$(printf '%s' "${doctor_json}" | nix shell nixpkgs#jq --command jq -r '.agent_trace_db.path // empty')"
if [ -z "${db_path}" ] || [ "${db_path}" = "null" ]; then
  fail "sce doctor did not report a usable agent_trace_db.path (got: '${db_path}')"
fi
if [ ! -f "${db_path}" ]; then
  fail "agent_trace_db.path reported by doctor does not exist on disk: ${db_path}"
fi
printf 'agent trace db: %s\n' "${db_path}"

turso_query() {
  local sql="$1"
  (cd "${repo_root}" && nix run .#turso -- --experimental-multiprocess-wal --readonly -m list -q "${db_path}" "${sql}")
}

assert_eq() {
  local description="$1" expected="$2" actual="$3"
  if [ "${actual}" != "${expected}" ]; then
    fail "${description}: expected '${expected}', got '${actual}'"
  fi
  printf 'OK: %s = %s\n' "${description}" "${actual}"
}

assert_count() {
  local description="$1" sql="$2" expected="$3"
  local actual
  actual="$(turso_query "${sql}")"
  assert_eq "${description}" "${expected}" "${actual}"
}

smoke_output="${scratch}/smoke-output.txt"
if [ ! -f "${smoke_output}" ]; then
  fail "smoke-output.txt was not created at ${smoke_output} — the scripted Bash mutation never executed"
fi
if ! grep -q 'sce-real-pi-smoke' "${smoke_output}"; then
  fail "smoke-output.txt exists but does not contain the expected marker 'sce-real-pi-smoke'"
fi
printf 'OK: smoke-output.txt exists and contains expected marker\n'

printf '\n=== mutation_trace_scopes (debug) ===\n'
turso_query "SELECT scope_id, actor_kind, status FROM mutation_trace_scopes;"
printf '\n=== mutation_trace_events (debug) ===\n'
turso_query "SELECT boundary_kind, attribution_kind, tainted, failure_kind FROM mutation_trace_events;"
printf '\n=== mutation_trace_scope_provenance (debug) ===\n'
turso_query "SELECT scope_id, session_id, model_id FROM mutation_trace_scope_provenance;"
printf '\n'

assert_count "total Pi mutation scopes" \
  "SELECT COUNT(*) FROM mutation_trace_scopes WHERE actor_kind = 'pi';" "1"

assert_count "closed Pi mutation scopes" \
  "SELECT COUNT(*) FROM mutation_trace_scopes WHERE actor_kind = 'pi' AND status = 'closed';" "1"

assert_count "final ai_exclusive close events" \
  "SELECT COUNT(*) FROM mutation_trace_events WHERE boundary_kind = 'close' AND attribution_kind = 'ai_exclusive' AND tainted = 0 AND failure_kind = 'healthy';" \
  "1"

assert_count "Pi session provenance rows" \
  "SELECT COUNT(*) FROM mutation_trace_scope_provenance WHERE session_id LIKE 'pi_%' AND model_id = 'sce-test-provider/sce-test-model';" \
  "1"

assert_count "worktrees left tainted or unresolved" \
  "SELECT COUNT(*) FROM mutation_trace_worktrees WHERE tainted = 1 OR needs_rebaseline = 1 OR failure_kind != 'healthy';" \
  "0"

printf '\nPASS: real Pi runtime mutation-attribution smoke\n'
