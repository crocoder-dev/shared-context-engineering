# Plan: simplify-task-context-sync-report

## Change summary

Simplify only the `sce-task-context-sync` report contract: stop rendering context-impact classification and the mandatory root-pass checklist, and add an `Updated files` section listing changed files outside `context/`. The plan-level context-sync report and its `Plan context requirements` section remain unchanged.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the check that proves it. `/validate` runs these checks; no task in the stack performs final validation.

- [x] AC1: Every task context-sync report variant shows the implementation's changed non-context files under `Updated files` and does not render a context-impact classification or root-pass checklist.
  - Validate: Generate the workflow payload and inspect the task `references/sync-report.md` output for all three variants.
- [x] AC2: The plan context-sync report contract retains its existing classification, plan context requirements, and root-pass sections.
  - Validate: Generate the workflow payload and compare the plan `references/sync-report.md` structure with the unchanged plan-specific contract.
- [x] AC3: The project-root `.pi` task sync report baseline matches the canonical Pkl-rendered task report contract.
  - Validate: `nix run .#pkl-check-generated`

### Full validation

Repository-wide checks `/validate` runs after the last task, regardless of which criterion they map to.

- `nix run .#pkl-check-generated`
- `nix flake check`

### Context sync

- `context/sce/context-workflow-rules.md` and `context/sce/shared-context-code-workflow.md` must describe the task synchronization behavior accurately if their report-output wording is affected.
- `context/architecture.md` and `context/patterns.md` must remain accurate about shared Pkl ownership and the project-root `.pi` behavioral baseline.

## Constraints and non-goals

- **In scope:** The task-specific report template in `config/pkl/base/workflow-context-sync.pkl` and `.pi/skills/sce-task-context-sync/references/sync-report.md`, including all task report status variants.
- **Out of scope:** The plan context-sync report template and other workflow result/report contracts.
- **Constraints:** Preserve the mandatory root-pass synchronization behavior; remove only its rendered report section. `Updated files` lists changed files from the execution handoff after excluding every path under `context/`.
- **Non-goal:** Removing context-impact classification from the task execution handoff or changing how task context synchronization decides which context to inspect or edit.

## Assumptions

- “Only in task context sync” means `Plan context requirements` requires no change because that section exists only in the plan report.
- `Updated files` belongs in each task report status variant so the invoking workflow can see non-context implementation changes whether synchronization is synced, unnecessary, or blocked.

## Task stack

- [x] T01: `Simplify the task context-sync report contract` (status:complete)
  - Task ID: T01
  - Goal: Update the canonical and project-root task context-sync report templates to omit classification/root-pass output and report changed non-context files.
  - Boundaries (in/out of scope): In — task-specific report composition in `workflow-context-sync.pkl`, the `.pi` task report baseline, and focused contract assertions/inspection. Out — plan report output and synchronization decision behavior.
  - Dependencies: none
  - Done when: All task report variants contain `Updated files` with non-context-file semantics, omit `Classification` and `Root pass`, the plan report remains structurally unchanged, and targeted generation/parity checks pass.
  - Verification notes (commands or checks): Evaluate `config/pkl/base/workflow-context-sync.pkl`; inspect or extract both task and plan `references/sync-report.md` documents; run `nix run .#pkl-check-generated`.
  - Implementation evidence: Split task-specific report fragments from the shared plan report structure, added `Updated files` filtering semantics to all three task variants, removed rendered task classification/root-pass sections, and updated the project-root `.pi` baseline.
  - Verification evidence:
    - `nix develop -c pkl eval config/pkl/base/workflow-context-sync.pkl` — passed; generated task and plan packages evaluated successfully.
    - Generated report inspection — passed; all three task variants contain `Updated files`, omit classification/root-pass sections, and the generated plan report is unchanged from its project-root baseline.
    - `nix run .#pkl-check-generated` — passed; deterministic ephemeral generation produced 73 files.

## Open questions

None. The clarification limits the change to task context sync, and the repository already has a role-parameterized source seam that can preserve the plan report contract.

## Validation Report

**Status:** validated  
**Date:** 2026-07-28

### Commands run

- `nix develop -c pkl eval -f json config/pkl/base/workflow-context-sync.pkl` plus generated-report inspection and baseline comparison -> exit 0 (all three task variants contain `Updated files` and omit context-impact/root-pass sections; the plan report retains context impact, plan context requirements, and root pass; both project-root `.pi` report baselines are byte-identical to canonical output)
- `nix run .#pkl-check-generated` -> exit 0 (deterministic ephemeral generation passed for 73 files; inventory SHA-256 `bad74106e457b66ac461e1f39a3e89be2bf2345276295d55dd35cf2259fdb64c`)
- `nix flake check` -> exit 0 (all checks passed on x86_64-linux)

### Scaffolding removed

None.

### Success-criteria verification

- [x] AC1: Every task context-sync report variant shows changed non-context files under `Updated files` and omits context-impact classification and root-pass output -> generated report inspection found three `Updated files` sections and no task `Context impact` or `Root pass` sections.
- [x] AC2: The plan context-sync report retains classification, plan context requirements, and root-pass sections -> generated report inspection found the expected sections in both applicable plan variants and confirmed byte identity with `.pi/skills/sce-plan-context-sync/references/sync-report.md`.
- [x] AC3: The project-root `.pi` task sync report baseline matches the canonical Pkl-rendered task report contract -> direct byte comparison with canonical generated output passed.

### Failed checks and follow-ups

None.

### Residual risks

- `nix run .#pkl-check-generated` validates deterministic generation but not project-root baseline parity; AC3 therefore also used a direct byte comparison.
