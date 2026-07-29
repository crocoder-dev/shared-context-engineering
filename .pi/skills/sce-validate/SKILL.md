---
name: sce-validate
description: >
  Validate one completed SCE plan and synchronize its durable context
---

# SCE Validate

## Purpose

Own this workflow from input parsing through its terminal user-visible response.
Execute the phases below directly and in order. Phase statuses are internal state,
not inter-skill handoffs. Do not invoke another SCE skill, sibling package, or
workflow command. Follow the canonical workflow's steps, gates, and stops exactly
as written: never invent, skip, reorder, or merge a step.

## User-visible output

Use `references/output.md` for every gate and terminal response. Render no raw
internal state. The reference contains only human-visible Markdown layouts.
User-visible output is limited to those layouts: never invent a layout, and never
wrap one in an added preamble, commentary, summary, or extra section.

## Canonical workflow

SCE VALIDATE `$ARGUMENTS`

## Input

`$ARGUMENTS` is the plan name or plan path.

- The plan name or path is required.
- Resolve exactly one plan. Do not invent a plan from the conversation or from
  incomplete nearby work.

When `$ARGUMENTS` is empty, report that a plan name or path is required, state
the expected argument, and stop. Do not infer the plan from repository state or
the conversation.

Pass the plan name or path to the **Validation phase** unmodified. Do not restate,
summarize, or pre-scope it.

Every `{plan-path}` and `{candidate-path}` emitted anywhere in this workflow is
the path carried by the **Validation phase** in its Markdown result (`Plan:`, or a
candidate path), so every emitted command is directly runnable.

## Workflow

### 1. Validate the plan

Run the **Validation phase** with the plan name or path.

the **Validation phase** exclusively owns:

- Resolving one plan.
- Confirming every implementation task is complete.
- Running full validation and acceptance-criteria checks.
- Removing temporary scaffolding.
- Writing the Validation Report into the plan.
- Returning one Markdown validation result.

Do not duplicate any of it. Do not write the Validation Report yourself.

Branch on the report's `Status:`.

`blocked` -> Do not run context synchronization. Print the blocked Markdown
report as returned. Do not rephrase it into a different layout. Stop.

`failed` -> Do not run context synchronization. Print the failed Markdown
report as returned. It is already a session handoff: self-contained, actionable,
and ending with `/validate {plan-path}` after repairs.

Do not rewrite it into a shorter summary. Do not drop the retry command. Do not
add an alternate continuation that replaces `/validate`.

Stop. Do not mark the plan finished. Do not continue to context synchronization.
Do not start the repair work in this workflow unless the user explicitly asks
to continue here; the default is that the handoff can leave this session.

`validated` -> Pass the complete validated Markdown result to
the **Plan context synchronization phase**.

Do not reconstruct, summarize, or reinterpret the validation result before
passing it.

### 2. Synchronize plan context

Run the **Plan context synchronization phase** only with a `Status: validated` Markdown result
from the **Validation phase**.

Do not run the **Plan context synchronization phase** for `failed` or `blocked`. Those are not
success states.

Pass the validated result verbatim. It is the authoritative handoff, and
the **Plan context synchronization phase** owns reading the plan path, required context paths,
validation evidence, and reported context impact out of it.

Do not restate, summarize, or reconstruct any part of the validation result.

Branch on the synchronization result.

`blocked` -> Validation itself succeeded and is already recorded in the plan.
Render the **Context synchronization blocked** layout from
`references/output.md`. Nothing records the skipped synchronization, so it is
lost once this session ends.

Stop.

`synced` | `no_context_change` -> Print out the report
the **Plan context synchronization phase** returned. Continue to the next step.

### 3. Report completion

Return exactly one completion block. Do not start another workflow.

Render the **Completion** layout from `references/output.md`.

When the synchronization status was `no_context_change`, keep the same
completion block. "Synchronized" here means the final context pass finished
successfully, including the case where no edit was warranted.

