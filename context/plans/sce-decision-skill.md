# Plan: sce-decision-skill

## Change summary

Add a standalone `sce-decision` skill that is generated for OpenCode, Claude, and Pi without adding a new user-facing command or prompt. The skill writes one dated ADR for one system-wide important decision, defaults new records to `Accepted` unless the user requested another status, follows the supplied ADR rules and template, and never edits an accepted ADR.

Extend the shared task and plan context-synchronization lifecycle so `sce-next-task` and `sce-validate` may invoke this sibling skill during synchronization. This is a narrow, explicit exception to the current self-contained workflow rule: all ordinary phase state remains internal, commands still route to exactly one workflow skill, and no other sibling-skill invocation is allowed.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the check that proves it. `/validate` runs these checks; no task in the stack performs final validation.

- [x] AC1: OpenCode, Claude, and Pi generated payloads each contain the standalone `sce-decision` package, while the user-facing command/prompt catalog remains limited to `change-to-plan`, `next-task`, `validate`, and `commit`.
  - Validate: generate a temporary payload with `nix run .#pkl-generate -- "$(mktemp -d)"`, inspect the three `config/.{opencode,claude,pi}/skills/sce-decision/` packages, and verify no `decision` command or prompt was emitted.
- [x] AC2: `sce-decision` creates exactly one `context/decisions/YYYY-MM-DD-<decision-slug>.md` record for one qualifying system-wide decision, uses `Accepted` by default unless the user requested another allowed status, and implements the supplied context, decision, rationale, alternatives, compatibility/risk, guardrail, consequence, follow-up, and reference rules.
  - Validate: inspect the generated `sce-decision` skill documents for all three targets and confirm the generation checks assert their required content and paths.
- [x] AC3: The decision gate triggers only for changes to system boundaries, ownership, interfaces, data models, persistence, compatibility contracts, security posture, deployment/distribution strategy, major dependencies, or similarly durable constraints; routine implementation details, local refactors, temporary experiments, and easily reversible choices do not create ADRs.
  - Validate: inspect the shared context-sync source and generated `sce-next-task`/`sce-validate` skills, and run the Pkl metadata and generation contract checks through `nix run .#pkl-check-generated`.
- [x] AC4: `sce-next-task` and `sce-validate` invoke `sce-decision` only during their successful context-synchronization phases, propagate a decision-writing blocker as a synchronization blocker, and make a written ADR available for current-state context linking before synchronization completes.
  - Validate: inspect temporary generated `sce-next-task` and `sce-validate` packages for the synchronization-only invocation, success/blocker branches, and ADR-path handoff; verify non-successful task execution and validation branches cannot invoke it.
- [x] AC5: Accepted ADRs remain immutable: corrections, reversals, and changed decisions create a new dated ADR that references and supersedes the original rather than modifying it.
  - Validate: inspect the canonical skill rules and generated target copies; verify the plan's new architecture decision supersedes rather than edits `context/decisions/2026-07-29-cross-target-workflow-skill-packages.md`.
- [x] AC6: The sibling-skill exception remains narrow: generated commands/prompts still invoke exactly one workflow skill, ordinary workflow phases remain internal, and only synchronization may invoke `sce-decision`.
  - Validate: run `nix run .#pkl-check-generated` and inspect metadata/generation contract assertions covering command routes, package inventories, and the allowed `sce-decision` reference.

### Full validation

Repository-wide checks `/validate` runs after the last task, regardless of which criterion they map to.

- `nix run .#pkl-check-generated`
- `nix flake check`

### Context sync

- `context/sce/shared-context-code-workflow.md`
- `context/sce/context-workflow-rules.md`
- `context/sce/dedup-ownership-table.md`
- `context/overview.md`
- `context/architecture.md`
- `context/patterns.md`
- `context/glossary.md`
- `context/context-map.md`
- A new dated ADR under `context/decisions/` that supersedes the no-sibling-invocation scope of `context/decisions/2026-07-29-cross-target-workflow-skill-packages.md` without modifying that accepted record.

## Constraints and non-goals

- **In scope:** The project-root `.pi/` behavioral baseline; canonical Pkl workflow/context-sync sources; shared and target renderers; generated inventory/reference checks; OpenCode skill permissions; generated OpenCode, Claude, and Pi decision packages; synchronization reports and durable workflow context.
- **Out of scope:** A `/decision` command or Pi prompt; invoking `sce-decision` from `/change-to-plan` or `/commit`; replacing current-state context with ADRs; changing implementation approval or final-validation gates; modifying previously accepted ADRs.
- **Constraints:** Preserve one decision per ADR; use `YYYY-MM-DD-<decision-slug>.md`; allow only `Proposed`, `Accepted`, `Rejected`, `Deprecated`, or `Superseded`; default to `Accepted`; run decision writing only during successful task or plan synchronization; keep stdout/user-visible layouts governed by the owning workflow; retain deterministic ephemeral generation and exact inventory checks.
- **Non-goal:** Generalize sibling-skill orchestration beyond the single `sce-decision` exception or turn every important context update into an ADR.

