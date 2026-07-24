---
name: sce-atomic-commit
description: |
  Write atomic, repo-style git commits from a change summary or diff. Use when preparing commit messages, splitting work into coherent commits, or reviewing whether a commit is too broad.
compatibility: opencode
---

## Purpose
- Produce exactly one faithful repository-style commit message for the current staged diff.

## Inputs
- Staged diff (preferred), optional changed-file notes, task/PR summary, or before/after behavior notes.

## Preconditions
1. Treat the staged diff as authoritative.
2. Require enough evidence to identify one message and any mandatory plan/task citations.

## Workflow
1. Review the staged diff as one unit.
2. Choose the smallest stable subsystem as scope.
3. Write one imperative `<scope>: <Subject>` line.
4. Add a body only when it adds why, conceptual change, or impact.
5. Cite staged plan slug(s) and task ID(s) when `context/plans/*.md` is included.
6. Apply context-file guidance based on context-only versus mixed staged scope.
7. Validate the single message against all staged changes.

## Guardrails
- Produce exactly one message and no split guidance.
- Do not invent plan/task or issue references.
- Avoid vague subjects, repeated bodies, playful tone for serious changes, and routine context-sync narration.

## Outputs
- Exactly one complete commit message.

## Completion criteria
- The message faithfully describes the staged diff as one coherent unit and satisfies repository grammar.

## Failure handling
- Stop for clarification when required plan/task citations cannot be inferred faithfully.
- Report insufficient staged evidence rather than guessing intent.

## Related units
- `/commit` — executes the resulting commit once.

## Reference
Use this message grammar:

```text
<scope>: <Imperative verb> <specific technical summary>

<optional body explaining why, conceptual change, and impact>

<optional issue reference, for example Fixes #123>
```

Use the smallest stable subsystem as scope. Do not end the subject with a period. Use a body only when it adds useful context.
