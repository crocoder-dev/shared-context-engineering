# Plan: compress-workflow-execution-preamble

## Change summary

Replace the repeated `Purpose`, `User-visible output`, and `Composite control flow`
sections with one shorter `Execution contract` in these five workflow entrypoints:
`sce-change-to-plan`, `sce-next-task`, `sce-commit`, `sce-validate`, and `sce-handover`.
Preserve their behavior, phase boundaries, arguments, permissions, gates, waits,
state transitions, and user-visible output layouts.

Author the compact contract once in canonical Pkl and render it inline into each
selected skill. Do not introduce a shared runtime skill or an additional Markdown
file that the agent must load. The reduction must occur in the emitted Markdown,
not merely in the Pkl source.

Planning baseline: `2d14959c6770d3635dfadb457e137c6fc05feba4` on `main`.
This plan started with finding 1 of the workflow duplication audit. PR #270 now also
contains a separate follow-up commit for the next-task approval-gate duplication: the
entrypoint routes the optional approval flag, task execution owns approval behavior, and
`references/output.md` owns only the exact gate layout/question. Full `nix flake check`
remains for PR CI.

## Acceptance criteria

- [x] AC1: Each selected workflow has exactly one compact `Execution contract`
  in generated Pi, Claude, OpenCode, and Codex entrypoints. Its former three
  generic preamble sections are absent. Existing phase-reference sections remain.
  - Validate: Extend and evaluate `config/pkl/renderers/generation-contract-check.pkl`
    through `nix run .#pkl-check-generated`; check all five workflows on all four targets.
- [x] AC2: Every requirement from the removed preamble has an explicit equivalent:
  workflow ownership, ordered steps/gates/stops, immediate continuation, defined
  waits, same-skill/same-session resume, private phase state, prescribed output,
  in-step helper boundaries, and sibling-workflow restrictions.
  - Validate: Compare old and new requirements clause by clause. Assert the expected
    rendered contract and each workflow's permission variant independently of the
    new helper's output. Reject a fixture with an omitted safeguard or wrong variant.
- [x] AC3: Only `sce-next-task` permits `sce-decision`, and only at the successful
  context-synchronization decision gate. No other selected workflow gains that
  permission. Relevant non-SCE helpers remain permitted without taking control.
  - Validate: Check the full generated entrypoints for the correct variant; include
    negative fixtures for a missing next-task exception and an exception in validate.
- [x] AC4: Apart from the selected preamble replacements, generated payloads retain
  the baseline inventory and content. Phase instructions, reference-loading rules,
  input sections, workflow branches, final Rules sections, output/reference files,
  frontmatter, commands, hooks, and metadata do not change. Brownfield and the
  standalone decision package remain unchanged.
  - Validate: Generate baseline and candidate payloads into separate temporary
    directories. Compare path inventories and bytes, allowing only the three named
    preamble sections to be replaced in the twenty selected SKILL.md files. Compare
    phase-reference sections separately when their position changes.
- [x] AC5: For each selected workflow and target, the rendered replacement uses at
  least 40% fewer UTF-8 bytes than the combined three removed sections. Report
  before/after bytes, words, and nonblank lines, plus total SKILL.md bytes.
  - Validate: Measure baseline and candidate generated files with identical section
    boundaries and line wrapping. Exclude unchanged phase-reference sections from
    the preamble comparison. Do not claim token savings without a named tokenizer.
- [x] AC6: Existing tracked Pi, Claude, and Codex skill mirrors receive the same
  targeted preamble change from generated output, without unrelated rewrites or
  tracked temporary generation directories.
  - Validate: Inspect the Git diff and compare selected mirror preambles with the
    corresponding generated target. Preserve all mirror content outside the named
    sections, including pre-existing differences unrelated to this change.

### Full validation

- `nix run .#pkl-check-generated`
- `nix flake check`
- `git diff --check`
- Inspect baseline/candidate artifact comparison, requirement mapping, negative
  fixture results, and rendered-size measurements recorded in task evidence.

### Context sync

- `context/sce/dedup-ownership-table.md`: record the compact contract's canonical
  owner and preserve the existing owner/consumer boundaries.
- `context/sce/handover-workflow.md`: update its description of the old three-section
  shared preamble without changing the writer/loader contract.
- Inspect other affected workflow context for references to the removed preamble
  headings. Change only statements made stale by this implementation.
