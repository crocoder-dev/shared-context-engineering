# SCE Atomic Commit

## Purpose

Turn the current staged changes into atomic repository-style commit messages.

Write messages matching:

the **Commit message rules** section in this reference

Return a result matching the **Atomic commit result contract** section in this
reference.

Committing is not this skill's job. The invoking `/commit` workflow decides
whether a returned message is committed, and it is the only thing that runs
`git commit`.

## Input

A mode (`regular` or `bypass`) and optional commit context, in free-form prose.

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

Follow the **Commit message rules** section in this reference for the subject pattern, the body
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

### 9. Return YAML

Return exactly one YAML document matching the **Atomic commit result contract** section in this reference:

- `proposal` in `regular` mode, with one or more messages.
- `bypass_message` in `bypass` mode, with exactly one message.
- `blocked` when messages cannot be written faithfully.

Return only the YAML document. Do not add explanatory prose before or after it.

## Bypass execution handoff

This phase returns the message; the invoking `/commit` workflow performs the
bypass commit. When the mode is `bypass`, the invoking workflow must:

1. Create the commit-message temp file outside the repository working tree,
   and write the returned `message` verbatim to it using a file-writing
   operation. Never interpolate a multiline message into shell source or a
   shell command.
2. Run `git commit -F <message-file>` exactly once.
3. After and only after a successful commit, run
   `git rev-parse --verify HEAD^{commit}` and use that explicit `HEAD` value as
   the reported hash. Never parse Git's human-readable commit output.
4. On any commit failure, report Git's failure and stop. Never retry, amend,
   stage more files, or fabricate a hash.
5. Delete the temp file after the commit attempt, including on failure, where
   practical.

`oneshot` and `skip` select this same bypass behavior; they differ only in the
trigger token.

## Atomic commit boundaries

Do not:

- Run `git commit`, or any command that writes to the repository or its index.
- Stage, unstage, restore, or otherwise modify repository or worktree files.
  The bypass commit-message temp file is the sole exception: it must live
  outside the working tree, so it is not a repository or worktree file.
- Ask the user to stage or confirm staging.
- Analyze unstaged or untracked changes.
- Return more than one message in `bypass` mode.
- Return split guidance in `bypass` mode.
- Stop for plan-citation ambiguity in `bypass` mode.
- Invent plan slugs, task IDs, or issue references.
- Mention `context/` synchronization activity in a commit message.
- Claim a message was committed.
- Run plan, task, or validation workflows.

## Commit message rules

The wording rules for every message `sce-atomic-commit` returns, in either
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


## Atomic commit result contract

The complete result contract is serialized below:

```yaml
version: 1
name: sce-atomic-commit-result

description: >
  Output contract for sce-atomic-commit. The skill returns exactly one YAML
  document representing proposal, bypass_message, or blocked.

output_rules:
  - Return exactly one result variant.
  - The top-level status must be proposal, bypass_message, or blocked.
  - Return YAML only, without a Markdown code fence or explanatory prose.
  - Include only fields belonging to the selected variant.
  - Omit optional fields that do not apply rather than sending them empty.
  - Do not return empty strings or null placeholders.
  - Return proposal only in regular mode, and bypass_message only in bypass
    mode. blocked is valid in both.
  - Every staged file must appear under exactly one commit's files.
  - Report file paths exactly as `git diff --cached --name-only` reports them.
  - Carry the message body inside message, separated from the subject by one
    blank line. Do not split it into a separate field.
  - Do not include a commit hash. This skill never commits.
  - Do not report guidance the mode forbids.

variants:

  proposal:
    meaning: >
      The staged changes were analyzed in regular mode and one or more atomic
      commit messages were written. Proposal-only: nothing was committed.

    required_fields:
      - status
      - mode
      - commits

    optional_fields:
      - split_rationale

    field_rules:
      - Include split_rationale only when commits holds more than one entry.

    shape:
      status: proposal
      mode: regular

      commits:
        - id: string
          message: string
          files:
            - string

      split_rationale: string

    example:
      status: proposal
      mode: regular

      commits:
        - id: C01

          message: |
            auth: Add token refresh endpoint

            Sessions expired without a way to renew them, forcing a full
            re-login. Adds the refresh handler and reuses the existing token
            issuer.

          files:
            - src/auth/refresh.ts
            - tests/auth/refresh.test.ts

        - id: C02

          message: |
            build: Pin the formatter to the toolchain version

            The formatter floated across environments and produced diff noise
            unrelated to any change.

          files:
            - flake.nix

      split_rationale: >
        The refresh endpoint and the formatter pin pursue unrelated goals and
        share no files. Committing them together would hide the build change
        behind a feature subject.

  bypass_message:
    meaning: >
      The staged changes were analyzed in bypass mode and exactly one commit
      message covering all staged files was written. The invoking workflow
      executes the commit.

    required_fields:
      - status
      - mode
      - message
      - files

    field_rules:
      - files lists every staged file, because one message covers all of them.
      - Never include commits or split_rationale.

    shape:
      status: bypass_message
      mode: bypass

      message: string

      files:
        - string

    example:
      status: bypass_message
      mode: bypass

      message: |
        auth: Add token refresh endpoint

        Sessions expired without a way to renew them, forcing a full re-login.
        Adds the refresh handler and reuses the existing token issuer.

      files:
        - src/auth/refresh.ts
        - tests/auth/refresh.test.ts

  blocked:
    meaning: >
      Faithful commit messages cannot be written from the staged changes.

    required_fields:
      - status
      - issues

    optional_fields:
      - mode
      - files

    field_rules:
      - Include mode whenever the workflow supplied one.
      - Include files when staged files were read before blocking.
      - Plan-citation ambiguity blocks in regular mode only. In bypass mode the
        citation is omitted instead.

    shape:
      status: blocked
      mode: regular | bypass

      files:
        - string

      issues:
        - id: string
          category: no_staged_changes | plan_citation_ambiguity | unreadable_diff | contradictory_context
          problem: string
          impact: string
          decision_required: string

    example:
      status: blocked
      mode: regular

      files:
        - context/plans/authentication.md

      issues:
        - id: B01
          category: plan_citation_ambiguity

          problem: >
            The staged plan diff changes two task checkboxes and does not
            expose which task this commit completes.

          impact: >
            The commit body would cite a task ID the staged diff does not
            support.

          decision_required: >
            State the completed task ID, or stage only that task's plan edit.
```


## Completion

The skill is complete after:

- The staged diff was read, or reading it failed and was reported.
- Messages were written for every staged file, or a blocker prevented it.
- One valid terminal YAML result matching the atomic-commit result contract was
  returned.


