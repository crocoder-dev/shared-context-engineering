# Plan: emit-composite-workflow-references

## Change summary

Complete the composite workflow package inventories emitted by the canonical Pkl
sources. The generated `/commit` packages currently contain
`references/atomic-commit.md` and `references/output.md`, but the phase document
still points at `references/commit-message-style.md` and
`references/commit-contract.yaml` without emitting either file. The generated
`/next-task` and `/validate` packages have the same missing-document problem for
`references/sync-report.md`.

Add those package-local documents to the composite inventories and remove the
stale `references/validation-result.md` name from validation content. The
validation result is already emitted as `references/output.md`; retaining the
old name makes the generated validation references point at a file that does not
exist. Update the derived inventory and generation-contract assertions so these
missing-file regressions fail deterministically.

## Acceptance criteria

- [x] AC1: Every generated `sce-commit` package contains
      `SKILL.md`, `references/atomic-commit.md`, `references/output.md`,
      `references/commit-message-style.md`, and `references/commit-contract.yaml`,
      and the commit phase reference resolves both newly emitted documents.
  - Validate: Generate an ephemeral payload and inspect the OpenCode, Claude, and
    Pi `sce-commit` directories with `find`; verify the two files exist and
    `grep` confirms the references resolve within each package.
- [x] AC2: Every generated `sce-next-task` and `sce-validate` package contains a
      package-local `references/sync-report.md`, and each context-sync reference
      points to that emitted document.
  - Validate: Generate an ephemeral payload and inspect all six target package
    directories; verify `references/sync-report.md` exists and run a package-local
    reference check for every `references/sync-report.md` citation.
- [x] AC3: No generated workflow document or active canonical composite reference
      names `references/validation-result.md`; validation documents consistently
      use the emitted `references/output.md` result document instead.
  - Validate: Generate an ephemeral payload and `grep` all generated workflow
    documents for `validation-result.md` (expect no matches), then inspect the
    canonical validation module for the same stale path.
- [x] AC4: The generated artifact inventory and metadata coverage contract include
      the new package-local documents and reject missing or extra paths
      deterministically.
  - Validate: `nix run .#pkl-check-generated` reports the updated exact inventory
    successfully, including the increased artifact count and complete workflow
    document coverage.

### Full validation

- `nix run .#pkl-check-generated`
- `nix flake check`

### Context sync

- `context/overview.md` (current generated workflow-package inventory and the
  absence of stale validation-result references)
- `context/architecture.md` (canonical Pkl package documents, composite
  materialization, and generated artifact-count contract)
- `context/patterns.md` (package-local reference resolution and exact inventory
  guard rules)
- `context/glossary.md` (workflow output/reference terminology and package
  shape)

## Constraints and non-goals

- **In scope:** canonical workflow Pkl modules for commit, next-task, validate,
  and context-sync; `config/pkl/renderers/metadata-coverage-check.pkl`;
  `config/pkl/renderers/generation-contract-check.pkl`; and any narrowly
  necessary durable context updates during `/validate` synchronization.
- **Out of scope:** Rust CLI behavior, target-specific runtime plugins, changes to
  workflow gates or status semantics, new dependencies, and manual edits to the
  root `.claude/`, `.pi/`, or `.opencode/` runtime-managed mirrors.
- **Constraints:** Use typed package/composite document inventories and
  mode-aware semantic references; do not restore phase-skill packages or solve
  missing references with prose-wide rewriting. Generated output remains
  ephemeral and must be checked in a temporary root.
- **Non-goal:** Redesigning the validation report or synchronization report
  content; this change only makes their emitted paths and citations resolve.

## Assumptions

- `references/output.md` is the composite validation result document, so replacing
  `references/validation-result.md` with `references/output.md` preserves the
  validation contract rather than changing its layout.
- The requested emitted files belong in each target's composite workflow package,
  not in the non-generated root runtime mirrors; canonical Pkl changes and
  ephemeral generation are the source of truth.
- The existing exact artifact count must increase by the number of newly emitted
  target files: two commit documents plus one synchronization report for each of
  the next-task and validate packages across three targets.

## Task stack

