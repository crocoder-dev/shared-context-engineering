---
name: sce-handover
description: >
  Write a session handover document, or load one for continuation
compatibility: opencode
---

# SCE Handover

## Purpose

Own this workflow from input parsing through its terminal user-visible response.
Execute the phases below directly and in order. Phase statuses are internal state,
not inter-SCE workflow handoffs. Do not invoke another SCE skill, sibling SCE
package, or SCE workflow command. Follow the canonical workflow's steps, gates,
and stops exactly as written: never invent, skip, reorder, or merge a step.

## User-visible output

Use `references/output.md` for every gate and terminal response. Render no raw
internal state. The reference contains only human-visible Markdown layouts.
User-visible output is limited to those layouts: never invent a layout, and never
wrap one in an added preamble, commentary, summary, or extra section.

## Composite control flow

Keep phase results as internal state and continue immediately whenever the
canonical workflow says to continue. Stop only at a user wait or terminal branch.
Any workflow-defined user wait resumes this same skill in the same session.
Never expose an internal phase result
as the workflow's final response.

Relevant non-SCE skills may be used as helper capabilities during the active step.
They are not workflow handoffs: when a helper returns, control returns to the active
step. Helper use must preserve the canonical phase order, gates, waits, writes,
validation, stops, and terminal user-visible output.

## Input

`$ARGUMENTS` is optional and selects the mode:

- Empty `$ARGUMENTS` selects **writer mode**.
- Exactly one whitespace-trimmed path argument selects **loader mode**.
- Anything else — more than one token, or a token that is clearly not a path —
  is invalid input: state the expected usage (`/handover` or
  `/handover context/handovers/<file>.md`) and stop without guessing a mode.

Never infer the mode from conversation content or repository state. Only the
presence or absence of a path argument decides it.

## Workflow

Follow exactly one path.

### Writer path (no arguments)

#### 1. Gather session and repository facts

Inspect the current conversation for task-relevant progress: the goal being
pursued, decisions made, work completed or in flight, and open questions or
blockers.

Ground those facts against repository state:

- `git status`, `git diff`, and `git diff --cached` for uncommitted work,
  including both unstaged and staged changes.
- `context/plans/*.md` for the active plan and task, when one is being worked.
- Recent commits, when they clarify what just landed.

Label any detail not directly evidenced by the conversation or repository state
as an assumption. Do not present an inferred detail as confirmed fact.

#### 2. Determine the file name

- When exactly one plan task is unambiguously active — one plan with one
  in-progress or next-actionable task identifiable from the conversation and
  repository state — use `context/handovers/{plan_name}-{task_id}.md`, where
  `plan_name` is the plan's file stem and `task_id` is its task ID (for
  example `T01`).
- Otherwise use the collision-safe timestamped fallback
  `context/handovers/handover-{YYYY-MM-DD-HHMMSS}.md`.

Never overwrite an existing file. If the resolved path already exists, use the
current timestamp for the fallback name, or append a further distinguishing
timestamp segment, rather than overwriting it.

#### 3. Compose the handover document

Read `references/handover-template.md` before composing. It defines the
persisted-document format and is the only template authority. Populate all
four required sections:

- `Current Task State`
- `Decisions Made`
- `Open Questions / Blockers`
- `Next Recommended Step`

Every section must contain real content. Write `None identified.` (or a
section-appropriate equivalent) when nothing applies — never omit a required
section and never leave template placeholders in the written file.

Label inferred or assumed details inline as assumptions; do not blend them with
confirmed facts.

#### 4. Confirm the context root

When `context/` does not exist, there is no durable location to write to.
Render the **Writer blocked** layout with `sce setup --bootstrap-context` as
the required action, and stop without writing a file.

#### 5. Write exactly one file

Write the composed document to the path resolved in step 2. Before reporting
success, confirm the written file contains all four required sections
populated with real content.

#### 6. Report

Render the **Writer success** layout from `references/output.md` with the
written path. Stop.

### Loader path (one path argument)

#### 1. Validate the path

The argument must resolve to an existing file under `context/handovers/` with
a `.md` extension. Reject:

- A path outside `context/handovers/`.
- A path with a different extension.
- A path that does not exist.

Do not guess an alternate file, and do not treat an arbitrary repository file
as a handover. When the path is rejected, render the **Loader blocked** layout
and stop.

#### 2. Validate handover completeness

Read the file and confirm it contains all four required sections:
`Current Task State`, `Decisions Made`, `Open Questions / Blockers`, and
`Next Recommended Step`. For each section, inspect the content up to the
next required heading (or the end of the file): it must contain non-whitespace
content, and it must not consist only of an empty list marker, a template
placeholder such as `{What is being worked on...}`, or other unreplaced
`{...}` scaffolding. Explicit statements such as `None identified.` are real
content and are valid.

When any required section is missing, empty, or placeholder-only, render the
**Loader blocked** layout (invalid handover) and stop.

#### 3. Present for continuation

Render the **Loader success** layout from `references/output.md`, surfacing
the handover's task state, decisions, open questions, and next recommended
step for continuation in the current session.

Loading is read-only: do not edit any file, mark a plan task complete, change
repository state, or begin the recommended next step. Presenting the loaded
guidance is the entire loader contract.

## Rules

- Handle at most one handover per invocation, in exactly one mode.
- Writer mode never overwrites an existing handover file.
- Writer mode never marks a plan task complete or edits any file outside the
  one handover document it writes.
- Loader mode never edits a file, writes a new file, or changes plan or task
  state.
- Never invoke another SCE skill, sibling SCE package, or SCE workflow command.
- Never treat a file outside `context/handovers/`, or a non-Markdown file, as a
  loadable handover.
- Never create the `context/` root; `sce setup --bootstrap-context` owns that.
- Do not begin, plan, or automate the loaded handover's recommended next step.
