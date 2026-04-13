# Plan: Refactor shared Pkl content into plan/code/commit groups

## Change summary
- Refactor manual shared-content authoring under `config/pkl/base/` so canonical SCE content is grouped by `plan`, `code`, and `commit` concerns instead of one large `shared-content.pkl` body.
- Apply the same grouped structure to the automated shared-content authoring layer under `config/pkl/base/`.
- Preserve generated behavior and output text exactly; this is a source-layout refactor only.

## Success criteria
- Manual shared-content canonical units are split into explicit `plan`, `code`, and `commit`-focused base modules under `config/pkl/base/`.
- Automated shared-content canonical units are split into matching `plan`, `code`, and `commit`-focused base modules under `config/pkl/base/`.
- Existing generated OpenCode and Claude outputs remain behaviorally unchanged; no intended output text or generated-file contract changes are introduced.
- The generation entrypoint and renderer imports still resolve deterministically after the refactor.
- Documentation/context that currently names `config/pkl/base/shared-content.pkl` as the sole canonical location is updated to match the new grouped current state where needed.

## Constraints and non-goals
- Scope is limited to the shared-content authoring layer in `config/pkl/base/`; broader `config/pkl/` reorganization is out of scope.
- Preserve current generated output semantics and text; this refactor must not intentionally change generated agent, command, skill, or metadata content.
- Keep task slicing atomic: one task should land as one coherent commit without bundling unrelated cleanup.
- Do not use this refactor to change workflow contracts, command wording, or automated-profile policy behavior.
- Avoid unnecessary renderer churn; only adjust imports/composition that are required to support the new grouped layout.

## Task stack
- [x] T01: Split manual shared-content into plan/code/commit base modules (status:done)
  - Task ID: T01
  - Goal: Introduce grouped manual shared-content modules for `plan`, `code`, and `commit` concerns and keep the manual canonical export surface consistent for downstream renderers.
  - Boundaries (in/out of scope):
    - In scope: create the new manual base modules, move or recompose manual agent/command/skill canonical content into the grouped files, and update the manual shared-content entrypoint/import wiring as needed.
    - Out of scope: automated profile changes, context documentation updates, and intentional generated-output wording changes.
  - Done when:
    - Manual shared-content authoring no longer keeps the relevant `plan`, `code`, and `commit` canonical bodies only in one monolithic file.
    - The manual shared-content layer exposes the same effective generated content contract after the regrouping.
    - Any required manual import/composition changes are localized and deterministic.
  - Verification notes (commands or checks): `nix develop -c pkl eval config/pkl/generate.pkl` or equivalent narrow Pkl evaluation for the manual path; defer full repo validation to T04.
  - Status: done
  - Completed: 2026-04-13
  - Files changed: `config/pkl/base/shared-content.pkl`, `config/pkl/base/shared-content-common.pkl`, `config/pkl/base/shared-content-plan.pkl`, `config/pkl/base/shared-content-code.pkl`, `config/pkl/base/shared-content-commit.pkl`
  - Evidence:
    - `nix develop -c pkl eval -m . config/pkl/generate.pkl` succeeded.
    - `nix run .#pkl-check-generated` succeeded (`Generated outputs are up to date.`).
  - Notes:
    - Kept `config/pkl/base/shared-content.pkl` as the stable aggregation surface to avoid downstream renderer churn during T01.
    - Root context references to `config/pkl/base/shared-content.pkl` as the sole canonical source remain deferred to T03 by plan scope.

- [x] T02: Split automated shared-content into matching plan/code/commit modules (status:done)
  - Task ID: T02
  - Goal: Mirror the manual grouping in the automated shared-content authoring layer so automated content is organized by the same `plan`, `code`, and `commit` concern boundaries.
  - Boundaries (in/out of scope):
    - In scope: create the automated grouped base modules, move or recompose automated canonical content into them, and update automated shared-content entrypoint/import wiring as needed.
    - Out of scope: manual-profile structural changes already completed in T01, context documentation changes, and behavior/policy changes to automated outputs.
  - Done when:
    - Automated shared-content authoring is split into explicit `plan`, `code`, and `commit` modules under `config/pkl/base/`.
    - Automated downstream imports still resolve deterministically after the regrouping.
    - Automated generated content remains semantically unchanged.
  - Verification notes (commands or checks): `nix develop -c pkl eval config/pkl/generate.pkl` or equivalent narrow Pkl evaluation for the automated path; defer full repo validation to T04.
  - Status: done
  - Completed: 2026-04-13
  - Files changed: `config/pkl/base/shared-content-automated.pkl`, `config/pkl/base/shared-content-automated-common.pkl`, `config/pkl/base/shared-content-automated-plan.pkl`, `config/pkl/base/shared-content-automated-code.pkl`, `config/pkl/base/shared-content-automated-commit.pkl`
  - Evidence:
    - `nix develop -c pkl eval -m . config/pkl/generate.pkl` succeeded.
    - `nix run .#pkl-check-generated` succeeded (`Generated outputs are up to date.`).
  - Notes:
    - Kept `config/pkl/base/shared-content-automated.pkl` as the stable aggregation surface to avoid downstream renderer churn.
    - Followed the exact same pattern as T01 for consistency between manual and automated profiles.
    - Downstream renderers (`opencode-automated-content.pkl`, `metadata-coverage-check.pkl`) required no changes.

