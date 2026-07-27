---
description: "Run `sce-atomic-commit` to turn staged changes into atomic commit message proposals"
argument-hint: "[oneshot|skip] [commit context]"
agent: "Shared Context Code"
entry-skill: "sce-atomic-commit"
skills:
  - "sce-atomic-commit"
---

SCE COMMIT `$ARGUMENTS`

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

Pass `commit context` to `sce-atomic-commit` unmodified. Do not restate,
summarize, or pre-scope it. Never pass the `mode-token` as commit context.

Staged changes are the source of truth for what is being committed. This
command never stages, unstages, or modifies files.

## Workflow

Follow exactly one path.

### Bypass path (`oneshot` or `skip`)

#### 1. Validate that staged content exists

Run `git diff --cached --quiet`. A zero exit status means nothing is staged.

When nothing is staged, stop with exactly:

`No staged changes. Stage changes before commit.`

Do not stage anything. Do not proceed to the skill.

#### 2. Request one commit message

Invoke `sce-atomic-commit` with `mode: bypass` and the commit context.

Bypass mode is the skill's contract for producing exactly one message. Do not
restate its overrides here; `sce-atomic-commit` owns them.

The skill must return a result matching its commit contract. Branch on
`status`:

`blocked` -> Present the issue and stop. Do not commit.

`bypass_message` -> Continue to the next step.

The skill never returns `proposal` in bypass mode. Treat a `proposal` result as
a contract violation: report it and stop without committing.

#### 3. Execute exactly one commit

Run `git commit` once with the returned message.

On success, report the resulting commit hash and stop.

On failure, report the failure as returned by Git and stop. Do not retry, do
not amend, do not stage additional files, and do not invent a fallback commit.

### Regular path (no mode token)

#### 1. Confirm staging

Before invoking the skill, stop and prompt the user:

```
Please run `git add <files>` for all changes you want included in this commit.
Atomic commits should only include intentionally staged changes.
Confirm once staging is complete.
```

Wait for the user's confirmation. Do not stage files on their behalf, and do
not skip this prompt because the working tree looks ready.

#### 2. Propose commits

After confirmation, invoke `sce-atomic-commit` with `mode: regular` and the
commit context.

`sce-atomic-commit` exclusively owns:

- Reading and analyzing the staged diff.
- Deciding whether staged changes form one coherent unit or several.
- Classifying staged scope and applying context-file guidance gating.
- Writing every commit message subject and body.
- Applying the plan-citation body rule.

Do not duplicate any of it. Do not write commit messages yourself.

The skill must return a result matching its commit contract. Branch on
`status`:

`blocked` -> Present the issue and the decision it requires. Stop.

`proposal` -> Present each proposed commit: its message, the files it covers,
and, when more than one commit is proposed, why the split is recommended.

Then stop. The regular path is proposal-only.

Do not run `git commit`. Do not offer to commit on the user's behalf. The user
runs the commits they accept.

## Rules

- Produce at most one commit per invocation, and only on the bypass path.
- Never commit on the regular path.
- Recognize `oneshot` and `skip` only as an exact case-insensitive first token.
  They are behaviorally identical.
- Do not duplicate the internal instructions of `sce-atomic-commit`.
- Do not stage, unstage, restore, or otherwise modify files.
- Do not amend, reset, revert, rebase, or push.
- Do not read unstaged or untracked changes as commit input.
- Do not infer success when `sce-atomic-commit` returns a non-success status.
- Do not proceed past a failed `git commit`.
- Do not run plan, task, or validation workflows from this command.
