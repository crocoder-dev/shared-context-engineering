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
| Plan authoring, clarification, and task slicing | `sce-plan-authoring` in `workflow-change-to-plan.pkl` | `/change-to-plan`; composed into `sce-change-to-plan`; thin OpenCode Plan agent | intentional/keep |
| Task resolution and readiness | `sce-plan-review` in `workflow-next-task.pkl` | `/next-task`; composed into `sce-next-task`; thin OpenCode Code agent | intentional/keep |
| Approval-gated one-task implementation | `sce-task-execution` in `workflow-next-task.pkl` | `/next-task`; composed into `sce-next-task`; thin OpenCode Code agent | intentional/keep |
| Post-task durable context synchronization | Task instance from `workflow-context-sync.pkl` | `/next-task`; composed into `sce-next-task` | dedup/shared skeleton |
| Final validation and validation report | `sce-validation` in `workflow-validate.pkl` | `/validate`; composed into `sce-validate`; thin OpenCode Code agent | intentional/keep |
| Validated-plan durable context synchronization | Retained plan instance from `workflow-context-sync.pkl` | No current workflow consumer; `/validate` is validation-only | retained source, not generated |
| Staged-diff analysis and commit-message authoring | `sce-atomic-commit` in `workflow-commit.pkl` | `/commit`; composed into `sce-commit`; thin OpenCode Code agent | intentional/keep |
| Workflow routing | Six command documents in the workflow modules | Thin OpenCode Plan/Code agents | intentional/keep |
| Standalone ADR writing contract | `decision-skill.pkl` | Cross-target `sce-decision` package; successful task synchronization invokes it through the shared decision gate | intentional/keep |

## Guardrails

- Keep Plan and Code routing roles separate without placing workflow doctrine in agent bodies.
- Keep commands thin: each routes to exactly one workflow skill and owns no phase behavior.
- Keep task and retained plan synchronization policy in the one shared Pkl skeleton; only task synchronization is composed into a current workflow.
- Keep SCE workflow control flow inside the owning workflow skill. Relevant non-SCE skills may assist as in-step helpers that return control to the active step; `sce-decision` remains the sole SCE sibling-skill exception, usable only from successful task synchronization's decision gate, once per qualifying decision.
- Do not reintroduce the removed `/validate` plan-context-sync handoff, legacy context-sync, or automated-profile Markdown ownership.
- Do not reintroduce phase skills as a generated surface. Workflow behavior belongs in the canonical modules and installation belongs to the six command-routed workflow packages (see [Atomic commit workflow](atomic-commit-workflow.md) for `/commit`). The standalone `sce-decision` package is a separate internal surface, not a generated phase package or user-facing workflow.
