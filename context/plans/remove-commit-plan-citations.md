# Plan: remove-commit-plan-citations

## Change summary

The `sce-commit` workflow currently has a plan-citation rule. When the staged
files include `context/plans/*.md`, the commit body must cite the affected plan
slug and every updated task ID. In regular mode, an unclear citation returns
`blocked`. In bypass mode, the citation is best-effort.

This plan removes that rule from every `sce-commit` surface. Commit messages
no longer cite plan slugs or task IDs, and plan-citation ambiguity no longer
blocks a regular-mode proposal. The change replaces existing behavior. Other
commit rules stay unchanged, including issue-reference rules and the boundary
that forbids running plan, task, or validation workflows from `/commit`.

Canonical behavior lives in `config/pkl/base/workflow-commit.pkl`. The
project-root `.pi/` package is the behavioral baseline. The committed
`.claude/`, `.opencode/`, and `.agents/` `sce-commit` packages carry the same
text and must match.

## Acceptance criteria

- [x] AC1: No `sce-commit` surface instructs the agent to write plan slugs, task IDs, or plan citations into commit messages.
  - Validate: `grep -rn -i -E "plan-citation|plan citation|plan slug|task ID" config/pkl/base/workflow-commit.pkl .pi/skills/sce-commit .claude/skills/sce-commit .opencode/skills/sce-commit .agents/skills/sce-commit` returns no matches.
- [x] AC2: Regular mode has no `blocked` branch for plan-citation ambiguity, and bypass mode has no best-effort citation step.
  - Validate: inspect `references/atomic-commit.md` in each root `sce-commit` package. The step list has no plan-citation step, the step numbers are sequential, and the validation and boundary lists have no citation bullets.
- [x] AC3: Unrelated commit rules remain intact.
  - Validate: `grep -n "Do not run plan, task, or validation workflows" .claude/skills/sce-commit/SKILL.md` matches, and the boundary list in `.claude/skills/sce-commit/references/atomic-commit.md` still forbids invented issue references.
- [x] AC4: Generated output from the canonical Pkl source matches the committed root packages.
  - Validate: generate into a temporary root with `nix run .#pkl-generate -- "$(mktemp -d)"` and diff its `sce-commit` packages against the root `.claude/`, `.opencode/`, `.pi/`, and `.agents/` packages.

### Full validation

- `nix run .#pkl-check-generated`
- `nix flake check`

### Context sync

- `context/sce/atomic-commit-workflow.md` must drop the `Plan citations` section, the plan-citation ownership bullet, and the bypass-mode citation relaxation.
- `context/glossary.md` must drop the plan-citation clause from `atomic commit bypass mode`.
- `context/context-map.md` must drop "plan-citation" from the atomic commit workflow annotation.

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** `config/pkl/base/workflow-commit.pkl`; the root `sce-commit` packages under `.pi/`, `.claude/`, `.opencode/`, and `.agents/`; Pkl contract fixtures only if they assert the removed text.
- **Out of scope:** other workflows, the commit-msg co-author trailer hook, issue-reference rules, and files under `context/tmp/`.
- **Constraints:** edit the canonical Pkl source and the `.pi/` baseline first. Do not use generated target Markdown as an authoring source.
- **Non-goal:** adding a new rule that forbids plan or task references in commit messages.

## Assumptions

- "Commit metadata" means the commit message subject and body that `sce-commit` writes.
- The boundary bullet `Invent plan slugs, task IDs, or issue references.` becomes `Invent issue references.`
- The line `Do not run plan, task, or validation workflows` is about workflow execution, not commit metadata, so it stays.

## Task stack

