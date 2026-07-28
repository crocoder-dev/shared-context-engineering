---
name: sce-atomic-commit
description: >
  Internal SCE workflow skill that analyzes the staged diff and returns atomic,
  repository-style commit messages: coherent-unit detection, split guidance,
  scope and subject wording, and the plan-citation body rule. Returns one Markdown
  result (proposal, bypass_message, or blocked). Use from /commit. Do not stage
  files, create commits, or ask the user to confirm staging.
compatibility: claude
---

# SCE Atomic Commit

## Purpose

Turn the current staged changes into atomic repository-style commit messages.

This skill owns:

- Reading and analyzing the staged diff.
- Deciding whether staged changes form one coherent unit or several.
- Choosing the scope and writing the subject and body of every message.
- Applying the plan-citation body rule.
- Classifying staged scope and applying context-file guidance gating.
- Returning one terminal Markdown result.

Write messages matching:

`references/commit-message-style.md`

Return a result matching:

the **Result contract** section in this file

Committing is not this skill's job. The invoking `/commit` workflow decides
whether a returned message is committed, and it is the only thing that runs
`git commit`.

## Input

The invoking workflow provides:

- A mode: `regular` or `bypass`.
- Optional commit context, in free-form prose.

The mode is supplied by the workflow from an explicit user-supplied token.
Never infer it, and never switch modes mid-analysis.

Commit context refines wording only. The staged diff decides what the change
is; context never overrides staged truth, and never adds a claim the diff does
not support.

Do not accept an unstaged diff, a working-tree summary, or a conversational
description as a substitute for the staged diff.

## Workflow

### 1. Read the staged diff

Read the staged changes with `git diff --cached`, and the staged file list with
`git diff --cached --name-status`.

Read staged file contents only when the diff alone does not explain the change.

Return `blocked` when nothing is staged.

### 2. Identify coherent units

Infer the main reason for the staged change from the diff first.

A coherent unit is one goal a reviewer would accept as a single commit. Group
staged files by that goal, not by directory.

In `bypass` mode, stop grouping here: the result is exactly one message
covering all staged files, whether or not the diff is coherent. Do not propose
splits, and do not report split guidance.

### 3. Choose a scope for each unit

Use the smallest stable subsystem or module name recognizable in the repository.

When no such name applies, use the primary directory or package of the unit's
changes.

### 4. Write each message

Follow `references/commit-message-style.md` for the subject pattern, the body
rules, issue references, the plan-citation rule, and the anti-patterns.

### 5. Apply the plan-citation rule

When the unit's staged files include `context/plans/*.md`, cite the affected
plan slug and updated task IDs in the body.

When the staged plan diff does not expose the slug or task ID clearly enough to
cite faithfully:

- In `regular` mode, return `blocked` and ask for the reference to be stated or
  staged explicitly.
- In `bypass` mode, infer the citation when the diff supports it, and otherwise
  omit it. Never stop, and never invent a slug or task ID.

### 6. Apply context-file guidance gating

This step applies in `regular` mode only. Skip it entirely in `bypass` mode; do
not classify staged scope there.

Classify the staged diff:

- Context-only (`context/**`): context-file-focused guidance is allowed.
- Mixed (`context/**` plus non-`context/**`): suppress default context-file
  commit reminders and give guidance that reflects the full staged scope.

### 7. Propose split guidance

This step applies in `regular` mode only.

When the units found in step 2 pursue unrelated goals, return one message per
unit, and state why the split is recommended and which staged files belong to
each.

When the staged changes form one unit, return one message and no split
guidance. Do not split coherent work to appear thorough.

### 8. Validate the result

Confirm before returning that:

- Every message describes its unit faithfully and covers only that unit's files.
- Every staged file belongs to exactly one returned message.
- No plan slug or task ID appears that the staged diff does not support.
- The mode's own constraints hold.

### 9. Return Markdown

Return exactly one Markdown document matching the **Result contract** section in this file:

- `proposal` in `regular` mode, with one or more messages.
- `bypass_message` in `bypass` mode, with exactly one message.
- `blocked` when messages cannot be written faithfully.

Return only the Markdown document. Do not add explanatory prose before or after it.

## Boundaries

Do not:

- Run `git commit`, or any command that writes to the repository or its index.
- Stage, unstage, or modify files.
- Ask the user to stage or confirm staging.
- Analyze unstaged or untracked changes.
- Return more than one message in `bypass` mode.
- Return split guidance in `bypass` mode.
- Stop for plan-citation ambiguity in `bypass` mode.
- Invent plan slugs, task IDs, or issue references.
- Mention `context/` synchronization activity in a commit message.
- Claim a message was committed.
- Run plan, task, or validation workflows.

## Completion

The skill is complete after:

- The staged diff was read, or reading it failed and was reported.
- Messages were written for every staged file, or a blocker prevented it.
- One valid terminal Markdown result matching the **Result contract** section in this file was
  returned.

## Result contract

# SCE Atomic Commit Result Contract

Return exactly one Markdown document using one layout below. `Status` is the
branch value consumed by `/commit`. Use every required heading and label
exactly as written, omit optional sections that do not apply, and do not add
prose outside the selected layout.

Report paths exactly as `git diff --cached --name-only` reports them. Every
staged file belongs to exactly one proposed commit. Keep each message's body
in the same fenced block as its subject, separated by one blank line. Never
report a commit hash or guidance forbidden by the mode.

## Status: `proposal`

Use only in regular mode. Nothing is committed.

````markdown
# Atomic Commit Result

Status: proposal
Mode: regular

## Commits

### {commit.id}

#### Message

```text
{subject}

{body}
```

#### Files

- {file}

#### Cites plan

{true|false}

## Split rationale

{split_rationale}

## Scope classification

{context_only|mixed}

## Notes

- {note}
````

`Commits` is required. Repeat the commit block as needed. Include `Split
rationale` only for multiple commits. `Scope classification` and `Notes` are
optional.

## Status: `bypass_message`

Use only in bypass mode. Exactly one message covers every staged file.

````markdown
# Atomic Commit Result

Status: bypass_message
Mode: bypass

## Message

```text
{subject}

{body}
```

## Files

- {file}

## Cites plan

{true|false}

## Notes

- {note}
````

`Message` and `Files` are required. `Cites plan` and `Notes` are optional.
Never include commits, split rationale, or scope classification.

## Status: `blocked`

````markdown
# Atomic Commit Result

Status: blocked
Mode: {regular|bypass}

## Files

- {file}

## Issues

### {issue.id}

- Category: {no_staged_changes|plan_citation_ambiguity|unreadable_diff|contradictory_context}
- Problem: {problem}
- Impact: {impact}
- Decision required: {decision_required}
````

`Issues` is required. Include `Mode` when supplied and `Files` when staged
files were read. Plan-citation ambiguity blocks only in regular mode; bypass
mode omits the citation instead.

## Control flow

This skill is one phase of a workflow, not a turn. Return the result to the
invoking command and let it continue in the same turn. Do not present the
result to the user as workflow output, and do not end your turn after
returning it — the invoking command decides what the user sees and when the
workflow stops.
