---
name: sce-context-load
description: >
  Internal SCE workflow skill that loads the durable context in `context/`
  relevant to one focus, reports gaps and context-versus-code drift, and returns
  one Markdown result: loaded or bootstrap_required. Use from /change-to-plan and
  any workflow that needs durable context before acting. Do not modify context,
  repair drift, plan, or implement.
compatibility: claude
---

# SCE Context Load

## Purpose

Load the durable context needed to reason about one focus, and no more.

`context/` is AI-first memory describing current state. This skill turns it into
a scoped brief so later phases start from recorded truth instead of rediscovering
the repository.

This skill owns:

- Confirming `context/` exists.
- Reading the context entry points.
- Selecting the domain context relevant to the focus.
- Reporting focus areas with no durable context.
- Reporting context that contradicts the code.
- Returning one structured context brief.

Return a result matching:

the **Result contract** section in this file

## Input

The invoking workflow provides:

- One focus: a change request, a task, or a named area.
- Optionally, paths or areas already known to be relevant.

## Workflow

### 1. Confirm the context root

When `context/` does not exist, return `bootstrap_required` immediately. Read
nothing further.

Bootstrapping is the invoking workflow's decision, not this skill's.

### 2. Read the entry points

Read, when present:

- `context/context-map.md`
- `context/overview.md`
- `context/glossary.md`

Read `context/architecture.md` when the focus touches structure, boundaries, or
data flow. Read `context/patterns.md` when it touches conventions the change
must follow.

A missing entry point is a gap, not a failure. Record it and continue.

### 3. Select the relevant domain context

Consult `context/context-map.md` before any broad exploration. The map's
annotations name what each domain file owns; use them to select files, rather
than globbing or searching `context/`.

Select only files whose subject overlaps the focus. Follow at most one level of
links out of a selected file, and only when the link is needed to understand the
focus.

Do not read every domain file. A brief that includes everything has selected
nothing.

Record focus areas with no matching context file under `gaps`.

### 4. Check recorded context against the code

For each selected file, spot-check its central claims against the code it
describes.

When context and code diverge, the code is the source of truth. Record the
divergence under `drift` with what context says, what the code shows, and the
repair the context needs.

Do not repair it here. Later phases decide whether repair belongs in the current
work.

Keep this proportional: check the claims the focus depends on, not every
sentence.

### 5. Return the brief

Return exactly one structured result:

- `loaded`
- `bootstrap_required`

Report facts the invoking workflow can act on. A brief that only lists file
paths has moved no knowledge.

Return only the structured result. Do not add explanatory prose before or after
it.

## Boundaries

Do not:

- Create, update, move, or delete any file under `context/`.
- Bootstrap `context/`.
- Repair drift or stale context.
- Modify application code or tests.
- Read the entire `context/` tree by default.
- Explore the repository beyond what the focus and the selected context require.
- Ask the user questions. Report gaps and drift, and let the invoking workflow
  decide.
- Author a plan, select a task, or implement anything.

## Completion

The skill is complete after:

- The context root was confirmed, or `bootstrap_required` was returned.
- The entry points were read, and the relevant domain context was selected and
  read.
- One valid result matching the **Result contract** section in this file was returned.

## Result contract

# SCE Context Load Result Contract

Return exactly one Markdown document using one layout below. The first line
after the title is the status source consumed by the invoking command. Use
the headings and labels exactly as written, omit optional sections that do
not apply, and do not add prose outside the selected layout.

List every file read and no file that was not read. Key facts are durable
statements, not planning or implementation recommendations. Empty required
lists must contain `- None.`. This skill never reports context edits.

## Status: `loaded`

Use when the context root exists and relevant context was read.

```markdown
# Context Load Result

Status: loaded

## Context root

{context_root}

## Entry points

### {path}

- Read: {true|false}
- Reason: {reason; omit only when read is true and no reason is needed}

## Domain context

### {path}

**Relevance:** {relevance}

**Key facts:**

- {key_fact}

## Gaps

### {area}

{detail}

## Drift

### {path}

- Context says: {context_says}
- Code says: {code_says}
- Repair needed: {repair_needed}
```

`Context root`, `Entry points`, `Domain context`, and `Gaps` are required.
Repeat the entry-point, domain-context, gap, and drift blocks as needed.
Include `Drift` only when recorded context contradicts code. Record gaps for
absent entry points and focus areas without durable context.

## Status: `bootstrap_required`

Use when the context root does not exist. Read and create nothing.

```markdown
# Context Load Result

Status: bootstrap_required

## Context root

{context_root}

## Reason

{reason}
```

## Control flow

This skill is one phase of a workflow, not a turn. Return the result to the
invoking command and let it continue in the same turn. Do not present the
result to the user as workflow output, and do not end your turn after
returning it — the invoking command decides what the user sees and when the
workflow stops.
