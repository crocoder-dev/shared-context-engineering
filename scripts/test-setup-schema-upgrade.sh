#!/usr/bin/env bash
set -euo pipefail

sce_bin="${SCE_BIN:-sce}"

usage() {
  cat <<'EOF'
Usage:
  scripts/test-setup-schema-upgrade.sh [--sce-bin <path>]

End-to-end check for config `$schema` series compatibility and the
`sce setup` schema upgrade. Runs a built `sce` binary against an isolated
temporary Git repository and isolated HOME/XDG directories, then asserts:

  1. read-only   version, config validate/show, doctor, and doctor --fix never
                 modify older, newer, or foreign declarations and print the
                 actionable `$schema` messages
  2. upgrade     setup changes exactly the `$schema` line in both canonical files
  3. idempotency a second setup leaves both files byte-identical and prints no
                 `Config schema:` section
  4. mixed       an invalid stale local file stays byte-identical and is
                 reported while a valid stale global file is upgraded
  5. newer       a newer-series local file stays byte-identical
  6. symlink     a symlinked global config is reported and left untouched

Flags:
      --sce-bin <path>   Binary to invoke. Default: $SCE_BIN or sce
  -h, --help             Show this help text
EOF
}

fail() {
  printf 'Setup schema upgrade test failed: %s\n' "$1" >&2
  exit 1
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --sce-bin)
      [[ -n "${2:-}" ]] || fail "--sce-bin requires a value"
      sce_bin="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      fail "unknown flag '$1'"
      ;;
  esac
done

if ! command -v "$sce_bin" >/dev/null 2>&1; then
  fail "sce binary '$sce_bin' was not found; set SCE_BIN or pass --sce-bin"
fi

