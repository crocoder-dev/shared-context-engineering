# SCE Workflow Inventory for Token-Footprint Analysis (T01)

## Purpose

Provide a canonical inventory of participants in the SCE Plan (`/change-to-plan`) and Execute (`/next-task`) workflows, including ownership and invocation boundaries used for later token-footprint analysis.

## Workflow inventory matrix

| Workflow | Component type | Component | Role in workflow | Ownership boundary | Shared vs role-specific |
| --- | --- | --- | --- | --- | --- |
| Plan (`/change-to-plan`) | Agent | `Shared Context Plan` | Planning-only role that converts scoped requests into atomic plan tasks and emits `/next-task` handoff. | Canonical role contract is owned in `config/pkl/base/shared-content.pkl` (`agents["shared-context-plan"].canonicalBody`); generated consumers in `.opencode` and `.claude`. | role-specific |
| Plan (`/change-to-plan`) | Command | `/change-to-plan` | Thin orchestration entrypoint for plan creation/update. | Command wrapper owns only orchestration text; clarification and plan-shape contracts are skill-owned. | shared orchestration pattern |
| Plan (`/change-to-plan`) | Skill | `sce-plan-authoring` | Owns clarification gate, plan-shape contract, and readiness output semantics for planning sessions. | Skill is canonical owner of detailed plan-session behavior used by command wrapper. | role-specific |
| Plan (`/change-to-plan`) | Context artifact | `context/plans/{plan_name}.md` | Durable plan state (task IDs, status, boundaries, done checks, verification notes). | Plan markdown is the continuity source for planning/execution handoff. | shared artifact |
| Plan (`/change-to-plan`) | Context artifact | `context/sce/shared-context-plan-workflow.md` | Canonical workflow definition for Plan role and handoff contract to `/next-task`. | Context-owned reference doc; reflects current-state behavior. | role-specific |
| Execute (`/next-task`) | Agent | `Shared Context Code` | Execution role that runs one approved task, validates behavior, and enforces context sync. | Canonical role contract is owned in `config/pkl/base/shared-content.pkl` (`agents["shared-context-code"].canonicalBody`); generated consumers in `.opencode` and `.claude`. | role-specific |
| Execute (`/next-task`) | Command | `/next-task` | Thin orchestration entrypoint sequencing review, execution, and context sync phases. | Command wrapper owns sequencing/gates; detailed phase behavior is skill-owned. | shared orchestration pattern |
| Execute (`/next-task`) | Skill | `sce-plan-review` | Resolves plan target + task, checks readiness, and enforces clarification before execution when needed. | Skill is canonical owner of review/readiness phase contract. | shared phase in execute workflow |
| Execute (`/next-task`) | Skill | `sce-task-execution` | Enforces implementation stop, scoped edits, task-level verification, and plan status update. | Skill is canonical owner of implementation-phase contract. | shared phase in execute workflow |
| Execute (`/next-task`) | Skill | `sce-context-sync` | Required done gate that synchronizes `context/` to code truth after task implementation. | Skill is canonical owner of sync-phase contract. | shared phase in execute workflow |
| Execute (`/next-task`) | Skill (conditional) | `sce-validation` | Final plan-task-only validation/cleanup phase. | Skill invoked only when current task is final task in the plan. | shared conditional phase |
| Execute (`/next-task`) | Context artifact | `context/sce/shared-context-code-workflow.md` | Canonical workflow definition for Execute role including mandatory gates. | Context-owned reference doc; reflects current-state behavior. | role-specific |

## Cross-workflow shared components

| Component domain | Canonical owner | Consumers |
| --- | --- | --- |
| Shared SCE baseline doctrine (core principles, `context/` authority, quality posture) | Shared snippet constants in `config/pkl/base/shared-content.pkl` | Shared Context Plan agent, Shared Context Code agent, generated target-specific agent files |
| Thin-command orchestration model | `commands["change-to-plan"].canonicalBody`, `commands["next-task"].canonicalBody` in `config/pkl/base/shared-content.pkl` | Generated command files for OpenCode/Claude |
| Skill-owned detailed contracts | `skills["sce-plan-authoring"]`, `skills["sce-plan-review"]`, `skills["sce-task-execution"]`, `skills["sce-context-sync"]`, `skills["sce-validation"]` in `config/pkl/base/shared-content.pkl` | Plan/Execute command wrappers and role agents |

## Invocation boundaries

### Plan workflow

1. `/change-to-plan` invokes `Shared Context Plan`.
2. Plan agent loads `sce-plan-authoring` to perform clarification + plan shaping.
3. Plan output emits an execution handoff command: `/next-task {plan_name} {T0X}`.

### Execute workflow

