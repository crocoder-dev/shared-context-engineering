# Automated Profile Contract

## Purpose

Defines deterministic behavior for the automated OpenCode profile at `config/automated/.opencode/**`, removing interactive approval/confirmation gates while preserving SCE safety constraints.

## Scope

- Applies to automated profile generation targets only
- Manual profile at `config/.opencode/**` remains unchanged with interactive gates
- Policy governs agent frontmatter permissions, command/skill bodies, and workflow behavior

## Gate Categories and Deterministic Policy

### P1. Permission Gates (Agent Frontmatter)

| Permission | Manual | Automated |
|------------|--------|-----------|
| `default` | `ask` | `allow` |
| `read` | `allow` | `allow` |
| `edit` | `allow` | `allow` |
| `glob` | `allow` | `allow` |
| `grep` | `allow` | `allow` |
| `list` | `allow` | `allow` |
| `bash` | `allow` | `allow` |
| `task` | `allow` | `allow` |
| `external_directory` | `ask` | `block` |
| `todowrite` | `allow` | `allow` |
| `todoread` | `allow` | `allow` |
| `question` | `allow` | `allow` |
| `webfetch` | `allow` | `allow` |
| `websearch` | `allow` | `allow` |
| `codesearch` | `allow` | `allow` |
| `lsp` | `allow` | `allow` |
| `doom_loop` | `ask` | `block` |
| `skill["*"]` | `ask` | `allow` |
| `skill["sce-*"]` | `allow` | `allow` |

**Rationale:**
- `external_directory: block` prevents automated profile from touching paths outside repository
- `doom_loop: block` prevents runaway execution
- `skill["*"]: allow` enables skill loading without per-skill prompts

### P2. Bootstrap Approval (Missing Context)

**Manual:** Ask once for approval to bootstrap `context/` if missing
**Automated:** `auto-block`

**Behavior:**
- If `context/` is missing, stop with error: "Automated profile requires existing context/. Run manual bootstrap first."
- Do not auto-create context structure

**Rationale:** Bootstrap is a one-time setup action requiring human oversight

### P3. Clarification Gate (Plan Authoring)

**Manual:** If critical detail unclear, ask 1-3 targeted questions and stop
**Automated:** `auto-block`

**Behavior:**
- If any critical detail is unclear (scope, success criteria, constraints, dependencies, domain ambiguity, architecture concerns, task ordering), stop with structured error
- Error must list all unresolved items with category labels
- Do not invent assumptions silently

**Missing-detail handling:**
- Emit structured blocker report: `BLOCKER: clarification_required`
- Include specific unresolved items
- Require human session to resolve before automated planning can proceed

**Rationale:** Automated planning must not invent requirements; unclear requests require human clarification

### P4. Implementation Stop (Task Execution)

**Manual:** Before writing code, pause and prompt user with scope/approach/risks
**Automated:** `auto-proceed` with logging

**Behavior:**
- Log implementation intent (task goal, scope, approach) to `context/tmp/automated-session-log.md`
- Proceed without waiting for confirmation
- Preserve all safety constraints (one-task, no scope expansion, no plan reordering)

**Log format:**
```
## [timestamp] T0X: {task_title}
- Goal: {goal}
- In scope: {in_scope}
- Out of scope: {out_of_scope}
- Expected files: {file_list}
- Approach: {approach_summary}
- Status: proceeding
```

**Rationale:** Implementation stop is a safety review gate; automated profile skips the pause but keeps constraints

### P5. Readiness Confirmation (Plan Review)

**Manual:** Ask explicit confirmation that reviewed task is ready for implementation
**Automated:** `auto-pass` when conditions met, `auto-block` otherwise

**Auto-pass conditions:**
1. Plan path and task ID both provided
2. Review reports no blockers
3. Review reports no ambiguity
4. Review reports no missing acceptance criteria

**Auto-block conditions:**
- Any blocker, ambiguity, or missing acceptance criteria → stop with structured error
- Missing task ID → use first unchecked task; if multiple plans exist, `auto-block` (see P10)

**Rationale:** Automated execution requires complete, unambiguous task specifications

### P6. Multi-Task Approval

