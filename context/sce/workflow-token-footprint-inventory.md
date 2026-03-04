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

## T04: Token-reduction strategy set, trade-offs, and rollout order

The strategy set below is ordered to reduce prompt footprint without weakening mandatory gates (`clarification`, `readiness`, `implementation stop`, `context sync`) or collapsing Plan/Code role boundaries.

| Priority | Strategy | Expected impact | Implementation risk | Affected artifacts | Rationale | Guardrail risk + mitigation |
| --- | --- | --- | --- | --- | --- | --- |
| P1 | Tighten thin-wrapper command prose to gate-only contracts | high | low | `config/pkl/base/shared-content.pkl` (`commands["change-to-plan"].canonicalBody`, `commands["next-task"].canonicalBody`, `commands["commit"].canonicalBody`); generated command outputs under `config/.opencode/**` and `config/.claude/**` | Wrapper text currently repeats detailed phase logic that is already skill-owned. Retaining only sequencing + mandatory confirmations removes repeat tokens at every invocation surface. | Risk: wrappers become too terse and hide requirements. Mitigation: keep explicit gate bullets and direct references to canonical skill owners in each wrapper. |
| P2 | Enforce single-owner detailed behavior (skill owns, command references) | high | medium | Skill bodies in `config/pkl/base/shared-content.pkl` for `sce-plan-authoring`, `sce-plan-review`, `sce-task-execution`, `sce-context-sync`, `sce-validation`; corresponding generated skills | Removes duplicated procedural detail split across command, agent, and skill by making skill text the only detailed contract source. | Risk: accidental behavior drift if wrappers and skills diverge semantically. Mitigation: add a parity checklist in plan tasks requiring gate-name exact-match verification after edits. |
| P3 | Convert repeated long workflow prose into canonical snippet constants | medium-high | medium | `config/pkl/base/shared-content.pkl` shared constants and interpolation sites in agent/command/skill canonical bodies | Centralized snippets cut repeated baseline doctrine and standard gate phrasing while preserving wording consistency across targets. | Risk: over-shared snippets can force awkward context-specific text. Mitigation: keep snippets scoped to stable doctrine/gates only; leave role-specific intent local. |
| P4 | Narrow root-context navigation repetition to one canonical index + short pointers | medium | low | `context/context-map.md` (canonical list), plus concise pointer edits in `context/overview.md`, `context/glossary.md`, and workflow docs under `context/sce/*.md` | Repeated "where to look" blocks inflate persistent context payload with low safety value; one canonical map plus short pointers keeps discoverability with fewer tokens. | Risk: reduced discoverability if pointers are too sparse. Mitigation: require every workflow doc to keep one explicit pointer to `context/context-map.md` and its nearest domain file. |
| P5 | Distinguish canonical-owner totals from generated parity copies in reporting defaults | medium | low | `context/sce/workflow-token-footprint-inventory.md` method/reporting sections; future token-report artifacts in `context/tmp/` | Prevents optimization work from targeting unavoidable cross-target duplication by default and keeps reduction efforts focused on editable canonical text. | Risk: teams ignore derived overhead entirely. Mitigation: keep optional derived-overhead pass mandatory for visibility, but separate from canonical reduction KPI. |
| P6 | Add guardrail-preservation acceptance checks to every reduction task | medium | low-medium | Future plan tasks in `context/plans/*.md`; relevant workflow context docs in `context/sce/*.md` | Makes "safe reduction" operational by requiring explicit checks that mandatory gates still exist and role boundaries remain unchanged after edits. | Risk: checklist quality varies by operator. Mitigation: standardize a minimal acceptance template (gate-presence + ownership-boundary verification) reused across tasks. |

### Rollout order

1. Apply P1 first to remove highest-volume wrapper duplication with minimal behavior risk.
2. Apply P2 next so detailed contracts are clearly skill-owned before broader refactors.
3. Apply P3 after ownership boundaries are stable to safely deduplicate shared wording.
4. Apply P4 to reduce background context-load overhead while preserving discoverability.
5. Apply P5 and P6 in parallel as measurement/governance controls for all reduction phases.

### Preserve-as-is constraints during rollout

- Keep explicit confirmation gates in workflow entrypoints (`readiness`, `implementation stop`) even when surrounding prose is reduced.
- Keep `sce-context-sync` as a required done gate and keep final-task `sce-validation` trigger conditions explicit.
- Keep Plan vs Code role separation and command split (`/change-to-plan`, `/next-task`) unchanged.
- Keep human decision authority and code-truth precedence language explicit where currently required.

## T05: Script input/output and manifest extraction contract

This section locks the deterministic contract for the T06 TypeScript implementation.

### Canonical manifest source (selected strategy)

- Canonical machine-readable source: `context/sce/workflow-token-footprint-manifest.json`.
- Human-readable mirror: the T03 counting-scope table in this document.
- Contract precedence: if markdown and JSON differ, the JSON manifest is implementation truth and this document must be synced.
- Rationale: checked-in JSON avoids brittle markdown parsing and keeps extraction rules explicit and testable.

