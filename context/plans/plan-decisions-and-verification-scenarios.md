# Plan: plan-decisions-and-verification-scenarios

## Change summary

Extend the SCE plan format with structured **Design decisions** (`D1..Dn`) and
**Verification scenarios** (`S1..Sn`), and link them to the existing acceptance
criteria (`AC1..ACn`) and tasks (`T01..T0N`) through fixed one-line reference
fields. Acceptance criteria gain `Decisions:` and `Scenarios:`, and tasks gain
`Covers:`. The result is a plan that maps AC → D, AC → S, T → AC, and S →
planned check with plain line matching, so `/validate` can record
scenario-level evidence and a later PR-review workflow can find where each
criterion is implemented and proven.

PR #305 (`context/plans/mutation-cursor-ref-reconciliation-wiring.md` on branch
`ref-recon`) showed the value of this: D1–D4 recorded the architecture, AC1–AC15
cited scenario IDs from their `Validate:` lines, and T05's coverage map tracked
each of S1–S39 as `Verified`, `Partially verified`, or `Not verified`. That plan
built this structure by hand and spread it across `Validate:` prose, a
traceability table, and a task's evidence block. This change makes a smaller
version of it part of the format. Each concept gets one fixed home, and
scenarios are authored at planning time rather than reconstructed during a
task.

The change extends existing behavior and does not replace it. Two PRs carry it:

- **PR 1 (T01–T03), plan format and authoring:** canonical plan template,
  plan-authoring phase, and `/next-task` plan review, gate, and execution
  consumption.
- **PR 2 (T04–T05), scenario-based validation:** the `/validate` phase and both
  Validation Report formats.

Plans without these sections, which includes every plan on disk today, keep
working unchanged. Historical plans are not migrated.

### Format at a glance

```markdown
## Design decisions

- D1: {decision}
  - Boundary: {architectural boundary the decision governs}
  - Rationale: {why}
  - Rejected: {alternative} — {why not}
  - Trade-offs: {known limitation or cost}

## Acceptance criteria

- [ ] AC1: {observable outcome}
  - Validate: `{check}`
  - Decisions: D1
  - Scenarios: S1, S2

### Verification scenarios

- S1: {short title}
  - Given: {preconditions}
  - When: {action}
  - Then: {exact observable result or invariant}
  - Check: {planned test, command, or named inspection}

## Task stack

- [ ] T01: `{title}` (status:todo)
  - Task ID: T01
  - Covers: AC1
  - ...
```

This plan uses the format it introduces.

## Design decisions

- D1: The plan's own Markdown carries the structure. Each relationship is a fixed one-line field.
  - Boundary: The persisted plan schema owned by `references/plan-template.md`, generated from `changeToPlanPlanTemplate` in `config/pkl/base/workflow-change-to-plan.pkl`.
  - Rationale: The plan already carries its other machine-read state (checkboxes, `(status:…)`, `Context synchronization:`) as fixed line prefixes. Reusing that convention means line matching finds every candidate definition and field without an LLM. The checkbox-aware pattern `^[[:space:]]*-[[:space:]]+(\[[ x]\][[:space:]]+)?(D[1-9][0-9]*|AC[1-9][0-9]*|S[1-9][0-9]*|T[0-9]{2,}):` matches definitions, including `- [ ] AC1:` and `- [ ] T01:`, and `^[[:space:]]*-[[:space:]]+(Decisions|Scenarios|Covers|Check|Validate):[[:space:]]` matches fields. A match counts only outside fenced code blocks and only inside its definition's own section: `D` under `## Design decisions`, `AC` under `## Acceptance criteria`, `S` under `### Verification scenarios`, and `T` under `## Task stack`, where a section ends at the next heading of any level. A fenced example, such as this plan's `Format at a glance`, therefore never defines anything. A field belongs to the definition it is indented under. Line matching only finds candidates; reference integrity is proven by the test-only structural check. It also keeps one plan representation.
  - Rejected: A YAML or JSON sidecar. It creates a second source of truth that drifts from the prose. A traceability table like PR #305's. Tables are hard to edit incrementally, and they duplicate links the fields already hold. A CLI parser or linter command. That is new runtime surface, which the request rules out.
  - Trade-offs: Prose contracts are enforced by workflow instructions plus generated-text contract checks. No production code parses a real plan in this change. The test-only checker in `config/pkl/renderers/plan-structure.pkl` parses the generated template's fenced examples and the checked-in plan fixtures during `nix run .#pkl-check-generated`. Extractability is a property of the format, not a shipped tool.
- D2: Every AC in a structured plan carries an explicit `Scenarios:` field. Its value is a list of scenario IDs or the literal `none (inspection)`. `Decisions:` is omitted when no decision applies.
  - Boundary: The acceptance-criteria shape and the relationship rules in the plan template.
  - Rationale: "Behavioral" versus "structural" is a judgement call. An explicit marker turns a missing scenario into a detectable defect (the field is absent) instead of an ambiguous one (did the author forget, or is it inspection-only?). Decisions are optional by design, so a missing `Decisions:` field is not a defect.
  - Rejected: Allowing `Scenarios:` to be omitted for inspection criteria. The gap could then not be detected without judgement.
  - Trade-offs: Documentation-only plans gain one boilerplate line per AC.
