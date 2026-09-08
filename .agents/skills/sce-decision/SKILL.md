---
name: sce-decision
description: >
  Write one immutable ADR for one decision already qualified by context synchronization
---

# SCE Decision

## Purpose

Write exactly one architecture decision record for one decision already qualified
by successful task context synchronization. Return a deterministic internal
handoff to the invoking synchronization phase. Do not render an independent
user-visible response.

## Input

Accept one structured decision request from `sce-next-task` task context
synchronization. It must identify:

- One decision stated as a single durable choice.
- Why it qualifies under the decision gate.
- The implementation / task-verification evidence establishing the decision.
- The resolved plan path and relevant task IDs, when applicable.
- Related current-state context and existing ADR paths.
- An optional requested status.

Do not accept raw workflow arguments, ordinary phase state, multiple decisions,
or direct user invocation. Do not reconstruct missing material facts.

## Qualification handoff

The invoking task context synchronization phase solely owns the decision
qualification threshold and decides whether this skill is invoked. Treat the
caller's gate result as authoritative; do not redefine, broaden, or independently
rerun that threshold here.

Require the request to state why the caller qualified the decision and include
supporting evidence. If the request explicitly represents a nonqualifying gate
outcome, return `not_qualified` (or `skipped` when the caller deliberately skipped
the gate) without writing an ADR. If a claimed qualification is unsupported by
its supplied evidence, return `not_qualified`. Missing, contradictory, or
otherwise unsafe material input remains `blocked`.

`not_qualified` and `skipped` are non-blocking to the invoking synchronization
phase.

## Workflow

### 1. Validate one decision

Confirm the request contains exactly one decision, qualifying evidence, a plan
path, references sufficient to make the record traceable, and no unresolved
material contradiction. If it contains several decisions, require the caller to
submit one request per decision.

Allowed statuses for a newly written ADR are exactly `Proposed`, `Accepted`,
`Rejected`, `Deprecated`, and `Superseded`. Use the explicitly requested allowed
status; otherwise default to `Accepted`. `Deprecated` and `Superseded` remain
distinct creation-time-only statuses: use them to describe the record when it is
created, but never mutate an existing ADR into or out of either status. Reject any
other status rather than guessing.

### 2. Inspect existing decision history

Read `context/decisions/` and the supplied related ADR paths before writing.

- Reuse an existing ADR only when it records an equivalent decision and has an
  active status: `Proposed` or `Accepted`. Return its path without creating a
  duplicate. Never reuse a `Rejected`, `Deprecated`, or `Superseded` ADR.
- Existing ADRs are immutable regardless of status. Never edit an ADR whose status is `Accepted`; do not edit, overwrite, or silently change the status of any existing record.
- A correction, reversal, or any changed decision always creates a new dated ADR;
  it references and supersedes the prior record when applicable, rather than
  modifying that record.

If `context/` or `context/decisions/` is absent, or history cannot be interpreted
without inventing facts, return `blocked` without creating directories.

### 3. Resolve the path

Use exactly `context/decisions/YYYY-MM-DD-<decision-slug>.md`, where the date is
the record creation date and `<decision-slug>` is a concise lowercase kebab-case
summary of the one decision. Resolve collisions by choosing a more specific
deterministic slug; never add an arbitrary counter and never overwrite a record.

### 4. Write the ADR

Read `references/adr-template.md` before writing. It is the sole authority for
the persisted ADR schema and section semantics. Populate that template from the
validated request and evidence; do not restate or invent a parallel section
contract here.

Use repository-relative Markdown links where practical. Describe durable truth,
not the implementation session. Do not edit current-state context; the invoking
synchronization phase owns linking the new ADR from authoritative context.

### 5. Verify the record

Confirm that exactly one ADR was created or one existing matching ADR was reused;
the filename, status, sections, and references satisfy this contract; every
referenced repository path exists when practical to check; and no accepted ADR
was modified.

### 6. Return internal state

Return exactly one internal handoff:

- `written`: include `status`, `adr_path`, `decision`, `decision_status`,
  `created` (`true` for a new ADR and `false` for reuse), `supersedes`, and
  concise verification evidence.
- `not_qualified` or `skipped`: include `status`, the reason the decision gate
  did not produce an ADR, and concise supporting evidence. These results are
  non-blocking; the invoking synchronization phase continues normally.
- `blocked`: include `status`, the specific `problem`, its `impact`, and the
  `required_action`. Use this only when decision writing cannot proceed safely.

Use stable field names and repository-relative paths. Return no prose before or
after the handoff. The invoking synchronization phase owns all user-visible
reporting.

## Boundaries

Do not:

- Write more than one ADR per request.
- Run outside successful task context synchronization.
- Create a command, prompt, context root, or decisions directory.
- Modify application code, tests, plans, current-state context, or existing
  accepted ADRs.
- Treat every context update as an architecture decision.
- Choose among unresolved material alternatives on the user's behalf.
- Create a Git commit or push changes.