- Normal `/next-task` context synchronization still applies. Do not rewrite existing
  architecture decisions or create an ADR merely because wording was shortened.

## Task context synchronization lifecycle

Every task carries `Context synchronization: pending | synced | blocked`.
A completed task must be `synced` before another task can start or the plan can
finish. Persist each transition to the plan, not to conversation state.
For `blocked`, record the blocker, required action, and retry condition.

## Constraints and non-goals

**In scope:** the three generic preamble sections in the five selected workflows,
a small canonical rendering helper, directly affected rendering call sites,
focused generation assertions, existing tracked mirrors, and corresponding context.

**Out of scope:** synchronization-debt repair, execution-handoff schemas, approval
logic, decision qualification, task slicing, commit execution, report templates,
phase references, terminal Rules sections, brownfield behavior, and new commands.

Keep `Phase references` tables and their read-before-action instructions intact.
Preserve target-specific frontmatter and Codex invocation-input wording.
Do not generalize this change into a new workflow framework or runtime dependency.
Do not globally shorten `helperSkillCompositionRule` if doing so changes unselected
consumers such as brownfield or package-mode documents.

Follow the current repository generation policy: use temporary output roots, keep
`config/.opencode/`, `config/.claude/`, `config/.pi/`, and `cli/assets/generated/`
absent, and do not modify the runtime-managed root `.opencode/` tree. Verify
OpenCode's generated payload without committing that runtime tree.

## Assumptions

- "First change" refers to finding 1: the oversized shared execution preamble.
- Existing behavior is the baseline, including unrelated inconsistencies found in
  the audit. Fix those in separate changes rather than silently resolving them here.
- Forty percent is a proposed acceptance threshold for this preamble-only change,
  not a measured result or a claim about reducing the whole workflow by that amount.
- Generated-source checks establish textual coverage and artifact consistency;
  they do not prove identical model behavior. Any agent-run evidence must be
  separately labeled with its model, harness, inputs, and observed outcome.

## Proposed execution contract

Use the following wording as the implementation starting point. Final wording must
pass the requirement comparison and size checks above.

```markdown
## Execution contract

Own this workflow from input through its terminal user-visible response.
Follow its steps, gates, and stops in order; do not add, skip, reorder, or merge them.
Keep phase results internal and continue immediately until a defined wait or stop.
Resume user waits in this same skill and session.
Use only the specified `references/output.md` layouts for gates and terminal
responses. Do not expose raw state or add text around a layout.
Non-SCE helpers must return to the active step without changing phase order,
gates, waits, writes, validation, stops, or terminal output.
Do not invoke another SCE skill, package, or workflow command.
```

For `sce-next-task`, replace the final sentence, rather than appending a conflicting
unconditional prohibition:

```markdown
Do not invoke another SCE skill, package, or workflow command except `sce-decision`,
and only from the successful context-synchronization decision gate.
```

## Implementation shape

| Canonical source | Targeted change |
| --- | --- |
| `config/pkl/base/workflow-content.pkl` | Add the small shared contract renderer; use it in `nextTaskSkillBody` and `validateSkillBody`. |
| `config/pkl/base/workflow-change-to-plan.pkl` | Replace the three sections in `changeToPlanSkillBody`. |
| `config/pkl/base/workflow-commit.pkl` | Replace the three sections in `commitSkillBody`. |
| `config/pkl/renderers/workflow-composite.pkl` | Apply the compact contract to handover's generic rendering path; preserve brownfield output. |
| `config/pkl/renderers/generation-contract-check.pkl` | Add focused coverage and update only assertions that intentionally pin replaced wording. |
| `.pi/skills/`, `.claude/skills/`, `.agents/skills/` | Refresh only the selected entrypoint preambles in existing tracked mirrors. |

Use two explicit contract variants: standard and next-task's decision exception.
Do not inherit the generic renderer's stale next-task-or-validate exception check:
the active validate body prohibits sibling invocation, and that must stay true.
Keep the helper at the source-generation level. Render its text directly into each
skill so loading behavior and the package inventory remain unchanged.

## Task stack

