# Commit output layouts

Use only the applicable layout. Values come from staged truth and internal
workflow state.

## Regular-mode staging gate

```markdown
Please run `git add <files>` for all changes you want included in this commit.
Atomic commits should only include intentionally staged changes.
Confirm once staging is complete.
```

## No staged changes

```markdown
No staged changes. Stage changes before commit.
```

## Regular proposal

For each proposal, present the complete commit message and covered files. When
more than one commit is proposed, also present the split rationale. Do not claim a
commit was created.

## Blocked

Present every issue's problem, impact, and required decision. Do not commit.

## Bypass success

```markdown
Committed {commit-hash}
```

## Bypass Git failure

Present Git's failure unchanged and stop without retrying.

# Commit message style

The wording rules for every message the **Atomic commit phase** returns, in either
mode. This file is the only authority for message content and shape.

Messages are carried in the result's `message` field, subject first, then one
blank line, then the body.

## Subject

Pattern:

`<scope>: <Imperative verb> <specific technical summary>`

- Scope is the smallest stable subsystem or module name recognizable in the
  repository. When no such name applies, use the primary directory or package
  of the change.
- Start the summary with an imperative verb: Fix, Add, Remove, Implement,
  Refactor, Simplify, Rename, Update, Ensure, Allow.
- Capitalize the verb. Do not end the subject with a period.
- Keep it concrete and technical. Name what changed, not how it felt.

## Body

Include a body whenever the subject alone leaves the change unexplained. Omit
it for changes whose subject is self-evident.

A body says what was wrong or missing, why it mattered, what changed
conceptually, and the impact. It does not restate the subject in longer words,
and it does not narrate the editing process.

Wrap the body at a readable width and separate paragraphs with a blank line.

## Issue references

Put each issue reference on its own line at the end of the body, for example
`Fixes #123`.

Reference only issues the staged diff or the supplied commit context names. Do
not infer an issue number from a branch name.

## Plan citations

When a commit's staged files include `context/plans/*.md`, the body must also
cite:

- The affected plan slug.
- Every updated task ID (`T0X`).

Cite only what the staged plan diff shows. When the diff does not expose the
slug or task IDs clearly enough to cite faithfully, the skill's mode decides
what happens — the ambiguity is not resolved here by guessing.

## Anti-patterns

- Vague subjects: "cleanup", "updates", "various fixes".
- A body that repeats the subject without adding why or impact.
- Playful tone in a fix, security change, or architectural change.
- Mentioning `context/` synchronization activity.
- Inventing plan slugs, task IDs, or issue references.
- Splitting changes that already form one coherent unit.
- Forcing unrelated goals into a single commit.
- Describing intent the staged diff does not support.
