# Plan: Drop `evals/` directory and remove eval-related Nix/docs references

## Change summary
- Delete the repository `evals/` workspace entirely, including repo-owned source files and bundled runtime artifacts under that directory.
- Remove Nix/dev-shell/check wiring that exists only to support the deleted eval harness.
- Remove current-state documentation and context references that describe `evals/` as an active working area or validation path.

## Success criteria
- The `evals/` directory is removed from the repository.
- Root `flake.nix` no longer describes the repo as a Bun + TypeScript eval environment and no longer exposes eval-specific validation/tooling.
- Bun/TypeScript support remains only where still required outside `evals/` (for example config-lib bash-policy tests or generation workflows).
- `AGENTS.md` and `context/` files no longer mention `evals/`, eval test commands, or eval-specific execution guidance as current-state behavior.
- Validation guidance reflects the post-evals repo shape without stale references.
- `nix flake check` passes after the cleanup.

## Constraints and non-goals
- Keep Bun, TypeScript, and other tooling that is still required elsewhere in the repo.
- Do not change application behavior beyond removing eval-harness ownership, references, and Nix wiring tied to it.
- Do not introduce replacement eval infrastructure in this change.
- Treat code as source of truth if existing context files overstate or misdescribe eval usage.

## Task stack
- [x] T01: Remove the `evals/` workspace from versioned sources (status:done)
  - Task ID: T01
  - Goal: Delete the `evals/` directory and all tracked files under it so the repository no longer carries the eval harness.
  - Boundaries (in/out of scope): In - `evals/` source files, lockfiles, scripts, tests, and bundled runtime artifacts such as `node_modules/` that live under the directory. Out - root flake wiring, repo docs, and context updates outside `evals/`.
  - Done when: No tracked `evals/` files remain in the repository tree.
  - Verification notes (commands or checks): File-tree review confirms `evals/` is absent; no repository-owned paths under `evals/` remain.

- [x] T02: Remove eval-specific Nix and validation wiring (status:done)
  - Task ID: T02
  - Goal: Update `flake.nix` and any adjacent repo-owned validation/config surfaces so they no longer reference the deleted eval harness.
  - Boundaries (in/out of scope): In - eval-specific flake description text, packages, checks, dev-shell entries, and validation commands that only existed for `evals/`. Out - Bun/TypeScript support still needed for non-eval parts of the repo.
  - Done when: No eval-specific Nix wiring remains, and retained Bun/TypeScript support is justified by non-eval ownership in the repo.
  - Verification notes (commands or checks): Manual diff review of `flake.nix` and related validation surfaces; `nix flake check` planned in T04.

- [x] T03: Sync repo docs and `context/` to the post-evals current state (status:done)
  - Task ID: T03
  - Goal: Remove stale `evals/` references from repo guidance and context files so documentation matches the new repository shape.
  - Boundaries (in/out of scope): In - `AGENTS.md`, `context/overview.md`, `context/patterns.md`, `context/glossary.md`, and any other current-state docs that mention evals or eval-specific commands. Out - historical narrative beyond what is needed to describe current state.
  - Done when: Current-state docs no longer describe `evals/` as a working area, test path, or operator workflow, and any retained Bun/TypeScript language refers only to surviving repo-owned usage.
  - Verification notes (commands or checks): Manual review of edited docs/context files for stale `evals` references and alignment with code truth.

- [x] T04: Validation and cleanup (status:done)
  - Task ID: T04
  - Goal: Run final repository validation, verify the cleanup is complete, and update the plan with the execution outcome.
  - Boundaries (in/out of scope): In - final validation commands, residual stale-reference check, and plan status updates. Out - new feature work or unrelated refactors.
  - Done when: Required validation passes, no intended `evals/` references remain in code/context, and the plan records completion evidence.
  - Verification notes (commands or checks): `nix flake check`; run `nix run .#pkl-check-generated` only if generated outputs change; targeted stale-reference search for `evals` across repo docs/context/config if needed.

