# SCE Dedup Ownership Table

## Scope and method

- Canonical source of truth: the grouped modules `config/pkl/base/shared-content-{plan,code,commit}.pkl`, each exposing `agents`/`commands`/`skills` mappings of `UnitSpec { title; body }`, aggregated by `config/pkl/base/shared-content.pkl` into `executionProfiles`, `workflows`, and `skills`.
- Generated consumers reviewed: `config/.opencode/{agent,command,skills}/**`, `config/.claude/{commands,skills}/**` (Claude has no generated `agents/` directory), and `config/.pi/{prompts,skills}/**`.
- Context references reviewed: `context/sce/plan-code-overlap-map.md` and `context/decisions/2026-03-03-plan-code-agent-separation.md`.
- Assignment rule: each behavior domain has one canonical owner; every other appearance is a reference-only, derived, or composed consumer.

## Ownership matrix

| Behavior domain | Canonical owner | Reference-only / derived consumers | Label |
| --- | --- | --- | --- |
| Plan role policy — mission, hard boundaries, doctrine, tool posture | `agents["shared-context-plan"].body` in `shared-content-plan.pkl`, bound as `executionProfiles["shared-context-plan"]` | `config/.opencode/agent/Shared Context Plan.md` (native); selectively composed into the Claude `change-to-plan` command and Pi prompt | intentional/keep |
| Code role policy — mission, hard boundaries, doctrine, tool posture | `agents["shared-context-code"].body` in `shared-content-code.pkl`, bound as `executionProfiles["shared-context-code"]` | `config/.opencode/agent/Shared Context Code.md` (native); selectively composed into the Claude code-workflow commands and Pi prompts | intentional/keep |
| Within-workflow rule ownership — one authoritative section per rule | each `workflows[...].body` (e.g. `commands["next-task"].body`) | OpenCode native command, Claude composed command, Pi composed prompt | dedup/complete |
| `/next-task` detailed phase contracts | `skills["sce-plan-review"]`, `skills["sce-task-execution"]`, `skills["sce-context-sync"]`, `skills["sce-validation"]` | `commands["next-task"].body` plus `config/.opencode/command/next-task.md`, `config/.claude/commands/next-task.md`, `config/.pi/prompts/next-task.md` | dedup/complete |
| `/change-to-plan` clarification and plan-shape contracts | `skills["sce-plan-authoring"]` | `commands["change-to-plan"].body` plus the three generated command/prompt files | dedup/complete |
| `/commit` commit grammar and atomic split guidance | `skills["sce-atomic-commit"]` | `commands["commit"].body` plus the three generated command/prompt files | dedup/complete |
| Related Units for every workflow | Typed workflow metadata (`executionProfile`, `entrySkill`, `requiredSkills`), derived by `composeProfile`/`nativeWorkflowBody` in `shared-content-common.pkl` | Every generated workflow projection | dedup/complete |
| Cross-body profile composition | `composeProfile(profile, workflow)` in `shared-content-common.pkl` (composes only Preconditions, Guardrails, Failure handling) | Claude and Pi workflow bodies | intentional/keep |

## Guardrails for follow-up tasks

- Keep Plan/Code role separation unchanged; role policy is authored inline in each profile body (there is no shared-snippet extraction — the former `sharedSce*` baseline constants were removed as dead code).
- Keep `/next-task`, `/change-to-plan`, and `/commit` command bodies at orchestration/gating scope; keep detailed acceptance and behavior contracts in skill-owned canonical bodies.
- Keep every normative rule in exactly one authoritative section of its workflow body; reference it elsewhere only for transition or enforcement semantics.
- Keep Related Units metadata-derived; author `body.relatedUnits` only for relationships that cannot be derived from `executionProfile`, `entrySkill`, or `requiredSkills`.