**Manual:** If multi-task execution requested, confirm explicit human approval
**Automated:** `auto-block`

**Behavior:**
- Multi-task execution is not supported in automated profile
- If requested, stop with error: "Automated profile does not support multi-task execution. Use single-task handoffs."
- Require explicit task-by-task execution

**Rationale:** One-task-per-session is a core SCE safety constraint; automated profile enforces strictly

### P7. Scope Expansion

**Manual:** If out-of-scope edits needed, stop and ask for approval
**Automated:** `auto-block`

**Behavior:**
- If implementation requires edits outside declared task scope, stop immediately
- Emit structured error: `BLOCKER: scope_expansion_required`
- List specific out-of-scope items detected
- Require human session to approve scope change or split task

**Rationale:** Scope expansion requires architectural judgment; automated profile does not auto-approve

### P8. Commit Staging Confirmation

**Manual:** Prompt user to confirm staging complete before commit proposal
**Automated:** `auto-proceed` with staged-content validation

**Behavior:**
- Skip staging confirmation prompt
- Validate staged content exists; if empty, emit error: "No staged changes. Stage changes before commit."
- Proceed directly to commit message proposal

**Rationale:** Automated profile assumes caller has staged correct changes; validation catches empty staging

### P9. Drift Fix Application

**Manual:** Ask whether to apply fixes or keep report-only
**Automated:** `auto-apply` with constraints

**Behavior:**
- Auto-apply drift fixes without confirmation
- Constraint: only apply fixes to `context/` files
- If code changes would be required, emit report-only with blocker: "Drift requires code changes. Manual intervention required."
- Log all applied fixes to `context/tmp/automated-drift-fixes.md`

**Rationale:** Context-only drift fixes are safe; code-requiring drift needs human judgment

### P10. Plan Selection (Multiple Plans)

**Manual:** If multiple plans exist and no explicit path provided, ask user to choose
**Automated:** `auto-block`

**Behavior:**
- If no plan path specified and multiple plans exist, stop with error
- Error must list available plans with paths
- Require explicit plan path in command

**Plan selection default:**
- Single plan + no path → auto-select the single plan
- Multiple plans + no path → `auto-block`
- Explicit path → use specified plan

**Rationale:** Automated profile requires deterministic plan resolution; guessing is unsafe

## Deterministic Defaults Summary

| Scenario | Manual | Automated |
|----------|--------|-----------|
| Plan selection (single) | Auto-select | Auto-select |
| Plan selection (multiple) | Ask user | Block: require explicit path |
| Missing context/ | Ask to bootstrap | Block: requires manual bootstrap |
| Unclear requirements | Ask clarifying questions | Block: emit structured unresolved items |
| Ready to implement | Ask confirmation | Auto-proceed if conditions met |
| Scope expansion needed | Ask approval | Block: require human session |
| Multi-task requested | Ask approval | Block: not supported |
| Drift fixes (context-only) | Ask to apply | Auto-apply with logging |
| Drift fixes (code required) | Ask to apply | Report-only with blocker |
| Empty staging | Prompt to stage | Block: no staged changes |

## Automated Profile Constraints

These constraints apply to automated profile behavior regardless of gate policies:

1. **One-task execution:** Always enforce single-task-per-session
2. **No plan mutation:** Do not reorder tasks or change plan structure
3. **No code invention:** Do not invent requirements, assumptions, or specifications
4. **Context authority:** Code is source of truth; context sync is required
5. **External isolation:** `external_directory: block` prevents repository escape
6. **Doom loop prevention:** `doom_loop: block` prevents runaway execution
7. **Logging:** All automated decisions logged to `context/tmp/automated-session-log.md`

## Implementation Notes

- Permission changes go in automated metadata variant (`opencode-automated-metadata.pkl`)
- Behavior changes go in automated content variants for affected agents/commands/skills
- Generator must emit both manual and automated trees
- Parity checks must validate both profiles

## Related Context

- `context/plans/sce-automated-opencode-profile.md` - Implementation plan
- `context/sce/shared-context-plan-workflow.md` - Plan workflow reference
- `context/sce/shared-context-code-workflow.md` - Code workflow reference
