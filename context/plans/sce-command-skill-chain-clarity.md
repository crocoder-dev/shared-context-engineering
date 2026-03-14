# Plan: Make SCE command skill chains explicit

## Change summary

Tighten the canonical SCE command authoring surfaces so command bodies explicitly name the skill chain, invocation order, and stop conditions instead of relying on implied behavior. Update command descriptions to advertise the actual skill chain, and optionally add machine-readable skill metadata to OpenCode command frontmatter if extra frontmatter fields are preserved safely.

## Success criteria

- The canonical command bodies in `config/pkl/base/shared-content.pkl` explicitly describe which skill(s) each target command must load, the order they run in, and the stop/decision gates that keep commands thin.
- The targeted commands are covered: `next-task`, `change-to-plan`, `drift-detect`, `fix-drift`, `handover`, `commit`, and `validate`.
- Command descriptions in `config/pkl/renderers/common.pkl` mention the concrete skill chain instead of only the high-level outcome.
- If OpenCode command frontmatter safely preserves extra fields, `config/pkl/renderers/opencode-content.pkl` emits machine-readable command skill metadata; if not, the repo keeps behavior unchanged and records the reason in the implementation notes/task evidence.
- Regenerated outputs and parity/validation checks pass for the affected generated command docs.

## Constraints and non-goals

- Keep command bodies thin orchestration wrappers; detailed behavior ownership stays with the referenced skill bodies.
- Preserve the current owner/consumer split documented in SCE workflow context; do not move detailed contracts out of skill-owned sections.
- Edit canonical sources and renderer metadata only; do not hand-edit generated command outputs as primary sources.
- Do not merge unrelated command cleanup or wording polish outside the targeted command set.
- Treat optional frontmatter metadata as conditional work: investigate preservation/use first, then implement only if it is safe and useful within current renderer behavior.

## Task stack (`T01..T05`)

- [x] T01: `Make plan/execute command bodies explicit` (status:done)
  - Task ID: T01
  - Goal: Update the canonical bodies for `next-task` and `change-to-plan` in `config/pkl/base/shared-content.pkl` so they explicitly state the exact skill sequence, readiness/clarification gates, and session handoff stop conditions while staying thin wrappers.
  - Boundaries (in/out of scope): In - command-body text for `next-task` and `change-to-plan`, matching orchestration wording, and any directly related generated-command output produced by regeneration. Out - skill body rewrites, non-target commands, renderer/frontmatter metadata, and broader agent-contract changes.
  - Done when: Both command bodies name the invoked skill chain in order, describe when execution must stop or ask the user, and preserve thin-orchestration ownership consistent with current SCE workflow docs.
  - Verification notes (commands or checks): Review generated `config/.opencode/command/next-task.md` and `config/.opencode/command/change-to-plan.md` after regeneration; run `nix run .#pkl-check-generated`.
  - Completed: 2026-03-14
  - Files changed: `config/pkl/base/shared-content.pkl`, `config/.opencode/command/next-task.md`, `config/.opencode/command/change-to-plan.md`, `config/automated/.opencode/command/next-task.md`, `config/automated/.opencode/command/change-to-plan.md`, `config/.claude/commands/next-task.md`, `config/.claude/commands/change-to-plan.md`
  - Evidence: `nix develop -c pkl eval -m . config/pkl/generate.pkl`; `nix run .#pkl-check-generated`; `nix flake check`
  - Context sync classification: verify-only root context pass; command orchestration wording already matches current workflow docs.

