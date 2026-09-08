# SCE Workflow Ownership Table

## Scope and method

- Canonical workflow sources: `config/pkl/base/workflow-{change-to-plan,next-task,validate,commit,handover,brownfield}.pkl`.
- Canonical standalone decision-skill source: `config/pkl/base/decision-skill.pkl`.
- Shared package model: `config/pkl/base/workflow-content.pkl`.
- Shared synchronization source: `config/pkl/base/workflow-context-sync.pkl`.
- Generated consumers: ephemeral `config/.opencode/**`, `config/.claude/**`, and `config/.pi/**` payloads beneath generation roots.
- Assignment rule: each workflow phase has one canonical module owner; commands and agents only orchestrate or route.
- Canonical phase modules are authoring inputs to `config/pkl/renderers/workflow-composite.pkl`. No target generates them as packages; each is composed into the workflow skill named in the consumer column.

## Ownership matrix

| Behavior domain | Canonical owner | Reference-only consumers | Label |
| --- | --- | --- | --- |
| Context discovery before planning | `sce-context-load` in `workflow-change-to-plan.pkl` | `/change-to-plan`; composed into `sce-change-to-plan` | intentional/keep |
| Plan authoring process, clarification, and plan-specific task slicing | `sce-plan-authoring` in `workflow-change-to-plan.pkl` | `/change-to-plan`; reads the template before every write/revision; composed into `sce-change-to-plan`; thin OpenCode Plan agent | intentional/keep |
| Persisted plan schema and generic authoring rules | `references/plan-template.md` generated from `changeToPlanPlanTemplate` in `workflow-change-to-plan.pkl` | Plan authoring derives plan-specific content and applies the template's acceptance, task, no-validation-task, completion-record, and existing-plan-update rules without restating them | intentional/keep |
| Task resolution and readiness | `sce-plan-review` in `workflow-next-task.pkl` | `/next-task`; composed into `sce-next-task`; thin OpenCode Code agent | intentional/keep |
| Approval-gated one-task implementation | `sce-task-execution` in `workflow-next-task.pkl` | `/next-task` parses and conditionally passes `approve`; `references/output.md` owns only exact gate content/order; composed into `sce-next-task`; thin OpenCode Code agent | intentional/keep |
| Live post-task execution handoff schema | composite `references/task-execution.md` generated from `nextTaskTaskExecutionReference` | `/next-task` passes the `complete` result verbatim; composite task context sync validates and consumes that contract without restating the live field list; package mode retains `references/execution-contract.yaml`; cross-session retry records remain a separate persisted shape | intentional/keep |
| Synchronization-debt recovery identity | `sce-plan-review` resolves the plan path, debt task identity, completed task record, and persisted blocker | `/next-task` only routes that resolved record to task context sync; task context sync validates and consumes it directly from the plan; no separate persisted handoff exists | intentional/keep |
| Post-task durable context synchronization | Task instance from `workflow-context-sync.pkl` | `/next-task`; composed into `sce-next-task` | dedup/shared skeleton |
| Mandatory five-root context pass | `sce-task-context-sync` mandatory-root-pass subsection in `workflow-context-sync.pkl` | Task execution only hands off `context_impact`; context discovery references the named pass instead of relisting its five files; impact and verification sections may enforce the owned contract without redefining the file set | intentional/keep |
| Final validation execution, evidence interpretation, and outcome classification | `references/validation.md` generated from `renderValidationSkillBody` in `workflow-validate.pkl` | `/validate` routes the phase; the persisted report and returned report formats consume established status/evidence without redefining command selection, pass/fail interpretation, or non-repairing boundaries | intentional/keep |
| Persisted plan-file Validation Report schema and presentation | `references/validation-report.md` generated from `renderValidationReport` in `workflow-validate.pkl` | Validation writes or replaces this section on `validated`/`failed`; it renders recorded command results, criterion states, failures, risks, and retry without re-running or redefining validation policy | intentional/keep |
| Validated-plan durable context synchronization | Retained plan instance from `workflow-context-sync.pkl` | No current workflow consumer; `/validate` is validation-only | retained source, not generated |
| Staged-diff analysis and commit-message authoring | `sce-atomic-commit` in `workflow-commit.pkl` | `/commit`; composed into `sce-commit`; thin OpenCode Code agent | intentional/keep |
| Bypass commit execution sequence | `references/atomic-commit.md` **Bypass execution handoff**, generated from `renderAtomicCommitSkillBody` in `workflow-commit.pkl` | `/commit` executes the handoff exactly once after `bypass_message`; the workflow owns success/failure layout selection but does not restate temp-file, commit, hash, cleanup, or retry procedure | intentional/keep |
| Handover persisted schema and completeness validity | `references/handover-template.md` generated from `renderPersistedFormatBody` in `workflow-handover.pkl` | Writer composes and validates against this contract; loader validates against the same contract; `sce-handover/SKILL.md` owns routing, path checks, read/write boundaries, and terminal layout selection without restating required-section or content-validity rules | intentional/keep |
| Compact workflow execution contract | `executionContract` in `workflow-content.pkl` | Inline in change-to-plan, next-task, commit, validate, and handover entrypoints on all four targets; no extra runtime read | shared rendering, preserved behavior |
| Workflow routing | Six command documents in the workflow modules | Thin OpenCode Plan/Code agents | intentional/keep |
| Decision qualification gate | `decisionGate` in `workflow-context-sync.pkl` | Successful task context synchronization decides qualification and invokes `sce-decision` once per qualifying decision; `sce-decision` consumes the caller's gate result without restating or broadening the threshold | intentional/keep |
| Standalone ADR lifecycle, history, path, and result contract | `sce-decision/SKILL.md` generated from `skillText` in `decision-skill.pkl` | Cross-target internal `sce-decision` package; consumes one already-qualified request, validates status/history/path safety, writes or reuses one ADR, and returns the internal result | intentional/keep |
| Persisted ADR schema and section semantics | `references/adr-template.md` generated from `templateText` in `decision-skill.pkl` | `sce-decision` reads and populates the template at the write boundary without restating the section schema or field semantics | intentional/keep |