Stop.

## Rules

- Validate at most one plan per invocation.
- Do not duplicate the internal instructions of embedded phases.
- Do not run final validation when implementation tasks remain; the **Validation phase**
  returns `blocked`, and this workflow stops.
- Run the **Plan context synchronization phase** only when the **Validation phase** returned
  `Status: validated`. Do not run it for `failed` or `blocked`.
- On `failed`, print the handoff Markdown as returned and stop. Preserve the
  retry `/validate {plan-path}` instruction. Do not synchronize context.
- Do not implement remaining plan tasks from this workflow unless the user
  explicitly continues in-session after a failed handoff.
- Do not create a Git commit or push changes.
- Do not mark the plan archived or delete the plan.
- Do not execute a follow-up `/next-task`, `/change-to-plan`, or `/validate`
  yourself.
- Do not infer success when an embedded phase returns a non-success status.
- Preserve validation evidence already written to the plan when context
  synchronization fails.

## Embedded phase behavior

## Internal phase: Validation phase

# SCE Validation

## Purpose

Prove that one finished SCE plan meets its acceptance criteria and repository
validation bar, then record the evidence on the plan and return one Markdown
result.

This skill owns:

- Resolving one plan.
- Confirming every implementation task is complete.
- Running the plan's full validation commands and each acceptance criterion
  check.
- Removing temporary scaffolding introduced by the change.
- Writing the Validation Report into the plan.
- Marking acceptance criteria against the evidence.
- Recording one Markdown validation result.

Return a result matching:

`references/output.md`

Write plan-file evidence matching:

the **Plan-file validation report** section embedded in this file

Context synchronization is not this skill's job. The invoking `/validate`
workflow runs the **Plan context synchronization phase** only after a `validated` result.

## Input

The invoking workflow provides:

- A plan name or path.

## Workflow

### 1. Resolve the plan

Resolve the supplied plan name or path to exactly one existing plan under
`context/plans/`.

When no plan can be found, set internal status `blocked`.

When multiple plans match and none can be selected safely, set internal status `blocked`
with the matching candidates.

Read the selected plan before exploring the repository.

### 2. Confirm implementation is finished

Set internal status `blocked` with incomplete tasks listed when any implementation task
remains incomplete.

Final validation measures finished work. Do not run the full suite against a
partial stack, and do not complete remaining tasks here.

### 3. Read the validation contract from the plan

From the plan, collect:

- Every acceptance criterion and its `Validate:` check.
- The `Full validation` command list.
- The `Context sync` requirements, for the context-impact handoff only.

Set internal status `blocked` when the plan has no usable acceptance criteria, or when no
validation commands can be determined from the plan or repository conventions.

Prefer the plan's authored checks. Fall back to repository-primary test, lint,
and format commands only when `Full validation` is absent, and record that
fallback under notes on a `validated` or `failed` result.

### 4. Remove temporary scaffolding

Before or while running checks, remove temporary scaffolding introduced during
the change when it is clearly throwaway:

- Debug-only patches or flags left enabled.
- Temporary files or intermediate artifacts not part of the delivered design.
- Local scaffolding the plan or task notes mark as temporary.

Do not delete durable product code, tests, configuration, or context files.

Record every removed path. When nothing temporary remains, report `None.`

### 5. Run full validation and acceptance checks

Run the plan's `Full validation` commands.

Then verify each acceptance criterion using its `Validate:` line. Prefer a
runnable command. Use a named inspection only when the criterion authorizes it,
and say exactly what was inspected.

When a check fails, record the failure and continue gathering evidence. Do not
modify tests, application code, or configuration to make a check pass. Final
validation measures the finished work; repair belongs to a later work session,
not this skill.

Never report a check as passed unless it ran successfully or the authorized
inspection confirmed the criterion.

Do not run task-by-task implementation work for incomplete tasks. That belongs
to `/next-task`.

### 6. Update the plan