- D3: The structured rules apply only to *structured plans*: a plan with a `### Verification scenarios` heading, or with any task carrying `Covers:`, outside fenced code blocks. Every other plan is a legacy plan and follows today's behavior exactly.
  - Boundary: Format detection in `/next-task` plan review and in `/validate`. Existing-plan revision rules in the template.
  - Rationale: This keeps the change backward compatible without migrating anything. Detection uses one deterministic textual test. Revising a legacy plan keeps it legacy unless the correction explicitly asks to adopt the structured format.
  - Rejected: Requiring D/S on every plan. That would break `/validate` on every historical plan. Inferring structure from `S\d` mentions in prose. PR #305 cites scenarios inside `Validate:` text, and treating that as structure would misclassify it.
  - Trade-offs: A half-converted plan (for example, `Covers:` without a scenarios section) counts as structured. Its missing pieces are reported as relationship failures rather than tolerated.
- D4: An AC is verified only by evidence `/validate` gathers in its own run. Every scenario gets one status: `Verified`, `Partially verified`, or `Not verified`. An AC is checked only when its `Validate:` check passes and every one of its scenarios is `Verified`.
  - Boundary: Evidence rules in the validation phase (`references/validation.md`) and the scenario section of both Validation Report formats.
  - Rationale: Task completion, a task's recorded `Verify` outcomes, and a `Check:` line describe intent or history. They are not proof of the finished system. PR #305's rule that "a related unit test never upgrades a production-path scenario" is the precedent for refusing indirect evidence.
  - Rejected: Inheriting scenario status from completed tasks' `Verify` records. That is cheaper, but it lets stale or narrower evidence count.
  - Trade-offs: `/validate` runs more checks on structured plans, and it fails when a planned check cannot be run or located.

## Acceptance criteria

How this plan is proven complete. `/validate` runs these checks; no task in the
stack performs final validation.

- [x] AC1: On every target, the generated `sce-change-to-plan/references/plan-template.md` defines the optional `## Design decisions` section (`Boundary:`, `Rationale:`, `Rejected:`, `Trade-offs:`), the AC fields `Decisions:` and `Scenarios:`, the `### Verification scenarios` section (`Given:`, `When:`, `Then:`, `Check:`), and the task field `Covers:`. Every existing compact task and completion field, the `Full validation` and `Context sync` sections, and the task context synchronization lifecycle section are kept unchanged.
  - Validate: the planned `structured-plan-template-schema` contract check and the existing `compact-plan-template-schema` check both pass inside `nix run .#pkl-check-generated`.
  - Decisions: D1, D2
  - Scenarios: S1, S2
- [x] AC2: The plan template states the deterministic relationship rules. These cover the ID grammar, unique definitions, sequential first-assignment numbering with no renumbering or reuse, the comma-separated reference-list grammar, resolution of every referenced ID, `Scenarios:` required on every AC of a structured plan (IDs or `none (inspection)`), every AC covered by at least one task, every scenario referenced by at least one AC, structured-plan detection, and the statement that a planned `Check:` is not execution evidence.
  - Validate: inspection of the generated plan template against the rule list in this criterion, plus the planned `structured-plan-template-schema` check and the `plan-template-structure` reference-integrity check.
  - Decisions: D1, D2, D3
  - Scenarios: S3
- [x] AC3: The plan-authoring phase authors decisions, then criteria, then scenarios, then tasks. It applies the template's new rules instead of restating them. It forbids manufactured decisions and invented test names, and it requires a reference-consistency pass before writing the plan.
  - Validate: the planned `plan-authoring-structured-references` check and the updated plan-template ownership check pass in `nix run .#pkl-check-generated`.
  - Decisions: D1
  - Scenarios: S4
- [x] AC4: For a structured plan, `/next-task` plan review includes the selected task's covered ACs, their scenarios, and their related decisions in the `ready` result, and the implementation gate shows them in an optional section. For a legacy plan, both outputs are unchanged. Readiness criteria, approval behavior, the `approved` flag, synchronization-debt handling, and continuation are unchanged. A dangling reference is reported as a review risk and does not block the task.
  - Validate: the planned `next-task-structured-references` check passes, and the existing next-task semantic checks still pass in `nix run .#pkl-check-generated`.
  - Decisions: D3
  - Scenarios: S5, S6
- [x] AC5: Task execution never checks an AC checkbox and never records a scenario status. Covered-AC scenarios may shape the task's own `Verify` checks. The completion record keeps its current fields; `Covers:` stays as authored, and `Files changed` stays baseline-relative.
  - Validate: the planned `next-task-structured-references` check and the existing `next-task-compact-completion-writing` check pass.
  - Decisions: D4
  - Scenarios: S6
- [x] AC6: For a structured plan, `/validate` runs or performs every scenario's `Check:` itself and gives each scenario exactly one status. It checks an AC only when its `Validate:` check passes and all of that AC's scenarios are `Verified`. Task completion, recorded task `Verify` outcomes, and the plan's own claims never count as evidence.
  - Validate: the planned `validate-scenario-evidence` check passes, and the existing `validation-repair` and `validate-decision-sync-boundary` checks still pass.
  - Decisions: D4
  - Scenarios: S7, S8
- [x] AC7: In a structured plan, a relationship-integrity defect makes validation `failed` and is listed under failed checks. The defects are a duplicate ID, a dangling reference, an AC without `Scenarios:`, an AC that no task covers, and a scenario that no AC references. Such a defect is never silently tolerated.
  - Validate: the planned `validate-scenario-evidence` check asserts this rule in the generated `validation.md`.
  - Decisions: D3, D4
  - Scenarios: S9
- [x] AC8: For structured plans, both Validation Report formats (the plan-file report and the returned report) contain a scenario-verification section. Each entry maps one scenario ID to its ACs, its evidence kind (`test`, `command`, or `inspection`), the exact test, command, or inspected location, the actual result, and its status. AC evidence lines cite scenario IDs. The section is omitted for legacy plans.
  - Validate: the planned `validation-report-scenario-section` check passes in `nix run .#pkl-check-generated`.
  - Decisions: D4
  - Scenarios: S10
