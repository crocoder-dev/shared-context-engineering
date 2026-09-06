# Task context synchronization phase

Run this phase for step 3 of the workflow, and only when task execution returned
`complete`. It updates durable repository knowledge in `context/` so the next
session inherits what this task established. It never touches code, tests, or
plan state.

Input: either the complete `complete` result from the task execution phase
(same-session), passed verbatim, or the plan path and task ID a plan-review
recovery step resolved for a `blocked` task, together with that task's own
completed record — read directly from the plan — and its persisted `Context
synchronization blocker` when present (cross-session retry). Whichever was
supplied is the authoritative source, and this phase owns reading the plan,
task, changed files, verification evidence, and reported context impact out
of it.

Do not restate, summarize, or reconstruct any part of it. Do not reconstruct a
missing execution result or completed task record from conversation history.

A live execution result must have:

```text
status: complete
```

A cross-session retry has no separate `status` field to check; the completed
task record's presence in the plan, identified by plan path and task ID, is
itself the authoritative signal.

Use the report format in:

`references/sync-report.md`

Treat whichever source was supplied — the live execution result, or the
completed task record read directly from the plan — as the authoritative
source for:

- The resolved plan and completed task.
- `changes.files_changed`, or the completed task record's own `Files changed`
  field on retry, already attributed relative to the pre-edit Git baseline.
- Files changed by implementation.
- The task's `Result` (or implementation summary, for a live result).
- `Verify` outcomes (or verification evidence, for a live result).
- Done-check evidence.
- Reported context impact.

This phase must not be run for `declined`, `blocked`, or `incomplete` execution
results.

## 3.1 Validate the handoff

Confirm that:

- A live execution result has `status` exactly `complete`; a cross-session
  retry has no `status` field to check and is authoritative by the completed
  task record's presence in the plan.
- A resolved plan path and task ID are present; a live execution result
  carries them in its `plan` and `task` objects, and a cross-session retry
  receives them directly from the caller that resolved the debt task.
- Exactly one completed task is identified, and — on retry — its record is
  read directly from the plan by that plan path and task ID rather than
  reconstructed in-band.
- Changed files and a `Result` (an implementation summary, for a live result)
  are present.
- `Verify` outcomes (verification evidence, for a live result) are present.
- Done-check evidence is present.
- A context-impact classification is present.

If the required information is missing, the completed task record cannot be
read from the plan, or either is internally contradictory, do not modify
context. Return a `blocked` Markdown report.

## 3.2 Confirm the context root

When `context/` does not exist, there is no durable memory to synchronize. Do not
create it, and do not write context files outside it.

Return a `blocked` report whose required action is:

`sce setup --bootstrap-context`

State that the task itself is complete and recorded in the plan, and that
synchronization should run again once the context root exists.

Bootstrapping is the user's action, not this phase's.

## 3.3 Discover applicable context

Start with the execution result:

- `context_impact.classification`
- `context_impact.affected_areas`
- Changed files.
- Implementation summary.
- Done-check evidence.

Use the reported context impact to scope discovery as well as editing:

- `none`: Inspect the implementation evidence only. Do not open unrelated context
  merely to prove the absence of impact. Return `no_context_change` when the
  classification is consistent with the completed change.
- `local`: Inspect only the nearest authoritative context for the affected feature,
  component, or subsystem.
- `domain`: Inspect authoritative context for the affected domain or subsystem.
  Inspect `context/context-map.md` only when discoverability, links, summaries, or
  ownership of context changed.
- `root`: Inspect the affected domain context plus only the root files implicated
  by the completed change.

Use these root signals when `root` context may be affected:

- Cross-cutting repository capability or externally observable repository-wide
  behavior -> `context/overview.md`.
- Architecture, ownership, dependency direction, or system boundary ->
  `context/architecture.md`.
- Canonical domain terminology -> `context/glossary.md`.
- Repository-wide implementation or workflow convention -> `context/patterns.md`.
- Context topology, indexing, file movement, or discoverability ->
  `context/context-map.md`.

Do not read a root file merely because it exists. Do not scan or rewrite the
entire `context/` tree by default.

Use existing context links to locate authoritative files when they are already in
scope. Do not create a new context file when an existing authoritative file can be
updated coherently.

If direct implementation evidence clearly contradicts the reported impact, widen
or narrow the verified classification only as far as the evidence requires and
explain the change in the report.

## 3.4 Determine whether durable context changed

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

Interpret impact classifications as follows:

- `none`: No durable context edit is warranted when the classification is
  consistent with the completed implementation.
- `local`: Update the nearest existing authoritative context only when the new
  behavior is durable, non-obvious, and not reliably discoverable from code.
- `domain`: Update affected domain context; update the context map only when its
  links, summaries, or ownership changed.
- `root`: Update affected domain context and only the root files implicated by
  the root signals above.

