# Workflow Token Count Flake App

## Purpose

Define the repository-root execution contract for static workflow token counting.

## Command contract

- Canonical entrypoint: `nix run .#token-count-workflows`
- Help entrypoint: `nix run .#token-count-workflows -- --help`
- Runtime wrapper behavior:
  - Resolve repository root via `git rev-parse --show-toplevel` with `pwd` fallback.
  - Require `evals/` to exist under repository root.
  - Execute `bun run token-count-workflows` from `evals/` inside `nix develop`.

## Implementation anchors

- Flake app definition: `flake.nix` (`apps.token-count-workflows`)
- App program implementation: `flake.nix` (`tokenCountWorkflowsApp`)
- Script implementation: `evals/token-count-workflows.ts`
- Evals script command alias: `evals/package.json` (`scripts.token-count-workflows`)

## Output contract

- Output directory: `context/tmp/token-footprint/`
- Deterministic latest outputs written each run:
  - `workflow-token-count-latest.json`
  - `workflow-token-count-latest.md`
- Optional archival JSON output when `--run-id` is supplied to the script.

## CI contract

- Workflow: `.github/workflows/workflow-token-count.yml`
- Trigger policy: `push` and `pull_request` events targeting `main`
- CI execution command: `nix run .#token-count-workflows`
- Uploaded artifact name: `workflow-token-footprint`
- Uploaded paths:
  - `context/tmp/token-footprint/workflow-token-count-latest.json`
  - `context/tmp/token-footprint/workflow-token-count-latest.md`
  - `context/tmp/token-footprint/workflow-token-count-*.json`
  - `context/tmp/token-footprint/workflow-token-count-*.md`

## Related context

- `context/sce/workflow-token-footprint-inventory.md`
- `context/sce/workflow-token-footprint-manifest.json`
- `context/overview.md`
- `context/patterns.md`
- `.github/workflows/workflow-token-count.yml`