- [x] T01: `Compress the five workflow execution preambles without changing behavior` (status:done)
  - Task ID: T01
  - Scope: In - the canonical sources, generation assertions, and selected tracked
    entrypoint mirrors listed above. Out - all phase logic and other audit findings.
  - Dependencies: none
  - Done when: AC1-AC6 are covered by the implementation and its focused evidence;
    every selected workflow has the compact contract, permissions remain correct,
    protected payloads remain unchanged, and the rendered-size target is met.
  - Verify: Evaluate the focused generation-contract checks; run the baseline/candidate
    payload comparison; inspect the requirement mapping and permission negative
    fixtures; measure rendered preamble sizes; run `git diff --check`.
  - Completed: 2026-09-08
  - Files changed: `config/pkl/base/workflow-content.pkl`, `config/pkl/base/workflow-change-to-plan.pkl`, `config/pkl/base/workflow-commit.pkl`, `config/pkl/renderers/workflow-composite.pkl`, `config/pkl/renderers/generation-contract-check.pkl`, `.pi/skills/sce-change-to-plan/SKILL.md`, `.pi/skills/sce-next-task/SKILL.md`, `.pi/skills/sce-commit/SKILL.md`, `.pi/skills/sce-validate/SKILL.md`, `.pi/skills/sce-handover/SKILL.md`, `.claude/skills/sce-change-to-plan/SKILL.md`, `.claude/skills/sce-next-task/SKILL.md`, `.claude/skills/sce-commit/SKILL.md`, `.claude/skills/sce-validate/SKILL.md`, `.claude/skills/sce-handover/SKILL.md`, `.agents/skills/sce-change-to-plan/SKILL.md`, `.agents/skills/sce-next-task/SKILL.md`, `.agents/skills/sce-commit/SKILL.md`, `.agents/skills/sce-validate/SKILL.md`, `.agents/skills/sce-handover/SKILL.md`, `context/sce/dedup-ownership-table.md`, `context/sce/handover-workflow.md`, `context/patterns.md`, `context/glossary.md`, `context/plans/compress-workflow-execution-preamble.md`.
  - Result: One inline compact contract renders across five workflows and four targets. Protected payloads and mirror content are preserved.
  - Verify outcome: All focused generation assertions, negative fixtures, baseline/candidate byte comparisons, deterministic-generation comparisons, and `git diff --check` passed.
  - Context impact: local; contract ownership and handover preamble descriptions updated; stale owner statements corrected in patterns and glossary.
  - Context synchronization: synced

Within T01, capture baseline artifacts and requirements before editing, introduce
and wire the shared contract, update directly affected checks, apply the targeted
mirror changes, and record evidence. Source, tests, and mirrors form one coherent
commit unit. Do not add a separate trailing validation or cleanup task.

## Open questions

None blocking. The change is limited to reducing duplicated instructions while
preserving their meaning. Revisit the wording rather than removing safeguards if
the proposed size target conflicts with requirement coverage.

## Source references

Paths are relative to this plan's intended location under `context/plans/`.

- [Repository editing and generation policy](../../AGENTS.md)
- [Workflow content model and next-task/validate bodies](../../config/pkl/base/workflow-content.pkl)
- [Composite renderer and alternate rendering paths](../../config/pkl/renderers/workflow-composite.pkl)
- [Generation contract checks and target inventory](../../config/pkl/renderers/generation-contract-check.pkl)
- [Workflow ownership](../sce/dedup-ownership-table.md)

## Continuation

Implementation and context synchronization are complete. Final repository validation remains required:

`/validate context/plans/compress-workflow-execution-preamble.md`

## Focused verification evidence

Baseline: `2d14959c6770d3635dfadb457e137c6fc05feba4`. Generated inventory: 141 files; exactly twenty selected entrypoints changed. All other artifacts matched byte-for-byte. Fifteen tracked mirrors matched the generated contract and retained all other content.

`pkl eval config/pkl/renderers/generation-contract-check.pkl`, `nix run .#pkl-check-generated`, and `git diff --check`: passed. Generation into two independent candidate directories was byte-identical. Full `nix flake check`: not run by preparation; delegated to ordinary PR CI.

The independent oracle rejected omitted safeguards, missing next-task permission, permission granted to a standard workflow, contradictory prohibition, and duplicate contracts. No model-execution equivalence or tokenizer-based reduction is claimed.

