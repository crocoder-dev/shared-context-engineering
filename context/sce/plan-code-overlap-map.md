# Shared Context Plan/Code Overlap Map

## Scope reviewed

- Canonical packages: `config/pkl/base/workflow-{change-to-plan,next-task,validate,commit}.pkl`
- Shared synchronization skeleton: `config/pkl/base/workflow-context-sync.pkl`
- Generated OpenCode routing agents and workflow commands under `config/.opencode/`
- Eight canonical phase modules, composed into four workflow skills for every target and rendered as packages for none

## Overlap matrix

| Surface | Shared reusable content | Surface-specific content | Ownership rule |
| --- | --- | --- | --- |
| Shared Context Plan agent | References the planning workflow | Routes Plan work to `/change-to-plan` | Keep routing-only; command and skills own behavior |
| Shared Context Code agent | References implementation lifecycle | Routes Code work to `/next-task`, `/validate`, and `/commit` | Keep routing-only; command and skills own behavior |
| `/change-to-plan` | Thin routing to one workflow skill | Routes to `sce-change-to-plan` | The skill runs the context-load and plan-authoring phases internally |
| `/next-task` | Thin routing to one workflow skill | Routes to `sce-next-task` | The skill runs review, one-task execution, task sync, and continuation internally |
| `/validate` | Thin routing to one workflow skill | Routes to `sce-validate` | The skill runs final validation then validated-only plan sync internally |
| `/commit` | Thin routing to one workflow skill | Routes to `sce-commit` | The skill runs mode routing, staged-diff analysis, and message authoring internally |
| Task/plan context sync | Root-pass, context hygiene, and synchronization policy | Distinct entry gates and report lifecycle | One parameterized Pkl skeleton composes into `sce-next-task` and `sce-validate` |

## Current dedup boundary

- Workflow commands own only routing to one workflow skill.
- Workflow skills own phase ordering, status branching, gates, edits, verification, and result/report formats.
- Thin OpenCode agents own only role-to-command routing and target permissions; their skill allowlists name exactly the four workflow slugs.
- Claude and Pi do not receive generated agents.
- Each workflow package carries exactly one reference file, `references/output.md`, so it remains self-contained without cross-package dependencies.
- The removed grouped shared-content catalog and automated OpenCode profile have no current owner or consumer.
