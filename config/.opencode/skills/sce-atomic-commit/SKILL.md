---
name: sce-atomic-commit
description: |
  Write atomic, repo-style git commits from a change summary or diff. Use when preparing commit messages, splitting work into coherent commits, or reviewing whether a commit is too broad.
compatibility: opencode
---

## Goal

Turn the current staged changes into atomic repository-style commit message proposals.

For this workflow:
- analyze the staged diff to identify coherent change units
- propose one or more commit messages when staged changes mix unrelated goals
- keep each proposed message focused on a single coherent change
- stay proposal-only: do not create commits automatically
- in bypass mode (command-invoked), relax proposal-only, split guidance, context-guidance gate, and plan-citation ambiguity rules

## Inputs

Accept any of:
- staged diff (preferred)
- changed file list with notes
- PR/task summary
- before/after behavior notes

## Output format

Produce commit message proposals that follow:
- `scope: Subject`
- imperative verb (Fix/Add/Remove/Implement/Refactor/Simplify/Rename/Update/Ensure/Allow)
- no trailing period in subject
- body when context is needed (why/what changed/impact)
- issue references on their own lines (for example `Fixes #123`)

When staged changes include `context/plans/*.md`, each commit body must also include:
- affected plan slug(s)
- updated task ID(s) (`T0X`)

If staged `context/plans/*.md` changes do not expose the plan slug or updated task ID clearly enough to cite faithfully, stop and ask for clarification instead of inventing references.

## Bypass mode

When this skill is invoked by the `/commit` command in bypass mode (`/commit oneshot` or `/commit skip`), the command passes overrides that relax the standard rules below:

- **Proposal-only → auto-commit allowed.** Do not block auto-commit; the command will execute `git commit` with the produced message.
- **Single message only.** Produce exactly one commit message. Do not propose splits. Do not emit split guidance.
- **Context-guidance gate skipped.** Do not classify staged diff scope as `context/`-only vs mixed. Do not apply context-file guidance gating.
- **Plan citations: best-effort only.** When staged changes include `context/plans/*.md`, make a best-effort inference to cite affected plan slug(s) and updated task ID(s). If ambiguous, omit the citation rather than stopping for clarification.

When NOT in bypass mode, follow the standard rules in the Procedure and Context-file guidance gating sections below.

## Procedure

1) Analyze the staged diff for coherent units
- Infer the main reason(s) for the staged change from the diff first.
- Use optional notes only to refine wording, not to override the staged truth.
- Identify whether staged changes represent one coherent unit or multiple unrelated goals.

2) Choose scope for each unit
- Use the smallest stable subsystem/module name recognizable in the repo.
- If unclear, use the primary directory/package of the change.

3) Write subject for each unit
- Pattern: `<scope>: <Imperative verb> <specific technical summary>`
- Keep concrete and targeted.

4) Add body when needed
- Explain what was wrong/missing, why it matters, what changed conceptually, and impact.
- Add issue references on separate lines.

5) In regular mode: apply the plan-update body rule when needed
- Check whether staged changes include `context/plans/*.md`.
- If yes, cite the affected plan slug(s) and updated task ID(s) in the body.
- If the staged plan diff is ambiguous, stop with actionable guidance asking the user to stage or clarify the plan/task reference explicitly.

6) In regular mode: propose split guidance when appropriate
- If staged changes mix unrelated goals (for example: a feature change plus unrelated refactoring), propose separate commit messages for each coherent unit.
- Explain why the split is recommended and which files belong to each proposed commit.
- If staged changes represent one coherent unit, propose a single commit message.

7) In regular mode: validate each proposed message
- Each message should describe its intended change faithfully.
- The subject should stay concise and technical.
- The body should add useful why/impact context instead of repeating the subject.
- Do not invent plan or task references.

## Context-file guidance gating

In regular mode:
- Check staged diff scope before proposing commit messaging guidance.
- If staged changes are context-only (`context/**`), context-file-focused guidance is allowed.
- If staged changes are mixed (`context/**` + non-`context/**`), avoid default context-file commit reminders and prioritize guidance that reflects the full staged scope.

## Anti-patterns

- vague subjects ("cleanup", "updates")
- body repeats subject without adding why
- playful tone in serious fixes/architecture changes
- mention `context/` sync activity in commit messages
- inventing plan slugs or task IDs for staged plan edits
- proposing splits for changes that are already coherent
- forcing unrelated changes into a single commit
- applying split guidance or plan-citation ambiguity stops when the command is in bypass mode