| Requirement | Compact contract clause |
|---|---|
| Workflow ownership | Own this workflow from input through its terminal user-visible response. |
| Ordered steps, gates, and stops | Follow in order; do not add, skip, reorder, or merge them. |
| Internal state and immediate continuation | Keep phase results internal and continue immediately until a defined wait or stop. |
| Wait/resume ownership | Resume user waits in this same skill and session. |
| Output authority and no extra text | Use only specified output layouts; no raw state or text around a layout. |
| Non-SCE helper permission and boundaries | Helpers may assist, return to the active step, and preserve every listed boundary. |
| Sibling restriction | Standard prohibition; next-task alone permits sce-decision at the successful synchronization gate. |

Measurements include one trailing newline per selected section, exclude unchanged phase-reference sections, and retain the existing prose wrapping.

| Target/workflow | Preamble bytes | Words | Nonblank lines | Total SKILL.md bytes | Reduction |
|---|---:|---:|---:|---:|---:|
| `.agents/skills/sce-change-to-plan` | 1375 -> 674 | 206 -> 106 | 21 -> 10 | 10973 -> 10270 | 50.98% |
| `.agents/skills/sce-commit` | 1375 -> 674 | 206 -> 106 | 21 -> 10 | 6873 -> 6170 | 50.98% |
| `.agents/skills/sce-handover` | 1375 -> 674 | 206 -> 106 | 21 -> 10 | 6955 -> 6252 | 50.98% |
| `.agents/skills/sce-next-task` | 1487 -> 764 | 218 -> 116 | 21 -> 11 | 10130 -> 9405 | 48.62% |
| `.agents/skills/sce-validate` | 1375 -> 674 | 206 -> 106 | 20 -> 10 | 4893 -> 4190 | 50.98% |
| `.claude/skills/sce-change-to-plan` | 1375 -> 674 | 206 -> 106 | 21 -> 10 | 10918 -> 10215 | 50.98% |
| `.claude/skills/sce-commit` | 1375 -> 674 | 206 -> 106 | 21 -> 10 | 6846 -> 6143 | 50.98% |
| `.claude/skills/sce-handover` | 1375 -> 674 | 206 -> 106 | 21 -> 10 | 6894 -> 6191 | 50.98% |
| `.claude/skills/sce-next-task` | 1487 -> 764 | 218 -> 116 | 21 -> 11 | 10087 -> 9362 | 48.62% |
| `.claude/skills/sce-validate` | 1375 -> 674 | 206 -> 106 | 20 -> 10 | 4864 -> 4161 | 50.98% |
| `.opencode/skills/sce-change-to-plan` | 1375 -> 674 | 206 -> 106 | 21 -> 10 | 10920 -> 10217 | 50.98% |
| `.opencode/skills/sce-commit` | 1375 -> 674 | 206 -> 106 | 21 -> 10 | 6848 -> 6145 | 50.98% |
| `.opencode/skills/sce-handover` | 1375 -> 674 | 206 -> 106 | 21 -> 10 | 6896 -> 6193 | 50.98% |
| `.opencode/skills/sce-next-task` | 1487 -> 764 | 218 -> 116 | 21 -> 11 | 10089 -> 9364 | 48.62% |
| `.opencode/skills/sce-validate` | 1375 -> 674 | 206 -> 106 | 20 -> 10 | 4866 -> 4163 | 50.98% |
| `.pi/skills/sce-change-to-plan` | 1375 -> 674 | 206 -> 106 | 21 -> 10 | 10896 -> 10193 | 50.98% |
| `.pi/skills/sce-commit` | 1375 -> 674 | 206 -> 106 | 21 -> 10 | 6824 -> 6121 | 50.98% |
| `.pi/skills/sce-handover` | 1375 -> 674 | 206 -> 106 | 21 -> 10 | 6872 -> 6169 | 50.98% |
| `.pi/skills/sce-next-task` | 1487 -> 764 | 218 -> 116 | 21 -> 11 | 10065 -> 9340 | 48.62% |
| `.pi/skills/sce-validate` | 1375 -> 674 | 206 -> 106 | 20 -> 10 | 4842 -> 4139 | 50.98% |

## Context synchronization evidence

The five root context files were reviewed for generation and workflow claims affected by this change. `overview.md`, `architecture.md`, and `context-map.md` require no change. `patterns.md` and `glossary.md` now identify the compact contract owner without changing workflow behavior. The existing ownership-table and handover links remain valid; both domain documents describe the implementation. No new feature or qualifying architecture decision was introduced.

