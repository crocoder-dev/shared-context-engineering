---
name: sce-atomic-commit
description: |
  Write atomic, repo-style git commits from a change summary or diff. Use when preparing commit messages, splitting work into coherent commits, or reviewing whether a commit is too broad.
compatibility: claude
---

## Purpose
- Convert intentionally staged changes into faithful repository-style commit-message proposal(s).
- Keep each proposed commit focused on one coherent change; honor command-provided bypass overrides when present.

## Inputs
- Staged diff (preferred), changed-file list with notes, PR/task summary, or before/after behavior notes.
- Optional command mode overrides for regular versus bypass behavior.

## Preconditions
1. Prefer the staged diff as authoritative change truth.
2. Require enough evidence to identify the change intent, scope, and any required plan/task citations.

## Workflow
1. Analyze the staged diff for one or more coherent change units.
2. Choose the smallest stable subsystem or module as scope for each unit.
3. Write an imperative, concrete subject using `<scope>: <Subject>`.
4. Add a body only when it contributes why, conceptual change, or impact.
5. Add issue references on separate lines when supported by the input.
6. In regular mode, cite affected plan slug(s) and task ID(s) when `context/plans/*.md` is staged; stop for clarification if they are ambiguous.
7. In regular mode, propose file split guidance when unrelated goals are staged together.
8. In bypass mode, produce exactly one message, omit split guidance, skip context-guidance gating, and treat plan citations as best-effort.
9. Validate every proposal against the staged diff.

## Guardrails
- Remain proposal-only in regular mode.
- Do not force an already coherent change into multiple commits.
- Do not combine unrelated goals merely to avoid split guidance.
- Do not invent plan slugs, task IDs, issue references, or rationale.
- Do not mention routine context-sync activity in commit messages.
- Avoid vague subjects such as `cleanup` or `updates`.

## Outputs
- Regular mode: one or more complete commit-message proposals and justified split guidance.
- Bypass mode: exactly one complete commit message.

## Completion criteria
- Every proposal faithfully describes its intended staged files as one coherent unit.
- Subjects are concise, technical, imperative, and punctuation-correct.
- Bodies add useful context rather than repeat the subject.

## Failure handling
- In regular mode, stop for clarification when required plan/task citations cannot be inferred faithfully.
- Report insufficient staged evidence instead of guessing change intent.
- In bypass mode, omit ambiguous plan citations rather than block the command.

## Related units
- `/commit` — selects regular or bypass mode and owns any `git commit` execution.
- `Shared Context Code` — default agent for commit workflows.

## Reference
Use this message grammar:

```text
<scope>: <Imperative verb> <specific technical summary>

<optional body explaining why, conceptual change, and impact>

<optional issue reference, for example Fixes #123>
```

Use the smallest stable subsystem as scope. Do not end the subject with a period. Use a body only when it adds useful context.