For `validated` and `failed` outcomes:

- Mark each acceptance criterion checkbox to match the evidence.
- Append or replace the plan's `## Validation Report` section using
  the **Plan-file validation report** section embedded in this file.
- When status is `failed`, the plan-file report must include the retry command
  `/validate {plan path}`.

Do not reopen completed tasks, rewrite task evidence, or change the task stack.

For `blocked`, leave the plan file unchanged.

### 7. Determine context impact for the handoff

On `validated` only, classify the durable context impact of the finished plan
so the **Plan context synchronization phase** can start from the plan's own requirements:

- Start from the plan's `Context sync` section.
- Inspect what the completed implementation actually changed when needed.
- Report required context paths and affected areas.
- Use `none`, `local`, `domain`, or `root` with the same meanings as task-level
  context sync.

Do not edit context files here.

On `failed` or `blocked`, omit context impact; context sync will not run.

### 8. Return the internal state

Set exactly one internal state:

- `validated` when every acceptance criterion is met, required full validation
  passed, and the Validation Report was written.
- `failed` when evidence was captured but required checks or criteria remain
  unsatisfied. Shape it as a session handoff per
  `references/output.md`, ending recommended work with
  `/validate {plan path}`.
- `blocked` when validation cannot proceed safely.

Record only the Markdown report. Do not add explanatory prose before or after
it. Do not return internal state.

## Boundaries

Do not:

- Validate more than one plan.
- Complete remaining implementation tasks.
- Modify tests, application code, or configuration to make a failing check pass.
- Apply lint or format auto-fixes that change product or test files as part of
  making validation green.
- Synchronize durable context under `context/` outside the plan file.
- Create the context root.
- Mark the plan archived or delete the plan.
- Create a Git commit or push changes.
- Invent acceptance criteria the plan does not state.
- Claim verification that was not performed.
- Return a internal state.
- Run plan context synchronization. The workflow owns that step.

## Completion

The skill is complete after:

- One plan was resolved, or resolution failed and was reported.
- Implementation completeness was checked.
- Validation ran to a terminal state, or a blocker prevented it.
- One valid internal state matching `references/output.md` was
  returned.

## Internal phase: Plan context synchronization phase

# SCE Plan Context Sync

## Purpose

Reconcile one fully validated plan with the repository's durable context and
return a Markdown report.

This skill owns:

- Validating the validation handoff.
- Confirming the context root exists.
- Discovering the context required by the finished plan.
- Deciding whether durable context changed.
- Editing and verifying the affected context files.
- Recording one Markdown synchronization report.

Use the report format in:

`references/output.md`

Task-level context sync may already have run after individual tasks. This skill
is the plan-level final pass: it starts from the plan's `Context sync`
requirements and the validated implementation, and closes gaps that remain.

## Input

The invoking workflow provides:

- The complete internal state returned by the **Validation phase**.

The validation result must report:

```markdown
**Status:** validated
**Plan:** {plan path}
```

Treat that Markdown as the authoritative handoff for:

- The resolved plan path.
- Validation commands and outcomes.
- Acceptance-criteria evidence.
- Scaffolding removals.
- Reported context impact, required context paths, and affected areas.

This skill must not be run for `failed` or `blocked` validation results.
Those are not success states. Same rule as the **Task context synchronization phase**: context sync
runs only after a successful prior phase.

Do not reconstruct a missing validation result from conversation history.

## Workflow

### 1. Validate the validation handoff

Confirm that:

- `Status:` is exactly `validated`.
- `Plan:` names an existing plan path.
- Acceptance-criteria evidence is present and every criterion is met.
- Commands run are present.
- A context-impact classification is present.

If the handoff is missing required information or is internally contradictory,
do not modify context. Return a `blocked` Markdown report.

### 2. Confirm the context root

When `context/` does not exist, there is no durable memory to synchronize.
Do not create it, and do not write context files outside it.

Return a `blocked` report whose required action is:

