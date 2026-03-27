# Plan: split-js-flake-checks

## Change summary

Add six separate `nix flake check` derivations for the two in-scope JavaScript package areas: `npm/` and `config/lib/bash-policy-plugin/`. Each directory should get its own Bun test check, Biome lint/check derivation, and Biome format verification derivation.

## Success criteria

- `nix flake check` exposes exactly six separate JS-surface checks.
- `npm/` has three distinct checks: Bun test, Biome lint/check, and Biome format verification.
- `config/lib/bash-policy-plugin/` has three distinct checks: Bun test, Biome lint/check, and Biome format verification.
- Each check targets only its intended directory and preserves the current root Biome scope contract.
- Check names are deterministic and clearly identify both tool and target directory.
- Final validation confirms all six checks evaluate through `nix flake check` and any affected current-state context is updated or explicitly verified.

## Constraints and non-goals

- In scope: root `flake.nix` check composition for the six requested derivations.
- In scope: narrow package or Nix workflow adjustments only if required to run the six checks deterministically.
- In scope: docs/context updates only where the flake-check contract changes.
- Out of scope: expanding Biome beyond `npm/` and `config/lib/bash-policy-plugin/`.
- Out of scope: broader dependency-materialization redesign beyond what the six requested checks require.
- Out of scope: collapsing the requested six derivations into fewer aggregate checks.
- Out of scope: unrelated Rust CLI validation changes.
- Every executable task must remain one coherent commit unit.

## Task stack

- [x] T01: `Add three separate flake checks for npm/` (status:done)
  - Task ID: T01
  - Goal: Wire `npm/` into `nix flake check` with three distinct derivations for Bun test, Biome lint/check, and Biome format verification.
  - Boundaries (in/out of scope): In - root flake check definitions, stable derivation naming, and any narrow command wiring needed for deterministic `npm/` Bun + Biome execution. Out - changes to `config/lib/bash-policy-plugin/`, repo-wide Biome scope changes, and broader package-management redesign.
  - Done when: `flake.nix` exposes exactly three `npm/`-specific checks and each derivation runs only the intended `npm/` Bun test, Biome lint/check, or Biome format verification flow.
  - Verification notes (commands or checks): Verify the flake defines three distinct `npm/` checks and that their commands scope execution to `npm/` only.
  - Completed: 2026-03-28
  - Files changed: `flake.nix`, `npm/test/install.test.js`
  - Evidence: `nix eval --json .#checks.x86_64-linux` now exposes `npm-bun-tests`, `npm-biome-check`, and `npm-biome-format`; `nix build .#checks.x86_64-linux.npm-bun-tests`; `nix build .#checks.x86_64-linux.npm-biome-check`; `nix build .#checks.x86_64-linux.npm-biome-format`
  - Notes: Important-change context-sync case because the root flake check inventory/contract changed.

- [x] T02: `Add three separate flake checks for config/lib/bash-policy-plugin/` (status:done)
  - Task ID: T02
  - Goal: Wire `config/lib/bash-policy-plugin/` into `nix flake check` with three distinct derivations for Bun test, Biome lint/check, and Biome format verification.
  - Boundaries (in/out of scope): In - root flake check definitions, stable derivation naming, and any narrow command wiring needed for deterministic config-lib Bun + Biome execution. Out - changes to `npm/`, repo-wide Biome scope changes, and broader package-management redesign.
  - Done when: `flake.nix` exposes exactly three `config/lib/bash-policy-plugin/`-specific checks and each derivation runs only the intended config-lib Bun test, Biome lint/check, or Biome format verification flow.
  - Verification notes (commands or checks): Verify the flake defines three distinct config-lib checks and that their commands scope execution to `config/lib/bash-policy-plugin/` only.
  - Completed: 2026-03-28
  - Files changed: `flake.nix`, `context/overview.md`, `context/architecture.md`, `context/glossary.md`, `context/plans/split-js-flake-checks.md`
  - Evidence: `nix eval --json .#checks.x86_64-linux`; `nix build .#checks.x86_64-linux.config-lib-bun-tests`; `nix build .#checks.x86_64-linux.config-lib-biome-check`; `nix build .#checks.x86_64-linux.config-lib-biome-format`
  - Notes: Important-change context-sync case because the root flake check inventory/contract changed.