The two additional root edits replace existing text without increasing line counts. No other root context changed. Full repository validation is still pending; successful focused checks do not imply that `nix flake check` has passed.

## Follow-up: approval-gate ownership deduplication

- [x] T02: `Deduplicate next-task approval-gate procedure` (status:done)
  - Scope: `sce-next-task` entrypoint, task-execution reference, output reference,
    canonical Pkl source, generation assertions, tracked Pi/Claude/Codex mirrors,
    and the ownership table.
  - Ownership after change: the entrypoint parses `approved` and conditionally passes
    `approve`; task execution exclusively owns show/wait/approve/decline/block and
    the no-edit-before-approval boundary; `references/output.md` owns the gate field
    order and exact approval question only.
  - Behavior preserved: the gate is always shown, pre-approval never skips it, the
    non-preapproved path waits in the same workflow, ambiguous answers may ask the
    same question once more, rejection returns `declined`, and editing remains
    forbidden before approval.
  - Verify: existing generation contracts pass; rendered package inspection confirms
    the exact approval question remains in `references/output.md`, not `SKILL.md` or
    `task-execution.md`; generated inventory stays unchanged.
  - Context synchronization: synced.


## Follow-up: complete execution-handoff schema ownership

- [x] T03: `Deduplicate next-task complete execution handoff schema` (status:done)
  - Scope: the composite same-session `complete` result produced by task execution,
    the next-task handoff boundary, task-context-sync consumption/validation,
    canonical Pkl sources, tracked Pi/Claude/Codex mirrors, and the ownership table.
  - Ownership after change: `references/task-execution.md` defines the composite
    complete handoff fields once; `/next-task` passes the result verbatim; context
    sync validates and consumes that contract without restating its live field list.
    Package mode retains its existing `references/execution-contract.yaml`.
  - Separate shape preserved: cross-session synchronization recovery continues to
    consume the persisted completed-task record and blocker; this task does not alter
    synchronization-debt recovery semantics.
  - Result: the selected next-task entrypoint/execution/context-sync documents shrink
    by 19 Markdown lines per tracked target.
  - Verify: `pkl eval config/pkl/renderers/generation-contract-check.pkl`,
    `nix run .#pkl-check-generated`, targeted ownership assertions across all three
    tracked targets, and `git diff --check`.
  - Context synchronization: synced.


## Follow-up: synchronization-debt recovery contract

- [x] T04: `Repair next-task synchronization-debt recovery` (status:done)
  - Scope: the composite `/next-task` sync-debt branch, plan-review's `sync_debt`
    result identity, the generated semantic contract, the Pi/Claude/Codex tracked
    mirrors, and the ownership table.
  - Ownership after change: plan review resolves the plan path, debt task identity,
    completed task record, and persisted blocker; `/next-task` routes that record
    verbatim; task context sync validates and consumes it. No separate persisted
    `Context synchronization handoff` object exists.
  - Behavior preserved: all-completed-task debt scanning, legacy incomplete-record
    blocking, lifecycle writes (`synced` / refreshed `blocked`), sync-specific blocked
    output, and post-recovery re-review are unchanged.
  - Verify: `pkl eval config/pkl/renderers/generation-contract-check.pkl`,
    `nix run .#pkl-check-generated`, targeted Pi/Claude/Codex ownership assertions,
    and `git diff --check`.
  - Context synchronization: synced.


## Follow-up: mandatory root-pass ownership

- [x] T05: `Deduplicate the mandatory five-root context pass` (status:done)
  - Scope: next-task task execution and task context synchronization, the retained
    plan-sync discovery list in the shared context-sync source, generated semantic
    checks, tracked Pi/Claude/Codex mirrors, and the ownership table.
  - Ownership after change: the task context-sync **mandatory root pass** subsection
    is the sole owner of the exact five root paths. Discovery references that named
    contract instead of relisting the paths; task execution only hands off
    `context_impact` and does not restate synchronization policy.
  - Behavior preserved: the same five root files remain mandatory on every task
    synchronization invocation; missing files remain reportable gaps; impact
    classifications still cannot waive the pass; synchronization verification still
    requires every root file to be checked against code truth.
  - Verify: `pkl eval config/pkl/renderers/generation-contract-check.pkl`,
    `nix run .#pkl-check-generated`, targeted Pi/Claude/Codex ownership assertions,
    and `git diff --check`.
  - Context synchronization: synced.
