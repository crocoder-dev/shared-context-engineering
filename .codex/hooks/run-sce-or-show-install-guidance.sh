#!/usr/bin/env bash
set -euo pipefail

if ! command -v sce >/dev/null 2>&1; then
  echo "sce CLI not found. Install it from https://sce.crocoder.dev/docs/getting-started#install-cli" >&2
  exit 0
fi

exec "$@"