- [x] AC9: Existing plans stay valid and untouched. No file under `context/plans/` other than this plan changes. The generated plan-review and validation references keep today's behavior for plans without the new sections.
  - Validate: `git diff --stat main...HEAD -- context/plans/` lists only this plan; the planned `next-task-structured-references` and `validate-scenario-evidence` checks assert the legacy-plan sentences.
  - Decisions: D3
  - Scenarios: S11
- [x] AC10: Generated output stays deterministic and target-neutral. The artifact path count stays 145, and every new contract check has a checked-in negative fixture that `config/pkl/check-generated.sh` proves fails with its diagnostic.
  - Validate: `nix run .#pkl-check-generated` and `nix flake check` pass.
  - Scenarios: S2, S12

### Verification scenarios

- S1: Structured template schema renders on all targets
  - Given: T01 is implemented and generation runs from the canonical Pkl sources.
  - When: `nix run .#pkl-check-generated` evaluates `generation-contract-check.pkl`.
  - Then: For the OpenCode, Claude, Pi, and Codex `plan-template.md` outputs, the planned `structured-plan-template-schema` check reports success. The `compact-plan-template-schema` check still reports success.
  - Check: `nix run .#pkl-check-generated` exits 0, and its output lists both checks as passing.
- S2: Missing structured-format token is rejected
  - Given: A negative fixture supplies a `plan-template.md` that lacks `### Verification scenarios` or `Covers:`.
  - When: `config/pkl/check-generated.sh` runs its fixture-failure step.
  - Then: Evaluating the fixture fails with the exact `structured-plan-template-schema` diagnostic.
  - Check: The planned fixture under `config/pkl/renderers/fixtures/` is registered with `expect_pkl_fixture_failure`, and `nix run .#pkl-check-generated` passes.
- S3: Relationships are extractable by line matching
  - Given: The canonical template's filled-in structured example in `config/pkl/base/workflow-change-to-plan.pkl`.
  - When: Candidate definitions and fields are extracted with `nix shell nixpkgs#ripgrep -c rg -n '^[[:space:]]*-[[:space:]]+(\[[ x]\][[:space:]]+)?(D[1-9][0-9]*|AC[1-9][0-9]*|S[1-9][0-9]*|T[0-9]{2,}):|^[[:space:]]*-[[:space:]]+(Decisions|Scenarios|Covers|Check|Validate):[[:space:]]' config/pkl/base/workflow-change-to-plan.pkl`, and the generator's structural reference-integrity check evaluates the generated template's fenced examples as separate sample documents.
  - Then: Every definition, including the checkbox-prefixed `- [ ] AC1:` and `- [ ] T01:` lines, and every reference field is matched on its own line. The structural check confirms that, outside fenced code blocks and within each definition's own section, every ID is defined once, every field sits on its own kind of definition, each reference value is a comma-separated ID list or `none (inspection)`, every referenced ID resolves, every AC is covered by a task, and every scenario is referenced by an AC.
  - Check: The rg command above for candidate extraction, plus `nix run .#pkl-check-generated`, whose `plan-template-structure` check and `config/pkl/renderers/fixtures/plan-structure/` fixtures prove reference integrity. An rg match alone does not prove reference integrity.
- S4: Plan authoring cites, and does not restate, the template rules
  - Given: T02 is implemented.
  - When: The plan-template ownership check and the planned `plan-authoring-structured-references` check evaluate the generated `plan-authoring.md`.
  - Then: `plan-authoring.md` cites the new template rule sections by name, states the D→AC→S→T order and the pre-write consistency pass, and contains none of the template-owned rule sentences. A negative fixture that omits the citation fails.
  - Check: `nix run .#pkl-check-generated` exits 0.
- S5: Structured plan surfaces coverage in review and gate
  - Given: T03 is implemented, and a structured plan whose selected task has `Covers: AC1`, where AC1 lists `Decisions: D1` and `Scenarios: S1`.
  - When: The generated `plan-review.md` and the `output.md` implementation gate are inspected.
  - Then: The `ready` result contract names the covered ACs, their scenario IDs with `Then:` and `Check:`, and the related decisions. The gate has an optional coverage section that is omitted when the plan is legacy. No readiness bullet, approval sentence, or status was removed or added.
  - Check: The planned `next-task-structured-references` check, plus `git diff` inspection of the readiness and gate sections in `config/pkl/base/workflow-next-task.pkl`.
- S6: Execution does not claim plan-level verification
  - Given: T03 is implemented.
  - When: The generated `task-execution.md` is evaluated.
  - Then: It states that task execution never checks an AC checkbox and never records scenario status. It keeps the compact completion-writing tokens, and it keeps `Covers:` as authored.
  - Check: The planned `next-task-structured-references` check and the existing `next-task-compact-completion-writing` check pass in `nix run .#pkl-check-generated`.
- S7: AC is not verified from task completion or plan claims
  - Given: T04 is implemented, and a structured plan where every task is done and AC2's scenario S3 has a `Check:` that was never executed.
  - When: The generated `validation.md` rules are applied.
  - Then: S3 is `Not verified`, AC2 stays unchecked, and the result is `failed`, even though the covering task is done and its `Verify` record says passed.
  - Check: The planned `validate-scenario-evidence` check asserts the evidence-source rule and the AC-verification rule in the generated `validation.md`.
- S8: Partial evidence does not upgrade a scenario
  - Given: A scenario whose check was run only at a narrower level than its `Then:` requires.
  - When: Validation classifies it.
  - Then: The scenario is `Partially verified`, its AC stays unchecked, and the gap appears under failed checks with a required action.
  - Check: The planned `validate-scenario-evidence` check asserts the three status names and the `Partially verified` rule.