- [x] T03: `Run final validation and cleanup for six JS flake checks` (status:done)
  - Task ID: T03
  - Goal: Validate the end-to-end flake-check inventory, confirm there are exactly six separate in-scope JS derivations, and sync any current-state context affected by the new check contract.
  - Boundaries (in/out of scope): In - full validation for touched Nix/JS surfaces, check-inventory verification, stale check-name cleanup, and required context-sync verification. Out - new tooling adoption beyond the six requested checks.
  - Done when: `nix flake check` includes exactly six separate in-scope JS derivations, validation passes, and any changed current-state context/docs are updated or explicitly verified.
  - Verification notes (commands or checks): Run the repo validation path that exercises `nix flake check`; verify the six targeted derivation names and directory scopes; run generated-output parity if touched; manually confirm root context/glossary references match the final check contract.
  - Completed: 2026-03-28
  - Files changed: `context/plans/split-js-flake-checks.md`
  - Evidence: `nix eval --json .#checks.x86_64-linux` shows exactly `npm-bun-tests`, `npm-biome-check`, `npm-biome-format`, `config-lib-bun-tests`, `config-lib-biome-check`, and `config-lib-biome-format`; `nix run .#pkl-check-generated`; `nix flake check`
  - Notes: Verify-only context-sync pass; root current-state context already matched the final six-check contract and required no additional edits.

## Open questions

- None. Scope is explicitly six separate `nix flake check` derivations: Bun test, Biome lint/check, and Biome format verification for each of `npm/` and `config/lib/bash-policy-plugin/`.

## Validation Report

### Commands run

- `nix eval --json .#checks.x86_64-linux` -> exit 0 (`JS_CHECK_COUNT 6`; JS checks resolved to `config-lib-biome-check`, `config-lib-biome-format`, `config-lib-bun-tests`, `npm-biome-check`, `npm-biome-format`, `npm-bun-tests`)
- `nix run .#pkl-check-generated` -> exit 0 (`Generated outputs are up to date.`)
- `nix flake check` -> exit 0 (evaluated and ran 10 flake checks including all six JS derivations)

### Temporary scaffolding

- None added for this task.

### Success-criteria verification

- [x] `nix flake check` exposes exactly six separate JS-surface checks -> confirmed by `nix eval --json .#checks.x86_64-linux` inventory output
- [x] `npm/` has three distinct checks: Bun test, Biome lint/check, and Biome format verification -> confirmed by `npm-bun-tests`, `npm-biome-check`, and `npm-biome-format`
- [x] `config/lib/bash-policy-plugin/` has three distinct checks: Bun test, Biome lint/check, and Biome format verification -> confirmed by `config-lib-bun-tests`, `config-lib-biome-check`, and `config-lib-biome-format`
- [x] Each check targets only its intended directory and preserves the current root Biome scope contract -> confirmed by `flake.nix` directory-scoped run commands plus `biome.json` includes limited to `npm/**` and `config/lib/bash-policy-plugin/**`
- [x] Check names are deterministic and clearly identify both tool and target directory -> confirmed by the six derivation names above
- [x] Final validation confirms all six checks evaluate through `nix flake check` and any affected current-state context is updated or explicitly verified -> confirmed by successful `nix flake check` plus verify-only context-sync pass over `context/overview.md`, `context/architecture.md`, `context/glossary.md`, `context/patterns.md`, and `context/context-map.md`

### Failed checks and follow-ups

- None.

### Residual risks

- Validation was run against a dirty working tree; evidence confirms the final six-check contract for the current state, but unrelated in-progress changes remain elsewhere in the repository.
