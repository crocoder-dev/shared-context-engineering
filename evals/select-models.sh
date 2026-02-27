#!/usr/bin/env bash

set -euo pipefail

opencode models | jq -Rnc '
[
  inputs
  | select(
      . == "openai/gpt-5.3-codex"
    )
  | capture("^(?<providerID>[^/]+)/(?<modelID>.+)$")
  | { providerID, modelID }
]
'