- S9: Broken references fail validation
  - Given: A structured plan with a duplicate `S2`, an AC citing an undefined `D5`, and an AC that no task's `Covers:` names.
  - When: Validation runs its relationship-integrity step.
  - Then: Each defect is listed under failed checks, and the status is `failed`.
  - Check: The planned `validate-scenario-evidence` check asserts the integrity rule list in the generated `validation.md`.
- S10: Report maps scenarios to actual evidence
  - Given: T05 is implemented.
  - When: The generated `validation-report.md` and the report variants in `validation.md` are inspected.
  - Then: Each format has a scenario-verification section with the fields scenario ID, ACs, evidence kind, evidence, result, and status. The section is marked omitted for legacy plans, and AC lines cite scenario IDs.
  - Check: The planned `validation-report-scenario-section` check passes, and its negative fixture fails with its diagnostic.
- S11: Legacy plans keep the legacy path
  - Given: `context/plans/remove-checkout-id.md` and `context/plans/agent-trace-export-readers.md`, which have no `### Verification scenarios` and no `Covers:`.
  - When: The generated `plan-review.md` and `validation.md` detection rules are applied.
  - Then: Both plans are classified as legacy. Review, gate, validation, and report behavior match today's. Neither plan file changes.
  - Check: Inspection of the detection sentence in both generated references against these two plans, plus `git diff --stat main...HEAD -- context/plans/`.
- S12: Parity and artifact inventory hold
  - Given: All tasks are implemented.
  - When: `nix flake check` runs.
  - Then: `pkl-generated` passes with `expectedArtifactPathCount` still 145, target-neutral reference parity passes, and every other check passes.
  - Check: `nix flake check` exits 0.

### Full validation

- `nix run .#pkl-check-generated`
- `nix flake check`

### Context sync

- `context/sce/shared-context-plan-workflow.md`: the structured plan format and the authoring order.
- `context/sce/shared-context-code-workflow.md`: `/next-task` coverage consumption, `/validate` scenario evidence, and the integrity rules.
- `context/sce/config-generation.md`: the new contract checks and fixtures.
- `context/sce/workflow-vocabulary.md`: the terms *design decision*, *verification scenario*, *structured plan*, and *legacy plan*, distinguishing a plan-level decision from an ADR written by `sce-decision`.

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** `config/pkl/base/workflow-change-to-plan.pkl` (`changeToPlanPlanTemplate`, `changeToPlanPlanAuthoring`), `config/pkl/base/workflow-next-task.pkl` (`nextTaskPlanReviewReference`, `nextTaskTaskExecutionReference`, `renderImplementationGate`/`nextTaskOutputReference`), `config/pkl/base/workflow-validate.pkl` (`renderValidationSkillBody`, `renderValidationResult`, `renderValidationReport`), `config/pkl/renderers/generation-contract-check.pkl`, the test-only `config/pkl/renderers/plan-structure.pkl` checker and its `plan-structure-check.pkl` driver, new fixtures under `config/pkl/renderers/fixtures/`, and `config/pkl/check-generated.sh` fixture registration.
- **Out of scope:** SCE PR Review; any Rust CLI command, parser, linter, storage, or API; migrating or editing historical plans; ADR (`sce-decision`) behavior; context-sync behavior; the unrendered legacy `planAuthoringPackage`/`renderPlanTemplate` locals in `workflow-change-to-plan.pkl`.
- **Constraints:** Edit canonical Pkl only. Generated trees are never committed. Rendered references must stay identical across all targets. `/validate` stays verification-only. Task IDs, checkbox and status markers, completion fields, and the synchronization lifecycle stay unchanged apart from the new `Covers:` field. Existing contract diagnostics keep their exact wording.
- **Non-goal:** Reproducing PR #305's traceability table, review-finding mapping, authoring-time observation section, or mutation-style evidence ("did the test fail on a broken implementation?").

## Assumptions

- The live plan template is `changeToPlanPlanTemplate`, rendered through `structuredComposite.referenceDocuments`. The `PLAN_TEMPLATE` in the `planAuthoringPackage` skills mapping is not rendered by any renderer, so it is left alone.
- Adding an optional coverage section to the implementation gate, omitted for legacy plans, does not change the execution gate. The approval question, the `approved` flag, readiness criteria, and statuses are unchanged.
- The contract-check names used in this plan (`structured-plan-template-schema`, `plan-authoring-structured-references`, `next-task-structured-references`, `validate-scenario-evidence`, `validation-report-scenario-section`) are planned names. They do not exist yet, and the implementing task may refine them.
- New fixtures add no generated artifact paths, so `expectedArtifactPathCount` stays 145.

## Task stack