`sce setup --bootstrap-context`

State that validation itself succeeded and is recorded in the plan, and that
plan context synchronization should run again once the context root exists.

Bootstrapping is the user's action, not this skill's.

### 3. Discover applicable context

Start with the validated internal state:

- **Context impact** classification, required context, and affected areas.
- Acceptance-criteria evidence.
- Commands run.

Then read the plan's `Context sync` section and inspect existing repository
context in this order when present:

1. Paths named by the plan's `Context sync` section
2. `context/context-map.md`
3. Context files for the affected domain or subsystem
4. `context/overview.md`
5. `context/architecture.md`
6. `context/glossary.md`
7. `context/patterns.md`
8. Operational, product, or decision records directly related to the finished
   change

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
finished implementation. A file that is absent is a gap; record it in the
report rather than creating it to satisfy the pass.

Report each of the five as verified or edited. Never declare synchronization
done while one of them is unchecked.

#### Plan context requirements

Every path or statement listed under the plan's `Context sync` section must be
accounted for in the report as already accurate or updated. A requirement the
finished code still does not satisfy is a blocker, not a note.

### 4. Determine whether durable context changed

Use the reported context impact as a strong hint, then verify it against the
finished implementation and existing context.

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
- Test output that belongs only in validation evidence.
- Speculation or future work not established by the finished plan.
- Generic engineering practices.

Interpret impact classifications as follows. Each governs which files are
*edited*; none of them waives the mandatory root pass or the plan's Context
sync requirements.

- `none`: Make no edits beyond any correction the root pass or unmet plan
  context requirement turns up.
- `local`: Update the nearest existing authoritative context only when the new
  behavior is not reliably discoverable from code.
- `domain`: Update affected domain context and the context map when its links or
  summaries changed.
- `root`: Update the relevant root context and any affected domain context.

If the reported classification is inconsistent with the actual change, use the
verified classification and explain the difference in the report.

### 5. Synchronize context

Make the smallest coherent documentation change that preserves repository truth.

When editing context:

- Describe the resulting behavior, not the validation session.
- Preserve repository terminology and document structure.
- Remove or correct statements contradicted by the finished implementation.
- Update cross-references when files are added, moved, renamed, or superseded.
- Keep one authoritative statement for each durable fact.
- Avoid copying the validation result verbatim into context files.
- Do not change application code, tests, or plan validation evidence.

Create a new context file only when:

- The knowledge is durable and non-obvious.
- No existing file owns it coherently.
- The new file has a clear place in the context map.

#### Feature existence

Every feature the finished plan implemented must have at least one durable
canonical description discoverable from `context/`, in a domain file under
`context/{domain}/` or in `context/overview.md` for a cross-cutting feature.

When the plan delivered a feature no context file describes, add that
description. Prefer a small, precise domain file over overloading
`overview.md` with detail.

This is not license to narrate the diff: describe what the feature is and how
it behaves, not what was edited during the plan.

#### Glossary

Add a `context/glossary.md` entry for any domain language the plan introduced.
New terminology is durable knowledge whatever the classification is.

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

- Every changed context file accurately reflects the finished implementation.
- No edited statement contradicts the code, plan, or validation evidence.
- Every file in the mandatory root pass was read and confirmed against code
  truth, whether or not it was edited.
- Every plan `Context sync` requirement is met.
- Each feature implemented by the plan has a durable canonical description
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

Do not rerun full-plan validation.

If synchronization cannot be completed without inventing facts or resolving a
material contradiction, preserve safe edits when appropriate and return a
`blocked` report.

### 7. Return the Markdown report

Set exactly one report status:

- `synced`
- `no_context_change`
- `blocked`

`synced` means context files were updated and verified. `no_context_change`
means existing context was checked and no edit was warranted. `blocked` means
context could not be synchronized safely.

Record only the Markdown report. Do not add explanatory prose before or after
it.

## Boundaries

Do not:

