# SCE workflow vocabulary

This file is the canonical language guide for SCE workflow and skill instructions.
Use these terms consistently in canonical Pkl, generated workflow packages, and
workflow context. Do not create a second glossary for the same concepts.

## Core terms

- **Command** — a user-facing invocation such as `/next-task` or `/validate`.
- **Workflow** — the complete end-to-end procedure coordinated by a workflow
  package's `SKILL.md`.
- **Skill** — an invokable instruction package. A workflow package is implemented
  as a skill; `sce-decision` is an internal skill. Embedded phase references are
  not skills.
- **Phase** — an embedded operation inside a workflow, such as plan review, task
  execution, context synchronization, validation, or atomic commit analysis.
- **Step** — one numbered action inside a workflow or phase.
- **Reference** — a Markdown instruction, policy, format, or template file read by
  a workflow, phase, or skill at a named boundary.
- **Internal result** — structured data returned to a caller and not rendered
  directly to the user. `status` is its outcome discriminator.
- **Report** — formatted Markdown intended for a user or a persisted report
  section.
- **Layout** — one named user-visible format in an output reference.
- **Handoff** — the act of passing an authoritative result or guidance to another
  phase, skill, workflow step, or session. Do not use *handoff* as a second name
  for every result object.
- **Completion record** — execution evidence persisted on a completed task in its
  plan.
- **Handover document** — the persisted session-continuation document under
  `context/handovers/`.

## Verbs

Use verbs according to the object they operate on:

- **invoke** a skill or command;
- **run** a workflow or phase;
- **return** an internal result;
- **branch on** a result's `status`;
- **render** a report or named layout;
- **write** or **update** a persisted file;
- **pass** an authoritative result unchanged when the receiving contract requires
  it.

## Lifecycle vocabulary

Keep lifecycle terms distinct because they describe different state domains:

- a plan task is `done`;
- task execution returns `complete`;
- task context synchronization returns `synced` or `no_context_change`;
- final plan validation returns `validated`.

When prose could be ambiguous, name the domain: *task execution is complete*,
*context synchronization is blocked*, or *plan validation failed*. Do not flatten
these into a generic *complete* or *success* state.

## Instruction and output style

- Use sentence-case headings without trailing punctuation.
- Use American English in workflow instructions and examples.
- Use **Next step** for user-visible continuation wording unless a persisted
  schema owns a different heading.
- Call private structured phase data an **internal result**, not *internal state*.
- Call formatted validation and synchronization Markdown a **report**, not a
  *result*.
- Keep user-visible output in the named layout or report that owns it. Do not add
  wrapper prose around a rendered layout or report.
- Keep ownership rationale in workflow context such as
  `dedup-ownership-table.md`; runtime instructions should say what to read or do,
  not repeat a long list of everything another file owns.

## Document roles

Use document structure according to responsibility rather than forcing unrelated
files into one template:

- **Workflow entrypoint:** execution contract, phase/reference routing, input,
  workflow control flow, rules.
- **Phase reference:** phase title, purpose/input as needed, ordered procedure,
  returned internal result or report, boundaries.
- **Output reference:** named user-visible layouts plus only the field-mapping or
  rendering rules they require.
- **Persisted template/reference:** document title, template/schema, semantic or
  completeness rules, and no workflow control flow.

Specialized sections are allowed when they own a real contract, but equivalent
concepts must keep the vocabulary above.