- [x] T01: `Add design decisions, verification scenarios, and coverage fields to the plan template` (status:done)
  - Task ID: T01
  - Covers: AC1, AC2, AC9, AC10
  - Scope: In — `changeToPlanPlanTemplate` in `config/pkl/base/workflow-change-to-plan.pkl`:
    - Add the optional `## Design decisions` section after `## Change summary`.
    - Add `Decisions:` and `Scenarios:` to the AC template, and add `### Verification scenarios` between the AC list and `### Full validation`.
    - Add `Covers:` after `Task ID:` in the task template and in the filled-in example.
    - Add new `Design decision rules`, `Verification scenario rules`, and `Reference rules` sections. The reference rules cover the ID grammar, uniqueness, sequential numbering with no renumbering or reuse (withdrawn entries keep their ID), the list grammar, resolution, coverage, structured-plan detection, and the statement that a planned check is not evidence.
    - Add a short structured filled-in example: one D, one behavioral AC with a scenario, one `none (inspection)` AC, and one task.
    - Extend `Updating an existing plan`: a legacy plan stays legacy unless the correction asks for the structured format, and new IDs are appended.
    - Add the `structured-plan-template-schema` assertion to `generation-contract-check.pkl`, and add the new rule sentences to `planTemplateOwnedPolicyTokens`.
    - Add one negative fixture and register it in `config/pkl/check-generated.sh`.
    Out — plan-authoring steps, `/next-task`, `/validate`, and historical plans.
  - Dependencies: none
  - Done when: The generated template on every target contains the new sections, fields, and rules, with every existing compact field intact. Both schema checks pass, and the new fixture fails with its diagnostic.
  - Verify: `nix run .#pkl-check-generated` — passed (145 files; `structured-plan-template-schema`, `compact-plan-template-schema`, and `plan-template-policy-ownership` all report success; the new fixture fails with its exact diagnostic). S3 rg inspection of the canonical structured example — passed: every definition (`D1`, `AC1`, `AC2`, `S1`, `T01`) and reference field matched on its own line, each value is an ID list or `none (inspection)`, and every referenced ID resolves. Deviation: S3's literal pattern `^\s*- ((D|AC|S)[1-9][0-9]*|T[0-9]{2}):` cannot match checkbox lines (`- [ ] AC1:`, `- [ ] T01:`), so the inspection used `^\s*- (\[[ x]\] )?((D|AC|S)[1-9][0-9]*|T[0-9]{2}):|^\s+- (Decisions|Scenarios|Covers|Check|Validate): `; the template's `Reference rules` state the checkbox-prefixed definition form.
  - Completed: 2026-10-23
  - Files changed: config/pkl/base/workflow-change-to-plan.pkl, config/pkl/renderers/generation-contract-check.pkl, config/pkl/renderers/fixtures/structured-plan-template-schema-check.pkl, config/pkl/check-generated.sh, context/plans/plan-decisions-and-verification-scenarios.md
  - Result: `changeToPlanPlanTemplate` now has an optional `## Design decisions` section, AC `Decisions:`/`Scenarios:` fields, `### Verification scenarios`, task `Covers:` (template and filled-in task example), `Design decision rules`, `Verification scenario rules`, `Reference rules`, a filled-in structured plan example, and legacy/next-ID rules under `Updating an existing plan`. Added the `structured-plan-template-schema` assertion, seven new template-owned policy tokens, and one registered negative fixture.
  - Context impact: `context/sce/shared-context-plan-workflow.md` (structured plan format and reference rules), `context/sce/config-generation.md` (new contract check and fixture).
  - Context synchronization: synced

- [x] T02: `Author decisions, scenarios, and coverage in the plan-authoring phase` (status:done)
  - Task ID: T02
  - Covers: AC3
  - Scope: In — `changeToPlanPlanAuthoring`:
    - Add a step that records design decisions before the AC step. Add only real architectural choices, and omit the section when there are none.
    - Make the AC step author `Decisions:` and `Scenarios:` per the template.
    - Add a scenario-authoring step that covers normal, boundary, failure, cancellation, concurrency, and recovery cases only where they are relevant, with no invented test names.
    - Make the task step add `Covers:`.
    - Before writing, run a reference-consistency pass that applies the template's `Reference rules`.
    - Update the template-rules citation sentence and `requiredPlanAuthoringTemplateReferences`.
    - Add the `plan-authoring-structured-references` assertion, one negative fixture, and its registration.
    Out — the template schema (T01) and the summary or output layouts.
  - Dependencies: T01
  - Done when: The generated `plan-authoring.md` states the D→AC→S→T order and the consistency pass, cites the template rule sections, and restates no template-owned rule. The checks pass, and the fixture fails as expected.
  - Verify: `nix run .#pkl-check-generated` — passed (145 files; `plan-template-policy-ownership` with the extended `requiredPlanAuthoringTemplateReferences` and the new `plan-authoring-structured-references` check both pass; the new fixture fails with its exact diagnostic). The generated `plan-authoring.md` is byte-identical across the OpenCode, Claude, Pi, and Codex targets.
  - Completed: 2026-10-23
  - Files changed: config/pkl/base/workflow-change-to-plan.pkl, config/pkl/renderers/generation-contract-check.pkl, config/pkl/renderers/fixtures/plan-authoring-structured-references-check.pkl, config/pkl/check-generated.sh, context/plans/plan-decisions-and-verification-scenarios.md
  - Result: `changeToPlanPlanAuthoring` now cites `Design decision rules`, `Verification scenario rules`, and `Reference rules`. It states the decisions → criteria → scenarios → tasks order, with a legacy-revision skip, and adds steps 2.5 `Record design decisions` (no manufactured decisions), 2.7 `Author the verification scenarios` (relevant case kinds only, no invented test names), and 2.9 `Check references` (a pre-write reference-consistency pass). The AC and task steps now author `Decisions:`/`Scenarios:` and `Covers:`. The write and return steps were renumbered to 2.10 and 2.11. Added the three citations to `requiredPlanAuthoringTemplateReferences`, the `plan-authoring-structured-references` assertion, and one registered negative fixture.
  - Context impact: `context/sce/shared-context-plan-workflow.md` (plan-authoring order and reference-consistency pass), `context/sce/config-generation.md` (new contract check and fixture).
  - Context synchronization: synced

