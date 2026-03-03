# Plan: sce-plan-code-convergence-and-sync-policy

## 1) Change summary
Plan and execute a coordinated SCE configuration update centered on `config/pkl/base/shared-content.pkl`: evaluate whether Shared Context Plan and Shared Context Code should be merged, formalize lightweight post-task test policy in `context/overview.md`, add canonical atomic-commit skill/command coverage, reduce unnecessary root context churn in sync behavior, deduplicate overlapping content across agents/commands/skills, and document a Shared Context Plan workflow parallel to the existing Shared Context Code workflow.

## 2) Success criteria
- Canonical content in `config/pkl/base/shared-content.pkl` defines all required behavior for: plan/code relationship decision, lightweight post-task test policy, atomic-commit command+skill, sync-significance gating, and dedup conventions.
- A new Shared Context Plan workflow document exists under `context/sce/` with structure and rigor comparable to `context/sce/shared-context-code-workflow.md`.
- Root context update policy is explicit: `context/overview.md`, `context/architecture.md`, and `context/glossary.md` are updated only for important cross-cutting changes; otherwise updates are routed to focused domain/workflow files plus `context/context-map.md` links.
- Shared content duplication across agents/commands/skills is reduced with clear canonical ownership and cross-reference patterns.
- Atomic commit workflow is represented canonically and generates target outputs consistently.
- Merge decision for Shared Context Plan vs Shared Context Code is recorded with rationale; if merge is approved, migration is implemented with guardrails and documented compatibility path.

## 3) Constraints and non-goals
- In scope: canonical SCE content and context artifacts (`config/pkl/base/*`, renderer wiring if needed, generated config outputs, `context/**` workflow docs, glossary/overview/patterns/architecture updates).
- In scope: evaluate-first approach for Plan/Code merge, with implementation only if the evaluated decision is to merge.
- Out of scope: unrelated application/runtime feature work outside SCE agent/command/skill/context behavior.
- Out of scope: changing repository git policy beyond adding atomic commit command/skill behavior and guidance.
- Non-goal: forcing a merge if evaluation shows role separation should remain.
- Non-goal: introducing heavy new dependencies/services for dedup or sync policy enforcement.

## 4) Task stack (T01..T07)
- [x] T01: Baseline inventory and overlap map for Plan/Code/skills/commands (status:done)
  - Task ID: T01
  - Goal: Build a concrete overlap matrix for Shared Context Plan, Shared Context Code, related commands (`/change-to-plan`, `/next-task`, `/commit`), and core skills to identify true duplication vs intentional separation.
  - Boundaries (in/out of scope):
    - In: canonical blocks in `config/pkl/base/shared-content.pkl`, existing workflow docs in `context/sce/`, and current command/skill artifacts that drive generation.
    - Out: implementing merge or content rewrites before overlap findings are finalized.
  - Done when:
    - A file-level overlap map exists in context (plan notes and/or supporting context doc) with explicit candidate dedup targets.
    - Distinction between "role-specific" and "shared reusable" instructions is documented.
  - Verification notes (commands or checks):
    - Manual consistency review across canonical content units and workflow docs.
    - Evidence captured in `context/sce/plan-code-overlap-map.md` (overlap matrix, role-specific vs shared-reusable split, and candidate dedup targets).

- [x] T02: Define canonical policy updates for lightweight tests and sync significance gating (status:done)
  - Task ID: T02
  - Goal: Canonically encode that lightweight tests run after every task are always documented in `context/overview.md`, and define "important change" criteria that gates root context file updates during sync.
  - Boundaries (in/out of scope):
    - In: updates to canonical skill/command instructions and context docs that establish policy language and ownership.
    - Out: broad rewrite of unrelated validation guidance.
  - Done when:
    - `context/overview.md` contains canonical lightweight post-task test baseline language.
    - `sce-context-sync` policy clearly distinguishes when root files must be edited vs verified-only.
    - `context/glossary.md` terms are aligned for "lightweight post-task verification baseline" and "important change" semantics.
  - Verification notes (commands or checks):
    - Manual read-through for policy consistency across overview, glossary, and sync skill text.
    - `nix develop -c pkl eval -m . config/pkl/generate.pkl`
    - `nix run .#pkl-check-generated`
    - `nix flake check`

- [x] T03: Add Shared Context Plan workflow document aligned to Shared Context Code workflow quality bar (status:done)
  - Task ID: T03
  - Goal: Create `context/sce/shared-context-plan-workflow.md` with command entrypoint, stepwise flow, gates, and Mermaid diagram analogous in quality and structure to `context/sce/shared-context-code-workflow.md`.
  - Boundaries (in/out of scope):
    - In: current-state workflow documentation for planning behavior and clarification gate.
    - Out: implementation agent runtime logic changes unrelated to documented workflow.
  - Done when:
    - New plan workflow doc exists with clear entrypoint(s), clarification stop conditions, output contract, and next-session handoff command.
    - `context/context-map.md` links to the new workflow doc.
  - Verification notes (commands or checks):
    - Manual parity check against structure in `context/sce/shared-context-code-workflow.md`.
    - `nix run .#pkl-check-generated`
    - `nix flake check`

- [x] T04: Canonicalize atomic commit command + skill integration and naming consistency (status:done)
  - Task ID: T04
  - Goal: Add/align atomic commit command and skill in canonical Pkl content using the behavior from `.opencode/command/commit.md` and `.opencode/skills/sce-atomic-commit/SKILL.md`, including naming consistency (`sce-atomic-commit` vs `atomic-commits`).
  - Boundaries (in/out of scope):
    - In: `shared-content.pkl` command/skill canonical bodies and renderer compatibility mapping if required.
    - Out: auto-running git commits from the command (must remain proposal-only unless separately approved).
  - Done when:
    - Canonical command+skill entries exist and generate consistent target artifacts.
    - Command behavior reflects empty args contract and staged-changes confirmation gate.
    - Naming mismatch is resolved and documented.
  - Verification notes (commands or checks):
    - Regenerate outputs and verify generated command/skill files match intended contract.
    - Executed:
      - `nix develop -c pkl eval -m . config/pkl/generate.pkl`
      - `nix run .#pkl-check-generated`
      - `nix flake check`