- [x] T03: Sync current-state context for grouped shared-content ownership (status:done)
  - Task ID: T03
  - Goal: Update durable context files so they describe the new grouped shared-content ownership model instead of pointing only to `config/pkl/base/shared-content.pkl`.
  - Boundaries (in/out of scope):
    - In scope: update the relevant current-state context files that document shared-content ownership, renderer layering, or canonical source locations.
    - Out of scope: unrelated prose cleanup, historical narration, or new behavior contracts beyond this refactor.
  - Done when:
    - Context references that would become inaccurate after T01-T02 are repaired.
    - Current-state docs explain that shared-content canonical authoring is grouped by `plan`, `code`, and `commit` for both manual and automated profiles where relevant.
    - No stale "single-file only" ownership statement remains in the touched context files.
  - Verification notes (commands or checks): read-through verification against code truth; ensure wording matches the final file layout exactly.
  - Status: done
  - Completed: 2026-04-13
  - Files changed: `context/glossary.md`, `context/overview.md`, `context/architecture.md`, `context/patterns.md`, `context/sce/atomic-commit-workflow.md`
  - Evidence:
    - `nix develop -c pkl eval -m . config/pkl/generate.pkl` succeeded.
    - `nix run .#pkl-check-generated` succeeded (`Generated outputs are up to date.`).
    - Pre-existing clippy error in `cli/src/command_surface.rs` (needless borrow) blocks full `nix flake check` but is unrelated to T03 context changes.
  - Notes:
    - Also staged untracked automated shared-content files from T02 (`shared-content-automated-{common,plan,code,commit}.pkl`) to unblock Nix flake check which requires git-tracked sources.
    - Context files now accurately describe the grouped shared-content ownership model with aggregation surfaces importing from `plan`, `code`, and `commit` modules.

- [x] T04: Validation and cleanup (status:done)
  - Task ID: T04
  - Goal: Run full regeneration/parity validation and confirm the refactor leaves generated outputs unchanged except for intended source-layout edits.
  - Boundaries (in/out of scope):
    - In scope: regeneration/parity checks, repo validation, and cleanup of any temporary refactor scaffolding.
    - Out of scope: new refactors or follow-on content rewrites.
  - Done when:
    - `nix run .#pkl-check-generated` succeeds.
    - `nix flake check` succeeds.
    - Generated outputs are confirmed to have no unintended behavior/text drift from the refactor.
    - Any temporary scaffolding introduced during regrouping is removed.
  - Verification notes (commands or checks): run `nix run .#pkl-check-generated` and `nix flake check` from repo root.
  - Status: done
  - Completed: 2026-04-13
  - Files changed: `cli/src/command_surface.rs`
  - Evidence:
    - `nix run .#pkl-check-generated` succeeded (`Generated outputs are up to date.`).
    - `nix flake check` succeeded (all 13 checks passed).
  - Notes:
    - Fixed pre-existing clippy `needless_borrow` error in `cli/src/command_surface.rs` line 190 (`&text` → `text`).
    - No temporary scaffolding was found; T01-T03 did not introduce any temporary files.
    - Generated outputs confirmed unchanged by parity check.

## Open questions
- None.

## Validation Report

### Commands run
- `nix run .#pkl-check-generated` -> exit 0 (`Generated outputs are up to date.`)
- `nix flake check` -> exit 0 (all 13 checks passed)

### Temporary scaffolding removed
- None found. T01-T03 did not introduce any temporary files.

### Success-criteria verification
- [x] Manual shared-content canonical units are split into explicit `plan`, `code`, and `commit`-focused base modules under `config/pkl/base/` -> confirmed via T01 completion
- [x] Automated shared-content canonical units are split into matching `plan`, `code`, and `commit`-focused base modules under `config/pkl/base/` -> confirmed via T02 completion
- [x] Existing generated OpenCode and Claude outputs remain behaviorally unchanged -> confirmed via `nix run .#pkl-check-generated` success
- [x] The generation entrypoint and renderer imports still resolve deterministically -> confirmed via Pkl evaluation success
- [x] Documentation/context updated to match the new grouped current state -> confirmed via T03 completion

### Residual risks
- None identified.
