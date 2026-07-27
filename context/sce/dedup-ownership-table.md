# SCE Workflow Ownership Table

## Scope and method

- Canonical workflow sources: `config/pkl/base/workflow-{change-to-plan,next-task,validate}.pkl`.
- Shared package model: `config/pkl/base/workflow-content.pkl`.
- Shared synchronization source: `config/pkl/base/workflow-context-sync.pkl`.
- Generated consumers: `config/.opencode/**`, `config/.claude/**`, and `config/.pi/**`.
- Assignment rule: each workflow phase has one canonical skill owner; commands and agents only orchestrate or route.

## Ownership matrix

| Behavior domain | Canonical owner | Reference-only consumers | Label |
| --- | --- | --- | --- |
| Context discovery before planning | `sce-context-load` in `workflow-change-to-plan.pkl` | `/change-to-plan`; generated target packages | intentional/keep |
| Plan authoring, clarification, and task slicing | `sce-plan-authoring` in `workflow-change-to-plan.pkl` | `/change-to-plan`; thin OpenCode Plan agent | intentional/keep |
| Task resolution and readiness | `sce-plan-review` in `workflow-next-task.pkl` | `/next-task`; thin OpenCode Code agent | intentional/keep |
| Approval-gated one-task implementation | `sce-task-execution` in `workflow-next-task.pkl` | `/next-task`; thin OpenCode Code agent | intentional/keep |
| Post-task durable context synchronization | Task instance from `workflow-context-sync.pkl` | `/next-task`; generated `sce-task-context-sync` packages | dedup/shared skeleton |
| Final validation and validation report | `sce-validation` in `workflow-validate.pkl` | `/validate`; thin OpenCode Code agent | intentional/keep |
| Validated-plan durable context synchronization | Plan instance from `workflow-context-sync.pkl` | `/validate`; generated `sce-plan-context-sync` packages | dedup/shared skeleton |
| Workflow routing | Three command documents in the workflow modules | Thin OpenCode Plan/Code agents | intentional/keep |

## Guardrails

- Keep Plan and Code routing roles separate without placing workflow doctrine in agent bodies.
- Keep commands at sequencing, branching, and handoff scope; phase behavior remains skill-owned.
- Keep task and plan synchronization packages self-contained even though one Pkl skeleton owns their shared policy.
- Do not reintroduce removed `/commit`, `/handover`, legacy context-sync, or automated-profile Markdown ownership.