- [x] T01: `Emit missing commit and synchronization reference documents` (status:done)
  - Task ID: T01
  - Goal: Add `commit-message-style.md`, `commit-contract.yaml`, and the
    role-specific `sync-report.md` documents to the composite package inventories
    and update derived document-path and exact-artifact-count contracts.
  - Boundaries (in/out of scope): In —
    `config/pkl/base/workflow-commit.pkl`,
    `config/pkl/base/workflow-next-task.pkl`,
    `config/pkl/base/workflow-validate.pkl`,
    `config/pkl/renderers/metadata-coverage-check.pkl`, and
    `config/pkl/renderers/generation-contract-check.pkl`. Out — validation-result
    cleanup and all generated/root target trees.
  - Dependencies: none
  - Done when: Ephemeral OpenCode, Claude, and Pi payloads each contain the two
    requested commit references plus the applicable synchronization report in
    the correct package-relative paths, and the metadata/generation contracts
    derive and accept the expanded inventory.
  - Verification notes (commands or checks): Evaluate the affected Pkl modules
    through `nix develop -c pkl eval`; generate a temporary payload and inspect
    the six affected package inventories; run the targeted generated-output
    contract check.
  - Completed: 2026-08-12
  - Files changed: `config/pkl/base/workflow-commit.pkl`, `config/pkl/base/workflow-next-task.pkl`, `config/pkl/base/workflow-validate.pkl`, `config/pkl/renderers/metadata-coverage-check.pkl`, `config/pkl/renderers/generation-contract-check.pkl`
  - Evidence: Affected Pkl modules evaluated successfully; generated payload inspection confirmed all six affected package inventories and new citations; `nix run .#pkl-check-generated` passed with 113 files.
  - Notes: Composite inventories now emit the two commit support documents and role-specific synchronization reports across OpenCode, Claude, and Pi; the exact artifact contract is updated from 101 to 113 paths. Stale validation-result cleanup remains assigned to T02.

- [x] T02: `Remove stale validation-result references` (status:done)
  - Task ID: T02
  - Goal: Make validation package references resolve to `references/output.md`
    and remove the obsolete `references/validation-result.md` document/reference
    from the canonical validation package representation.
  - Boundaries (in/out of scope): In —
    `config/pkl/base/workflow-validate.pkl` and narrowly related generated
    reference assertions. Out — changing validation output fields, report
    layouts, validation status behavior, or synchronization semantics.
  - Dependencies: T01
  - Done when: No generated workflow document or active canonical composite text
    names `references/validation-result.md`; the validation package still emits
    exactly one `references/output.md` containing the existing result layouts,
    and all validation references resolve locally.
  - Verification notes (commands or checks): Generate an ephemeral payload;
    `grep` generated workflow documents and the affected canonical Pkl source
    for `validation-result.md` expecting no matches; inspect the validation
    package inventory and run `nix run .#pkl-check-generated`.
  - Completed: 2026-08-12
  - Files changed: `config/pkl/base/workflow-validate.pkl`
  - Evidence: Repointed the validation result reference and package document to
    `references/output.md`, removed all stale `validation-result.md` text from
    the canonical validation module, and preserved the existing result layouts.
    Ephemeral generation produced `sce-validate/references/output.md` without a
    `references/validation-result.md` file for OpenCode, Claude, and Pi.
  - Verification evidence: `nix develop -c pkl eval
    config/pkl/base/workflow-validate.pkl` passed; generated workflow inspection
    found no `validation-result.md` matches and confirmed the output document;
    `nix run .#pkl-check-generated` passed with 113 files.

## Open questions

None. The requested document names and the existing Pkl package/composite
boundary determine the implementation surface; the only local choice is to
preserve `references/output.md` as the validation result owner, which matches
current generated inventories and repository patterns.

## Validation Report

**Status:** validated  
**Date:** 2026-08-12

### Commands run

- `nix run .#pkl-check-generated` -> exit 0 (ephemeral generation and exact 113-file contract passed)
- `nix flake check` -> exit 0 (all flake checks passed)
- `nix run .#pkl-generate -- <temporary-root>` -> exit 0 (generated OpenCode, Claude, and Pi payloads)
- Ephemeral `find`/`grep` package acceptance inspection -> exit 0 (AC1–AC3 assertions passed across all target packages)

### Scaffolding removed

- None.

### Success-criteria verification

- [x] AC1: Every generated `sce-commit` package contains the required support documents and resolves both commit references -> all three target packages contained the five required files and both citations resolved.
- [x] AC2: Every generated `sce-next-task` and `sce-validate` package contains and cites `references/sync-report.md` -> the document existed in all six packages and each package-local context-sync reference cited it.
- [x] AC3: No generated workflow document or active canonical validation text names `references/validation-result.md` -> generated workflow inspection and canonical validation-module inspection found no matches.
- [x] AC4: The generated artifact inventory and metadata coverage contract include the expanded package documents -> `nix run .#pkl-check-generated` passed with the exact 113-file inventory.

### Failed checks and follow-ups

- None.

### Residual risks

- None identified.
