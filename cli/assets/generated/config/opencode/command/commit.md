---
description: "Use `sce-atomic-commit` to propose atomic commit message(s) from staged changes"
agent: "Shared Context Code"
entry-skill: "sce-atomic-commit"
skills:
  - "sce-atomic-commit"
---

Load and follow the `sce-atomic-commit` skill.

Input:
`$ARGUMENTS`

## Bypass path (`/commit oneshot` or `/commit skip`)

If `$ARGUMENTS` starts with `oneshot` or `skip` (case-insensitive, first token only):

- **Skip the staging confirmation prompt.** Do not ask the user to stage files or confirm staging.
- **Validate staged content exists.** Check that `git diff --cached` is non-empty. If no staged changes exist, stop with the error: "No staged changes. Stage changes before commit." Do not proceed.
- **Skip context-guidance gate classification.** Do not classify staged diff scope as `context/`-only vs mixed. Do not apply context-file guidance gating.
- **Produce exactly one commit message.** Run `sce-atomic-commit` with these overrides:
  - Produce exactly one commit message. Do not propose splits. Do not emit split guidance.
  - When staged changes include `context/plans/*.md`, make a best-effort inference to cite affected plan slug(s) and updated task ID(s). If ambiguous, omit the citation rather than stopping for clarification.
- **Auto-execute `git commit`.** Use the produced commit message to run `git commit -m "<message>"`.
  - If `git commit` succeeds, report the commit hash and stop.
  - If `git commit` fails, stop and report the failure. Do not invent fallback commits, retry, or amend.

## Regular path (no arguments or non-bypass arguments)

If `$ARGUMENTS` does not start with `oneshot` or `skip`:

Behavior:
- If arguments are empty, treat input as unstated and infer commit intent from staged changes only.
- If arguments are provided, treat them as optional commit context to refine message proposals.
- Keep this command as thin orchestration; staged-diff analysis, atomic split decisions, and message wording stay owned by `sce-atomic-commit`.
- Before running `sce-atomic-commit`, explicitly stop and prompt the user:

  "Please run `git add <files>` for all changes you want included in this commit.
  Atomic commits should only include intentionally staged changes.
  Confirm once staging is complete."

- After confirmation:
  - Classify staged diff scope (`context/`-only vs mixed `context/` + non-`context/`) and apply the context-guidance gate from `sce-atomic-commit`.
  - Run `sce-atomic-commit` to produce commit-message proposals and any needed split guidance.
- Do not create commits automatically; stop after returning proposed commit message(s) and split guidance when needed.
