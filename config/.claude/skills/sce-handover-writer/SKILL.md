---
name: sce-handover-writer
description: |
  Creates a structured handover document summarizing task context, decisions made, open questions, and recommended next steps — saved to `context/handovers/`. Use when a user wants to hand off, transition, or pass a task to someone else, create handover notes, write a task transition document, or capture current progress for a future session. Trigger phrases include "create a handover", "hand this off", "write handover notes", "pass this task on", or "document where I'm up to".
compatibility: claude
---

## What I do
- Create a new handover file in `context/handovers/`.
- Capture:
  - current task state
  - decisions made and rationale
  - open questions or blockers
  - next recommended step

## How to run this

1. **Gather context** - review the current task, recent changes, and repo state.
2. **Create the file** - use task-aligned naming: `context/handovers/{plan_name}-{task_id}.md`.
3. **Fill each section** - follow the template below, labelling any inferred details as assumptions.
4. **Verify completeness** - confirm all four sections are populated before finishing.

If key details are missing, infer from repo state and clearly label assumptions.

## Handover document template

```markdown
# Handover: {plan_name} - {task_id}

## Current Task State
- What was being worked on and how far along it is.
- e.g. "Implementing OAuth login flow; token generation complete, redirect handling in progress."

## Decisions Made
- Key choices and their rationale.
- e.g. "Chose JWT over session cookies for statelessness. Rejected library X due to licence constraints."

## Open Questions / Blockers
- Unresolved issues or outstanding dependencies.
- e.g. "Awaiting confirmation on token expiry policy from product team."

## Next Recommended Step
- The single most important action for whoever picks this up.
- e.g. "Complete the redirect handler in `src/auth/callback.ts`, then run the auth integration tests."
```

## Expected output
- A complete handover document in `context/handovers/` using task-aligned naming when possible.