- [x] T03: `Surface covered criteria, scenarios, and decisions in /next-task review, gate, and execution` (status:done)
  - Task ID: T03
  - Covers: AC4, AC5, AC9
  - Scope: In —
    - `nextTaskPlanReviewReference`: apply the structured-plan detection rule. For structured plans, the `ready` result also carries the task's `Covers:` ACs, their scenarios (`Then:` and `Check:`), and their decisions. A dangling reference becomes a review risk, not a blocker. The legacy-plan sentence keeps behavior unchanged.
    - Implementation gate layout: add an optional `## Acceptance coverage` section before `## Verification`, omitted when the plan is legacy.
    - `nextTaskTaskExecutionReference`: covered scenarios may inform the task's `Verify` checks. Execution never checks AC checkboxes or records scenario status, and `Covers:` is preserved.
    - Add the `next-task-structured-references` assertion, one negative fixture, and its registration.
    Out — readiness criteria, approval wording, sync-debt recovery, continuation, and context-sync references.
  - Dependencies: T01
  - Done when: The generated next-task references carry the coverage contract and the legacy sentence. All existing next-task semantic checks pass unchanged, and the new fixture fails as expected.
  - Verify: `nix run .#pkl-check-generated` — passed (145 files; the new `next-task-structured-references` check reports success, all existing next-task semantic checks including `next-task-compact-completion-writing` and `plan-review-reads-completed-record` still pass, and the new fixture fails with its exact diagnostic). The generated `plan-review.md`, `output.md`, and `task-execution.md` are byte-identical across the OpenCode, Claude, Pi, and Codex targets. `git diff` review — passed: the diff only adds lines (0 removed), so no readiness bullet, approval sentence, status, sync-debt rule, or continuation changed.
  - Completed: 2026-10-23
  - Files changed: config/pkl/base/workflow-next-task.pkl, config/pkl/renderers/generation-contract-check.pkl, config/pkl/renderers/fixtures/next-task-structured-references-check.pkl, config/pkl/check-generated.sh, context/plans/plan-decisions-and-verification-scenarios.md
  - Result: `nextTaskPlanReviewReference` 1.3 now detects structured plans, reads the selected task's covered ACs with their scenarios and decisions, and treats dangling references as review risks rather than blockers. 1.5 lists the structured `ready` coverage fields, and a legacy-plan sentence leaves legacy review unchanged. Both copies of the implementation gate gained an optional `## Acceptance coverage` section before `## Verification`, omitted for legacy plans. `nextTaskTaskExecutionReference` 2.6 lets covered scenarios shape `Verify` checks and states that execution never checks an AC checkbox or records a scenario status. 2.7 keeps `Covers:` as authored. Added the `next-task-structured-references` assertion and one registered negative fixture.
  - Context impact: `context/sce/shared-context-code-workflow.md` (`/next-task` coverage consumption and the execution boundary), `context/sce/config-generation.md` (new contract check and fixture).
  - Context synchronization: synced

- [x] T04: `Verify scenarios and relationship integrity in the /validate phase` (status:done)
  - Task ID: T04
  - Covers: AC6, AC7, AC9
  - Scope: In — `renderValidationSkillBody` in `config/pkl/base/workflow-validate.pkl`:
    - Step 3 also collects decisions, scenarios, and `Covers:` when the plan is structured.
    - Add a relationship-integrity step that applies the template's `Reference rules` and records each defect as a failed check.
    - Step 4 runs or performs every scenario's `Check:` and assigns `Verified`, `Partially verified`, or `Not verified`. An AC is checked only when its `Validate:` check passes and all of its scenarios are `Verified`. Task completion, recorded `Verify` outcomes, and plan claims are excluded as evidence.
    - The legacy-plan sentence keeps today's steps.
    - The verification-only boundary is unchanged.
    - Add the `validate-scenario-evidence` assertion, one negative fixture, and its registration.
    Out — report formats (T05) and any repair or synchronization behavior.
  - Dependencies: T01
  - Done when: The generated `validation.md` carries the integrity rules, the evidence rules, the three statuses, the AC rule, and the legacy sentence. The `validation-repair` and `validate-decision-sync-boundary` checks still pass, and the new fixture fails as expected.
  - Verify: `nix run .#pkl-check-generated` — passed (exit 0, 145 files). Running `pkl eval` on `generation-contract-check.pkl` directly reports success for the new `validate-scenario-evidence` check and for the existing `validation-observational` (`validation-repair` fixture), `validate-decision-sync-boundary`, and `validation-report-policy-ownership` checks. The new fixture fails with its exact diagnostic. The generated `validation.md` is byte-identical across the OpenCode, Claude, Pi, and Codex targets.
  - Completed: 2026-10-23
  - Files changed: config/pkl/base/workflow-validate.pkl, config/pkl/renderers/generation-contract-check.pkl, config/pkl/renderers/fixtures/validate-scenario-evidence-check.pkl, config/pkl/check-generated.sh, context/plans/plan-decisions-and-verification-scenarios.md
  - Result: In `renderValidationSkillBody`, step 3 now detects structured plans with a legacy-plan sentence and collects decisions, AC `Decisions:`/`Scenarios:`, scenarios with `Then:`/`Check:`, and task `Covers:`. A new step 4 `Check plan references` applies the template's `Reference rules` and lists the five relationship-integrity defects; each is recorded as a failed check and makes the result `failed`. Step 5 runs or performs every scenario's `Check:` and assigns `Verified`, `Partially verified`, or `Not verified`. It excludes task completion, recorded `Verify` outcomes, planned `Check:` lines, and plan claims as evidence, and it meets an AC only when its `Validate:` check passes and all of its scenarios are `Verified` (`none (inspection)` ACs need only `Validate:`). The update and return steps were renumbered to 6 and 7. Added the `validate-scenario-evidence` assertion and one registered negative fixture.
  - Context impact: `context/sce/shared-context-code-workflow.md` (`/validate` scenario evidence and integrity rules), `context/sce/config-generation.md` (new contract check and fixture).
  - Context synchronization: synced