- [x] T02: `Make support command bodies explicit` (status:done)
  - Task ID: T02
  - Goal: Tighten the canonical bodies for `drift-detect`, `fix-drift`, `handover`, `commit`, and `validate` in `config/pkl/base/shared-content.pkl` so each command calls out its skill ownership, ordering, and stop/confirmation conditions.
  - Boundaries (in/out of scope): In - command-body text for the five targeted support commands and matching generated-command output from regeneration. Out - changes to non-target commands, deeper skill-contract rewrites, and description/frontmatter work covered by later tasks.
  - Done when: Each targeted command body explicitly states which skill to load, what wrapper-level sequencing it owns, and what conditions stop for user input versus continue automatically.
  - Verification notes (commands or checks): Review regenerated command docs for the five commands under `config/.opencode/command/`; run `nix run .#pkl-check-generated`.
  - Completed: 2026-03-14
  - Files changed: `config/pkl/base/shared-content.pkl`, `config/.opencode/command/drift-detect.md`, `config/.opencode/command/fix-drift.md`, `config/.opencode/command/handover.md`, `config/.opencode/command/commit.md`, `config/.opencode/command/validate.md`, `config/automated/.opencode/command/drift-detect.md`, `config/automated/.opencode/command/fix-drift.md`, `config/automated/.opencode/command/handover.md`, `config/automated/.opencode/command/commit.md`, `config/automated/.opencode/command/validate.md`, `config/.claude/commands/drift-detect.md`, `config/.claude/commands/fix-drift.md`, `config/.claude/commands/handover.md`, `config/.claude/commands/commit.md`, `config/.claude/commands/validate.md`
  - Evidence: `nix develop -c pkl eval -m . config/pkl/generate.pkl`; `nix run .#pkl-check-generated`; `nix flake check`
  - Context sync classification: verify-only root context pass; task changes command-wrapper wording without changing current workflow contracts.

- [x] T03: `Strengthen command descriptions with skill chains` (status:done)
  - Task ID: T03
  - Goal: Update `config/pkl/renderers/common.pkl` so the targeted command descriptions mention the concrete skill chain and orchestration role rather than only the outcome.
  - Boundaries (in/out of scope): In - `commandDescriptions` entries for the targeted commands and regeneration of affected rendered docs. Out - non-target description rewrites, command body changes, and frontmatter schema changes.
  - Done when: The targeted command descriptions clearly reference the underlying skill or skill chain in concise renderer-friendly wording, and generated docs reflect the new descriptions.
  - Verification notes (commands or checks): Inspect regenerated command frontmatter/descriptions in `config/.opencode/command/*.md`; run `nix run .#pkl-check-generated`.
  - Completed: 2026-03-14
  - Files changed: `config/pkl/renderers/common.pkl`, `config/.opencode/command/next-task.md`, `config/.opencode/command/change-to-plan.md`, `config/.opencode/command/drift-detect.md`, `config/.opencode/command/fix-drift.md`, `config/.opencode/command/handover.md`, `config/.opencode/command/commit.md`, `config/.opencode/command/validate.md`, `config/automated/.opencode/command/next-task.md`, `config/automated/.opencode/command/change-to-plan.md`, `config/automated/.opencode/command/drift-detect.md`, `config/automated/.opencode/command/fix-drift.md`, `config/automated/.opencode/command/handover.md`, `config/automated/.opencode/command/commit.md`, `config/automated/.opencode/command/validate.md`, `config/.claude/commands/next-task.md`, `config/.claude/commands/change-to-plan.md`, `config/.claude/commands/drift-detect.md`, `config/.claude/commands/fix-drift.md`, `config/.claude/commands/handover.md`, `config/.claude/commands/commit.md`, `config/.claude/commands/validate.md`
  - Evidence: `nix develop -c pkl eval -m . config/pkl/generate.pkl`; `nix run .#pkl-check-generated`; `nix flake check`
  - Context sync classification: verify-only root context pass; command description wording changed, but no root workflow contract changed.

