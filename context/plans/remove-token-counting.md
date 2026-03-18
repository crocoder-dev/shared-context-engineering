# Plan: Remove token counting from the project

## Change summary

Remove the repository's workflow token-counting feature end-to-end with no replacement surface for now. This includes the runnable tooling, flake/package wiring, CI automation, token-footprint artifact contract, and the durable context/docs that currently describe token counting as part of the project's current state.

## Success criteria

- No supported repository command, script alias, or flake app remains for workflow token counting.
- No CI workflow or artifact publication path remains for workflow token counting.
- Token-counting-specific context artifacts and root context references are removed or updated so `context/` no longer presents token counting as a current project capability.
- The token-footprint artifact path contract under `context/tmp/token-footprint/` is removed from current-state docs and any tracked scaffolding tied only to token counting is deleted.
- Validation confirms generated/parity checks and repo checks still pass after the removal.

## Constraints and non-goals

- Treat this as full removal with no placeholder, renamed command, or future-facing stub.
- Keep the change scoped to token-counting behavior and its contracts; do not bundle unrelated eval, flake, CI, or context cleanup.
- Preserve existing SCE planning/execution boundaries; this plan only defines removal work and does not approve implementation beyond one selected task.
- Treat code as source of truth if current context and implementation references differ during execution.
- If token-counting references appear in historical/completed plan artifacts, do not rewrite them unless they are needed for active-plan continuity or validation evidence.

## Task stack (`T01..T04`)

- [x] T01: `Remove token-count runtime surfaces` (status:done)
  - Task ID: T01
  - Status: done
  - Completed: 2026-03-18
  - Files changed: evals/token-count-workflows.ts (deleted), evals/package.json, flake.nix, AGENTS.md
  - Evidence: nix run .#pkl-check-generated passed; nix flake check passed (cli-tests, cli-clippy, cli-fmt, pkl-parity)
  - Notes: Removed token-count-workflows.ts, removed script and js-tiktoken dependency from evals/package.json, removed tokenCountWorkflowsApp and apps.token-count-workflows from flake.nix, removed command references from AGENTS.md
  - Goal: Remove the executable token-counting feature from canonical runtime/code paths, including the eval script and the repo entrypoints that invoke it.
  - Boundaries (in/out of scope): In - token-counting implementation and wiring such as `evals/token-count-workflows.ts`, `evals/package.json`, `flake.nix`, and any directly coupled tests or script references required to keep the repo building cleanly. Out - CI workflow deletion, durable context cleanup, and unrelated eval or flake refactors.
  - Done when: No supported local command path remains for token counting, the repo no longer wires `nix run .#token-count-workflows` or `bun run token-count-workflows`, and any directly affected checks/tests are updated to reflect feature removal.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; targeted inspection confirming token-count runtime entrypoints are absent; run the narrowest affected validation if a test or build target directly covered the removed surfaces.

- [x] T02: `Remove token-count CI automation` (status:done)
  - Task ID: T02
  - Status: done
  - Completed: 2026-03-18
  - Files changed: `.github/workflows/workflow-token-count.yml` (already deleted before this task)
  - Evidence: `nix run .#pkl-check-generated` passed; `nix flake check` passed (cli-tests, cli-clippy, cli-fmt, pkl-parity)
  - Notes: Verified workflow file was already removed; no other CI automation artifacts remain for token counting. Context file references to the workflow are out of scope (T03).
  - Goal: Delete workflow token-count CI automation and any repository automation references that only exist to run or publish token-footprint outputs.
  - Boundaries (in/out of scope): In - `.github/workflows/workflow-token-count.yml` and any directly paired repository automation references that become invalid once runtime support is removed. Out - runtime/code deletion already covered by T01, broad CI cleanup unrelated to token counting, and durable context file updates covered by T03.
  - Done when: No GitHub Actions workflow remains for workflow token counting, no CI artifact publication path remains for token-footprint outputs, and repository automation references stay internally consistent after the workflow removal.
  - Verification notes (commands or checks): Inspect `.github/workflows/` and any referenced automation docs/config touched by the task; `nix run .#pkl-check-generated` if generated assets are affected.

