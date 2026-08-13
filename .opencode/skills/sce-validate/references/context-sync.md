# SCE Plan Context Sync

## Purpose

Reconcile one fully validated plan with the repository's durable context and
return a Markdown report.

This phase owns:

- Validating the validation handoff.
- Confirming the context root exists.
- Discovering the context required by the finished plan.
- Deciding whether durable context changed.
- Editing and verifying the affected context files.
- Returning one Markdown synchronization report.

Use the report format in:

the **Plan Context Sync Report** below in this file

Task-level context sync may already have run after individual tasks. This phase
is the plan-level final pass: it starts from the plan's `Context sync`
requirements and the validated implementation, and closes gaps that remain.



## Input

The complete Markdown result returned by the validation phase.

The validation result must report:

```markdown
**Status:** validated
**Plan:** {plan path}
```

Treat that Markdown as the authoritative handoff for:

- The resolved plan path.
- Validation commands and outcomes.
- Acceptance-criteria evidence.
- Reported context impact, required context paths, and affected areas.

This phase must not be invoked for `failed` or `blocked` validation results.
Those are not success states. Same rule as `sce-task-context-sync`: context sync
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

Bootstrapping is the user's action, not this phase's.

### 3. Discover applicable context

Start with the validated Markdown result:

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

### 5. Record qualifying architecture decisions

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

Use the discovered context, existing decision records, and this evidence:

- acceptance-criteria and validation evidence.

Identify each qualifying decision, then handle qualifying decisions in
deterministic order:

1. Reuse a written ADR path already returned during this plan when it records the
   same decision.
2. Otherwise invoke `sce-decision` once with exactly one structured decision
   request containing the decision, qualifying evidence, plan and task references,
   related context and ADR paths, and any user-requested status.
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

### 6. Synchronize context

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

Every context file this phase writes must satisfy:

- One topic per file.
- At most 250 lines. When an edit would push a file past 250 lines, split it
  into focused files and link them rather than letting it grow.
- Relative paths in every link to another context file.
- A Mermaid diagram where structure, boundaries, or flows are complex enough
  that prose alone would not carry them.
- Concrete code examples only where they clarify non-trivial behavior.

When detail outgrows a shared file, migrate it into `context/{domain}/`, leave a
concise pointer behind, and link the new file from `context/context-map.md`.

### 7. Verify synchronization

After edits, verify:

- Every changed context file accurately reflects the finished implementation.
- No edited statement contradicts the code, plan, or validation evidence.
- Every qualifying decision has one written or reused ADR path in the report,
  and the report states when no decision qualified.
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

### 8. Return the Markdown report

Return exactly one report status:

- `synced`
- `no_context_change`
- `blocked`

`synced` means context files were updated and verified. `no_context_change`
means existing context was checked and no edit was warranted. `blocked` means
context could not be synchronized safely.

Return only the Markdown report. Do not add explanatory prose before or after
it.

## Plan context synchronization boundaries

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
- Invoke any sibling SCE skill, sibling SCE package, or SCE workflow command
  except `sce-decision`, or invoke `sce-decision` outside the decision gate in
  successful context synchronization.
- Delete a context file that has uncommitted changes.
- Return YAML.

## Completion

The phase is complete after:

- The context root was confirmed, or a `blocked` report named
  `sce setup --bootstrap-context` as the required action.
- The mandatory root pass was run.
- Plan context requirements were checked.
- The decision gate recorded every qualifying ADR path, found no qualifying
  decision, or returned a synchronization blocker.
- Applicable durable context was synchronized and verified, no context change
  was warranted, or a synchronization blocker was reported.
- One Markdown report matching the **Plan Context Sync Report** below in this file was returned.



# Plan Context Sync Report

Return only one completed Markdown report using the applicable variant below.
Do not include unused sections, placeholders, YAML, or a fenced code block.

The `Status` value must be exactly one of:

- `synced`
- `no_context_change`
- `blocked`

The input validation status is always `validated` and does not need to be
repeated as a separate workflow state. This report is not produced for
`failed` or `blocked` validation results.

## Synced variant

# Plan Context Sync Report

**Status:** synced  
**Plan:** `{plan path}`

## Context impact

**Classification:** `{local | domain | root}`  
**Affected areas:** `{comma-separated areas}`

{Explain which durable behavior, architecture, terminology, operation, or
constraint required plan-level synchronization after validation.}

## Plan context requirements

- `{required context path or statement from the plan}` — {met by edit | already accurate}

## Updated context

- `{context file}` — {concise description of the durable truth updated}

## Architecture decisions

- `{written or reused ADR path}` — {decision and status}
- None qualified.

## Root pass

- `context/overview.md` — {verified | edited | absent}
- `context/architecture.md` — {verified | edited | absent}
- `context/glossary.md` — {verified | edited | absent}
- `context/patterns.md` — {verified | edited | absent}
- `context/context-map.md` — {verified | edited | absent}

## Feature existence

- `{feature}` — `{context file that canonically describes it}`

## Verification

- {How the edited context was checked against the finished implementation and validation evidence.}
- {File hygiene: line counts, relative links, diagrams where structure is complex.}
- {Documentation, link, or formatting checks that were run, when applicable.}

## Notes

{Include only non-blocking information worth retaining.
Omit this section when unnecessary.}

---

## No-context-change variant

# Plan Context Sync Report

**Status:** no_context_change  
**Plan:** `{plan path}`

## Context impact

**Classification:** none

{Explain why the finished plan introduced no durable, non-obvious repository
knowledge requiring an update, or why existing context already matched.}

## Plan context requirements

- `{required context path or statement from the plan}` — already accurate
- None listed by the plan.

## Context reviewed

- `{context file or area}` — {what was checked and why it remains accurate}

## Architecture decisions

- `{reused ADR path}` — {decision and status}
- None qualified.

## Root pass

- `context/overview.md` — {verified | absent}
- `context/architecture.md` — {verified | absent}
- `context/glossary.md` — {verified | absent}
- `context/patterns.md` — {verified | absent}
- `context/context-map.md` — {verified | absent}

## Feature existence

- `{feature}` — `{context file that canonically describes it}`, already present.

## Verification

- {How existing context was compared with the finished implementation and validation evidence.}

---

## Blocked variant

# Plan Context Sync Report

**Status:** blocked  
**Plan:** `{plan path}`

## Blocker

**Problem:** {specific synchronization blocker}  
**Impact:** {why context cannot be made authoritative safely}  
**Required action:** {decision or correction required}

## Context changes

- {List safe context edits preserved, or state `No context files were changed.`}

## Architecture decisions

- `{ADR path written or reused before the blocker}` — {decision and status}
- None written or reused before the blocker.

## Retry condition

{State the concrete condition under which plan context synchronization should
run again.}

## Report rules

- Name exact context files when they were changed or reviewed.
- Under **Architecture decisions**, list every ADR path written or reused during
  the decision gate. In a successful report, state `None qualified.` when the
  gate skipped invocation. In a blocked report, state
  `None written or reused before the blocker.` when applicable.
- Report every file in the root pass, including any that is absent.
- Report the missing context root as `blocked`, with `sce setup
  --bootstrap-context` as the required action and the existence of `context/` as
  the retry condition.
- Cover every path or statement listed in the plan's `Context sync` section
  under **Plan context requirements**.
- Omit **Feature existence** only when the plan implemented no feature.
- Describe durable truth, not validation-session chronology.
- Keep evidence concise and factual.
- Do not claim implementation tasks remain open.
- Do not reopen validation checks.
- Do not recommend a next implementation task unless context cannot be repaired
  without one, and then only as the required action.
