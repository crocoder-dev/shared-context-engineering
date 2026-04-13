---
description: "Use `sce-atomic-commit` to propose atomic commit message(s) from staged changes"
agent: "Shared Context Code"
---

Load and follow the `sce-atomic-commit` skill.

Input:
`$ARGUMENTS`

Behavior:
- If arguments are empty, treat input as unstated and infer commit intent from staged changes only.
- If arguments are provided, treat them as optional commit context to refine the one commit message.
- Skip staging confirmation prompt.
- Validate staged content exists; if empty, stop with error: "No staged changes. Stage changes before commit."
- Classify staged diff scope (`context/`-only vs mixed `context/` + non-`context/`) and apply the context-guidance gate from `sce-atomic-commit`.
- Run `sce-atomic-commit` to produce exactly one commit message for the staged diff.
- Do not branch into multi-commit or split guidance.
- Use the resulting message to run `git commit` against the staged changes.
- If `git commit` fails, stop and report the failure without inventing fallback commits.
