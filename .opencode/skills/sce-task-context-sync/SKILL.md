---
name: sce-task-context-sync
description: >
  Internal SCE workflow skill that accepts a successful status: complete result
  from sce-task-execution, reconciles the completed implementation with durable
  repository context, and returns a Markdown synchronization report. Invoke only
  after one task has been implemented and verified successfully. Do not implement
  application code, change plan state, determine whether the plan is complete,
  run final validation, or select another task.
compatibility: opencode
---

# SCE Task Context Sync

## Purpose

Reconcile one completed task with the repository's durable context and return a
Markdown report.

This skill owns:

- Validating the execution handoff.
- Confirming the context root exists.
- Discovering the context affected by one completed task.
- Deciding whether durable context changed.
- Editing and verifying the affected context files.
- Returning one Markdown synchronization report.

Use the report format in:

`references/sync-report.md`

## Input

The invoking workflow provides:

- The complete result returned by `sce-task-execution`.

The execution result must have:

```yaml
status: complete
```

Treat the execution result as the authoritative handoff for:

- The resolved plan and completed task.
- Files changed by implementation.
- Implementation summary.
- Verification evidence.
- Done-check evidence.
- Reported context impact.

This skill must not be invoked for `declined`, `blocked`, or `incomplete`
execution results.

Do not reconstruct a missing execution result from conversation history.

## Workflow

### 1. Validate the execution handoff

Confirm that:

- `status` is exactly `complete`.
- A `plan` object with a `path` is present.
- Exactly one completed task is identified.
- Changed files and an implementation summary are present.
- Verification evidence is present.
- Done-check evidence is present.
- A context-impact classification is present.

If the handoff is missing required information or is internally contradictory,
do not modify context. Return a `blocked` Markdown report.

### 2. Confirm the context root

When `context/` does not exist, there is no durable memory to synchronize.
Do not create it, and do not write context files outside it.

Return a `blocked` report whose required action is:

`sce setup --bootstrap-context`

State that the task itself is complete and recorded in the plan, and that
synchronization should run again once the context root exists.

Bootstrapping is the user's action, not this skill's.

### 3. Discover applicable context

Start with the execution result:

- `context_impact.classification`
- `context_impact.affected_areas`
- Changed files.
- Implementation summary.
- Done-check evidence.

Then inspect existing repository context in this order when present:

1. `context/context-map.md`
2. Context files for the affected domain or subsystem
3. `context/overview.md`
4. `context/architecture.md`
5. `context/glossary.md`
6. `context/patterns.md`
7. Operational, product, or decision records directly related to the change

Use the context map and existing links to locate authoritative files.

Do not scan or rewrite the entire `context/` tree by default.

Do not create a new context file when an existing authoritative file can be
updated coherently.

#### The mandatory root pass

Every invocation verifies these five files against code truth, whatever the
reported classification is:

- `context/overview.md`
- `context/architecture.md`
- `context/glossary.md`
- `context/patterns.md`
- `context/context-map.md`

Verifying is not editing. A classification that warrants no root edit still
requires reading each of these and confirming it is not contradicted by the
completed implementation. A file that is absent is a gap; record it in the
report rather than creating it to satisfy the pass.

Report each of the five as verified or edited. Never declare synchronization
done while one of them is unchecked.

Do not create a new context file when an existing authoritative file can be
updated coherently.

### 4. Determine whether durable context changed

Use the reported context impact as a strong hint, then verify it against the
implementation and existing context.

Durable context includes non-obvious repository knowledge such as:

- User-visible or externally observable behavior.
- Architecture, boundaries, ownership, and dependency direction.
- Public interfaces, data contracts, and persistence behavior.
- Operational procedures and important failure modes.
- Security or privacy behavior.
- Shared terminology.
- Intentional limitations and meaningful design decisions.

Do not document:

- Details already obvious from the implementation.
- Temporary debugging information.
- A file-by-file narration of the change.
- Test output that belongs only in task evidence.
- Speculation or future work not established by the completed implementation.
- Generic engineering practices.

Interpret impact classifications as follows. Each governs which files are
*edited*; none of them waives the mandatory root pass.

- `none`: Make no edits beyond any correction the root pass turns up.
- `local`: Update the nearest existing authoritative context only when the new
  behavior is not reliably discoverable from code.
- `domain`: Update affected domain context and the context map when its links or
  summaries changed.
- `root`: Update the relevant root context and any affected domain context.