## Open questions
- None.

## Task log

### T01
- Status: done
- Completed: 2026-03-27
- Files changed: evals/.gitignore, evals/bun.lock, evals/evals.test.ts, evals/package.json, evals/select-models.sh, evals/setup-opencode.sh, evals/test-setup.ts, evals/tsconfig.json, context/patterns.md, context/plans/drop-evals-directory-and-clean-nix-references.md
- Evidence: `evals/` directory contents removed from versioned sources; post-change file-tree review confirms no repository-owned files remain under `evals/`; root context sync removed the stale `evals/` execution note from `context/patterns.md`.
- Notes: Removed the tracked eval harness files in-scope for T01; broader Nix/reference/doc cleanup remains for later tasks.

### T02
- Status: done
- Completed: 2026-03-27
- Files changed: flake.nix, context/plans/drop-evals-directory-and-clean-nix-references.md
- Evidence: Updated the root flake description to remove the stale eval-harness framing while retaining Bun/TypeScript packages required for surviving config-lib TypeScript/Bun workflows; `nix flake metadata .` succeeded and reports `Description: Shared Context Engineering CLI and config workflows`.
- Notes: No eval-specific checks or dev-shell entries remained in `flake.nix`; retained Bun/TypeScript support is still justified by `config/lib/bash-policy-plugin/` test and source ownership outside the deleted `evals/` workspace.

### T03
- Status: done
- Completed: 2026-03-27
- Files changed: AGENTS.md, context/plans/drop-evals-directory-and-clean-nix-references.md
- Evidence: Updated repo guidance to remove the deleted `evals/` workspace, eval-specific Bun commands, and eval-only verification guidance; targeted stale-reference searches now return no `evals/` matches in `AGENTS.md` and no non-plan matches under `context/`.
- Notes: This task was a localized docs cleanup, so root shared context files remained verify-only; surviving Bun/TypeScript guidance now points only to `config/lib/bash-policy-plugin/` workflows.

### T04
- Status: done
- Completed: 2026-03-27
- Files changed: context/plans/drop-evals-directory-and-clean-nix-references.md
- Evidence: `nix flake check` passed successfully; repo-wide `rg -n "evals" .` now returns matches only in this active plan file, confirming no residual non-plan `evals` references remain in current code/context surfaces.
- Notes: `nix run .#pkl-check-generated` was not required because this task did not modify generated outputs; context sync remained verify-only because no root current-state context drift was found.

## Validation Report

### Commands run
- `nix flake check` -> exit 0 (`cli-tests`, `cli-clippy`, `cli-fmt`, `pkl-parity`, and `config-lib-tests` all passed via the root flake check surface)
- `rg -n "evals" .` -> exit 0 (matches remain only in this active plan file)

### Failed checks and follow-ups
- None.

### Success-criteria verification
- [x] The `evals/` directory is removed from the repository. Verified by prior completed cleanup tasks plus the final stale-reference search showing no remaining non-plan `evals/` paths.
- [x] Root `flake.nix` no longer describes the repo as a Bun + TypeScript eval environment and no longer exposes eval-specific validation/tooling. Verified by prior T02 evidence and by `nix flake check` passing on the post-cleanup flake surface.
- [x] Bun/TypeScript support remains only where still required outside `evals/`. Verified by current repo guidance and the retained `config-lib-tests` flake check passing.
- [x] `AGENTS.md` and `context/` files no longer mention `evals/`, eval test commands, or eval-specific execution guidance as current-state behavior. Verified by final `rg -n "evals" .` output matching only this plan artifact.
- [x] Validation guidance reflects the post-evals repo shape without stale references. Verified by the shared-context verify-only sync pass over root files with no drift found.
- [x] `nix flake check` passes after the cleanup. Verified directly by `nix flake check` exit 0.

### Residual risks
- None identified.