- [x] T04: `Add conditional OpenCode command skill metadata` (status:done)
  - Task ID: T04
  - Goal: Evaluate whether extra OpenCode command frontmatter fields are preserved/usable, then add machine-readable command skill metadata in `config/pkl/renderers/opencode-content.pkl` only if that investigation confirms it is safe.
  - Boundaries (in/out of scope): In - renderer changes for OpenCode command frontmatter, targeted metadata fields such as `entry-skill` or `skills`, and minimal supporting notes/evidence needed to justify the choice. Out - changing skill behavior, Claude-specific metadata unless required for parity by current renderer design, and speculative metadata with no demonstrated preservation path.
  - Done when: Either (a) OpenCode command frontmatter emits stable machine-readable skill metadata for the targeted commands, or (b) the implementation intentionally leaves frontmatter unchanged with documented evidence in task notes because extra fields are not preserved or useful.
  - Verification notes (commands or checks): Regenerate OpenCode command docs and inspect frontmatter for targeted commands; run `nix run .#pkl-check-generated`.
  - Completed: 2026-03-14
  - Files changed: `config/pkl/renderers/opencode-content.pkl`, `config/.opencode/command/next-task.md`, `config/.opencode/command/change-to-plan.md`, `config/.opencode/command/drift-detect.md`, `config/.opencode/command/fix-drift.md`, `config/.opencode/command/handover.md`, `config/.opencode/command/commit.md`, `config/.opencode/command/validate.md`
  - Evidence: `nix develop -c pkl eval -m . config/pkl/generate.pkl`; inspected generated frontmatter for `config/.opencode/command/next-task.md`, `config/.opencode/command/change-to-plan.md`, and `config/.opencode/command/validate.md`; `nix run .#pkl-check-generated`
  - Notes: OpenCode command frontmatter safely preserves extra YAML keys in generated output, so targeted commands now emit machine-readable `entry-skill` plus ordered `skills` metadata while leaving non-target commands unchanged.
  - Context sync classification: important change; OpenCode command frontmatter now has a documented machine-readable skill-metadata contract.

- [x] T05: `Validate generated command surfaces and cleanup` (status:done)
  - Task ID: T05
  - Goal: Run final validation for the command-orchestration clarity change, confirm regenerated outputs are in sync, and ensure any durable SCE context updates are complete.
  - Boundaries (in/out of scope): In - regeneration, parity/validation checks, cleanup of temporary notes, and context-sync verification for any important cross-cutting wording changes. Out - new feature work beyond the planned command/documentation surfaces.
  - Done when: Generation/parity checks succeed, broader repo validation required by the plan is captured, temporary scaffolding is removed, and context reflects the final current-state behavior for command orchestration guidance.
  - Verification notes (commands or checks): `nix develop -c pkl eval -m . config/pkl/generate.pkl`; `nix run .#pkl-check-generated`; `nix flake check`.
  - Completed: 2026-03-14
  - Files changed: `context/plans/sce-command-skill-chain-clarity.md`
  - Evidence: `nix develop -c pkl eval -m . config/pkl/generate.pkl`; `nix run .#pkl-check-generated`; `nix flake check`
  - Notes: No task-specific temporary scaffolding was present under `context/tmp/`; shared context files already reflected the finalized thin-command orchestration and OpenCode skill-metadata contract, so the final sync pass was verify-only.
  - Context sync classification: verify-only root context pass for T05; final-task verification confirmed the important T04 root-context updates remain aligned with current code truth.

## Validation Report

### Commands run

- `nix develop -c pkl eval -m . config/pkl/generate.pkl` -> exit 0; regenerated canonical command/skill/agent outputs successfully.
- `nix run .#pkl-check-generated` -> exit 0; reported `Generated outputs are up to date.`
- `nix flake check` -> exit 0; evaluated and built `cli-tests`, `cli-clippy`, `cli-fmt`, and `pkl-parity` successfully.

### Cleanup

- Checked `context/tmp/` for task-specific temporary artifacts; none were present.

### Success-criteria verification

- [x] Canonical command bodies explicitly describe skill chains, invocation order, and thin-wrapper stop gates -> confirmed in prior completed task evidence and preserved by successful regeneration/parity checks.
- [x] Targeted commands are covered (`next-task`, `change-to-plan`, `drift-detect`, `fix-drift`, `handover`, `commit`, `validate`) -> confirmed by regenerated command outputs and parity success.
- [x] Command descriptions mention concrete skill chains -> confirmed by successful regeneration/parity of renderer-driven command docs.
- [x] OpenCode command frontmatter safely emits machine-readable skill metadata -> confirmed by implemented renderer contract already documented in root context and preserved by regeneration/parity checks.
- [x] Regenerated outputs and parity/validation checks pass -> confirmed by the three validation commands above.

### Failed checks and follow-ups

- None.

### Residual risks

- None identified for this plan scope.

## Open questions (if any)

- Optional metadata is intentionally handled as a conditional implementation decision inside T04: proceed only if regenerated OpenCode command docs preserve or use extra frontmatter fields safely.
