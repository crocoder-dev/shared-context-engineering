# Plan: fix-composite-blank-line-joins-and-dedup-guard

## Change summary

`context/plans/shorten-generated-workflow-docs.md` removed the duplicated
layouts and dead references from the four composite Claude workflow packages,
but two small items were fenced off from that plan and remain:

1. Stray double/triple blank lines in composite-rendered `SKILL.md` output.
   The root cause is in `config/pkl/base/workflow-content.pkl`:
   `packageOnlyBlock`/`compositeOnlyBlock` already append their own `"\n\n"`
   separator, but several call sites in `config/pkl/base/workflow-next-task.pkl`,
   `workflow-validate.pkl`, `workflow-change-to-plan.pkl`,
   `workflow-commit.pkl`, and `workflow-handover.pkl` also leave a blank source
   line before the interpolation. When the active render mode makes that block
   render empty (e.g. a `packageOnlyBlock` under composite mode), the blank
   source line plus the following `"\n\n"`-joined section produces 2-3 blank
   lines instead of one. Confirmed today in the generated
   `.claude/skills/*/SKILL.md`: `sce-next-task` (5 occurrences), `sce-validate`
   (3), `sce-change-to-plan` (2), `sce-commit` (2), `sce-handover` (1, around
   its `persistedFormatInline` reference) — 13 total. `sce-decision` does not
   use the `packageOnlyBlock`/`compositeOnlyBlock`/`semanticReference` inline
   pattern and shows no occurrence; it needs no source fix, but the new
   contract check in item 2 still covers it since it inspects every generated
   workflow document.
2. Nothing in `config/pkl/renderers/generation-contract-check.pkl` guards
   against the `SKILL.md`/`references/output.md` dedup (from the prior plan)
   regressing. A future edit could reintroduce an inlined layout that
   duplicates an `output.md` section, or reintroduce a stray multi-blank-line
   join, and the generation contract would not catch it.

This plan fixes the source-level blank-line pattern (formatting only, no
instruction changes) and adds two contract checks, following the existing
`generation-contract-check.pkl` / `fixtures/*.pkl` pattern already used for
`workflow-references`, `decision-invocation`, etc.

## Acceptance criteria

- [x] AC1: No generated workflow `SKILL.md`, across all three targets
      (`.opencode`, `.claude`, `.pi`) and all five workflows (`sce-change-to-plan`,
      `sce-next-task`, `sce-validate`, `sce-commit`, `sce-handover`), contains
      two or more consecutive blank lines. `sce-decision` is confirmed clean
      and unchanged.
  - Validate: generate with `nix run .#pkl-generate -- "$(mktemp -d -t sce-gen-XXXX)"`,
    then confirm no `skills/**/SKILL.md` file matches a run of 2+ blank lines
    (e.g. `awk` blank-run scan used during planning), for all 15 workflow
    packages (5 workflows × 3 targets).
- [x] AC2: `nix run .#pkl-check-generated` includes a `no-blank-line-runs`
      contract check that fails if any generated workflow document contains
      two or more consecutive blank lines, with a negative fixture proving the
      check actually fires.
  - Validate: `nix run .#pkl-check-generated`; inspect
    `config/pkl/renderers/fixtures/blank-line-run-check.pkl` and its wiring in
    `config/pkl/check-generated.sh`.
- [x] AC3: `nix run .#pkl-check-generated` includes an `output-dedup` contract
      check that fails if any `SKILL.md` reproduces a `references/output.md`
      section's fenced layout verbatim, with a negative fixture proving it
      fires.
  - Validate: `nix run .#pkl-check-generated`; inspect
    `config/pkl/renderers/fixtures/output-dedup-check.pkl` and its wiring in
    `config/pkl/check-generated.sh`.
- [x] AC4: No step, gate, branch, wait, or prohibition in any of the five
      workflows changes. Only whitespace in the rendered Markdown and new
      contract-check machinery change.
  - Validate: diff each generated `.claude/skills/sce-*/SKILL.md` against the
    pre-change generation root; every non-whitespace line is unchanged.

### Full validation

- `nix run .#pkl-check-generated`
- `nix flake check`