- Accept a validation result whose status is not `validated`.
- Accept `failed` or `blocked` validation results.
- Implement or modify application code.
- Modify tests.
- Change task completion status, acceptance-criteria marks, or the Validation
  Report.
- Rerun full-plan validation.
- Select or execute an implementation task.
- Create a Git commit or push changes.
- Create the context root. `sce setup --bootstrap-context` owns that.
- Narrate changed files as documentation. Feature existence is the only reason
  to document a change that introduced no other durable knowledge.
- Delete a context file that has uncommitted changes.
- Return internal state.

## Completion

The skill is complete after:

- The context root was confirmed, or a `blocked` report named
  `sce setup --bootstrap-context` as the required action.
- The mandatory root pass was run.
- Plan context requirements were checked.
- Applicable durable context was synchronized and verified, no context change
  was warranted, or a synchronization blocker was reported.
- One Markdown report matching `references/output.md` was returned.

## Internal persisted-document format: Plan-file validation report

# Plan-file Validation Report

The Markdown section the **Validation phase** appends to the plan file when returning
`validated` or `failed`. Write it at the end of `context/plans/{plan_name}.md`
under exactly one `## Validation Report` heading.

This is plan-file content. The result returned to the workflow is defined
separately in `references/output.md`.

Do not author this section while planning. Only `/validate` through
the **Validation phase** writes it.

## Layout

```markdown
## Validation Report

**Status:** {validated | failed}  
**Date:** {YYYY-MM-DD}

### Commands run

- `{command}` -> exit {code} ({concise outcome summary})
- `{command}` -> exit {code} ({concise outcome summary})

### Scaffolding removed

- `{path}` — {why it was temporary}
- None.

### Success-criteria verification

- [x] AC1: {criterion statement} -> {evidence}
- [ ] AC2: {criterion statement} -> {evidence of failure or not checked}

### Failed checks and follow-ups

- {check}: {problem}; evidence: {command output or inspection}; required: {decision or next action}
- None.

### Residual risks

- {risk}
- None identified.

### Retry

{Only when Status is failed:}

After repairs, rerun:

`/validate {plan path}`
```

## Rules

- Use **Status:** `validated` only when every acceptance criterion is met and
  every required full-validation command passed.
- Use **Status:** `failed` when evidence was captured but required checks or
  criteria remain unsatisfied.
- List every command that ran under **Commands run**, including ones that
  failed. Do not invent exit codes or outcomes.
- Prefer the plan's `Full validation` commands and each criterion's `Validate:`
  line over rediscovering project defaults. Fall back to repository conventions
  only when the plan omits them.
- Mark each acceptance criterion checkbox in the plan's `## Acceptance criteria`
  section to match the evidence. Do not mark a criterion met unless the check
  ran successfully or the inspection named by `Validate:` confirms it.
- Under **Scaffolding removed**, list only temporary debug code, intermediate
  artifacts, or throwaway files introduced during the change. Write `None.` when
  nothing temporary remained.
- Under **Failed checks and follow-ups**, record the failing check and its
  evidence only. Do not describe code or test edits made during validation;
  validation does not modify tests or product code to clear failures. Write
  `None.` when status is `validated`.
- When status is `failed`, always include **Retry** with the exact
  `/validate {plan path}` command. Omit **Retry** when status is `validated`.
- Keep evidence concise and factual. Do not narrate the whole implementation
  history.
- Do not claim context synchronization completed. Plan context sync is a later
  workflow step and runs only after `validated`.
- Do not rewrite task evidence or reopen completed tasks.
- When a previous `## Validation Report` already exists, replace it with the new
  one rather than stacking duplicates.

## Composite control flow

Keep phase results as internal state and continue immediately whenever the
canonical workflow says to continue. Stop only at a user wait or terminal branch.
Approval, clarification, revision, failed-validation repair, and bootstrap waits
resume this same skill in the same session. Never expose an internal phase result
as the workflow's final response.
