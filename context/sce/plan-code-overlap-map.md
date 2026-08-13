# Shared Context Plan/Code Overlap Map

## Scope reviewed

- Canonical packages: `config/pkl/base/workflow-{change-to-plan,next-task,validate,commit,handover,brownfield}.pkl`
- Shared synchronization skeleton: `config/pkl/base/workflow-context-sync.pkl`
- Generated OpenCode routing agents and workflow commands in ephemeral target payloads
- Eight canonical phase modules, composed into six workflow skills for every target; phase modules are not rendered as packages

## Overlap matrix

| Surface | Shared reusable content | Surface-specific content | Ownership rule |
| --- | --- | --- | --- |
| Shared Context Plan agent | References the planning workflow | Routes Plan work to `/change-to-plan` | Keep routing-only; command and skills own behavior |
| Shared Context Code agent | References implementation lifecycle | Routes Code work to `/next-task`, `/validate`, `/commit`, `/handover`, and `/brownfield` | Keep routing-only; command and skills own behavior |
| `/change-to-plan` | Thin routing to one workflow skill | Routes to `sce-change-to-plan` | The skill runs the context-load and plan-authoring phases internally |
| `/next-task` | Thin routing to one workflow skill | Routes to `sce-next-task` | The skill runs review, one-task execution, task sync, and continuation internally |
| `/validate` | Thin routing to one workflow skill | Routes to `sce-validate` | The skill runs final validation, writes the Validation Report, and reports its status |
| `/commit` | Thin routing to one workflow skill | Routes to `sce-commit` | The skill runs mode routing, staged-diff analysis, and message authoring internally |
| `/handover` | Thin routing to one workflow skill | Routes to `sce-handover` | The phase-free skill owns writer/loader modes and handover reporting internally |
| `/brownfield` | Thin routing to one workflow skill | Routes to `sce-brownfield` | The phase-free skill owns local investigation, clarification, context writes, and reporting internally |
| Task context sync | Root-pass, context hygiene, and synchronization policy | Task execution handoff, lifecycle, and report ownership | One parameterized Pkl skeleton composes into `sce-next-task`; the retained plan-sync source is not invoked by `/validate` |

## Current dedup boundary

- Workflow commands own only routing to one workflow skill.
- Workflow skills own phase ordering, status branching, gates, edits, verification, and result/report formats.
- Thin OpenCode agents own only role-to-command routing and target permissions; their ordered skill permissions allow ordinary non-SCE helpers, deny arbitrary `sce-*` skills, and allow only catalog-derived owned workflow slugs, with the Code-only synchronization exception for `sce-decision`.
- Claude and Pi do not receive generated agents.
- Each workflow package owns its package-local phase or persisted-document references and exactly one `references/output.md`, so it remains self-contained without cross-package dependencies.
- `/validate` does not compose a plan-context-sync phase; the retained plan-sync source has no generated package owner or user-facing route.
- The removed grouped shared-content catalog and automated OpenCode profile have no current owner or consumer.
