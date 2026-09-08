# Context load phase

Run this phase for step 1 of the workflow, with the change request as the focus.

`context/` is durable AI-first memory describing current state. Load it before
planning so the plan starts from recorded truth. Where context and code disagree,
the code is the source of truth.

This phase reads and reports; it never writes.

## 1.1 Confirm the context root

When `context/` does not exist, set internal status `bootstrap_required`
immediately. Read nothing further.

Bootstrapping is the workflow's decision, not this phase's.

## 1.2 Read the entry points

Read, when present:

- `context/context-map.md`
- `context/overview.md`
- `context/glossary.md`

Read `context/architecture.md` when the focus touches structure, boundaries, or
data flow. Read `context/patterns.md` when it touches conventions the change must
follow.

A missing entry point is a gap, not a failure. Record it and continue.

## 1.3 Select the relevant domain context

Consult `context/context-map.md` before any broad exploration. The map's
annotations name what each domain file owns; use them to select files, rather
than globbing or searching `context/`.

Select only files whose subject overlaps the focus. Follow at most one level of
links out of a selected file, and only when the link is needed to understand the
focus.

Do not read every domain file. A brief that includes everything has selected
nothing.

Record focus areas with no matching context file under `gaps`.

## 1.4 Check recorded context against the code

For each selected file, spot-check its central claims against the code it
describes.

When context and code diverge, the code is the source of truth. Record the
divergence under `drift` with what context says, what the code shows, and the
repair the context needs.

Do not repair it here. Later phases decide whether repair belongs in the current
work.

Keep this proportional: check the claims the focus depends on, not every
sentence.

## 1.5 Return the brief

Set exactly one internal state:

- `loaded`
- `bootstrap_required`

Report facts the workflow can act on. A brief that only lists file paths has
moved no knowledge.

Record only the internal state. Do not add explanatory prose before or after it.

Step 2 consumes a `loaded` brief verbatim and treats its `key_facts` as recorded
current state, its `gaps` as areas with no durable context, and its `drift` as
context the code has already outrun.

## Context load boundaries

Do not:

- Create, update, move, or delete any file under `context/`.
- Bootstrap `context/`.
- Repair drift or stale context.
- Modify application code or tests.
- Read the entire `context/` tree by default.
- Explore the repository beyond what the focus and the selected context require.
- Ask the user questions. Report gaps and drift, and let the workflow decide.
- Author a plan, select a task, or implement anything.