A change is `root` when it introduces cross-cutting behavior, repository-wide
policy or contracts, an architecture or ownership boundary, or a change to
canonical terminology. A change confined to one feature or domain, with no
repository-wide behavior, architecture, or terminology impact, is `domain` or
`local`: capture its detail in domain files and leave the root files unedited.

If the reported classification is inconsistent with the actual change, use the
verified classification and explain the difference in the report.

### 5. Synchronize context

Make the smallest coherent documentation change that preserves repository truth.

When editing context:

- Describe the resulting behavior, not the implementation session.
- Preserve repository terminology and document structure.
- Remove or correct statements contradicted by the completed implementation.
- Update cross-references when files are added, moved, renamed, or superseded.
- Keep one authoritative statement for each durable fact.
- Avoid copying the execution result verbatim into context files.
- Do not change application code, tests, or plan state.

Create a new context file only when:

- The knowledge is durable and non-obvious.
- No existing file owns it coherently.
- The new file has a clear place in the context map.

#### Feature existence

Every feature the completed task implemented must have at least one durable
canonical description discoverable from `context/`, in a domain file under
`context/{domain}/` or in `context/overview.md` for a cross-cutting feature.

When the task implemented a feature no context file describes, add that
description. A feature that fits no existing domain file gets a new focused
file; do not defer it to a later task. Prefer a small, precise domain file over
overloading `overview.md` with detail.

This is the one case where documentation is warranted by the change itself
rather than by a gap in durable knowledge. It is not license to narrate the
diff: describe what the feature is and how it behaves, not what was edited.

#### Glossary

Add a `context/glossary.md` entry for any domain language the task introduced.
New terminology is durable knowledge whatever the classification is: a `domain`
change that names a new concept still earns its glossary entry.

#### File hygiene

Every context file this skill writes must satisfy:

- One topic per file.
- At most 250 lines. When an edit would push a file past 250 lines, split it
  into focused files and link them rather than letting it grow.
- Relative paths in every link to another context file.
- A Mermaid diagram where structure, boundaries, or flows are complex enough
  that prose alone would not carry them.
- Concrete code examples only where they clarify non-trivial behavior.

When detail outgrows a shared file, migrate it into `context/{domain}/`, leave a
concise pointer behind, and link the new file from `context/context-map.md`.

### 6. Verify synchronization

After edits, verify:

- Every changed context file accurately reflects the completed implementation.
- No edited statement contradicts the code, plan, or execution evidence.
- Every file in the mandatory root pass was read and confirmed against code
  truth, whether or not it was edited.
- Each feature implemented by the task has a durable canonical description
  reachable from `context/`.
- Every changed file is at or below 250 lines, covers one topic, and links other
  context files by relative path.
- Diagrams are present where structure, boundaries, or flows are complex.
- Links and referenced paths resolve when practical to check.
- New context files are reachable from the context map or another authoritative
  index.
- Root context remains concise and delegates details to domain files.
- Unrelated context was not changed.

Use focused documentation, link, or formatting checks when available.

Do not run full application or plan validation.

If synchronization cannot be completed without inventing facts or resolving a
material contradiction, preserve safe edits when appropriate and return a
`blocked` report.

### 7. Return the Markdown report

Return exactly one report status:

- `synced`
- `no_context_change`
- `blocked`

`synced` means context files were updated and verified. `no_context_change`
means existing context was checked and no edit was warranted. `blocked` means
context could not be synchronized safely.

Return only the Markdown report. Do not add explanatory prose before or after
it.

Do not determine whether the plan is complete. The invoking `/next-task`
workflow owns that decision after context synchronization.

## Boundaries

Do not:

- Accept an execution result whose status is not `complete`.
- Implement or modify application code.
- Modify tests.
- Change task completion status or plan evidence.
- Determine whether the plan is complete.
- Select or execute another task.
- Run full-plan validation.
- Mark the plan validated, closed, or archived.
- Create a Git commit or push changes.
- Create the context root. `sce setup --bootstrap-context` owns that.
- Narrate changed files as documentation. Feature existence is the only reason
  to document a change that introduced no other durable knowledge.
- Delete a context file that has uncommitted changes.
- Return an execution-style YAML result.

## Completion

The skill is complete after:

- The context root was confirmed, or a `blocked` report named
  `sce setup --bootstrap-context` as the required action.
- The mandatory root pass was run.
- Applicable durable context was synchronized and verified, no context change
  was warranted, or a synchronization blocker was reported.
- One Markdown report matching `references/sync-report.md` was returned.
