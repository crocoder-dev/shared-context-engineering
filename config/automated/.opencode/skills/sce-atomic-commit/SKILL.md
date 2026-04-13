---
name: sce-atomic-commit
description: |
  Write atomic, repo-style git commits from a change summary or diff. Use when preparing commit messages, splitting work into coherent commits, or reviewing whether a commit is too broad.
compatibility: opencode
---

## Goal

Turn the current staged changes into one straightforward repository-style commit message.

For this workflow:
- produce exactly one commit message
- keep the message focused on the staged change as a single coherent unit
- do not default to multi-commit split planning

## Inputs

Accept any of:
- staged diff (preferred)
- changed file list with notes
- PR/task summary
- before/after behavior notes

## Output format

Produce one commit message that follows:
- `scope: Subject`
- imperative verb (Fix/Add/Remove/Implement/Refactor/Simplify/Rename/Update/Ensure/Allow)
- no trailing period in subject
- body when context is needed (why/what changed/impact)
- issue references on their own lines (for example `Fixes #123`)

When staged changes include `context/plans/*.md`, the commit body must also include:
- affected plan slug(s)
- updated task ID(s) (`T0X`)

If staged `context/plans/*.md` changes do not expose the plan slug or updated task ID clearly enough to cite faithfully, stop and ask for clarification instead of inventing references.

## Procedure

1) Review the staged change as one unit
- Infer the main reason for the staged change from the staged diff first.
- Use optional notes only to refine wording, not to override the staged truth.

2) Choose scope
- Use the smallest stable subsystem/module name recognizable in the repo.
- If unclear, use the primary directory/package of the change.

3) Write subject
- Pattern: `<scope>: <Imperative verb> <specific technical summary>`
- Keep concrete and targeted.

4) Add body when needed
- Explain what was wrong/missing, why it matters, what changed conceptually, and impact.
- Add issue references on separate lines.

5) Apply the plan-update body rule when needed
- Check whether staged changes include `context/plans/*.md`.
- If yes, cite the affected plan slug(s) and updated task ID(s) in the body.
- If the staged plan diff is ambiguous, stop with actionable guidance asking the user to stage or clarify the plan/task reference explicitly.

6) Validate the single-message result
- The message should describe the staged diff faithfully as one coherent change.
- The subject should stay concise and technical.
- The body should add useful why/impact context instead of repeating the subject.
- Do not invent plan or task references.

## Context-file Guidance gating

- Check staged diff scope before proposing commit messaging guidance.
- If staged changes are context-only (`context/**`), context-file-focused guidance is allowed.
- If staged changes are mixed (`context/**` + non-`context/**`), avoid default context-file commit reminders and prioritize guidance that reflects the full staged scope.

## Anti-patterns

- vague subjects ("cleanup", "updates")
- body repeats subject without adding why
- playful tone in serious fixes/architecture changes
- mention `context/` sync activity in commit messages
- inventing plan slugs or task IDs for staged plan edits
