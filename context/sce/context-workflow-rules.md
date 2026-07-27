# Context Workflow Rules

## Purpose

`context/` is durable, AI-first repository memory. It records the current state
needed to plan, implement, and validate changes without rediscovering the
repository. Code and executable configuration remain the source of truth; when
context disagrees with them, context must be repaired.

The workflow lifecycle is described in
[Shared Context Plan Workflow](shared-context-plan-workflow.md) and
[Shared Context Code Workflows](shared-context-code-workflow.md).

## Bootstrap boundary

When `sce-context-load` reports that `context/` is absent, the active workflow
stops. It does not create durable context. The user bootstraps it with:

`sce setup --bootstrap-context`

That command owns creation of the baseline context tree. The baseline contains:

- `context/overview.md`
- `context/architecture.md`
- `context/patterns.md`
- `context/glossary.md`
- `context/context-map.md`
- `context/plans/`
- `context/handovers/`
- `context/decisions/`
- `context/tmp/` with a `.gitignore` that ignores scratch content

Bootstrap must not invent application details. For a repository without
application code, baseline documents remain empty or placeholder-only. The
context map links the baseline entry points, and the resulting tree is committed
as shared memory. After bootstrap, the waiting workflow reloads context and
continues with the original request.

## Ongoing invariants

### Current-state orientation

- Describe resulting behavior and structure, not completed-work summaries or a
  narrative changelog.
- Repair context when code or executable configuration has outrun it.
- Keep one authoritative location for each durable fact.

### File hygiene

- Keep one topic per file and every context file at or below 250 lines.
- Split growing detail into focused domain files and link them with relative
  Markdown links.
- Use a Mermaid diagram when structure, boundaries, or flow are too complex for
  prose alone.
- Include code examples only when they clarify non-trivial behavior.
- Keep root context concise and put feature detail under `context/{domain}/`.

### Authority and deletion safety

- Context-maintenance phases may create, edit, move, rename, or delete files
  under `context/`.
- New top-level context domains may be created when durable knowledge needs one.
- Do not delete a context file with uncommitted changes.
- Planning reads durable context but does not modify it outside an authored plan.
  Planning does not authorize implementation.

### Durable and disposable areas

- `context/plans/` contains active execution artifacts, not durable history.
- `context/handovers/` contains task-transition notes.
- `context/tmp/` is ignored scratch space.
- Promote lasting outcomes into current-state domain files or
  `context/decisions/` when rationale must remain discoverable.

### Discoverability and feature existence

- `context/context-map.md` is the index. New canonical context must be linked
  there, and moved or renamed files require updated cross-references.
- Every implemented feature has at least one durable canonical description
  reachable from the map: a focused domain file, or `context/overview.md` for a
  genuinely cross-cutting feature.

## Synchronization lifecycle

Synchronization is split by lifecycle boundary. Both phases share the canonical
rules in `config/pkl/base/workflow-context-sync.pkl`, but receive different
authoritative handoffs.

### Task synchronization

`sce-task-context-sync` runs from `/next-task` only after
`sce-task-execution` returns `complete`. It reconciles one implemented and
verified task with durable context. Declined, blocked, and incomplete executions
do not enter synchronization.

### Plan synchronization

`sce-plan-context-sync` runs from `/validate` only after `sce-validation`
returns `Status: validated`. It performs the final plan-level pass using the
plan's Context sync requirements and validation evidence. Failed or blocked
validation does not enter synchronization.

A synchronization blocker does not undo the successful prior phase. The task or
validation evidence remains recorded, but the workflow stops because durable
context is out of date and must be reconciled before continuing or closing the
plan.

### Mandatory synchronization pass

Every task and plan synchronization verifies these files against code truth,
whether or not an edit is warranted:

- `context/overview.md`
- `context/architecture.md`
- `context/glossary.md`
- `context/patterns.md`
- `context/context-map.md`

Synchronization then inspects the affected domain, classifies the impact, and
makes the smallest coherent documentation change:

- `none`: no edit unless the mandatory pass finds drift.
- `local`: update the nearest authoritative context when behavior is not obvious
  from code.
- `domain`: update affected domain context and map metadata or links.
- `root`: update root and domain context for cross-cutting policy, architecture,
  ownership, or terminology changes.

Every pass also accounts for feature existence, adds glossary entries for new
domain language, verifies relative links and line limits, and reports each root
file as verified, edited, or absent. Task synchronization does not run full-plan
validation, and plan synchronization does not rerun final validation.

## Canonical sources

- `config/pkl/base/workflow-change-to-plan.pkl`
- `config/pkl/base/workflow-next-task.pkl`
- `config/pkl/base/workflow-validate.pkl`
- `config/pkl/base/workflow-context-sync.pkl`