- [x] T03: `Sync context after token-count removal` (status:done)
  - Task ID: T03
  - Status: done
  - Completed: 2026-03-19
  - Files changed: context/sce/workflow-token-footprint-inventory.md (deleted), context/sce/workflow-token-footprint-manifest.json (deleted), context/sce/workflow-token-count-workflow.md (deleted), evals/README.md (deleted), context/context-map.md, context/overview.md, context/glossary.md, context/architecture.md, context/patterns.md
  - Evidence: `nix run .#pkl-check-generated` passed; `nix flake check` passed (cli-tests, cli-clippy, cli-fmt, pkl-parity)
  - Notes: Deleted three token-counting context artifacts and evals/README.md, removed token-counting references from context-map.md, overview.md, glossary.md, architecture.md, and patterns.md. Context navigation no longer points to deleted token-count artifacts.
  - Goal: Remove token-counting current-state documentation/contracts from `context/` and update shared context files so future sessions no longer load token counting as an active capability.
  - Boundaries (in/out of scope): In - deleting token-counting-specific context artifacts such as `context/sce/workflow-token-footprint-inventory.md`, `context/sce/workflow-token-footprint-manifest.json`, and `context/sce/workflow-token-count-workflow.md` if they are no longer needed, plus updating `context/context-map.md`, `context/overview.md`, `context/glossary.md`, and any other current-state context references tied to token counting. Out - historical rewrite of completed plan files unless needed for active continuity, and any non-token-count documentation cleanup.
  - Done when: Durable context files no longer describe token counting, token-footprint artifact-path contracts are removed from current-state docs, and context navigation no longer points future sessions to deleted token-count artifacts.
  - Verification notes (commands or checks): Review all updated `context/` references for current-state accuracy against code truth; confirm removed context artifacts are no longer referenced by `context/context-map.md`, `context/overview.md`, or `context/glossary.md`.

- [x] T04: `Validate removal and clean up` (status:done)
  - Task ID: T04
  - Status: done
  - Completed: 2026-03-19
  - Files changed: evals/bun.lock (regenerated to remove stale js-tiktoken entry)
  - Evidence: `nix run .#pkl-check-generated` passed; `nix flake check` passed (cli-tests, cli-clippy, cli-fmt, pkl-parity); no token-count-specific files/contracts remain outside historical plan artifacts
  - Notes: Regenerated bun.lock to prune stale js-tiktoken entry; all validation checks pass; token-counting removal is complete
  - Goal: Run final validation for the token-counting removal, confirm no tracked token-count scaffolding remains, and capture the final current-state evidence in the plan.
  - Boundaries (in/out of scope): In - repo validation, parity checks, cleanup of tracked token-count-specific temporary scaffolding if present, and final context-sync verification. Out - new feature work, replacement analytics tooling, or unrelated repository cleanup.
  - Done when: Required validation passes, tracked token-count-specific leftovers are removed, and the plan records final evidence showing the project no longer exposes token counting as a supported capability.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; `nix flake check`; confirm no tracked token-count-specific files/contracts remain outside historical plan artifacts.

## Validation Report

### Commands run
- `nix run .#pkl-check-generated` -> exit 0 ("Generated outputs are up to date.")
- `nix flake check` -> exit 0 (cli-tests, cli-clippy, cli-fmt, pkl-parity all passed)
- `bun install` (in evals/) -> exit 0 (regenerated lockfile, removed 1 stale package)

### Temporary scaffolding removed
- None. `context/tmp/` contains only `.gitignore`.

### Success-criteria verification
- [x] No supported repository command, script alias, or flake app remains for workflow token counting -> confirmed via flake.nix inspection (no tokenCountWorkflowsApp or apps.token-count-workflows)
- [x] No CI workflow or artifact publication path remains for workflow token counting -> confirmed via `.github/workflows/` inspection (only publish-tiles.yml and release-agents.yml remain)
- [x] Token-counting-specific context artifacts and root context references are removed -> confirmed via context/ search (only plan file contains references)
- [x] The token-footprint artifact path contract removed -> confirmed (context/tmp/token-footprint/ does not exist)
- [x] Validation confirms generated/parity checks and repo checks still pass -> confirmed (nix flake check passed)

### Residual risks
- None identified. Token-counting removal is complete.

## Open questions (if any)

- None. The user confirmed full end-to-end removal, removal of the `context/tmp/token-footprint/` contract, and no replacement surface for now.
