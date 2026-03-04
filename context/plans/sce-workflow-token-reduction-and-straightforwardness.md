# Plan: sce-workflow-token-reduction-and-straightforwardness

## 1) Change summary
Reduce unnecessary workflow prompt/context tokens and make SCE planning/execution flows more straightforward by tightening plan lifecycle policy, limiting context-sync edits to important changes, constraining commit guidance to staged-diff reality, and enforcing one-task/one-atomic-commit plan slicing.

## 2) Success criteria
- Completed implementation plans are treated as disposable execution artifacts and are not referenced as durable history from core context files.
- `/commit` guidance proposes context-file-only commit messaging only when staged changes are exclusively under `context/`; mixed code+context diffs do not trigger that guidance.
- Task execution no longer defaults to editing `context/overview.md`, `context/patterns.md`, and `context/architecture.md` on every task; updates are required only for important cross-cutting changes.
- Plan tasks are authored so each executable task maps to one atomic commit boundary.
- Shared Context Plan/Code command+skill contracts remain thin, deterministic, and non-duplicative after updates.

## 3) Constraints and non-goals
- In scope: SCE command/skill contract text, canonical Pkl source updates, generated parity, and context policy docs needed to codify new lifecycle/sync behavior.
- In scope: reducing recurring token-heavy instructions that do not affect safety or correctness.
- Out of scope: changing command names, collapsing Plan/Code into one role, or introducing auto-commit behavior.
- Out of scope: application/runtime feature work outside SCE workflow authoring surfaces.
- Non-goal: removing all repetition; safety-critical reminders may remain when they protect correctness.

## Assumptions
- Completed plans are deleted from `context/plans/` and are not treated as durable references from root context navigation.
- `/commit` context-only messaging is conditional on staged diff scope (context-only).
- Root context files (`overview.md`, `patterns.md`, `architecture.md`) are updated only for important cross-cutting changes.
- Each implementation task in a plan is scoped to a single atomic commit.

## 4) Task stack (T01..T06)
- [x] T01: Define disposable plan lifecycle and durable-context boundaries (status:done)
  - Task ID: T01
  - Goal: Establish canonical policy for when plans are kept, deleted, and referenced so durable context does not depend on completed plan files.
  - Boundaries (in/out of scope):
    - In: policy updates in `context/sce/` workflow docs plus any required root-context wording alignment.
    - Out: implementing code-task execution behavior changes.
  - Done when:
    - A single canonical policy states completed plans are disposable and not a durable context source.
    - Root context navigation no longer frames `context/plans/` as task-history storage.
  - Verification notes (commands or checks):
    - Manual trace from `context/context-map.md`, `context/overview.md`, and `context/sce/shared-context-plan-workflow.md` to confirm policy consistency.
    - Canonical prompt-surface alignment in `config/pkl/base/shared-content.pkl` and regenerated outputs for Shared Context Plan + `sce-plan-review` across OpenCode/Claude.
    - `nix develop -c pkl eval -m . config/pkl/generate.pkl`
    - `nix run .#pkl-check-generated`
    - `nix flake check`

- [ ] T02: Enforce "important change only" context-sync updates (status:todo)
  - Task ID: T02
  - Goal: Update plan/execution skill contracts so root context files are edited only when task impact is cross-cutting and important.
  - Boundaries (in/out of scope):
    - In: `sce-task-execution`, `sce-context-sync`, related workflow/glossary definitions.
    - Out: removing required context sync entirely.
  - Done when:
    - Skill language makes root-context edits conditional instead of default.
    - Verification-only tasks can close without forced root-context churn when no important change occurred.
  - Verification notes (commands or checks):
    - Manual contract review of canonical skill bodies and generated skill outputs for conditional sync wording.

- [ ] T03: Constrain `/commit` context-file guidance to context-only staged diffs (status:todo)
  - Task ID: T03
  - Goal: Remove noisy context-commit reminders from mixed-change commit proposals while preserving context-only commit support.
  - Boundaries (in/out of scope):
    - In: `/commit` command wrapper and `sce-atomic-commit` guidance contract.
    - Out: automatic git operations or commit creation behavior.
  - Done when:
    - Commit guidance explicitly gates context-only reminders by staged diff scope.
    - Mixed staged diffs no longer include default "commit context files" recommendations.
  - Verification notes (commands or checks):
    - Scenario-based contract walkthrough for (a) context-only staged diff and (b) mixed code+context staged diff.

- [ ] T04: Enforce one-task/one-atomic-commit planning contract (status:todo)
  - Task ID: T04
  - Goal: Make plan-authoring contracts require atomic executable task slicing and reject broad multi-commit tasks.
  - Boundaries (in/out of scope):
    - In: `sce-plan-authoring` task-shape requirements and related `/change-to-plan` wording.
    - Out: changing repository git safety policy.
  - Done when:
    - Plan task format includes explicit atomic-commit boundary expectations.
    - New plans default to executable units that can each land as one coherent commit.
  - Verification notes (commands or checks):
    - Manual review of updated planning instructions and one sample plan skeleton for atomicity compliance.

- [ ] T05: Regenerate outputs and align context map/glossary discoverability (status:todo)
  - Task ID: T05
  - Goal: Regenerate generated command/skill artifacts and ensure discoverability docs reflect the new low-noise workflow contracts.
  - Boundaries (in/out of scope):
    - In: deterministic regeneration outputs and focused context docs impacted by T01-T04.
    - Out: unrelated context refactors.
  - Done when:
    - Generated OpenCode/Claude outputs are updated to match canonical source changes.
    - `context/context-map.md` and `context/glossary.md` accurately describe the revised plan lifecycle and sync/commit behavior.
  - Verification notes (commands or checks):
    - `nix develop -c pkl eval -m . config/pkl/generate.pkl`
    - Manual read-through of affected generated command/skill files and context docs.

- [ ] T06: Validation and cleanup (status:todo)
  - Task ID: T06
  - Goal: Run final quality gates, confirm success criteria evidence, and leave plan state implementation-ready/traceable.
  - Boundaries (in/out of scope):
    - In: required repo validation checks and final context-sync verification.
    - Out: follow-on enhancements beyond this request.
  - Done when:
    - Validation commands pass and outputs are deterministic.
    - Success criteria have explicit evidence across updated contracts.
    - Plan checklist/status reflects final state clearly.
  - Verification notes (commands or checks):
    - `nix run .#pkl-check-generated`
    - `nix flake check`
    - Final manual context-sync review against updated workflow policy.

## 5) Open questions
- None.