- [x] T05: `Add scenario verification to the Validation Report formats` (status:done)
  - Task ID: T05
  - Covers: AC8, AC10
  - Scope: In —
    - `renderValidationReport`: add `### Scenario verification` after `### Success-criteria verification` with one entry per scenario: `S{n}` (AC IDs) → evidence kind, exact test, command, or location, actual result, and status. AC lines cite scenario IDs. The section is omitted for legacy plans.
    - `renderValidationResult`: add matching sections to the validated and failed variants, and add scenario entries under `What failed` for `Partially verified` and `Not verified` scenarios.
    - Add the `validation-report-scenario-section` assertion, one negative fixture, and its registration.
    Out — validation execution rules (T04).
  - Dependencies: T04
  - Done when: Both generated report formats carry the scenario section and the legacy-omission rule. The checks pass, the fixture fails as expected, and the artifact count is unchanged.
  - Verify: `nix run .#pkl-check-generated` — passed (exit 0, 145 files). Running `pkl eval` on `generation-contract-check.pkl` directly reports success for the new `validation-report-scenario-section` check and for the existing `validation-report-policy-ownership`, `validation-observational`, `validate-scenario-evidence`, `no-blank-line-runs`, `output-dedup`, and `target-neutral-references` checks. The new fixture fails with its exact diagnostic. The generated `validation.md` and `validation-report.md` are byte-identical across the OpenCode, Claude, and Pi targets. `nix flake check` — passed (all checks passed). It needed the new fixture marked intent-to-add (`git add -N`) because the flake source only includes tracked files.
  - Completed: 2026-10-23
  - Files changed: config/pkl/base/workflow-validate.pkl, config/pkl/renderers/generation-contract-check.pkl, config/pkl/renderers/fixtures/validation-report-scenario-section-check.pkl, config/pkl/check-generated.sh, context/plans/plan-decisions-and-verification-scenarios.md
  - Result: `renderValidationReport` now has a `### Scenario verification` section after `### Success-criteria verification`, with one `S{n} ({AC ids})` entry per scenario giving the evidence kind, exact evidence, actual result, and status. AC lines cite scenario IDs, and presentation rules omit the section for legacy plans and render only statuses established by validation execution. `renderValidationResult` gained matching `## Scenario verification` sections in the validated and failed variants, scenario entries under `What failed` for `Partially verified` and `Not verified` scenarios, scenario citations on AC lines, and report rules for the structured and legacy cases. Added the `validation-report-scenario-section` assertion and one registered negative fixture.
  - Context impact: `context/sce/shared-context-code-workflow.md` (scenario section in both Validation Report formats), `context/sce/config-generation.md` (new contract check and fixture).
  - Context synchronization: synced

## Open questions

- Every new check is a generated-text contract. Nothing exercises an agent actually authoring or validating a structured plan. Would a manual dry run be worth adding as a follow-up: `/validate` on a small throwaway structured plan, including one deliberately dangling reference? It is not needed to land the format.

## Validation Report

**Status:** validated  
**Date:** 2026-10-23

### Commands run

- `nix run .#pkl-check-generated` -> exit 0 (Ephemeral Pkl generation passed: 145 files; every negative fixture and plan-structure fixture produced its expected result)
- `nix flake check` -> exit 0 (all checks passed; the 17 `checks.x86_64-linux` derivations, including `pkl-generated`, `cli-tests`, `cli-clippy`, and `cli-fmt`)
- `nix develop -c sh -c 'pkl eval config/pkl/renderers/generation-contract-check.pkl'` -> exit 0 (every contract check reports success, including `structured-plan-template-schema`, `compact-plan-template-schema`, `plan-template-structure`, `plan-template-policy-ownership`, `plan-authoring-structured-references`, `next-task-structured-references`, `next-task-compact-completion-writing`, `validation-observational`, `validate-decision-sync-boundary`, `validate-scenario-evidence`, `validation-report-scenario-section`, `target-neutral-references`, and `artifact-paths` = exactly 145)
- `nix shell nixpkgs#ripgrep -c rg -n '<S3 pattern>' config/pkl/base/workflow-change-to-plan.pkl` -> exit 0 (every definition, including `- [ ] AC1:` and `- [ ] T01:`, and every reference field matched on its own line)
- `pkl eval -p planPath=<plan> config/pkl/renderers/plan-structure-check.pkl` on this plan -> exit 0 (`structured plan: reference integrity holds`)
- `pkl eval -p planPath=<plan> config/pkl/renderers/plan-structure-check.pkl` on `context/plans/remove-checkout-id.md` and `context/plans/agent-trace-export-readers.md` -> exit 0 (both `legacy plan: structured reference rules do not apply`)
- `nix run .#pkl-generate -- <scratch dir>` -> exit 0 (generated references used for the AC2, S5, and S11 inspections)
- `git diff --stat main...HEAD -- context/plans/` -> exit 0 (only `context/plans/plan-decisions-and-verification-scenarios.md`)
- `git diff main...HEAD --numstat -- config/pkl/base/workflow-next-task.pkl` -> exit 0 (48 added, 0 removed)

### Success-criteria verification

