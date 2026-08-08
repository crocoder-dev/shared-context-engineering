---
name: sce-commit
description: >
  Analyze staged changes and run the regular or explicit bypass commit workflow
compatibility: claude
---

# SCE Commit

## Purpose

Own this workflow from input parsing through its terminal user-visible response.
Execute the phases below directly and in order. Phase statuses are internal state,
not inter-skill handoffs. Do not invoke another SCE skill, sibling package, or
workflow command. Follow the canonical workflow's steps, gates,
and stops exactly as written: never invent, skip, reorder, or merge a step.

## User-visible output

Use `references/output.md` for every gate and terminal response. Render no raw
internal state. The reference contains only human-visible Markdown layouts.
User-visible output is limited to those layouts: never invent a layout, and never
wrap one in an added preamble, commentary, summary, or extra section.

## Composite control flow

Keep phase results as internal state and continue immediately whenever the
canonical workflow says to continue. Stop only at a user wait or terminal branch.
Approval, clarification, revision, failed-validation repair, and bootstrap waits
resume this same skill in the same session. Never expose an internal phase result
as the workflow's final response.

## Input

`$ARGUMENTS` is optional. Split it into two parts before invoking the skill:

`[mode-token] [commit context]`

- `mode-token` is present only when the first whitespace-separated token is
  exactly `oneshot` or `skip`, compared case-insensitively. Any other first
  token is not a mode token.
- `commit context` is everything else: free-form prose that refines message
  wording only.

A `mode-token` selects the bypass path. Its absence selects the regular path.
Do not infer the bypass path from anything else — not from the commit context,
not from repository state, and not from the conversation.

Empty `$ARGUMENTS` is valid. It selects the regular path with no commit
context, and commit intent is inferred from the staged changes alone.

Pass `commit context` to the **Atomic commit phase** unmodified. Do not restate,
summarize, or pre-scope it. Never pass the `mode-token` as commit context.

Staged changes are the source of truth for what is being committed. This
command never stages, unstages, or modifies files.

## Workflow

Follow exactly one path.

### Regular path (no mode token)

#### 1. Confirm staging

Before running the phase, stop and prompt the user with the **Regular-mode
staging gate** layout from `references/output.md`.

Wait for the user's confirmation. Do not stage files on their behalf, and do
not skip this prompt because the working tree looks ready.

#### 2. Propose commits

After confirmation, run the **Atomic commit phase** with `mode: regular` and the
commit context.

The **Atomic commit phase** exclusively owns:

- Reading and analyzing the staged diff.
- Deciding whether staged changes form one coherent unit or several.
- Classifying staged scope and applying context-file guidance gating.
- Writing every commit message subject and body.
- Applying the plan-citation body rule.

Do not duplicate any of it. Do not write commit messages yourself.

The mode is supplied by the workflow from an explicit user-supplied token.
Never infer it, and never switch modes mid-analysis.

Commit context refines wording only. The staged diff decides what the change
is; context never overrides staged truth, and never adds a claim the diff does
not support.

Do not accept an unstaged diff, a working-tree summary, or a conversational
description as a substitute for the staged diff.

##### 2.1 Read the staged diff

Read the staged changes with `git diff --cached`, and the staged file list with
`git diff --cached --name-status`.

Read staged file contents only when the diff alone does not explain the change.

Set internal status `blocked` when nothing is staged.

##### 2.2 Identify coherent units

Infer the main reason for the staged change from the diff first.

A coherent unit is one goal a reviewer would accept as a single commit. Group
staged files by that goal, not by directory.

In `bypass` mode, stop grouping here: the result is exactly one message
covering all staged files, whether or not the diff is coherent. Do not propose
splits, and do not report split guidance.

##### 2.3 Choose a scope for each unit

Use the smallest stable subsystem or module name recognizable in the repository.

When no such name applies, use the primary directory or package of the unit's
changes.

##### 2.4 Write each message

