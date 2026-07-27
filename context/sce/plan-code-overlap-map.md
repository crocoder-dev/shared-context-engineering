# Shared Context Plan/Code Overlap Map

## Scope reviewed

- Canonical packages: `config/pkl/base/workflow-{change-to-plan,next-task,validate}.pkl`
- Shared synchronization skeleton: `config/pkl/base/workflow-context-sync.pkl`
- Generated OpenCode routing agents and workflow commands under `config/.opencode/`
- Seven canonical phase skills rendered for OpenCode, Claude, and Pi

## Overlap matrix

| Surface | Shared reusable content | Surface-specific content | Ownership rule |
| --- | --- | --- | --- |
| Shared Context Plan agent | References the planning workflow | Routes Plan work to `/change-to-plan` | Keep routing-only; command and skills own behavior |
| Shared Context Code agent | References implementation lifecycle | Routes Code work to `/next-task` and `/validate` | Keep routing-only; command and skills own behavior |
| `/change-to-plan` | Structured handoffs and result branching | Sequences `sce-context-load` then `sce-plan-authoring` | Skills own context loading and plan authoring |
| `/next-task` | Structured handoffs and result branching | Sequences review, one-task execution, task sync, and continuation | `sce-plan-review`, `sce-task-execution`, and `sce-task-context-sync` own phases |
| `/validate` | Structured handoffs and result branching | Sequences final validation then validated-only plan sync | `sce-validation` and `sce-plan-context-sync` own phases |
| Task/plan context sync | Root-pass, context hygiene, and synchronization policy | Distinct handoff gates and report lifecycle | One parameterized Pkl skeleton emits two self-contained packages |

## Current dedup boundary

- Workflow commands own phase ordering, result branching, and authoritative handoff forwarding.
- Skills own detailed phase policy, gates, edits, verification, and result/report formats.
- Thin OpenCode agents own only role-to-command routing and target permissions.
- Claude and Pi do not receive generated agents.
- Package-local reference files are duplicated in generated targets by design so every skill package remains self-contained.
- The removed grouped shared-content catalog and automated OpenCode profile have no current owner or consumer.