### Context sync

- `context/patterns.md` (generation-contract-check.pkl's list of asserted
  contracts, if it names them individually)
- `context/architecture.md` (composite rendering description, if it names the
  specific contract checks)

## Constraints and non-goals

- **In scope:** `config/pkl/base/workflow-next-task.pkl`,
  `config/pkl/base/workflow-validate.pkl`,
  `config/pkl/base/workflow-change-to-plan.pkl`,
  `config/pkl/base/workflow-commit.pkl`,
  `config/pkl/base/workflow-handover.pkl`,
  `config/pkl/renderers/generation-contract-check.pkl`,
  `config/pkl/renderers/fixtures/`, `config/pkl/check-generated.sh`. The new
  contract checks in T02 run over every generated workflow document,
  including `sce-decision`, even though it needs no T01 source fix.
- **Out of scope:** any wording, instruction, step, or layout change;
  re-opening any non-goal already fenced off by
  `shorten-generated-workflow-docs.md` (context-sync phase internal
  duplication, ownership lists, per-phase boilerplate, etc.).
- **Constraints:** Preserve the existing `packageOnlyBlock`/`compositeOnlyBlock`
  separator convention documented in `workflow-content.pkl`; fix call sites,
  not the shared helper's contract, unless a call-site-only fix cannot express
  the intended output.
- **Non-goal:** Introducing a general Markdown post-processing/normalization
  pass over rendered output. The fix stays at the Pkl template-authoring level,
  consistent with the "no post-processing prose" rule in `context/patterns.md`.

## Task stack