### Manifest schema (required fields)

Top-level object fields:
- `manifest_version`: string, currently `"1"`.
- `plan_name`: string, currently `"sce-workflow-token-footprint-analysis"`.
- `task_id`: string, currently `"T05"` for this contract definition.
- `surfaces`: array of surface entries (non-empty).

Per-surface entry fields:
- `surface_id`: stable slug (matches T03 report rows).
- `workflow`: `"plan"` or `"execute"`.
- `artifact_class`: `"agent" | "command" | "skill" | "context_artifact"`.
- `path`: repo-relative path.
- `scope_rule`: object with `type` and rule-specific fields:
  - `{"type":"entire-file"}`
  - `{"type":"canonical-body-subsection","owner_path":"agents[\"shared-context-plan\"].canonicalBody"}` (example)
- `conditional`: boolean (`true` only for conditional surfaces such as `validation-skill`).

### Extraction rules

General extraction rules:
1. Read target file as UTF-8.
2. Normalize line endings to `\n`.
3. Apply `scope_rule` exactly.

`entire-file` extraction:
- Count normalized full file text with no additional trimming.

`canonical-body-subsection` extraction (for `config/pkl/base/shared-content.pkl`):
- Locate the exact owner path key declared in `scope_rule.owner_path`.
- Extract only the assigned `canonicalBody` string payload.
- Preserve interior text exactly after newline normalization.
- If key lookup is ambiguous or missing, fail the run with a deterministic error that includes `surface_id` and `owner_path`.

### Tokenizer contract

- Preferred tokenizer: `o200k_base`.
- Fallback tokenizer: `cl100k_base`.
- Single-tokenizer-per-run rule: all surfaces in one run must use the same tokenizer.
- Report both `requested_tokenizer` and `resolved_tokenizer`; when fallback occurs, set a run note explaining why.

### Baseline/delta input contract

- Optional input: `--baseline <path-to-prior-report.json>`.
- If omitted: `baseline_tokens` and `delta_tokens` are `null` for all surfaces; summary delta fields are `null`.
- If provided:
  - baseline report must include per-surface rows keyed by `surface_id`.
  - tokenizer must match `resolved_tokenizer`, else fail with deterministic mismatch error.
  - surfaces absent in baseline produce `baseline_tokens = null` and `delta_tokens = null`.

### Report output contract

Output directory and deterministic naming:
- Directory: `context/tmp/token-footprint/` (create if missing).
- Deterministic latest artifacts (always overwritten):
  - `context/tmp/token-footprint/workflow-token-count-latest.json`
  - `context/tmp/token-footprint/workflow-token-count-latest.md`
- Optional archival artifact (when `--run-id` provided):
  - `context/tmp/token-footprint/workflow-token-count-<run_id>.json`

Required per-surface row fields:
- `surface_id`, `workflow`, `artifact_class`, `path`, `scope_rule`, `tokenizer`, `tokens`, `baseline_tokens`, `delta_tokens`, `conditional`.

Required run-level fields:
- `run_id`, `timestamp_utc`, `git_sha`, `plan_name`, `task_id`, `tokenizer`, `plan_total_tokens`, `execute_total_tokens`, `combined_total_tokens`, `combined_delta_tokens`, `notes`.

Additional required run-level fields for deterministic diagnostics:
- `requested_tokenizer`
- `resolved_tokenizer`
- `manifest_path`
- `baseline_path` (nullable)

### Field mapping to T03 schema

| T03 field | Manifest source | Report source |
| --- | --- | --- |
| `surface_id` | `surfaces[].surface_id` | per-surface row |
| `workflow` | `surfaces[].workflow` | per-surface row |
| `artifact_class` | `surfaces[].artifact_class` | per-surface row |
| `path` | `surfaces[].path` | per-surface row |
| `scope_rule` | `surfaces[].scope_rule` | per-surface row |
| `conditional` | `surfaces[].conditional` | per-surface row |
| `tokenizer` | run tokenizer contract | per-surface row + run summary |
| `tokens` | computed | per-surface row |
| `baseline_tokens` | baseline lookup by `surface_id` | per-surface row |
| `delta_tokens` | `tokens - baseline_tokens` when baseline exists | per-surface row |
| `plan_total_tokens` | computed where `workflow=plan` | run summary |
| `execute_total_tokens` | computed where `workflow=execute` | run summary |
| `combined_total_tokens` | computed | run summary |
| `combined_delta_tokens` | computed when baseline totals exist | run summary |

### Discoverability links

- Canonical workflow context: `context/sce/shared-context-plan-workflow.md`, `context/sce/shared-context-code-workflow.md`.
- Plan execution state: `context/plans/sce-workflow-token-footprint-analysis.md`.
- Temporary artifacts location contract: `context/tmp/token-footprint/`.
