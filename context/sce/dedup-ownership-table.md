# SCE Dedup Ownership Table

## Scope and method

- Canonical source of truth: `config/pkl/base/shared-content.pkl`.
- Generated consumers reviewed: `config/.opencode/{agent,command,skills}/**` and `config/.claude/{agents,commands,skills}/**`.
- Context references reviewed: `context/sce/plan-code-overlap-map.md` and `context/decisions/2026-03-03-plan-code-agent-separation.md`.
- Assignment rule: each behavior domain has one canonical owner and one or more reference-only consumers.

## Ownership matrix

| Behavior domain | Canonical owner | Reference-only consumers | Label |
| --- | --- | --- | --- |
| Shared baseline doctrine for both SCE roles (core principles, `context/` authority, quality posture) | Shared snippet constants in `config/pkl/base/shared-content.pkl` (`sharedSceCorePrinciplesSection`, `sharedSceContextAuthoritySection`, `sharedSceQualityPosturePrefixBullets`, `sharedSceLongTermQualityBullet`) | `agents["shared-context-plan"].canonicalBody`, `agents["shared-context-code"].canonicalBody`, generated Plan/Code agent files in OpenCode and Claude | dedup/complete |
| Plan role mission, hard boundaries, startup, and planning procedure | `agents["shared-context-plan"].canonicalBody` in `config/pkl/base/shared-content.pkl` | `config/.opencode/agent/Shared Context Plan.md`, `config/.claude/agents/shared-context-plan.md` | intentional/keep |
| Code role mission, hard boundaries, one-task execution flow, and feedback loop | `agents["shared-context-code"].canonicalBody` in `config/pkl/base/shared-content.pkl` | `config/.opencode/agent/Shared Context Code.md`, `config/.claude/agents/shared-context-code.md` | intentional/keep |
| `/next-task` detailed phase contracts | `skills["sce-plan-review"]`, `skills["sce-task-execution"]`, and `skills["sce-context-sync"]` in `config/pkl/base/shared-content.pkl` | `commands["next-task"].canonicalBody`, `config/.opencode/command/next-task.md`, `config/.claude/commands/next-task.md` | dedup/complete |
| `/change-to-plan` clarification and plan-shape contracts | `skills["sce-plan-authoring"]` in `config/pkl/base/shared-content.pkl` | `commands["change-to-plan"].canonicalBody`, `config/.opencode/command/change-to-plan.md`, `config/.claude/commands/change-to-plan.md` | dedup/complete |
| `/commit` commit grammar and atomic split guidance | `skills["sce-atomic-commit"]` in `config/pkl/base/shared-content.pkl` | `commands["commit"].canonicalBody`, `config/.opencode/command/commit.md`, `config/.claude/commands/commit.md` | dedup/complete |
| Skill phase contracts with command-level invocation overlap | Per-skill canonical bodies in `config/pkl/base/shared-content.pkl` | Related command wrappers (`/next-task`, `/change-to-plan`, `/commit`) and generated command docs | intentional/keep (layered ownership) |

## Guardrails for follow-up tasks

- Keep Plan/Code role separation unchanged; dedup is shared-baseline extraction plus thin-command delegation, not role merge.
- Keep `/next-task`, `/change-to-plan`, and `/commit` command bodies at orchestration/gating scope.
- Keep detailed acceptance and behavior contracts in skill-owned canonical bodies.
