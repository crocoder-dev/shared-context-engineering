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

## Related context

- `context/sce/workflow-token-footprint-inventory.md`
- `context/sce/workflow-token-footprint-manifest.json`
- `context/overview.md`
- `context/patterns.md`