1. `/next-task` invokes `Shared Context Code`.
2. Execute flow runs `sce-plan-review` first.
3. After readiness and explicit implementation-stop confirmation, execute flow runs `sce-task-execution`.
4. Done gate always runs `sce-context-sync`; `sce-validation` runs only on final plan task.

## Sources used

- `context/sce/shared-context-plan-workflow.md`
- `context/sce/shared-context-code-workflow.md`
- `context/sce/plan-code-overlap-map.md`
- `context/sce/dedup-ownership-table.md`
- `.opencode/command/change-to-plan.md`
- `.opencode/command/next-task.md`
- `.opencode/agent/Shared Context Plan.md`
- `.opencode/agent/Shared Context Code.md`

## T02: Token-heavy prompt surfaces and duplication hotspots

| Rank | Prompt surface | Source location(s) | Why token cost accumulates | Safety-critical verbosity | Keep vs reduce recommendation |
| --- | --- | --- | --- | --- | --- |
| 1 | Execute workflow orchestration + phase-contract restatement | `.opencode/command/next-task.md`; `.opencode/skills/sce-plan-review/SKILL.md`; `.opencode/skills/sce-task-execution/SKILL.md`; `.opencode/skills/sce-context-sync/SKILL.md`; `.opencode/agent/Shared Context Code.md` | Same execution gates are repeated across command wrapper, role agent, and three phase skills to preserve tool-agnostic reliability. | yes (readiness gate, mandatory implementation stop, required context-sync gate) | Keep gate semantics and stop conditions verbatim; reduce surrounding explanatory duplication by keeping command wrapper thin and linking to skill-owned contracts. |
| 2 | Shared baseline doctrine duplicated across Plan and Code roles | `config/pkl/base/shared-content.pkl` shared snippets rendered into Plan/Code generated agents for OpenCode + Claude | Cross-role and cross-target projection repeats core principles and `context/` authority text in multiple generated surfaces. | yes (human-decision authority, code-truth precedence, context durability) | Keep canonical shared snippets as single owner; reduce drift risk by avoiding role-local rewrites and preserving snippet reuse only. |
| 3 | Plan workflow clarification/readiness language appearing in both command and skill layers | `.opencode/command/change-to-plan.md`; `.opencode/skills/sce-plan-authoring/SKILL.md`; `context/sce/shared-context-plan-workflow.md` | Clarification and readiness semantics are intentionally restated in wrapper and skill docs for discoverability and enforcement. | yes (clarification gate and explicit readiness contract) | Keep clarification gate requirements; reduce command prose to invocation contract and defer full decision logic to skill-owned sections. |
| 4 | `/commit` workflow guidance split across command and atomic-commit skill docs | `.opencode/command/commit.md`; `.opencode/skills/sce-atomic-commit/SKILL.md`; related glossary/overview entries | Message grammar and atomic split policy can be repeated in wrapper text plus skill-level details. | medium (staged-only and no-auto-commit guardrails are safety-relevant) | Keep wrapper-level staged confirmation and proposal-only constraints; reduce by treating skill as sole owner of detailed commit grammar and split heuristics. |
| 5 | Context navigation/redundant cross-link lists across overview/workflow/glossary files | `context/overview.md`; `context/context-map.md`; `context/glossary.md`; `context/sce/*.md` | Repeated "where to look" sections aid discoverability but add persistent token overhead in background context loading. | no (mostly discoverability, not execution safety) | Reduce long repeated lists by maintaining one canonical map entry per artifact and using shorter pointers elsewhere. |
| 6 | Generated cross-target parity text duplication (OpenCode and Claude outputs) | `config/.opencode/**`; `config/.claude/**` from same canonical Pkl content | Same canonical instructions are emitted twice by design for target parity; static footprint appears doubled even when authored once. | medium (parity guarantees and capability differences must remain explicit) | Keep parity model; reduce analysis noise by counting canonical owner text once and reporting generated parity copies as derived overhead. |

### Hotspot classification notes

- Intentional guardrail hotspots (keep verbose): readiness/clarification gates, implementation-stop contract, context-sync required done gate, and authority/safety doctrine.
- Reducible duplication hotspots: wrapper-level explanatory prose beyond gating contract, repeated long discoverability lists, and command-level detail that should remain skill-owned.
- Parity-only duplication should be treated as expected derived overhead, not primary authoring duplication, when prioritizing reduction work.

## T03: Static token accounting method and evidence template

### Counting scope (exact inputs)

Use these workflow prompt surfaces as the canonical static-count manifest for this plan:

| Surface ID | Workflow | Artifact class | File path | Count scope |
| --- | --- | --- | --- | --- |
| `plan-agent-canonical` | Plan | Agent (canonical owner) | `config/pkl/base/shared-content.pkl` | Count only the rendered source block for `agents["shared-context-plan"].canonicalBody`. |
| `code-agent-canonical` | Execute | Agent (canonical owner) | `config/pkl/base/shared-content.pkl` | Count only the rendered source block for `agents["shared-context-code"].canonicalBody`. |
| `change-to-plan-command` | Plan | Command wrapper | `.opencode/command/change-to-plan.md` | Count entire file. |
| `next-task-command` | Execute | Command wrapper | `.opencode/command/next-task.md` | Count entire file. |
| `plan-authoring-skill` | Plan | Skill contract | `.opencode/skills/sce-plan-authoring/SKILL.md` | Count entire file. |
| `plan-review-skill` | Execute | Skill contract | `.opencode/skills/sce-plan-review/SKILL.md` | Count entire file. |
| `task-execution-skill` | Execute | Skill contract | `.opencode/skills/sce-task-execution/SKILL.md` | Count entire file. |
| `context-sync-skill` | Execute | Skill contract | `.opencode/skills/sce-context-sync/SKILL.md` | Count entire file. |
| `validation-skill` | Execute | Skill contract (conditional) | `.opencode/skills/sce-validation/SKILL.md` | Count entire file; include but tag as conditional. |
| `shared-plan-workflow-doc` | Plan | Context artifact | `context/sce/shared-context-plan-workflow.md` | Count entire file. |
| `shared-code-workflow-doc` | Execute | Context artifact | `context/sce/shared-context-code-workflow.md` | Count entire file. |

Optional derived-overhead pass (reported separately, not merged into canonical authoring total):
- matching generated Claude surfaces under `config/.claude/**` for the same command/agent/skill slugs;
- matching generated OpenCode surfaces under `config/.opencode/**` when they are not already the counted canonical execution surfaces.

### Tokenizer assumptions

- Primary tokenizer assumption: `o200k_base` (closest available static approximation for current OpenAI-family models).
- Fallback tokenizer assumption when `o200k_base` is unavailable: `cl100k_base`.
- Every report must include the tokenizer name used; cross-tokenizer totals are not directly comparable.

### Deterministic counting procedure

1. Capture run metadata: date/time (UTC), current git commit SHA, plan name, task ID, operator.
2. Materialize the exact surface manifest from the table above (same surface IDs and file paths).
3. Read files as UTF-8 text and normalize newlines to `\n` before counting.
4. For each surface, apply scope rule (`entire file` or `canonicalBody subsection`) and produce the exact counted text payload.
5. Count tokens for each payload with one tokenizer for the whole run; do not mix tokenizers within a run.
6. Record per-surface token counts, then compute workflow subtotals (`Plan`, `Execute`) and combined total.
7. If a previous baseline exists, compute deltas per surface and for each subtotal/total.
8. Store evidence in a dated artifact under `context/tmp/` and summarize key totals in the active plan task evidence notes.

### Report schema (required fields)

Per-surface row fields:
- `surface_id`
- `workflow` (`plan` or `execute`)
- `artifact_class` (`agent`, `command`, `skill`, `context_artifact`)
- `path`
- `scope_rule`
- `tokenizer`
- `tokens`
- `baseline_tokens` (nullable)
- `delta_tokens` (nullable)
- `conditional` (`true` for validation-skill, else `false`)

Run-level summary fields:
- `run_id`
- `timestamp_utc`
- `git_sha`
- `plan_name`
- `task_id`
- `tokenizer`
- `plan_total_tokens`
- `execute_total_tokens`
- `combined_total_tokens`
- `combined_delta_tokens` (nullable)
- `notes`

### Evidence template (copy/paste)

```markdown
# Static token accounting run: <run_id>

- timestamp_utc: <YYYY-MM-DDTHH:MM:SSZ>
- git_sha: <short_sha>
- plan_name: sce-workflow-token-footprint-analysis
- task_id: T03
- tokenizer: <o200k_base|cl100k_base>

| surface_id | workflow | artifact_class | path | scope_rule | tokens | baseline_tokens | delta_tokens | conditional |
| --- | --- | --- | --- | --- | ---: | ---: | ---: | --- |
| ... | ... | ... | ... | ... | ... | ... | ... | ... |

## Totals

- plan_total_tokens: <n>
- execute_total_tokens: <n>
- combined_total_tokens: <n>
- combined_delta_tokens: <n or null>
- notes: <assumptions, anomalies, exclusions>
```

### Known limitations

- Static counts do not include runtime/system-level hidden prompt frames, tool IO payload sizes, or conversation-history growth.
- Subsection extraction for canonical Pkl agent bodies depends on stable key names; renamed keys require manifest update.
- Derived parity copies can make footprint appear inflated; keep canonical-owner totals and derived-overhead totals separate.
- Token totals are tokenizer-dependent estimates, not billing-accurate usage measurements.
