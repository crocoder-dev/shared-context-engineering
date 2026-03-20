---
name: sce-task-execution
description: |
  Executes a single planned implementation task with a mandatory approval gate, scope guardrails, and evidence capture. Use when a user wants to implement, run, or execute a specific task from a project plan — such as coding a feature, applying a patch, or making targeted file changes — while enforcing explicit scope boundaries, a pre-implementation confirmation prompt, test/lint verification, and status tracking in context/plans/{plan_id}.md.
compatibility: opencode
---

## Scope rule
- Execute exactly one task per session by default.
- If multi-task execution is requested, confirm explicit human approval.

## Mandatory implementation stop
- Before writing or modifying any code, pause and prompt the user.
- The prompt must explain:
  - task goal
  - boundaries (in/out of scope)
  - done checks
  - expected files/components to change
  - key approach, trade-offs, and risks
- Then ask explicitly whether to continue.
- Do not edit files, generate code, or apply patches until the user confirms.

**Example mandatory stop prompt:**
```
Task goal: Add input validation to the user registration endpoint.
In scope: src/routes/register.ts, src/validators/user.ts
Out of scope: Auth logic, database schema, frontend forms
Done checks: All existing tests pass; new validation tests cover empty/invalid email and short passwords
Expected changes: ~2 files modified, no new dependencies
Approach: Use the existing `validateSchema` helper; add a Zod schema for registration payload
Trade-offs: Zod adds minor overhead; keeps validation consistent with other routes
Risks: Existing callers that omit optional fields may start failing validation

Continue with implementation now? (yes/no)
```

## Required sequence
1) Restate task goal, boundaries, done checks, and expected file touch scope.
2) Propose approach, trade-offs, and risks.
3) Stop and ask: "Continue with implementation now?" (yes/no).
4) Implement minimal in-scope changes.
5) Run light task-level tests/checks and lints first, and run a build when the build is light/fast (targeted over full-suite unless requested), then capture evidence.
6) Record whether the implementation is an important change for context sync (root-edit required) or verify-only (no root edits expected).
7) Keep session-only scraps in `context/tmp/`.
8) Update task status in `context/plans/{plan_id}.md`.

**Example task status update (`context/plans/{plan_id}.md`):**
```markdown
## Task: Add input validation to registration endpoint
- **Status:** done
- **Completed:** 2025-06-10
- **Files changed:** src/routes/register.ts, src/validators/user.ts
- **Evidence:** 14/14 tests passed, lint clean, build succeeded (12s)
- **Notes:** Zod schema added; no breaking changes to existing callers
```

## Scope expansion rule
- If out-of-scope edits are needed, stop and ask for approval.