## Guardrails

- Keep Plan and Code routing roles separate without placing workflow doctrine in agent bodies.
- Keep commands thin: each routes to exactly one workflow skill and owns no phase behavior.
- Keep task and retained plan synchronization policy in the one shared Pkl skeleton; only task synchronization is composed into a current workflow.
- Keep SCE workflow control flow inside the owning workflow skill. Relevant non-SCE skills may assist as in-step helpers that return control to the active step; `sce-decision` remains the sole SCE sibling-skill exception, usable only from successful task synchronization's decision gate, once per qualifying decision.
- Do not reintroduce the removed `/validate` plan-context-sync handoff, legacy context-sync, or automated-profile Markdown ownership.
- Do not reintroduce phase skills as a generated surface. Workflow behavior belongs in the canonical modules and installation belongs to the six command-routed workflow packages (see [Atomic commit workflow](atomic-commit-workflow.md) for `/commit`). The standalone `sce-decision` package is a separate internal surface, not a generated phase package or user-facing workflow.

## Workflow language conventions

- **Command** is the user-facing invocation such as `/next-task`.
- **Workflow** is the complete procedure owned by a generated workflow `SKILL.md`.
- **Phase** is an embedded operation such as plan review, task execution, or validation.
- **Step** is one numbered action inside a workflow or phase.
- **Internal result** is structured phase data that is not shown directly. Its discriminator is `status`.
- **Report** is formatted Markdown shown to the user or persisted as a report section.
- **Completion record** is execution evidence persisted on a completed task in its plan.
- **Handover document** is the persisted session document under `context/handovers/`.

Use the verbs consistently: run a phase, return an internal result, render a report or layout, and write a persisted file. Preserve distinct lifecycle terms such as task `done`, execution `complete`, context `synced`, and plan `validated`; do not collapse them into one generic success term.