if [[ "$sce_bin" == */* ]]; then
  sce_bin="$(realpath "$sce_bin")"
else
  sce_bin="$(command -v "$sce_bin")"
fi

tmp_dir="$(mktemp -d)"
cleanup() {
  rm -rf "$tmp_dir"
}
trap cleanup EXIT

export HOME="$tmp_dir/home"
export XDG_CONFIG_HOME="$tmp_dir/xdg-config"
export XDG_STATE_HOME="$tmp_dir/xdg-state"
export XDG_CACHE_HOME="$tmp_dir/xdg-cache"
export GIT_CONFIG_NOSYSTEM=1
export GIT_CONFIG_GLOBAL="$tmp_dir/gitconfig"
unset SCE_CONFIG_FILE
mkdir -p "$HOME" "$XDG_CONFIG_HOME" "$XDG_STATE_HOME" "$XDG_CACHE_HOME"
: > "$GIT_CONFIG_GLOBAL"

case "$(uname -s)" in
  Darwin) global_config="$HOME/Library/Application Support/sce/config.json" ;;
  *) global_config="$XDG_CONFIG_HOME/sce/config.json" ;;
esac

current_version="$("$sce_bin" version 2>/dev/null | sed -n 's/^shared-context-engineering \([^ ]*\).*/\1/p')"
[[ -n "$current_version" ]] || fail "could not read the CLI version from 'sce version'"
current_major="${current_version%%.*}"
current_minor="${current_version#*.}"
current_minor="${current_minor%%.*}"

schema_url() {
  printf 'https://sce.crocoder.dev/v%s/config.json' "$1"
}

current_url="$(schema_url "$current_version")"
older_url="$(schema_url 0.3.3)"
newer_url="$(schema_url "${current_major}.$((current_minor + 1)).0")"
foreign_url='https://example.com/s.json'

older_message="declares SCE config schema version 0.3.3"
newer_message="which was written by a newer SCE release"
foreign_message="is not a canonical SCE schema URL"

stale_local_config() {
  cat <<EOF
{
    "\$schema":   "$1",
  "auth": { "mode": "optional" },
  "telemetry": {"enabled": false},
  "integrations": {
    "target": ["claude",   "opencode"]
  },
  "policies": {
    "bash": {
      "custom": [
        { "id": "no-curl", "match": { "argv_prefix": ["curl"] }, "message": "Use the fetch helper." }
      ]
    }
  }
}
EOF
}

stale_global_config() {
  cat <<EOF
{
  "\$schema": "$1",
  "auth": {"mode": "optional"},
  "telemetry": { "enabled": false }
}
EOF
}

hash_of() {
  sha256sum "$1" | cut -d ' ' -f 1
}

new_repo() {
  local repo="$tmp_dir/$1"
  git init -q "$repo"
  git -C "$repo" remote add origin https://example.com/acme/demo.git
  mkdir -p "$repo/.sce"
  printf '%s\n' "$repo"
}

reset_global() {
  rm -f "$global_config"
  mkdir -p "$(dirname "$global_config")"
}

run_sce() {
  local repo="$1"
  shift
  (cd "$repo" && "$sce_bin" "$@" </dev/null 2>&1) || true
}

run_setup() {
  local repo="$1"
  local output
  if ! output="$(cd "$repo" && "$sce_bin" setup --hooks --repo "$repo" </dev/null 2>&1)"; then
    printf '%s\n' "$output" >&2
    fail "'sce setup --hooks' exited non-zero in '$repo'"
  fi
  printf '%s\n' "$output"
}

assert_contains() {
  local haystack="$1" needle="$2" label="$3"
  [[ "$haystack" == *"$needle"* ]] || {
    printf '%s\n' "$haystack" >&2
    fail "$label: expected output to contain '$needle'"
  }
}

assert_not_contains() {
  local haystack="$1" needle="$2" label="$3"
  [[ "$haystack" != *"$needle"* ]] || {
    printf '%s\n' "$haystack" >&2
    fail "$label: expected output not to contain '$needle'"
  }
}

assert_unchanged() {
  local path="$1" expected_hash="$2" label="$3"
  [[ "$(hash_of "$path")" == "$expected_hash" ]] || fail "$label: '$path' was modified"
}

assert_only_schema_changed() {
  local original="$1" upgraded="$2" label="$3"
  local expected
  expected="$(mktemp "$tmp_dir/expected.XXXXXX")"
  sed "s|$older_url|$current_url|" "$original" > "$expected"
  cmp -s "$expected" "$upgraded" || {
    diff "$original" "$upgraded" >&2 || true
    fail "$label: '$upgraded' differs from the original by more than the \$schema value"
  }
  [[ "$(diff "$original" "$upgraded" | grep -c '^[<>]')" == 2 ]] \
    || fail "$label: expected exactly one changed line in '$upgraded'"
}

printf 'Testing sce %s (accepts %s.%s.x) at %s\n' "$current_version" "$current_major" "$current_minor" "$sce_bin"

# Phase 1: read-only commands never modify older, newer, or foreign declarations.
for variant in older newer foreign; do
  case "$variant" in
    older) url="$older_url"; message="$older_message" ;;
    newer) url="$newer_url"; message="$newer_message" ;;
    foreign) url="$foreign_url"; message="$foreign_message" ;;
  esac

  repo="$(new_repo "readonly-$variant")"
  reset_global
  stale_local_config "$url" > "$repo/.sce/config.json"
  stale_global_config "$url" > "$global_config"
  local_hash="$(hash_of "$repo/.sce/config.json")"
  global_hash="$(hash_of "$global_config")"

  for command in "version" "config validate" "config show" "doctor" "doctor --fix"; do
    # shellcheck disable=SC2086
    output="$(run_sce "$repo" $command)"
    label="read-only ($variant): sce $command"
    assert_unchanged "$repo/.sce/config.json" "$local_hash" "$label"
    assert_unchanged "$global_config" "$global_hash" "$label"
    case "$command" in
      "config validate"|"config show"|doctor*) assert_contains "$output" "$message" "$label" ;;
    esac
  done
done
printf 'ok: read-only commands left older, newer, and foreign declarations unchanged\n'

# Phase 2: setup upgrades exactly the $schema line in both canonical files.
repo="$(new_repo upgrade)"
reset_global
stale_local_config "$older_url" > "$repo/.sce/config.json"
stale_global_config "$older_url" > "$global_config"
cp "$repo/.sce/config.json" "$tmp_dir/upgrade-local.orig"
cp "$global_config" "$tmp_dir/upgrade-global.orig"