- [x] T01: `Remove stray blank-line joins around conditional blocks in the five composite workflows` (status:done)
  - Task ID: T01
  - Goal: Eliminate all 13 known double/triple-blank-line occurrences (and any
    others of the same shape) in the generated `sce-next-task`, `sce-validate`,
    `sce-change-to-plan`, `sce-commit`, and `sce-handover` `SKILL.md` files by
    fixing the source blank-line placement around
    `packageOnlyBlock`/`compositeOnlyBlock`/`semanticReference` interpolations
    in the five base workflow `.pkl` files. `sce-decision` is inspected and
    confirmed to need no change (it doesn't use this inline pattern).
  - Boundaries (in/out of scope): In — blank-line placement in
    `workflow-next-task.pkl`, `workflow-validate.pkl`,
    `workflow-change-to-plan.pkl`, `workflow-commit.pkl`,
    `workflow-handover.pkl`, and, only if the fix genuinely belongs there, the
    `packageOnlyBlock`/`compositeOnlyBlock` helpers in `workflow-content.pkl`.
    Out — any instruction, heading, or section text change; any change to
    `decision-skill.pkl`/`sce-decision` beyond confirming it needs none.
  - Dependencies: none
  - Done when: generating all three targets produces no `SKILL.md` with 2+
    consecutive blank lines, and a line-level diff against the current
    generated output shows only blank-line removals (no other line added,
    removed, or reworded).
  - Verification notes (commands or checks): `nix run .#pkl-generate -- "$(mktemp -d -t sce-gen-XXXX)"`
    then scan each `skills/**/SKILL.md` for 2+ consecutive blank lines; diff
    against current `.claude/skills/`, `.opencode/skills/`, `.pi/skills/` to
    confirm only blank lines changed.
  - Evidence: Fixed 11 call sites across the five base workflow files by
    gluing a trailing prose/bullet line directly onto the following
    `packageOnlyBlock`/`compositeOnlyBlock`/`semanticReference` interpolation
    (moving any needed leading blank line for package mode inside the
    interpolated block argument instead of leaving it as a raw template line),
    so an empty composite render leaves no orphaned blank line behind:
    `workflow-next-task.pkl` (3 sites: plan-review boundaries/Completion,
    execution boundaries/Completion, execution-contract intro merge),
    `workflow-validate.pkl` (1 site), `workflow-change-to-plan.pkl` (2 sites),
    `workflow-commit.pkl` (2 sites), `workflow-handover.pkl` (1 site,
    `persistedFormatInline`). Also fixed the shared `skillBody` helper in
    `workflow-context-sync.pkl` (used by both `sce-task-context-sync` and
    `sce-plan-context-sync`, imported by `workflow-next-task.pkl` and
    `workflow-validate.pkl`), which joined a possibly-empty `completion`
    section with `"\n\n"` unconditionally; changed it to filter out
    empty-rendering sections before joining. This file was not in the
    plan's originally listed in-scope file set, but the actual T01 defect for
    `sce-next-task` and part of `sce-validate` lived entirely inside its
    shared `skillBody` join, invisible from any call site in the five listed
    workflow files, so the fix (whitespace/join-logic only, no
    instruction/text change) belonged there per the task's own allowance for
    touching a shared helper when a call-site-only fix cannot express the
    intended output.
  - Verification: `nix run .#pkl-generate -- "$(mktemp -d -t sce-gen-XXXX)"`,
    then scanned every `skills/**/SKILL.md` (all 3 targets × 5 workflows) for
    runs of 2+ consecutive blank lines — none found; `sce-decision`'s
    generated `SKILL.md` diffed byte-identical against the pre-change
    committed copy. Diffed each of the 5 fixed workflows' generated
    `SKILL.md` against the pre-change committed copy with blank lines
    stripped from both sides — zero non-blank-line differences in any of the
    3 targets. `git diff --stat` over `.claude/.opencode/.pi` shows exactly
    15 files changed, 66 deletions, 0 insertions. `nix run
    .#pkl-check-generated` passed. `nix flake check` passed (all 5 checks).

- [x] T02: `Add blank-line-run and output-dedup contract checks with negative fixtures` (status:done)
  - Task ID: T02
  - Goal: Guard the T01 fix and the earlier `SKILL.md`/`output.md` dedup
    against regressing, by adding two checks to
    `generation-contract-check.pkl` — `no-blank-line-runs` (fails on any 2+
    consecutive blank lines in a generated workflow document) and
    `output-dedup` (fails if any `SKILL.md` reproduces a
    `references/output.md` fenced layout verbatim) — each with a negative
    fixture under `config/pkl/renderers/fixtures/` wired into
    `config/pkl/check-generated.sh`, following the existing
    `forbidden-workflow-reference-check.pkl` pattern.
  - Boundaries (in/out of scope): In — the two new contract checks, their
    negative fixtures, and `check-generated.sh` wiring. Out — changing any
    existing contract check's behavior; re-adding artifact paths (this task
    adds no new generated files, only check logic).
  - Dependencies: T01
  - Done when: `nix run .#pkl-check-generated` runs and passes both new checks
    against the (now clean) generated output, and both negative fixtures fail
    with the expected diagnostic when run directly, proving each check
    actually detects the violation it targets.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`;
    `pkl eval config/pkl/renderers/fixtures/blank-line-run-check.pkl` and
    `pkl eval config/pkl/renderers/fixtures/output-dedup-check.pkl` each fail
    with the expected diagnostic message.
  - Evidence: Added `hidden fencedBlocksIn` (splits document text on the
    triple-backtick fence; the odd-indexed segments after `split` are the
    fenced bodies, reassembled with their fence markers) plus two `local`
    assert functions to `config/pkl/renderers/generation-contract-check.pkl`:
    `assertNoBlankLineRuns` (fails if any entry in `workflowDocuments` —
    already every `SKILL.md`/`references/output.md` across all 3 targets, all
    5 composite workflows, and `sce-decision` — contains the substring
    `"\n\n\n"`, i.e. 2+ consecutive blank lines) and
    `assertOutputNotDuplicated` (for every `references/output.md` entry,
    derives its sibling `SKILL.md` path and fails if any of its fenced blocks
    appear verbatim, fence markers included, in that `SKILL.md` text).
    Registered both as `["no-blank-line-runs"]` and `["output-dedup"]` in
    `contractChecks`. Added
    `config/pkl/renderers/fixtures/blank-line-run-check.pkl` and
    `config/pkl/renderers/fixtures/output-dedup-check.pkl`, each reimplementing
    its check inline against tampered `contract.workflowDocuments` data
    (matching the existing `forbidden-workflow-reference-check.pkl` pattern,
    since the assert functions are `local` and not importable). Wired both
    into `config/pkl/check-generated.sh` via two new
    `expect_pkl_fixture_failure` calls alongside the existing three.
  - Verification: `pkl eval config/pkl/renderers/generation-contract-check.pkl`
    — both new checks pass against the current generated output
    (`no-blank-line-runs`: "no blank-line runs"; `output-dedup`: "no output.md
    layout duplicated"). `pkl eval
    config/pkl/renderers/fixtures/blank-line-run-check.pkl` and `pkl eval
    config/pkl/renderers/fixtures/output-dedup-check.pkl` each fail as
    expected, with the exact configured diagnostic string present in the
    error output (confirmed this matches the pre-existing
    `forbidden-workflow-reference-check.pkl` fixture's identical `error()`
    reporting shape in this Pkl version, so the negative-fixture pattern is
    unchanged). `./config/pkl/check-generated.sh` run directly inside the Nix
    dev shell passed end-to-end (all five `expect_pkl_fixture_failure` calls,
    full ephemeral generation, 61 files). `git diff --stat` confirms only the
    two in-scope files changed for this task
    (`config/pkl/check-generated.sh`, +6/-0;
    `config/pkl/renderers/generation-contract-check.pkl`, +28/-0) plus the two
    new fixture files — no other file touched.

## Validation Report

**Status:** validated  
**Date:** 2026-07-30

### Commands run

- `nix run .#pkl-check-generated` -> exit 0 (ephemeral Pkl generation passed: 61 files; all contract checks, including `no-blank-line-runs` and `output-dedup`, passed)
- `nix flake check` -> exit 0 (all checks passed, including `pkl-generated`, `cli-tests`, `cli-clippy`, `cli-generated-input`)
- `nix run .#pkl-generate -- "$(mktemp -d -t sce-gen-XXXX)"` -> exit 0 (fresh ephemeral generation for direct AC1/AC4 inspection)
- Blank-run scan (`awk`) over all 18 generated `skills/**/SKILL.md` files (6 workflows × 3 targets) -> 0 matches for 2+ consecutive blank lines
- Diff of each generated `.claude/.opencode/.pi` `skills/sce-*/SKILL.md` against the currently staged committed copy, with runs of blank lines squeezed on both sides -> 0 non-blank-line differences across all 18 files

