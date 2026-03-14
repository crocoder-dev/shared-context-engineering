---
name: sce-atomic-commit
description: Write atomic, repo-style git commits from a change summary or diff. Use when preparing commit messages, splitting work into coherent commits, or reviewing whether a commit is too broad.
compatibility: claude
---

## Goal

Turn a set of code changes (diff, file list, PR summary, or notes) into atomic commits with repository-style commit messages.

Atomic means:
- one coherent change per commit
- minimal scope that still builds/tests logically
- a short, technical, actionable commit message

## Inputs

Accept any of:
- staged diff (preferred)
- changed file list with notes
- PR/task summary
- before/after behavior notes

If changes mix unrelated goals, split them.

## Output format

Commit messages must follow:
- `scope: Subject`
- imperative verb (Fix/Add/Remove/Implement/Refactor/Simplify/Rename/Update/Ensure/Allow)
- no trailing period in subject
- body when context is needed (why/what changed/impact)
- issue references on their own lines (for example `Fixes #123`)

## Procedure

1) Identify smallest coherent units
- Group by reason and user-visible effect, not file type.
- Typical atomic boundaries: pure refactor, behavior change, tests, build/config, docs.
- Avoid mixing refactor and behavior unless strictly required.

2) Choose scope
- Use the smallest stable subsystem/module name recognizable in the repo.
- If unclear, use the primary directory/package of the change.

3) Write subject
- Pattern: `<scope>: <Imperative verb> <specific technical summary>`
- Keep concrete and targeted.

4) Add body when needed
- Explain what was wrong/missing, why it matters, what changed conceptually, and impact.
- Add issue references on separate lines.

5) Validate atomicity
- Reverting the commit should cleanly remove one logical change.
- Review should not require unrelated context.
- No drive-by formatting or unrelated refactors.

If atomicity fails, split further.

## Split guidance

Split when you see:
- renames with behavior changes
- pure refactors bundled with fixes
- formatting-only changes mixed with logic
- multiple features shipped together

Default split order:
1. mechanical rename/refactor
2. behavior change
3. tests
4. docs

## Context-file guidance gating

- Check staged diff scope before proposing commit messaging guidance.
- If staged changes are context-only (`context/**`), context-file-focused guidance is allowed.
- If staged changes are mixed (`context/**` + non-`context/**`), avoid default context-file commit reminders and prioritize guidance that reflects the full staged scope.

## Anti-patterns

- vague subjects ("cleanup", "updates")
- body repeats subject without adding why
- unrelated changes in one commit
- playful tone in serious fixes/architecture changes
- mention `context/` sync activity in commit messages