Follow `references/output.md` for the subject pattern, the body
rules, issue references, the plan-citation rule, and the anti-patterns.

##### 2.5 Apply the plan-citation rule

When the unit's staged files include `context/plans/*.md`, cite the affected
plan slug and updated task IDs in the body.

When the staged plan diff does not expose the slug or task ID clearly enough to
cite faithfully:

- In `regular` mode, set internal status `blocked` and ask for the reference to be stated or
  staged explicitly.
- In `bypass` mode, infer the citation when the diff supports it, and otherwise
  omit it. Never stop, and never invent a slug or task ID.

##### 2.6 Apply context-file guidance gating

This step applies in `regular` mode only. Skip it entirely in `bypass` mode; do
not classify staged scope there.

Classify the staged diff:

- Context-only (`context/**`): context-file-focused guidance is allowed.
- Mixed (`context/**` plus non-`context/**`): suppress default context-file
  commit reminders and give guidance that reflects the full staged scope.

##### 2.7 Propose split guidance

This step applies in `regular` mode only.

When the units found in step 2 pursue unrelated goals, return one message per
unit, and state why the split is recommended and which staged files belong to
each.

When the staged changes form one unit, return one message and no split
guidance. Do not split coherent work to appear thorough.

##### 2.8 Validate the result

Confirm before returning that:

- Every message describes its unit faithfully and covers only that unit's files.
- Every staged file belongs to exactly one returned message.
- No plan slug or task ID appears that the staged diff does not support.
- The mode's own constraints hold.

##### 2.9 Return internal state

Set exactly one internal state:

- `proposal` in `regular` mode, with one or more messages.
- `bypass_message` in `bypass` mode, with exactly one message.
- `blocked` when messages cannot be written faithfully.

Record only the internal state. Do not add explanatory prose before or after it.

#### Atomic commit boundaries

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

Branch on `status`:

`blocked` -> Render the **Blocked** layout from `references/output.md`. Stop.

`proposal` -> Render the **Regular proposal** layout from `references/output.md`,
which covers each proposed commit's message and files, and the split rationale
when more than one commit is proposed.

Then stop. The regular path is proposal-only.

Do not run `git commit`. Do not offer to commit on the user's behalf. The user
runs the commits they accept.

### Bypass path (`oneshot` or `skip`)

#### 1. Validate that staged content exists

Run `git diff --cached --quiet`. A zero exit status means nothing is staged.

When nothing is staged, stop with the **No staged changes** layout from
`references/output.md`.

Do not stage anything. Do not proceed to the skill.

#### 2. Request one commit message

Run the **Atomic commit phase** (described at the Regular path's step 2 above) with `mode: bypass` and the commit context.

Bypass mode is the skill's contract for producing exactly one message. Do not
restate its overrides here; the **Atomic commit phase** owns them.

Branch on `status`:

`blocked` -> Render the **Blocked** layout from `references/output.md` and stop. Do not commit.

`bypass_message` -> Continue to the next step.

The skill never returns `proposal` in bypass mode. Treat a `proposal` result as
a contract violation: report it and stop without committing.

#### 3. Execute exactly one commit

Run `git commit` once with the returned message.

On success, render the **Bypass success** layout from `references/output.md` and
stop.

On failure, render the **Bypass Git failure** layout from the same file and stop.

Do not retry, do not amend, do not stage additional files, and do not invent a
fallback commit.

## Rules

- Produce at most one commit per invocation, and only on the bypass path.
- Never commit on the regular path.
- Recognize `oneshot` and `skip` only as an exact case-insensitive first token.
  They are behaviorally identical.
- Do not duplicate the internal instructions of the **Atomic commit phase**.
- Do not stage, unstage, restore, or otherwise modify files.
- Do not amend, reset, revert, rebase, or push.
- Do not read unstaged or untracked changes as commit input.
- Do not infer success when the **Atomic commit phase** returns a non-success status.
- Do not proceed past a failed `git commit`.
- Do not run plan, task, or validation workflows from this command.
