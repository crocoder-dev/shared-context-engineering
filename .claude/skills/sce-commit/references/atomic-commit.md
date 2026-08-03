# Atomic commit phase

Both workflow paths run this phase. The regular path runs it with `mode: regular`
after the staging gate; the bypass path runs it with `mode: bypass` after
confirming staged content exists.

Input: the mode, supplied by the workflow from an explicit user-supplied token,
and the commit context.

This phase exclusively owns:

- Reading and analyzing the staged diff.
- Deciding whether staged changes form one coherent unit or several.
- Classifying staged scope and applying context-file guidance gating.
- Writing every commit message subject and body.
- Applying the plan-citation body rule.

Do not duplicate any of it elsewhere in the workflow. Do not write commit messages
outside this phase.

Never infer the mode, and never switch modes mid-analysis.

Commit context refines wording only. The staged diff decides what the change is;
context never overrides staged truth, and never adds a claim the diff does not
support.

Do not accept an unstaged diff, a working-tree summary, or a conversational
description as a substitute for the staged diff.

## 1. Read the staged diff

Read the staged changes with `git diff --cached`, and the staged file list with
`git diff --cached --name-status`.

Read staged file contents only when the diff alone does not explain the change.

Set internal status `blocked` when nothing is staged.

## 2. Identify coherent units

Infer the main reason for the staged change from the diff first.

A coherent unit is one goal a reviewer would accept as a single commit. Group
staged files by that goal, not by directory.

In `bypass` mode, stop grouping here: the result is exactly one message covering
all staged files, whether or not the diff is coherent. Do not propose splits, and
do not report split guidance.

## 3. Choose a scope for each unit

Use the smallest stable subsystem or module name recognizable in the repository.

When no such name applies, use the primary directory or package of the unit's
changes.

## 4. Write each message

Follow `references/output.md` for the subject pattern, the body rules, issue
references, the plan-citation rule, and the anti-patterns.

## 5. Apply the plan-citation rule

When the unit's staged files include `context/plans/*.md`, cite the affected plan
slug and updated task IDs in the body.

When the staged plan diff does not expose the slug or task ID clearly enough to
cite faithfully:

- In `regular` mode, set internal status `blocked` and ask for the reference to be
  stated or staged explicitly.
- In `bypass` mode, infer the citation when the diff supports it, and otherwise
  omit it. Never stop, and never invent a slug or task ID.

## 6. Apply context-file guidance gating

This step applies in `regular` mode only. Skip it entirely in `bypass` mode; do
not classify staged scope there.

Classify the staged diff:

- Context-only (`context/**`): context-file-focused guidance is allowed.
- Mixed (`context/**` plus non-`context/**`): suppress default context-file commit
  reminders and give guidance that reflects the full staged scope.

## 7. Propose split guidance

This step applies in `regular` mode only.

When the units found in step 2 pursue unrelated goals, return one message per
unit, and state why the split is recommended and which staged files belong to
each.

When the staged changes form one unit, return one message and no split guidance.
Do not split coherent work to appear thorough.

## 8. Validate the result

Confirm before returning that:

- Every message describes its unit faithfully and covers only that unit's files.
- Every staged file belongs to exactly one returned message.
- No plan slug or task ID appears that the staged diff does not support.
- The mode's own constraints hold.

## 9. Return internal state

Set exactly one internal state:

- `proposal` in `regular` mode, with one or more messages.
- `bypass_message` in `bypass` mode, with exactly one message.
- `blocked` when messages cannot be written faithfully.

Record only the internal state. Do not add explanatory prose before or after it.

## Atomic commit boundaries

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
