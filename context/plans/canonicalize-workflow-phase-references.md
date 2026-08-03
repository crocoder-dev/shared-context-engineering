# Plan: canonicalize-workflow-phase-references

## Change summary

Promote the current staged `.claude/skills` refactor into the canonical Pkl workflow model, then render the same package-relative phase-reference shape for Claude, Pi, and OpenCode. The four phase-based workflow skills keep control flow in `SKILL.md` while moving phase instructions and persisted-document formats into package-local Markdown references that are read only when their workflow step is reached.

This revises the current two-file workflow-package contract without restoring sibling phase skills or inter-skill result transport: each workflow remains one skill invocation with internal phase state and same-session waits. Phase-free workflows remain unchanged, and `references/output.md` remains the sole owner of human-visible gates and terminal layouts.

## Acceptance criteria

- [x] AC1: The staged Claude package shape for `sce-change-to-plan`, `sce-next-task`, `sce-validate`, and `sce-commit` is produced from canonical Pkl rather than maintained as Claude-only hand edits.
  - Validate: generate into a temporary root with `nix run .#pkl-generate -- "$(mktemp -d -t sce-gen-XXXX)"` and compare the four generated `config/.claude/skills/` packages with the intended staged `.claude/skills/` package-relative files and contents.
- [x] AC2: OpenCode, Claude, and Pi emit the same package-relative phase-reference inventory for each phase-based workflow, with only supported target frontmatter differing.
  - Validate: inspect the temporary payload and compare the relative file sets plus normalized document bodies under `config/.{opencode,claude,pi}/skills/{sce-change-to-plan,sce-next-task,sce-validate,sce-commit}/`.
- [x] AC3: Each workflow remains one self-contained skill invocation: `SKILL.md` owns phase ordering, branching, waits, and same-session resume; each phase reference is read before that phase runs; no command invokes a phase skill or transports phase state between packages.
  - Validate: inspect generated commands/prompts and the four generated workflow skills on all three targets; run the metadata and generation contract checks through `nix run .#pkl-check-generated`.
- [x] AC4: Package-local document ownership is unambiguous: phase instructions and persisted-file templates live in their named reference files, `references/output.md` alone defines user-visible layouts, and phase-free workflow plus decision-package inventories are unchanged.
  - Validate: inspect generated reference inventories and confirm the generation contract rejects stale or missing phase references, duplicated output layouts, unresolved references, and unintended changes to `sce-handover`, `sce-brownfield`, or `sce-decision`.

### Full validation

- `nix run .#pkl-check-generated`
- `nix flake check`

### Context sync

- `context/overview.md`
- `context/architecture.md`
- `context/patterns.md`
- `context/glossary.md`
- `context/sce/shared-context-plan-workflow.md`
- `context/sce/shared-context-code-workflow.md`
- `context/sce/atomic-commit-workflow.md`
- `context/context-map.md`
- A new ADR superseding the accepted two-file-package clauses in `context/decisions/2026-07-29-cross-target-workflow-skill-packages.md`, while preserving single-skill control flow and internal phase state.

## Constraints and non-goals

- **In scope:** the current staged `.claude/skills` phase-reference design; canonical workflow/Pkl models and renderers; exact metadata and generated-artifact contract checks; and matching root `.claude/skills`, `.pi/skills`, and `.opencode/skills` workflow packages.
- **Out of scope:** restoring generated phase-skill packages; changing command-to-workflow routing; changing workflow status semantics, gates, waits, or user-visible layouts; altering `sce-handover`, `sce-brownfield`, or `sce-decision` package shapes; Rust CLI behavior; and non-workflow plugins, hooks, or extensions.
- **Constraints:** preserve the current staged Claude behavior and wording unless canonical generation requires a target-neutral path/frontmatter adaptation; keep phase references package-local; read each reference before phase side effects; preserve the no-improvisation rule and output-layout deduplication guard; use canonical Pkl as the source of truth for all three targets.
- **Non-goal:** reducing total generated Markdown size. The change moves phase material out of the initially loaded `SKILL.md`; it does not delete phase behavior.

## Assumptions

- “Same shape” applies to the four phase-based workflow packages currently changed under `.claude/skills`; phase-free `sce-handover` and `sce-brownfield` do not gain empty phase references.
- The current staged Claude filenames and phase boundaries are the intended baseline: `context-load.md`, `plan-authoring.md`, `plan-template.md`, `plan-review.md`, `task-execution.md`, `context-sync.md`, `validation.md`, `validation-report.md`, and `atomic-commit.md` in their owning packages.
- Root `.claude/skills`, `.pi/skills`, and `.opencode/skills` are synchronized installed/configuration surfaces for this repository and should match the canonical generated payload for the affected packages.

