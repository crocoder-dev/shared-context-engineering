---
description: "Propose atomic commit message(s) from staged changes"
allowed-tools: Task, Read, Glob, Grep, Edit, Write, Question, Skill, Bash
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
  1. Inspect staged changes and classify staged diff scope:
     - context-only: all staged paths are under `context/`
     - mixed: staged paths include both `context/` and non-`context/` files
  2. For context-only staged diffs, allow context-file-focused commit guidance.
  3. For mixed staged diffs, do not include default context-file commit reminders.
  4. Delegate commit-message grammar, atomic split decisions, and split guidance to `sce-atomic-commit`.

- Do not create commits automatically.
- Output only proposed commit message(s) and split guidance when needed.