## Assumptions

- `sce-decision` is an internal generated skill with no independently generated command or prompt because the request limits its entrypoints to `sce-next-task` and `sce-validate` synchronization.
- During synchronization, the decision gate runs after impact discovery/classification and before current-state context edits, so a newly written ADR can be linked from the same synchronization pass.
- When both task sync and later plan sync observe the same decision, the existing ADR and its plan/task references are reused; the one-decision rule does not permit duplicate records for the same decision.
- The skill returns an internal written-or-blocked handoff containing the ADR path; it does not render an independent terminal response because the invoking synchronization phase owns user-visible reporting.

## Task stack

- [x] T01: `Add the cross-target sce-decision package` (status:done)
  - Task ID: T01
  - Goal: Author the standalone decision-writing skill and emit its complete package for OpenCode, Claude, and Pi with deterministic inventory and content coverage.
  - Boundaries (in/out of scope): In — `.pi/skills/sce-decision/` behavioral baseline; a focused canonical Pkl decision-skill source; target skill-document assembly; generator, metadata coverage, generation contract, and required-reference assertions; the supplied ADR rules and template. Out — context-sync invocation, OpenCode permission changes, user-facing commands/prompts, and durable architecture documentation.
  - Dependencies: none
  - Done when: Each target generates a complete `sce-decision` package; the skill enforces the qualifying-decision gate input, one-decision/dated-filename/status/default-accepted/immutable-accepted-ADR rules, the supplied ADR structure, and a deterministic written-or-blocked handoff; exact generated inventory checks pass without adding a command or prompt.
  - Verification notes (commands or checks): Evaluate affected Pkl modules directly with `nix develop -c pkl eval <module-path>`; generate a temporary payload and inspect all three decision packages; run `nix run .#pkl-check-generated`.
  - Completed: 2026-07-30
  - Files changed: `.pi/skills/sce-decision/SKILL.md`, `.pi/skills/sce-decision/references/adr-template.md`, `config/pkl/base/decision-skill.pkl`, `config/pkl/renderers/{opencode-content,claude-content,pi-content,metadata-coverage-check,generation-contract-check}.pkl`
  - Evidence: Direct Pkl evaluation passed for `decision-skill.pkl`, `metadata-coverage-check.pkl`, and `generation-contract-check.pkl`; a temporary payload contained exactly `SKILL.md` and `references/adr-template.md` under all three `sce-decision` packages and no command or prompt referenced `sce-decision`; the Pi baseline matched its generated package; `nix run .#pkl-check-generated` passed with a deterministic 52-file inventory.
  - Notes: The decision skill remains outside the command workflow catalog and OpenCode allowlist. Exact inventory and content assertions cover the qualifying threshold, status/default, dated path, accepted-ADR immutability, written-or-blocked handoff, and required ADR sections.

- [x] T02: `Invoke sce-decision during context synchronization` (status:done)
  - Task ID: T02
  - Goal: Add the narrow sibling-skill exception to task and plan synchronization so qualifying decisions are recorded before current-state context synchronization completes.
  - Boundaries (in/out of scope): In — `workflow-context-sync.pkl`; `workflow-next-task.pkl`; `workflow-validate.pkl`; shared composite preamble/rules; synchronization output/report data; OpenCode Code-agent skill permission derivation; reference/internalization guards; matching `.pi/skills/sce-next-task` and `.pi/skills/sce-validate` baselines. Out — invoking decision writing from non-success branches, other workflows, commands/prompts, or any non-decision sibling skill.
  - Dependencies: T01
  - Done when: Both successful synchronization roles apply the exact system-wide decision threshold, skip routine/reversible work, invoke `sce-decision` during sync when required, reuse an already-written ADR for the same decision, block synchronization when decision writing is unsafe, and include a written ADR path in synchronization evidence; generated commands still route to one workflow skill and only the documented decision exception permits a sibling invocation.
  - Verification notes (commands or checks): Evaluate affected Pkl modules directly; inspect temporary generated OpenCode/Claude/Pi `sce-next-task` and `sce-validate` skills plus OpenCode permissions; run `nix run .#pkl-check-generated`.
  - Completed: 2026-07-30
  - Files changed: `config/pkl/base/workflow-{context-sync,next-task,validate}.pkl`, `config/pkl/renderers/{workflow-composite,opencode-metadata,generation-contract-check}.pkl`, `.pi/skills/sce-{next-task,validate}/{SKILL.md,references/output.md}`
  - Evidence: Both synchronization roles now run the exact qualifying-decision gate only after successful task execution or plan validation, reuse matching ADR paths, invoke `sce-decision` once per remaining decision, propagate decision blockers as synchronization blockers before current-state context edits, and report written or reused ADR paths. The composite preamble and workflow rules permit only this synchronization-scoped sibling invocation; generated command routes remain one-to-one. OpenCode grants `sce-decision` only to the Code agent. Direct Pkl evaluation passed for the affected workflow, metadata, and generation-contract modules; temporary OpenCode, Claude, and Pi payload inspection confirmed matching workflow content, no command/prompt decision route, and the expected OpenCode permission; `nix run .#pkl-check-generated` passed with a deterministic 52-file inventory.
  - Notes: Synchronization reports include an Architecture decisions section for success, no-change, and blocked outcomes; generation guards reject `sce-decision` references outside `sce-next-task` and `sce-validate` skill entrypoints.