output="$(run_setup "$repo")"
assert_contains "$output" "Config schema:" "upgrade"
assert_contains "$output" "upgraded '$repo/.sce/config.json' from v0.3.3 to v$current_version" "upgrade"
assert_contains "$output" "upgraded '$global_config' from v0.3.3 to v$current_version" "upgrade"
assert_only_schema_changed "$tmp_dir/upgrade-local.orig" "$repo/.sce/config.json" "upgrade"
assert_only_schema_changed "$tmp_dir/upgrade-global.orig" "$global_config" "upgrade"

output="$(run_sce "$repo" config validate)"
assert_contains "$output" "SCE config validation: valid" "upgrade: sce config validate"
printf 'ok: setup upgraded only the $schema line in both canonical files\n'

# Phase 3: a second setup run is a no-op for both files.
local_hash="$(hash_of "$repo/.sce/config.json")"
global_hash="$(hash_of "$global_config")"
output="$(run_setup "$repo")"
assert_not_contains "$output" "Config schema:" "idempotency"
assert_unchanged "$repo/.sce/config.json" "$local_hash" "idempotency"
assert_unchanged "$global_config" "$global_hash" "idempotency"
printf 'ok: second setup run left both files byte-identical\n'

# Phase 4: an invalid stale local file does not block upgrading a valid stale global file.
repo="$(new_repo mixed)"
reset_global
cat > "$repo/.sce/config.json" <<EOF
{
  "\$schema": "$older_url",
  "not_a_real_key": true
}
EOF
stale_global_config "$older_url" > "$global_config"
local_hash="$(hash_of "$repo/.sce/config.json")"
cp "$global_config" "$tmp_dir/mixed-global.orig"

output="$(run_setup "$repo")"
assert_unchanged "$repo/.sce/config.json" "$local_hash" "mixed"
assert_only_schema_changed "$tmp_dir/mixed-global.orig" "$global_config" "mixed"
assert_contains "$output" "upgraded '$global_config' from v0.3.3 to v$current_version" "mixed"
assert_contains "$output" "left '$repo/.sce/config.json' unchanged; upgrading '\$schema' to v$current_version would fail validation:" "mixed"
assert_contains "$output" "not_a_real_key" "mixed"
assert_contains "$output" "rerun 'sce setup'" "mixed"
printf 'ok: invalid stale local file was reported and kept while the global file was upgraded\n'

# Phase 5: setup never downgrades a newer-series declaration.
repo="$(new_repo newer)"
reset_global
stale_local_config "$newer_url" > "$repo/.sce/config.json"
local_hash="$(hash_of "$repo/.sce/config.json")"

output="$(run_setup "$repo")"
assert_unchanged "$repo/.sce/config.json" "$local_hash" "newer"
assert_contains "$output" "left '$repo/.sce/config.json' unchanged; it declares newer schema series ${current_major}.$((current_minor + 1))" "newer"
[[ ! -e "$global_config" ]] || fail "newer: setup created a missing global config"
printf 'ok: newer-series local file was left byte-identical\n'

# Phase 6: a symlinked global config is reported and its target left untouched.
repo="$(new_repo symlink)"
reset_global
mkdir -p "$tmp_dir/dotfiles"
stale_global_config "$older_url" > "$tmp_dir/dotfiles/sce-config.json"
ln -s "$tmp_dir/dotfiles/sce-config.json" "$global_config"
target_hash="$(hash_of "$tmp_dir/dotfiles/sce-config.json")"

output="$(run_setup "$repo")"
[[ -L "$global_config" ]] || fail "symlink: global config is no longer a symlink"
[[ "$(readlink "$global_config")" == "$tmp_dir/dotfiles/sce-config.json" ]] \
  || fail "symlink: global config symlink target changed"
assert_unchanged "$tmp_dir/dotfiles/sce-config.json" "$target_hash" "symlink"
assert_contains "$output" "left '$global_config' unchanged; it is a symlink" "symlink"
printf 'ok: symlinked global config was reported and left untouched\n'

printf 'Setup schema upgrade test passed.\n'