- [x] AC1: Generated plan template defines decisions, AC reference fields, verification scenarios, and `Covers:` while keeping the compact schema -> `structured-plan-template-schema` and `compact-plan-template-schema` pass inside `nix run .#pkl-check-generated`; scenarios: S1, S2
- [x] AC2: Plan template states the deterministic relationship rules -> inspection of the generated Claude `plan-template.md` `## Reference rules` section confirms the ID grammar, single definition, sequential numbering with no renumbering or reuse, comma-separated list grammar, resolution, required `Scenarios:` (IDs or `none (inspection)`), task coverage of every AC, AC reference of every scenario, structured-plan detection, and "A planned `Check:` is not execution evidence"; `structured-plan-template-schema` and `plan-template-structure` pass; scenarios: S3
- [x] AC3: Plan authoring orders D→AC→S→T, cites rather than restates template rules, forbids manufactured decisions and invented test names, and runs a pre-write consistency pass -> `plan-authoring-structured-references` and `plan-template-policy-ownership` pass; scenarios: S4
- [x] AC4: `/next-task` review and gate surface coverage for structured plans and keep legacy behavior -> `next-task-structured-references` and all existing next-task semantic checks pass; generated `plan-review.md` carries the detection, coverage, and dangling-reference-as-risk rules, and `output.md` has the optional `## Acceptance coverage` section; scenarios: S5, S6
- [x] AC5: Task execution never checks an AC or records scenario status, and completion fields are unchanged -> `next-task-structured-references` and `next-task-compact-completion-writing` pass; scenarios: S6
- [x] AC6: `/validate` runs every scenario check itself and checks an AC only on passing `Validate:` plus all scenarios `Verified` -> `validate-scenario-evidence`, `validation-observational` (`validation-repair` fixture), and `validate-decision-sync-boundary` pass; scenarios: S7, S8
- [x] AC7: Relationship-integrity defects make validation `failed` -> `validate-scenario-evidence` asserts the defect list in the generated `validation.md`; scenarios: S9
- [x] AC8: Both Validation Report formats carry a scenario-verification section, omitted for legacy plans -> `validation-report-scenario-section` passes and its negative fixture fails with its diagnostic; scenarios: S10
- [x] AC9: Existing plans stay valid and untouched -> `git diff --stat main...HEAD -- context/plans/` lists only this plan; the legacy-plan sentences are asserted by `next-task-structured-references` and `validate-scenario-evidence`; scenarios: S11
- [x] AC10: Generated output stays deterministic and target-neutral, 145 artifact paths, every new check has a failing negative fixture -> `nix run .#pkl-check-generated` and `nix flake check` pass; `artifact-paths` reports exactly 145; scenarios: S2, S12

### Scenario verification

- S1 (AC1): command `nix run .#pkl-check-generated` plus direct `pkl eval config/pkl/renderers/generation-contract-check.pkl` -> exit 0; both `structured-plan-template-schema` and `compact-plan-template-schema` report success; `target-neutral-references` passes; status: Verified
- S2 (AC1, AC10): test `config/pkl/renderers/fixtures/structured-plan-template-schema-check.pkl` registered with `expect_pkl_fixture_failure` in `config/pkl/check-generated.sh` -> `nix run .#pkl-check-generated` exit 0, so the fixture failed with the exact diagnostic; status: Verified
- S3 (AC2): command the S3 `rg` extraction on `config/pkl/base/workflow-change-to-plan.pkl`, plus `plan-template-structure` and the `config/pkl/renderers/fixtures/plan-structure/` fixtures inside `nix run .#pkl-check-generated` -> every definition, including checkbox-prefixed AC and T lines, and every reference field matched on its own line; `plan-template-structure` reports success, and all five pass fixtures and every failure fixture produced their expected results; status: Verified
- S4 (AC3): command `nix run .#pkl-check-generated` -> exit 0; `plan-template-policy-ownership` and `plan-authoring-structured-references` report success, and the negative fixture `plan-authoring-structured-references-check.pkl` failed with its diagnostic; status: Verified
- S5 (AC4): test `next-task-structured-references` plus inspection of `git diff main...HEAD -- config/pkl/base/workflow-next-task.pkl` -> check passes; diff is 48 added and 0 removed lines, so no readiness bullet, approval sentence, or status was removed; generated `output.md` `## Acceptance coverage` is marked omitted for legacy plans; status: Verified
- S6 (AC4, AC5): test `next-task-structured-references` and `next-task-compact-completion-writing` in `nix run .#pkl-check-generated` -> both report success; status: Verified
- S7 (AC6): test `validate-scenario-evidence` -> passes; the generated `validation.md` states the evidence-source exclusions and the AC-verification rule; status: Verified
- S8 (AC6): test `validate-scenario-evidence` -> passes; asserts the three status names and the `Partially verified` rule; status: Verified
- S9 (AC7): test `validate-scenario-evidence` -> passes; asserts the relationship-integrity defect list; status: Verified
- S10 (AC8): test `validation-report-scenario-section` and fixture `validation-report-scenario-section-check.pkl` -> check passes and the fixture failed with its exact diagnostic; status: Verified
- S11 (AC9): inspection of the detection sentences in the generated `sce-next-task/references/plan-review.md` (line 83) and `sce-validate/references/validation.md` (line 66) against `context/plans/remove-checkout-id.md` and `context/plans/agent-trace-export-readers.md`, plus `plan-structure-check.pkl` on both and `git diff --stat main...HEAD -- context/plans/` -> neither plan has `### Verification scenarios` or `Covers:`, both are classified legacy, and neither file changed; status: Verified
- S12 (AC10): command `nix flake check` -> exit 0, all checks passed, `pkl-generated` included, `artifact-paths` exactly 145; status: Verified

### Failed checks and follow-ups

- None.

### Residual risks

- Every new check is a generated-text contract. No check exercises an agent authoring or validating a structured plan end to end (see Open questions).
- The installed `.claude/skills/sce-validate` used for this run predates T04–T05 and has no structured-plan steps. Scenario evidence was still gathered per AC4–AC8's rules and the freshly generated `validation.md`.
- The T05 changes are uncommitted in the working tree, and the new fixture is tracked only through `git add`. `nix flake check` validated the working-tree source.
