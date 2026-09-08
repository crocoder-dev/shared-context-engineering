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
not inter-SCE workflow handoffs. Do not invoke another SCE skill, sibling SCE
package, or SCE workflow command. Follow the canonical workflow's steps, gates,
and stops exactly as written: never invent, skip, reorder, or merge a step.

## Phase reference

Both paths below dispatch to the same phase, whose steps and boundaries live in
`references/atomic-commit.md`. This document holds the control flow — which path
runs, what the phase receives, and how its result branches — and the reference
holds the phase itself.

Read `references/atomic-commit.md` before running the phase, not after. A regular
run that stops at the staging gate, and a bypass run that finds nothing staged,
both end without ever needing it.

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

After confirmation, read `references/atomic-commit.md`, then run the **Atomic
commit phase** with `mode: regular` and the commit context.

Do not write commit messages yourself.

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

Read `references/atomic-commit.md`, then run the **Atomic commit phase** with
`mode: bypass` and the commit context.

Bypass mode is the skill's contract for producing exactly one message. Do not
restate its overrides here; the **Atomic commit phase** owns them.

Branch on `status`:

`blocked` -> Render the **Blocked** layout from `references/output.md` and stop. Do not commit.

`bypass_message` -> Continue to the next step.

The skill never returns `proposal` in bypass mode. Treat a `proposal` result as
a contract violation: report it and stop without committing.

#### 3. Execute exactly one commit

Follow the **Bypass execution handoff** in `references/atomic-commit.md`:

1. Create the commit-message temp file outside the repository working tree, and
   write the returned `message` verbatim to it using a file-writing operation. Do
   not interpolate the multiline message into shell source or a shell command.
2. Run `git commit -F <message-file>` exactly once.
3. Only after that command succeeds, retrieve the commit hash explicitly with
   `git rev-parse --verify HEAD^{commit}`. Do not parse Git's human-readable
   output.
4. Delete the temp file after the commit attempt, including on failure, where
   practical.

On success, render the **Bypass success** layout from `references/output.md` and
stop.

On failure, render the **Bypass Git failure** layout from the same file and stop.

Do not retry, do not amend, do not stage additional files, and do not fabricate a
commit hash.

## Rules

- Produce at most one commit per invocation, and only on the bypass path.
- Never commit on the regular path.
- Recognize `oneshot` and `skip` only as an exact case-insensitive first token.
  They are behaviorally identical.
- Read `references/atomic-commit.md` before running the phase.
- Do not duplicate the internal instructions of the **Atomic commit phase**.
- Do not stage, unstage, restore, or otherwise modify repository or worktree
  files. The bypass commit-message temp file is the sole exception: it must live
  outside the working tree, so it is not a repository or worktree file.
- Do not amend, reset, revert, rebase, or push.
- Do not read unstaged or untracked changes as commit input.
- Do not infer success when the **Atomic commit phase** returns a non-success status.
- Do not proceed past a failed `git commit`.
- Do not run plan, task, or validation workflows from this command.
