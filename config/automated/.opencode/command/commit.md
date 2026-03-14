---
description: "Use `sce-atomic-commit` to propose atomic commit message(s) from staged changes"
agent: "Shared Context Code"
---

Load and follow the `sce-atomic-commit` skill.

Input:
`$ARGUMENTS`

Behavior:
- If arguments are empty, treat input as unstated and infer commit intent from staged changes only.
- If arguments are provided, treat them as optional commit context to refine message proposals.
- Skip staging confirmation prompt.
- Validate staged content exists; if empty, stop with error: "No staged changes. Stage changes before commit."
- Classify staged diff scope (`context/`-only vs mixed `context/` + non-`context/`) and apply the context-guidance gate from `sce-atomic-commit`.
- Delegate commit-message grammar, atomic split decisions, and split guidance to `sce-atomic-commit`.

- Do not create commits automatically.
- Output only proposed commit message(s) and split guidance when needed.
