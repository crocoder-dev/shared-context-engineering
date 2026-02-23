---
description: Resolve code-context drift using SCE rules
allowed-tools: Task, Read, Glob, Grep, Edit, Write, Bash
---

Run the `drift-fixer` skill.

Use the `shared-context` agent to audit `context/` and ensure it correctly describes the system as implemented.

Required behavior:
- Treat code as authoritative.
- Summarize each discrepancy clearly.
- Propose exact context updates.
- Apply updates once the user confirms (or immediately if already authorized).

Make updates directly in `context/` and keep files concise, current-state oriented, and linked from `context/context-map.md` when relevant.
