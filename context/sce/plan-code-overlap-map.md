# Shared Context Plan/Code Overlap Map (T01)

## Scope reviewed

- Canonical source: `config/pkl/base/shared-content.pkl`
- Workflow context: `context/sce/shared-context-code-workflow.md`
- Generated OpenCode artifacts: `.opencode/agent/Shared Context Plan.md`, `.opencode/agent/Shared Context Code.md`, `.opencode/command/change-to-plan.md`, `.opencode/command/next-task.md`, `.opencode/command/commit.md`
- Core skills: `.opencode/skills/sce-plan-review/SKILL.md`, `.opencode/skills/sce-task-execution/SKILL.md`, `.opencode/skills/sce-context-sync/SKILL.md`, `.opencode/skills/sce-atomic-commit/SKILL.md`

## Overlap matrix

| Surface | Shared reusable content | Role-specific content | Overlap type | Dedup target |
| --- | --- | --- | --- | --- |
| Shared Context Plan agent | Core principles, `context/` authority, quality posture | Planning-only boundaries (no app code, no shell), plan-authoring procedure, handoff to `/next-task` | Intentional structural overlap with Code agent | Extract shared baseline block for both agents; keep mission/procedure separate |
| Shared Context Code agent | Core principles, `context/` authority, quality posture | One-task execution boundary, implementation + validation flow, context sync + feedback loop | Intentional structural overlap with Plan agent | Same shared baseline block; keep execution gates role-local |
| `/change-to-plan` command | Clarification gate and plan write contract mirror `sce-plan-authoring` | Plan-session-only handoff behavior | Functional overlap with `sce-plan-authoring` | Define command as thin wrapper; source gate text from skill-owned canonical section |
| `/next-task` command | End-to-end orchestration text mirrors `sce-plan-review`, `sce-task-execution`, and `sce-context-sync` | Auto-pass condition and implementation-stop wording | Functional overlap with three skills | Keep command orchestration short; reference skill contracts instead of repeating long behavior block |
| `/commit` command | Atomic-commit workflow intent overlaps atomic commit skill | Staged-changes confirmation gate and no-auto-commit policy | Duplication + naming mismatch risk | Align command to `sce-atomic-commit` slug and move detailed commit-style contract ownership to skill |
| `sce-plan-review` + `sce-task-execution` + `sce-context-sync` skills | Readiness, stop-before-edit, context-sync rules reused by `/next-task` and Code agent | Each skill owns one phase boundary | Expected layering overlap (command invokes skills) | Keep phase contracts skill-owned; command/agent should summarize, not duplicate |

## Role-specific vs shared-reusable split

Role-specific (should remain separate):
- Agent mission and hard boundaries (Plan never edits app code; Code executes one approved task).
- Procedure sequencing tied to the agent role (plan authoring vs implementation execution).

Shared reusable (dedup candidates):
- Core principles block (`human owns decisions`, `context is durable memory`, `code truth wins`).
- Repeated context authority block (create/update/move/delete rules in `context/`).
- Repeated quality posture language (current-state docs, avoid prose-heavy changelogs).

## Candidate dedup targets for follow-up tasks

- Create reusable canonical snippets in `config/pkl/base/shared-content.pkl` for shared principles/authority blocks consumed by both agents.
- Reduce `/next-task` behavior verbosity by delegating to skill contracts while preserving required gates.
- Resolve `/commit` command skill slug mismatch (`atomic-commits` vs `sce-atomic-commit`) and make skill the single owner of detailed atomic-commit message rules.
- Add a new Shared Context Plan workflow doc (`context/sce/shared-context-plan-workflow.md`) to mirror Code workflow structure and reduce role ambiguity.