- [x] T01: `Remove the plan-citation rule from sce-commit` (status:done)
  - Task ID: T01
  - Scope: In — remove step 5 "Apply the plan-citation rule" and renumber later steps; remove the plan-citation validation bullet, the bypass-mode citation boundary bullet, and plan/task wording in the invent bullet; remove "Applying the plan-citation body rule" from the regular-path ownership list and "the plan-citation body rule" from the internal skill description; apply the same edits to `.pi/`, `.claude/`, `.opencode/`, and `.agents/` `sce-commit` packages. Out — durable context edits, other workflows.
  - Dependencies: none
  - Done when: AC1, AC2, and AC3 hold, and temporary Pkl generation reproduces the committed root `sce-commit` packages.
  - Verify: the AC1 `grep` returns nothing; `nix run .#pkl-generate -- "$(mktemp -d)"` followed by a diff of the `sce-commit` packages shows no difference; `nix run .#pkl-check-generated` passes.
  - Completed: 2026-11-27
  - Files changed: `config/pkl/base/workflow-commit.pkl`, `.pi/skills/sce-commit/references/atomic-commit.md`, `.claude/skills/sce-commit/references/atomic-commit.md`, `.opencode/skills/sce-commit/references/atomic-commit.md`, `.agents/skills/sce-commit/references/atomic-commit.md`
  - Result: Removed the "Apply the plan-citation rule" step and renumbered "Propose split guidance" to step 5 and "Validate the result" to step 6. Removed the plan slug/task ID validation bullet and the bypass-mode plan-citation boundary bullet. Changed the invent bullet to `Invent issue references.` In the Pkl source, also removed the plan-citation ownership bullet and the plan-citation clause from the internal atomic-commit skill description. Those two Pkl surfaces render only into the internal skill, not into the root packages. Pkl contract fixtures were not changed because they only reference "layout citation".
  - Verify: AC1 grep returned no matches (passed). `nix run .#pkl-generate -- "$(mktemp -d)"` wrote packages under `<tmp>/config/{.claude,.opencode,.pi,.agents}/skills/sce-commit`, and `diff -r` against the root packages showed no difference (passed). `nix run .#pkl-check-generated` passed (157 files). AC2: `atomic-commit.md` steps are numbered 1–6 with no citation step (passed). AC3: the SKILL.md boundary line matches and `Invent issue references.` is still present (passed).
  - Context impact: localized to the atomic commit workflow. `context/sce/atomic-commit-workflow.md`, `context/glossary.md` (`atomic commit bypass mode`), and `context/context-map.md` still describe the removed plan-citation rule.
  - Context synchronization: synced

## Open questions

- Should `sce-commit` forbid plan slugs and task IDs in commit messages explicitly? This plan only removes the instruction. Without a prohibition, an agent may still add plan references on its own.

## Validation Report

**Status:** validated  
**Date:** 2026-11-27

### Commands run

- `nix run .#pkl-check-generated` -> exit 0 (ephemeral Pkl generation passed: 157 files)
- `nix flake check` -> exit 0 (all 7 flake checks passed, including clippy, fmt, tests, and pkl-generated-check)
- AC1 `grep -rn -i -E "plan-citation|plan citation|plan slug|task ID" ...` -> exit 1 (no matches)
- `grep -n "Do not run plan, task, or validation workflows" .claude/skills/sce-commit/SKILL.md` -> exit 0 (line 149)
- `nix run .#pkl-generate -- "$(mktemp -d)"` -> exit 0; `diff -r` of `sce-commit` against `.claude/`, `.opencode/`, `.pi/`, `.agents/` -> exit 0 for all four (no differences)

### Success-criteria verification

- [x] AC1: No `sce-commit` surface instructs plan slugs, task IDs, or plan citations in commit messages -> AC1 grep returned no matches.
- [x] AC2: No plan-citation `blocked` branch or best-effort bypass citation step -> inspected `references/atomic-commit.md` in all four root packages: steps numbered 1–6 sequentially with no citation step; validation and boundary lists contain no citation bullets; no `citation`/`cite` matches.
- [x] AC3: Unrelated commit rules remain intact -> SKILL.md boundary line matches at line 149; `.claude/skills/sce-commit/references/atomic-commit.md` boundary list still contains `Invent issue references.`
- [x] AC4: Generated output matches committed root packages -> temporary `pkl-generate` output diffed identical for all four `sce-commit` packages.

### Failed checks and follow-ups

- None.

### Residual risks

- Without an explicit prohibition, agents may still add plan or task references to commit messages on their own (tracked as an open question; explicitly a non-goal).
