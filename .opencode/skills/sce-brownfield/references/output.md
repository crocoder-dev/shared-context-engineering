# Brownfield output layouts

Use only the applicable layout. Values come from internal workflow state.

## Invalid usage

```markdown
-------------------------------------

# Brownfield: invalid arguments

`/brownfield` takes an optional leading `rebuild` token followed by any number
of local documentation paths: `/brownfield [rebuild] [path ...]`.

Problem: {unrecognized token, misplaced `rebuild`, or unreadable path}

Received: `{$ARGUMENTS}`

Nothing was investigated and nothing was written.
```

## Missing context bootstrap gate

```markdown
-------------------------------------

# This repository has no durable context.

Bootstrap it, then continue in this session:

`sce setup --bootstrap-context`

Nothing was investigated and nothing was written.
```

## Clarification gate

```markdown
-------------------------------------

# Clarification needed.

No context file was written.

{count} facts could not be established from repository evidence alone. Answer
each group below. Every question accepts one of its options or a freeform
answer.

## {group-id} · {area}

{question}

- A. {option}
- B. {option}
- C. {option, when one applies}
- Or answer freely.

Evidence found: {what the repository showed}
Why this blocks: {which context statement cannot be written as truth}
```

## Blocked

Present each blocker's problem, impact, and required action. State what was
investigated, that no context file was written, and the concrete condition
under which `/brownfield` should run again.

## Contradiction disclosure

Rendered as a section of the completed report, never on its own.

```markdown
## Contradictions

- **{subject}** — {classification: stale documentation | superseded decision |
  divergent implementations | unexplained history}
  - Code says: {what the code shows}
  - Other evidence says: {what documentation or history claims}
  - Resolved as: {the interpretation written to context, and why}
```

## Completed report

```markdown
-------------------------------------

# Brownfield reconstruction complete.

Mode: {additive | rebuild}

{written-count} context files written, {skipped-count} left untouched.

## Written

- `{context path}` — {what durable truth it now records}

## Left untouched

- `{context path}` — already present; additive mode does not overwrite it.

## Fact ledger

| Fact | Status | Score | Evidence |
| --- | --- | --- | --- |
| {fact} | {Verified \| Strongly supported \| Inferred \| Contradiction resolved} | {1-100} | {file, test, or commit range that supports it} |

## Contradictions

{Render the **Contradiction disclosure** layout here, or `None found.`}

## Gaps

- {Area the repository's own evidence could not establish, and what would
  resolve it, or `None identified.`}

## Verification

- {Which quality-audit checks ran and their outcome.}
```

# Report rules

- Every written path must be exact so the reader can open it directly.
- The fact ledger reports the score the workflow assigned; it is chat output
  only and is never written into a context file.
- A fact scored below `50` never appears in the ledger as written truth. It is
  either resolved through the clarification gate or listed under **Gaps**.
- **Contradictions** is never omitted. State `None found.` when none were
  found; silence reads as none were looked for.
- Never claim a file was written unless the write actually completed.
- Never report a quality-audit check as passed unless it ran.
- Report `Mode: rebuild` only when the literal `rebuild` token was supplied.
- Do not recommend a follow-up workflow command. The reconstruction is the
  whole deliverable.
