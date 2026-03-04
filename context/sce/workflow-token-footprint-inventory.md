# SCE Workflow Inventory for Token-Footprint Analysis (T01)

## Purpose

Provide a canonical inventory of participants in the SCE Plan (`/change-to-plan`) and Execute (`/next-task`) workflows, including ownership and invocation boundaries used for later token-footprint analysis.

## Workflow inventory matrix

| Workflow | Component type | Component | Role in workflow | Ownership boundary | Shared vs role-specific |
| --- | --- | --- | --- | --- | --- |
| Plan (`/change-to-plan`) | Agent | `Shared Context Plan` | Planning-only role that converts scoped requests into atomic plan tasks and emits `/next-task` handoff. | Canonical role contract is owned in `config/pkl/base/shared-content.pkl` (`agents["shared-context-plan"].canonicalBody`); generated consumers in `.opencode` and `.claude`. | role-specific |
| Plan (`/change-to-plan`) | Command | `/change-to-plan` | Thin orchestration entrypoint for plan creation/update. | Command wrapper owns only orchestration text; clarification and plan-shape contracts are skill-owned. | shared orchestration pattern |
| Plan (`/change-to-plan`) | Skill | `sce-plan-authoring` | Owns clarification gate, plan-shape contract, and readiness output semantics for planning sessions. | Skill is canonical owner of detailed plan-session behavior used by command wrapper. | role-specific |
| Plan (`/change-to-plan`) | Context artifact | `context/plans/{plan_name}.md` | Durable plan state (task IDs, status, boundaries, done checks, verification notes). | Plan markdown is the continuity source for planning/execution handoff. | shared artifact |
| Plan (`/change-to-plan`) | Context artifact | `context/sce/shared-context-plan-workflow.md` | Canonical workflow definition for Plan role and handoff contract to `/next-task`. | Context-owned reference doc; reflects current-state behavior. | role-specific |
| Execute (`/next-task`) | Agent | `Shared Context Code` | Execution role that runs one approved task, validates behavior, and enforces context sync. | Canonical role contract is owned in `config/pkl/base/shared-content.pkl` (`agents["shared-context-code"].canonicalBody`); generated consumers in `.opencode` and `.claude`. | role-specific |
| Execute (`/next-task`) | Command | `/next-task` | Thin orchestration entrypoint sequencing review, execution, and context sync phases. | Command wrapper owns sequencing/gates; detailed phase behavior is skill-owned. | shared orchestration pattern |
| Execute (`/next-task`) | Skill | `sce-plan-review` | Resolves plan target + task, checks readiness, and enforces clarification before execution when needed. | Skill is canonical owner of review/readiness phase contract. | shared phase in execute workflow |
| Execute (`/next-task`) | Skill | `sce-task-execution` | Enforces implementation stop, scoped edits, task-level verification, and plan status update. | Skill is canonical owner of implementation-phase contract. | shared phase in execute workflow |
| Execute (`/next-task`) | Skill | `sce-context-sync` | Required done gate that synchronizes `context/` to code truth after task implementation. | Skill is canonical owner of sync-phase contract. | shared phase in execute workflow |
| Execute (`/next-task`) | Skill (conditional) | `sce-validation` | Final plan-task-only validation/cleanup phase. | Skill invoked only when current task is final task in the plan. | shared conditional phase |
| Execute (`/next-task`) | Context artifact | `context/sce/shared-context-code-workflow.md` | Canonical workflow definition for Execute role including mandatory gates. | Context-owned reference doc; reflects current-state behavior. | role-specific |

## Cross-workflow shared components

| Component domain | Canonical owner | Consumers |
| --- | --- | --- |
| Shared SCE baseline doctrine (core principles, `context/` authority, quality posture) | Shared snippet constants in `config/pkl/base/shared-content.pkl` | Shared Context Plan agent, Shared Context Code agent, generated target-specific agent files |
| Thin-command orchestration model | `commands["change-to-plan"].canonicalBody`, `commands["next-task"].canonicalBody` in `config/pkl/base/shared-content.pkl` | Generated command files for OpenCode/Claude |
| Skill-owned detailed contracts | `skills["sce-plan-authoring"]`, `skills["sce-plan-review"]`, `skills["sce-task-execution"]`, `skills["sce-context-sync"]`, `skills["sce-validation"]` in `config/pkl/base/shared-content.pkl` | Plan/Execute command wrappers and role agents |

## Invocation boundaries

### Plan workflow

1. `/change-to-plan` invokes `Shared Context Plan`.
2. Plan agent loads `sce-plan-authoring` to perform clarification + plan shaping.
3. Plan output emits an execution handoff command: `/next-task {plan_name} {T0X}`.

### Execute workflow

1. `/next-task` invokes `Shared Context Code`.
2. Execute flow runs `sce-plan-review` first.
3. After readiness and explicit implementation-stop confirmation, execute flow runs `sce-task-execution`.
4. Done gate always runs `sce-context-sync`; `sce-validation` runs only on final plan task.

## Sources used

- `context/sce/shared-context-plan-workflow.md`
- `context/sce/shared-context-code-workflow.md`
- `context/sce/plan-code-overlap-map.md`
- `context/sce/dedup-ownership-table.md`
- `.opencode/command/change-to-plan.md`
- `.opencode/command/next-task.md`
- `.opencode/agent/Shared Context Plan.md`
- `.opencode/agent/Shared Context Code.md`
