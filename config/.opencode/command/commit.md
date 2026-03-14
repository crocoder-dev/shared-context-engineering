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