## Task stack

- [x] T01: `Canonicalize and propagate phase-reference workflow packages` (status:done)
  - Task ID: T01
  - Goal: Make the staged Claude phase-reference package design canonical in Pkl and render the same affected workflow package shape for Claude, Pi, and OpenCode.
  - Boundaries (in/out of scope): In — `config/pkl/base/workflow-content.pkl`, the phase-based `workflow-{change-to-plan,next-task,validate,commit}.pkl` modules and shared `workflow-context-sync.pkl` as needed, `config/pkl/renderers/workflow-composite.pkl`, metadata/generation contract checks, and affected root target skill packages. Out — phase-free workflows, decision-package behavior, routing metadata semantics, Rust code, and non-workflow target assets.
  - Dependencies: none
  - Done when: canonical generation reproduces the intended staged Claude packages; all three targets have matching package-relative phase references and control-flow shape; exact artifact inventories and reference guards reflect the new contract; no phase behavior, gate, wait, branch, or output layout is lost; targeted generation validation passes.
  - Verification notes (commands or checks): generate pre/post temporary payloads and review package diffs; compare generated Claude output to the staged baseline; compare package-relative inventories and normalized bodies across all three targets; `nix run .#pkl-check-generated`; `nix flake check`.
  - Completed: 2026-08-03
  - Files changed: `config/pkl/base/workflow-content.pkl`, affected phase-based workflow modules, `config/pkl/renderers/{workflow-composite,metadata-coverage-check,generation-contract-check}.pkl`, and affected root `.claude`, `.pi`, and `.opencode` skill packages.
  - Evidence: Temporary generation reproduced the staged Claude package bytes and produced matching cross-target inventories and normalized bodies; `nix run .#pkl-check-generated` passed with 101 artifacts; `nix flake check` passed.
  - Notes: Phase-free workflows, the decision package, routing semantics, Rust code, and non-workflow assets were unchanged. The generated artifact contract increased intentionally from 71 to 101 paths.

## Open questions

None. The staged Claude files define the intended phase boundaries and filenames, and the request explicitly requires Pkl ownership plus cross-target parity.

## Validation Report

**Status:** validated  
**Date:** 2026-08-03

### Commands run

- `nix run .#pkl-generate -- "$(mktemp -d -t sce-gen-XXXXXX)"` plus four-package Claude diffs -> exit 0 (temporary generation matched all intended staged Claude workflow packages byte-for-byte).
- Cross-target inventory/body comparison with fixed-offset frontmatter stripping -> exit 1 (the inspection harness used different target frontmatter lengths; no generated-content mismatch was established).
- Cross-target inventory/body comparison with delimiter-based frontmatter normalization -> exit 0 (all four workflow inventories and normalized bodies matched across OpenCode, Claude, and Pi).
- Generated-to-root package diffs plus routing/reference inspection -> exit 0 (affected workflows matched installed surfaces; phase-free and decision packages were unchanged; commands route once to composite workflow skills and skills require phase references before execution).
- `nix run .#pkl-check-generated` -> exit 0 (ephemeral generation and contract checks passed for 101 artifacts; inventory SHA-256 `c29f4c0dfa028cff8aa23f69e17f4320ad3ae7fb07d2bf2a46312d43db7946df`).
- `nix flake check` -> exit 0 (all evaluated checks passed on `x86_64-linux`; zero checks required rebuilding).

### Scaffolding removed

- None.

### Success-criteria verification

- [x] AC1: The staged Claude package shape is canonical Pkl output -> temporary generation matched the four staged `.claude/skills/` packages byte-for-byte.
- [x] AC2: OpenCode, Claude, and Pi emit the same package-relative inventory and normalized bodies -> delimiter-normalized cross-target comparisons passed for all four workflows and all references.
- [x] AC3: Each workflow remains one self-contained invocation with internal phase control -> generated routing invokes only each composite workflow skill, generated skills retain same-session state and require references before phase execution, and `nix run .#pkl-check-generated` passed.
- [x] AC4: Reference ownership and unaffected package inventories remain correct -> generated packages matched root surfaces, phase-free and decision packages were unchanged, and the generation contract passed its 101-artifact inventory and ownership guards.

### Failed checks and follow-ups

- None.

### Residual risks

- Cross-system flake outputs for `aarch64-darwin`, `aarch64-linux`, and `x86_64-darwin` were not evaluated by the host-specific `nix flake check`.