A change is `root` when it introduces cross-cutting behavior, repository-wide
policy or contracts, an architecture or ownership boundary, or a change to
canonical terminology. A change confined to one feature or domain, with no
repository-wide behavior, architecture, or terminology impact, is `domain` or
`local`.

## 3.5 Record qualifying architecture decisions

During this successful synchronization, determine whether the completed change
establishes or changes a system-wide important constraint involving one or more
of:

- System boundaries or ownership.
- Public or cross-domain interfaces.
- Data models or persistence.
- Compatibility contracts.
- Security posture.
- Deployment or distribution strategy.
- A major dependency.
- A similarly durable constraint that is costly or risky to reverse.

Routine implementation details, local refactors, naming and formatting choices,
temporary experiments, and easily reversible choices do not qualify. Do not
invoke a decision skill for them.

Use the completed task's execution and done-check evidence as the primary signal.
Inspect existing decision records only when that evidence indicates a qualifying
system-wide decision or a directly related ADR is already known from in-scope
context.

Identify each qualifying decision, then handle qualifying decisions in
deterministic order:

1. Reuse a written ADR path already returned during this plan when it records the
   same decision.
2. Otherwise invoke `sce-decision` once with exactly one structured decision
   request containing the decision, qualifying evidence, plan and task
   references, related in-scope context and ADR paths, and any user-requested
   status.
3. On `written`, retain the returned `adr_path` as synchronization evidence and
   make it available for current-state context links before synchronization
   completes. Reuse is valid evidence; do not create a duplicate ADR.
4. On `blocked`, stop before current-state context edits and return a `blocked`
   synchronization report carrying the decision-writing problem, impact, required
   action, and retry condition.

Invoke `sce-decision` only here, after a successful execution or validation
handoff and during context synchronization. Do not invoke it from a non-success
branch or for any non-decision purpose. When no decision qualifies, continue
without invoking it and record that outcome in synchronization evidence.

## 3.6 Synchronize context

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

Do not create context merely because the task implemented a feature. A feature
needs a durable canonical description only when the completed implementation
establishes non-obvious behavior, a contract, boundary, invariant, operational
fact, or other knowledge future work would not reliably recover from code.

Add or update `context/glossary.md` only when the task introduced or changed
canonical domain language that future work needs to share. New identifiers or
implementation-local names do not qualify by themselves.

### File hygiene

Every context file this phase writes must satisfy:

- One topic per file.
- At most 250 lines. When an edit would push a file past 250 lines, split it into
  focused files and link them rather than letting it grow.
- Relative paths in every link to another context file.
- A Mermaid diagram where structure, boundaries, or flows are complex enough that
  prose alone would not carry them.
- Concrete code examples only where they clarify non-trivial behavior.

When detail outgrows a shared file, migrate it into `context/{domain}/`, leave a
concise pointer behind, and link the new file from `context/context-map.md`.

## 3.7 Verify synchronization

After edits, verify:

- Every context file selected by the verified impact scope was checked against the
  completed implementation.
- Every changed context file accurately reflects the completed implementation.
- No edited statement contradicts the code, plan, or execution evidence.
- Every qualifying decision has one written or reused ADR path in the report, and
  the report states when no decision qualified.
- Every changed file is at or below 250 lines, covers one topic, and links other
  context files by relative path.
- Diagrams are present where structure, boundaries, or flows are complex.
- Links and referenced paths resolve when practical to check.
- New context files are reachable from the context map or another authoritative
  index.
- No context outside the verified scope was modified.
- Any widening or narrowing of the reported impact is explained by direct
  implementation evidence.

Use focused documentation, link, or formatting checks when available.

Do not run full application or plan validation.

If synchronization cannot be completed without inventing facts or resolving a
material contradiction, preserve safe edits when appropriate and return a
`blocked` report.

## 3.8 Return the Markdown report

Set exactly one report status:

- `synced`
- `no_context_change`
- `blocked`

`synced` means scoped context files were updated and verified.
`no_context_change` means the completed task was checked at the scope implied by
its verified impact and no edit was warranted. `blocked` means context could not
be synchronized safely.

A `blocked` report always writes the plan path and task ID/title as identity,
plus a `Context synchronization blocker` section (blocker, required action,
retry condition), using the same field names the plan's completion record
uses, so the plan-review recovery step can persist the blocker verbatim and a
future retry can read the completed task record directly from the plan by
plan path and task ID.

Record only the Markdown report. Do not add explanatory prose before or after it.

Do not determine whether the plan is complete. The `/next-task` workflow owns
that decision after context synchronization.

## Task context synchronization boundaries

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
- Narrate changed files as documentation.
- Turn task synchronization into a whole-repository context audit; whole-context
  reconciliation belongs to the explicit audit workflow.
- Invoke any sibling SCE skill, sibling SCE package, or SCE workflow command
  except `sce-decision`, or invoke `sce-decision` outside the decision gate in
  successful context synchronization.
- Delete a context file that has uncommitted changes.
- Return an execution-style internal state.
