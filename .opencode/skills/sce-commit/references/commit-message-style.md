# Commit Message Guide

Use this repository style when writing new commits.

## Core rules

- Start with `scope: Subject` for most code changes.
  - Common scopes: `runtime`, `language`, `objects`, `tests`, `CI`, `README`.
  - Combined scopes are fine when needed (for example `language+runtime`).
- Use an imperative verb in the subject: `Fix`, `Add`, `Refactor`, `Remove`, `Implement`, `Update`, `Rewrite`, `Use`, `Allow`.
- Keep the subject specific and technical (name the subsystem and actual change).
- Keep the subject to one line and do not end it with a period.
- Add a body when the change is non-trivial.
  - Explain why the change is needed.
  - Explain how it works at a high level.
  - Include impact/tradeoffs/follow-ups when relevant.
- For performance-related commits, include concrete measurements and benchmark context.
  - Include regressions as well as improvements.
- Add issue references when relevant on their own lines.
  - Example: `Fixes #123`
  - Example: `Ref: https://...`

## Practical template

```text
<scope>: <Imperative summary>

<Why this change is needed.>
<How it works at a high level.>
<Impact: perf/correctness/risk/follow-ups.>
Fixes #<id>   (optional)
```

## Size-based defaults

1. Small fix: subject + 1 short reason line.
2. Medium refactor: subject + short why + short what changed.
3. Large architectural change: subject + context + bullets for major changes + impact/tradeoffs.

## Anti-patterns to avoid

- Vague subjects like "misc updates" or "cleanup".
- Bodies that only repeat the subject without explaining why or impact.
- Overly playful tone in serious bug-fix or architectural change.
