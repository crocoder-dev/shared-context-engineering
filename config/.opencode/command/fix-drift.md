---
description: "Run `sce-drift-fixer` to repair context files from current code truth"
agent: "Shared Context Drift"
entry-skill: "sce-drift-fixer"
skills:
  - "sce-drift-fixer"
---

Load and follow the `sce-drift-fixer` skill.

Behavior:
- Keep this command as thin orchestration; drift analysis, proposed repairs, and context edits stay owned by `sce-drift-fixer`.
- Run `sce-drift-fixer` to audit `context/` against implemented code, treat code as authoritative, and summarize the exact context updates needed.
- Preserve the fixer confirmation gate: apply updates only after explicit user confirmation unless the user already authorized fixes in the current session.
- After approved fixes are applied, stop with the updated context state and any follow-up verification notes from `sce-drift-fixer`.