### Scaffolding removed

- None. (Ephemeral generation directories used for validation were created under system tmp via `mktemp -d` and removed after inspection; none were committed or left behind.)

### Success-criteria verification

- [x] AC1: No generated workflow `SKILL.md` contains 2+ consecutive blank lines -> confirmed via fresh `nix run .#pkl-generate` output scanned across all 18 files (6 workflows incl. `sce-decision` × 3 targets), 0 matches
- [x] AC2: `no-blank-line-runs` contract check exists and fires -> confirmed via `nix run .#pkl-check-generated` passing, and `config/pkl/renderers/fixtures/blank-line-run-check.pkl` present and wired into `config/pkl/check-generated.sh` (per T02 evidence, negative fixture fails as expected)
- [x] AC3: `output-dedup` contract check exists and fires -> confirmed via `nix run .#pkl-check-generated` passing, and `config/pkl/renderers/fixtures/output-dedup-check.pkl` present and wired into `config/pkl/check-generated.sh` (per T02 evidence, negative fixture fails as expected)
- [x] AC4: Only whitespace changed, no instruction/step/gate change -> confirmed via diff of all 18 generated `SKILL.md` files against the staged committed copies with blank lines squeezed on both sides: 0 non-blank-line differences

### Failed checks and follow-ups

- None.

### Residual risks

- None identified.

## Open questions

None. Both items were already scoped and confirmed present by a prior
analysis pass (12 blank-line occurrences located and confirmed by direct
inspection of generated `SKILL.md` files; the missing dedup-regression guard
was called out explicitly in `shorten-generated-workflow-docs.md`'s own
follow-up notes). The fix is mechanical and the two new checks reuse an
existing, already-proven pattern (`generation-contract-check.pkl` +
`fixtures/*.pkl` + `check-generated.sh`), so there is no design choice left
to surface here.
