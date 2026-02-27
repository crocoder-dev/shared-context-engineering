---
description: "Resolve code-context drift using SCE rules"
allowed-tools: Task, Read, Glob, Grep, Edit, Write, Question, Skill, Bash
---

Use the `shared-context-drift` agent, then load and follow the `sce-drift-fixer` skill.

Audit the `context/` and ensure it correctly describes the system as implemented

- treat code as authoritative
- summarize each discrepancy clearly
- propose exact context updates
- apply updates once the user confirms (or immediately if already authorized)

Make updates directly in `context/` and keep files concise, current-state oriented, and linked from `context/context-map.md` when relevant.
