#!/usr/bin/env bash
set -euo pipefail

# Manual smoke test for Turso sync support.
# Contains test credentials supplied for local manual testing only.
# Do not commit this file if the token is sensitive or long-lived.

export DATABASE_URL=""
export DATABASE_AUTH_TOKEN=""

export SCE_SYNC_URL="$DATABASE_URL"
export SCE_SYNC_TOKEN="$DATABASE_AUTH_TOKEN"

STATE_DIR="$(mktemp -d)"
export XDG_STATE_HOME="$STATE_DIR"

echo "Using isolated state dir: $XDG_STATE_HOME"

echo "1. Show sync command help"
nix develop -c sh -c 'cd cli && cargo run -- sync --help'

echo "2. Run setup, including best-effort initial pull"
nix develop -c sh -c 'cd cli && cargo run -- setup --opencode --non-interactive'

echo "3. Pull from Turso"
nix develop -c sh -c 'cd cli && cargo run -- sync pull'

echo "4. Push to Turso"
nix develop -c sh -c 'cd cli && cargo run -- sync push'

echo "5. Confirm local Agent Trace DB exists"
test -f "$XDG_STATE_HOME/sce/agent-trace.db"
ls -lh "$XDG_STATE_HOME/sce/agent-trace.db"

echo "Manual Turso sync smoke test passed."
