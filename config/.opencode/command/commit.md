---
description: "Propose atomic commit message(s) from staged changes"
agent: "Shared Context Code"
---

Load and follow the `sce-atomic-commit` skill.

Input:
`$ARGUMENTS`

Behavior:
- If arguments are empty, treat input as unstated and infer commit intent from staged changes only.
- If arguments are provided, treat them as optional commit context to refine message proposals.
- Before invoking `sce-atomic-commit`, explicitly prompt the user:

  "Please run `git add <files>` for all changes you want included in this commit.
  Atomic commits should only include intentionally staged changes.
  Confirm once staging is complete."

- After confirmation:
  1. Inspect staged changes.
  2. Delegate commit-message grammar, atomic split decisions, and split guidance to `sce-atomic-commit`.

- Do not create commits automatically.
- Output only proposed commit message(s) and split guidance when needed.