- [x] T05: Decision task - evaluate Plan+Code merge with architecture/risk trade-offs (status:done)
  - Task ID: T05
  - Goal: Produce a formal decision on whether to keep Shared Context Plan and Shared Context Code separate or merge them, based on overlap map, workflow clarity, risk, and maintainability.
  - Boundaries (in/out of scope):
    - In: decision analysis, migration impact, compatibility considerations for existing commands and skills.
    - Out: executing merge edits before explicit decision outcome is recorded.
  - Done when:
    - Decision is recorded in context (plan + decision note if needed) with rationale and chosen path.
    - If decision is "no merge", explicit dedup strategy for separate agents is documented.
    - If decision is "merge", T06 is explicitly marked required.
  - Verification notes (commands or checks):
    - Manual review confirms all critical concerns are addressed: boundaries, acceptance criteria, sequencing, and rollback/compatibility considerations.
  - Decision outcome:
    - Decision recorded: keep Shared Context Plan and Shared Context Code as separate agents.
    - Rationale and compatibility analysis captured in `context/decisions/2026-03-03-plan-code-agent-separation.md`.
    - Dedup strategy (while remaining separate): keep shared baseline principles/authority text canonicalized once, keep role-specific mission/procedure contracts in their respective agents/skills, and keep `/next-task` as thin orchestration over phase-owning skills.

- [x] T06: Conditional merge implementation task (execute only if T05 decides merge) (status:not-applicable)
  - Task ID: T06
  - Goal: If approved by T05 decision, merge Shared Context Plan and Shared Context Code into a unified agent contract with updated command routing and minimal disruption.
  - Boundaries (in/out of scope):
    - In: canonical agent/command updates, deprecation notes, and compatibility path for `/change-to-plan` and `/next-task` behaviors.
    - Out: unrelated command ecosystem refactors.
  - Done when:
    - Unified agent contract is canonicalized and generated outputs remain coherent.
    - Legacy invocation paths are either preserved or clearly deprecated with migration guidance.
    - If T05 decides "no merge", this task is marked not applicable with rationale.
  - Verification notes (commands or checks):
    - Regenerate outputs and inspect command-to-agent routing and required skill-loading sequences.
  - Applicability update:
    - Marked not applicable by T05 decision to keep Plan and Code separate (`context/decisions/2026-03-03-plan-code-agent-separation.md`).

- [x] T07: Validation and cleanup (status:done)
  - Task ID: T07
  - Goal: Run final validation, verify generated parity and context consistency, and ensure no residual ambiguity remains.
  - Boundaries (in/out of scope):
    - In: full checks, generated drift checks, final context sync verification, cleanup of temporary planning artifacts.
    - Out: new feature additions.
  - Done when:
    - All success criteria have evidence.
    - Generated outputs are deterministic and up to date.
    - Context files (`overview`, `architecture`, `glossary`, `patterns`, `context-map`, workflow docs) are consistent with final decisions.
  - Verification notes (commands or checks):
    - `nix develop -c pkl eval -m . config/pkl/generate.pkl`
    - `nix run .#pkl-check-generated`
    - `nix flake check`
  - Execution evidence:
    - `nix develop -c pkl eval -m . config/pkl/generate.pkl` completed and emitted deterministic generated target file list.
    - `nix run .#pkl-check-generated` reported: "Generated outputs are up to date."
    - `nix flake check` completed successfully (with expected incompatible-systems notice only).

## 5) Open questions
- None. Merge is explicitly evaluate-first, with conditional implementation in T06 only if selected by T05.

## 6) Final validation report (T07)

- Commands run:
  - `nix develop -c pkl eval -m . config/pkl/generate.pkl` (exit: 0)
  - `nix run .#pkl-check-generated` (exit: 0)
  - `nix flake check` (exit: 0)
- Key outputs:
  - Generator emitted deterministic target file list for both OpenCode and Claude outputs.
  - `pkl-check-generated` reported generated outputs are up to date.
  - `flake check` completed successfully; only expected incompatible-systems notice was reported.
- Failed checks and follow-ups:
  - None.
- Context consistency verification:
  - Verified `context/overview.md`, `context/architecture.md`, `context/glossary.md`, `context/patterns.md`, and `context/context-map.md` remain aligned with final plan outcomes.
  - Verified durable feature/decision discoverability via `context/context-map.md` links to `context/sce/shared-context-plan-workflow.md`, `context/sce/plan-code-overlap-map.md`, `context/sce/atomic-commit-workflow.md`, and `context/decisions/2026-03-03-plan-code-agent-separation.md`.
- Success-criteria verification summary:
  - Canonical content coverage for plan/code decision, lightweight checks baseline, atomic commit command/skill, sync significance gating, and dedup conventions: satisfied.
  - Shared Context Plan workflow document quality bar and discoverability: satisfied.
  - Root context update policy clarity and gating semantics: satisfied.
  - Shared-content dedup direction and ownership boundaries: satisfied.
  - Atomic commit canonical representation and generated consistency: satisfied.
  - Plan vs Code merge decision recorded with rationale and compatibility path: satisfied (no-merge path selected; T06 marked not applicable).
- Residual risks:
  - Minor cross-system coverage gap remains unless `nix flake check --all-systems` is run in appropriate environments.