- [x] T03: `Record the decision-skill architecture exception` (status:done)
  - Task ID: T03
  - Goal: Establish the durable current-state contract and immutable architecture history for decision writing during synchronization.
  - Boundaries (in/out of scope): In — a new accepted dated ADR superseding only the no-sibling-invocation and exact-package-inventory scope of `2026-07-29-cross-target-workflow-skill-packages.md`; focused workflow/context-sync ownership docs; root context, glossary, patterns, map, and related inventory descriptions that the new package changes. Out — editing any accepted ADR, implementation code changes, unrelated context cleanup, or historical plan rewrites.
  - Dependencies: T02
  - Done when: A new ADR follows the requested template and links this plan, relevant tasks, the superseded accepted ADR, and current-state architecture documents; accepted historical ADRs are unchanged; durable context accurately describes the system-wide decision threshold, default acceptance status, immutable ADR policy, synchronization timing, cross-target package inventory, and sole sibling-skill exception.
  - Verification notes (commands or checks): Compare `git diff -- context/decisions/2026-07-29-cross-target-workflow-skill-packages.md` to confirm no modification; inspect ADR filename/status/template/linkage and context-map reachability; check changed context files remain focused and at or below 250 lines; run focused Markdown/path searches for stale “no sibling skill” and exact-inventory claims.
  - Completed: 2026-07-30
  - Files changed: `context/decisions/2026-07-30-synchronization-scoped-decision-writing.md`, `context/sce/context-workflow-rules.md`, `context/patterns.md`, `context/context-map.md`
  - Evidence: Added one accepted 2026-07-30 ADR that links T01–T03, the plan, implementation evidence, current-state workflow documents, and the superseded accepted ADR while limiting supersession to exact inventory and the absolute sibling-invocation prohibition. Focused context now states the qualifying threshold, synchronization-only timing, allowed/default statuses, accepted-ADR immutability, 52-path inventory, and sole sibling-skill exception. `git diff --exit-code` confirmed the superseded accepted ADR is unchanged; shell checks verified every ADR section and linked path, context-map reachability, no stale active 46-path/no-sibling claims, all relevant files at or below 250 lines, and `git diff --check` cleanliness.
  - Notes: Existing uncommitted T01/T02 implementation and synchronization edits were preserved. No application or generated-config code was changed for T03.

## Open questions

None. The user explicitly approved the sibling-skill exception, supplied the system-wide decision threshold, chose automatic acceptance unless overridden, and placed invocation during synchronization.

## Validation Report

**Status:** validated  
**Date:** 2026-07-30

### Commands run

- `nix run .#pkl-check-generated` -> exit 0 (ephemeral generation passed with the deterministic 52-file inventory)
- `nix flake check` -> exit 0 (all compatible-system repository checks passed)
- `nix run .#pkl-generate -- "$(mktemp -d)"` plus generated-package, route, synchronization-gate, and ADR inspection -> exit 0 (all acceptance inspections passed for OpenCode, Claude, and Pi)

### Scaffolding removed

- None.

### Success-criteria verification

- [x] AC1: Cross-target package inventory without a user-facing decision route -> temporary generation contained both decision-package files for OpenCode, Claude, and Pi and no decision command/prompt reference.
- [x] AC2: One qualifying dated ADR with status and content rules -> all three generated skill documents and generation checks contain the required path, default status, immutability, and ADR-section contracts.
- [x] AC3: Narrow qualifying-decision threshold -> shared/generated synchronization documents contain the required threshold and exclusions; `nix run .#pkl-check-generated` passed.
- [x] AC4: Synchronization-only invocation and ADR-path/blocker handoff -> generated next-task and validate packages for every target passed focused inspection and generation-contract checks.
- [x] AC5: Accepted ADR immutability -> the prior accepted ADR has no diff and the new dated ADR explicitly supersedes its narrowed scope.
- [x] AC6: Sole sibling-skill exception -> package/route assertions and `nix run .#pkl-check-generated` passed with the deterministic 52-file inventory.

### Failed checks and follow-ups

- None.

### Residual risks

- None identified.
