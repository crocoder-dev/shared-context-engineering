# Decision: Use Workflow-Oriented Pkl Generation

Date: 2026-07-27
Status: Accepted (Claude inventory and transport superseded 2026-07-29)
Plan: `context/plans/rebuild-pkl-workflow-markdown.md`

## Decision

- Model generated SCE Markdown as four canonical workflow packages: `/change-to-plan`, `/next-task`, `/validate`, and `/commit`.
- Use the project-root `.pi/` prompts, skills, and package-local references as the behavioral baseline.
- Generate the same eight self-contained skill packages for Pi, OpenCode, and Claude. A generated skill may share Pkl source with another skill, but it must not depend on a sibling generated package.
- Preserve canonical YAML phase-result contracts for Pi and OpenCode, while Claude renders target-specific Markdown contracts for the five structured phase skills; fixed statuses and labeled sections preserve command handoff semantics.
- Instantiate `sce-task-context-sync` and `sce-plan-context-sync` from one role-parameterized Pkl skeleton while preserving their distinct handoff gates, lifecycle boundaries, and reports.
- Keep only two generated agents, both for OpenCode and both routing-only: Plan routes to `/change-to-plan`; Code routes to `/next-task`, `/validate`, and `/commit`. Claude and Pi receive no generated agents.
- Remove the automated OpenCode profile and obsolete generated commit, handover, bootstrap, atomic-commit, and legacy context-sync Markdown surfaces.
- Preserve retained non-Markdown generation for OpenCode plugins/config, Claude settings/hooks, the Pi extension, and the SCE config schema.

## Rationale

Workflow packages align generated ownership with the actual planning, task-execution, and final-validation lifecycles. Self-contained package references keep each target installable without cross-skill coupling, while one Pkl sync skeleton prevents task/plan policy drift. Thin agents avoid duplicating behavior already owned by commands and skills.

The automated profile and obsolete Markdown catalog represented a second behavior surface with no required compatibility contract. Removing them reduces drift and makes the generated target inventory exact and parity-checkable.

## Consequences

- Canonical workflow content lives in `config/pkl/base/workflow-{change-to-plan,next-task,validate,commit}.pkl`, with shared types in `workflow-content.pkl` and synchronization policy in `workflow-context-sync.pkl`; Claude phase-result overrides live in `config/pkl/renderers/claude-workflow-results.pkl`.
- Generated inventory is four commands and eight skill packages for each target, plus two OpenCode agents. Pi also retains its extension; Claude retains settings and hooks.
- `config/pkl/renderers/metadata-coverage-check.pkl`, `config/pkl/check-generated.sh`, and the root flake parity check enforce exact inventories, nested package references, and forbidden removed trees.
- `config/automated/.opencode` and `config/.claude/agents` are forbidden outputs rather than compatibility surfaces.

## Superseded scope

This decision supersedes the generated path matrix, paired-output counts, and target inventory in `2026-02-28-pkl-generation-architecture.md`. That decision's canonical-Pkl-source, deterministic-rendering, and generated-vs-runtime ownership principles remain in force.

`2026-07-29-claude-workflow-skill-packages.md` supersedes this decision's claims that Claude receives the same eight phase packages and transports Markdown phase results. Pi/OpenCode decomposition and all other ownership decisions here remain in force.

The separate Plan/Code role decision in `2026-03-03-plan-code-agent-separation.md` remains in force, with agents now explicitly limited to routing.
