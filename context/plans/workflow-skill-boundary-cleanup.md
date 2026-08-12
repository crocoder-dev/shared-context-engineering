# Plan: workflow-skill-boundary-cleanup

## Change summary

Cleans up redundant generated workflow references, makes context-synchronization
lifecycle state durable across sessions, tightens several workflow boundaries
(validation must not repair what it validates, next-task's execution handoff
must be explicit and Git-baseline-aware, handover must not trust
placeholder-only sections), hardens bypass-commit execution, cleans up
decision semantics, removes misleading generic composite boilerplate, and
strengthens `generation-contract-check.pkl` with semantic (not just
inventory/path) checks. `config/pkl/**` remains the sole canonical source;
`.pi/**`, `.claude/**`, and `.opencode/**` are never hand-edited — every
change lands in Pkl and is proven through regeneration.

This extends existing behavior; it does not replace the six-workflow,
single-skill-control-flow, package-local-reference architecture. Direct
inspection before authoring confirmed several premises as real, current gaps
rather than already-fixed ones:

- `config/pkl/base/workflow-validate.pkl:731` currently renders
  `references/output.md` from the validation phase's `validated`/`failed`/
  `blocked` result variants, while the composite command body (`:73-77`)
  already cites a `**Context synchronization blocked**` layout and a
  `**Completion**` layout from `references/output.md` that do not currently
  exist as headings anywhere in that file — a live reference-to-missing-
  heading bug T01 fixes and T12 guards against recurring.
- `config/pkl/base/workflow-next-task.pkl:1882-2023` embeds a full, literal
  `# Context Sync Report` block (all three variants) inside
  `nextTaskOutputReference` (next-task's own `references/output.md`), which
  is a byte-for-byte duplicate of `contextSync.taskOutputReport`, separately
  emitted as `references/sync-report.md` (`:2032`) — the exact duplication
  T02 removes.
- `config/pkl/base/workflow-validate.pkl:316-327` deletes scaffolding
  ("temporary files or intermediate artifacts... local scaffolding...
  marked as temporary") as a real step inside validation itself, and the
  report format (`:471-474`, `:567-570`) carries a **Scaffolding removed**
  field — the repair-during-validation behavior T05 removes.
- `config/pkl/base/workflow-handover.pkl:128` grounds writer mode only in
  `git status` and `git diff` (no `--cached`), and its loader-mode
  validation (`:198-205`) checks only that the four required headings are
  present, with no check for real section content — the gaps T07 closes.
- `config/pkl/base/workflow-change-to-plan.pkl:125-133` models both "answer
  earlier clarification questions" and "answer with changes to an
  already-written plan" through the same undifferentiated prose, with no
  named `original_request`/`clarification_answers` vs. `plan_path`/
  `correction` structure — the gap T04 closes.
- `config/pkl/renderers/opencode-metadata.pkl` derives the Code agent's
  `skill:` permission block from the full workflow catalog unconditionally,
  including the optional `sce-brownfield` workflow, with no per-repository
  installed-selection concept available at generation time. This is
  intentional, existing, ADR-backed behavior: `context/decisions/
  2026-07-31-install-time-optional-workflows.md` states the `optional` flag
  "must never condition generation, composition, routing, permissions, or
  the artifact-path contract" and accepts the dangling `sce-brownfield`
  permission as inert-by-design. T13 is scoped to add a generation-time
  integrity assertion only; it does not change this behavior. (Resolved via
  clarification; see Open questions.)

## Acceptance criteria

- [ ] AC1: The 9 named generated files no longer exist under any of
  `.pi/skills/**`, `.claude/skills/**`, `.opencode/skills/**` in a fresh
  generation, and none of the forbidden replacement files exist either.
  - Validate: `nix run .#pkl-generate -- "$(mktemp -d)"`, then inspect the
    output tree for the absence of `sce-commit/references/commit-contract.yaml`,
    `sce-commit/references/commit-message-style.md`,
    `sce-validate/references/sync-report.md` (three targets each), and the
    absence of `sce-commit/references/commit-contract.md` and
    `sce-validate/references/validation-result.md`.
- [ ] AC2: `sce-next-task` is still the only package generating
  `references/sync-report.md`, and its own `references/output.md` no longer
  contains an embedded Context Sync Report block or its variants.
  - Validate: `grep -c "# Context Sync Report" <target>/sce-next-task/references/output.md` is `0`; `references/sync-report.md` still exists per target.
- [ ] AC3: Context-synchronization lifecycle (task-level and plan-level:
  `pending → synced | blocked`) is persisted in the plan file, survives a
  fresh session, blocks `/next-task` from starting a new implementation task
  while an earlier task's sync debt is unresolved, and blocks `/validate`
  from treating the plan as finishable while task-level sync debt remains.
  - Validate: read the generated `sce-next-task`/`sce-validate` `SKILL.md`
    and reference content for the new lifecycle fields and the gating rule
    text; confirm `references/plan-template.md` documents the new fields.
- [ ] AC4: The generated `sce-change-to-plan` `SKILL.md` models
  initial-clarification continuation (`original_request`,
  `clarification_answers`, `loaded_context_brief`) and existing-plan
  revision (`plan_path`, `correction`, `loaded_context_brief`) as distinct,
  explicitly named continuations, and the original change request is never
  re-requested from the user across a clarification wait.
  - Validate: read generated `sce-change-to-plan/SKILL.md` steps 1/3/4 for
    the named fields.
- [ ] AC5: A failed final validation records the result, emits the failure
  handoff, and stops — it performs no scaffolding deletion and no repair of
  application/test/config code; `Scaffolding removed` no longer appears as a
  successful-validation field.
  - Validate: `grep -i scaffold <target>/sce-validate/references/*.md`
    returns only failure-evidence language (leftover artifacts reported,
    never deleted); no step deletes files.
- [ ] AC6: The generated `sce-next-task` execution phase reference states
  explicit required handoff fields, captures a pre-edit Git baseline,
  computes `files_changed` from that baseline rather than the whole working
  tree, and states deterministic stale-handoff behavior under
  auto-approval.
  - Validate: read the generated execution reference for the baseline-
    capture step and the `files_changed` attribution rule.
- [ ] AC7: The generated `sce-handover` writer mode reads both `git diff`
  and `git diff --cached`; loader mode rejects a handover whose required
  sections are present only as empty/placeholder headings.
  - Validate: read generated `sce-handover/SKILL.md` (and
    `references/handover-template.md` if extracted) for both changes.
- [ ] AC8: The generated `sce-commit` package contains exactly `SKILL.md`
  and `references/{atomic-commit.md,output.md}`; every rule and
  result-contract field previously carried by `commit-contract.yaml` and
  `commit-message-style.md` that the composite workflow actually needs now
  lives in `atomic-commit.md`.
  - Validate: directory listing of generated `sce-commit/references/` per
    target; grep `atomic-commit.md` for commit-message rules and the result
    contract.
- [ ] AC9: Bypass commit (`oneshot`/`skip`) writes the commit message to a
  file (or pipes it via stdin) rather than interpolating it into a shell
  command, runs `git commit` exactly once, and retrieves the resulting
  commit hash explicitly from `HEAD` rather than parsing Git's
  human-readable output; on failure it does not retry, amend, stage more
  files, or fabricate a hash.
  - Validate: read the generated bypass-mode instructions in
    `sce-commit/references/atomic-commit.md` for the message-file/stdin
    mechanism and the explicit `HEAD` hash-retrieval step.
- [ ] AC10: A nonqualifying decision-gate invocation reports
  `not_qualified` or `skipped`, never `blocked`, and synchronization
  continues normally after it; existing ADRs are immutable regardless of
  status; a changed decision always creates a new dated ADR; only an
  equivalent *active* ADR is reused.
  - Validate: read generated `sce-decision/SKILL.md` for the nonqualifying
    result vocabulary and the reuse-only-active-ADR rule.
- [ ] AC11: The shared composite preamble (and generated command
  boilerplate) no longer claims every workflow supports clarification,
  validation repair, or bootstrap waits; it instead states that any
  workflow-defined user wait resumes the same skill in the same session,
  and workflow-specific wait semantics stay in the workflows that own them.
  - Validate: grep every generated workflow `SKILL.md` for the old
    overclaiming boilerplate (absent) and the new generic wording (present).
- [ ] AC12: `generation-contract-check.pkl` enforces the 9 semantic checks
  from the change request (heading-vs-reference match, package-local path
  existence, validate/commit forbidden-file checks, atomic-commit.md dual
  content, next-task non-duplication, Pi/Claude/OpenCode reference parity,
  absence of the stale "Nothing records the skipped synchronization" line,
  no-repair-during-validation), each proven by a negative fixture.
  - Validate: `nix run .#pkl-check-generated` passes; each new negative
    fixture demonstrably fails with its intended diagnostic when evaluated.
- [ ] AC13: The generated OpenCode Code-agent `skill:` permission block
  stays catalog-derived and unconditional (including the inert
  `sce-brownfield` permission), and a new generation-time assertion
  confirms every explicitly allowed `sce-*` permission names a workflow
  artifact the generator is capable of emitting for that target.
  Installation status is not consulted or required.
  - Validate: read `config/pkl/renderers/opencode-metadata.pkl` (unchanged
    permission derivation) and the new assertion in
    `generation-contract-check.pkl`; confirm a deliberately-misspelled
    permission fixture fails the assertion.

### Full validation

- `nix run .#pkl-check-generated`
- `nix flake check`

### Context sync

- `context/architecture.md` — package-relative reference inventories for
  `sce-validate`, `sce-next-task`, `sce-commit` (file counts and names
  change); the `renderSkill` preamble description (T11); the
  generation-contract-check description (T12, T13); the plan template
  description (T03).
- `context/patterns.md` — the Pkl renderer layering / phase-reference
  bullets describing `sce-validate`'s and `sce-commit`'s package-local
  reference inventories, and the generic composite preamble bullet (T11).
- `context/sce/shared-context-code-workflow.md` — the `sce-validate` and
  `sce-next-task` package file listings (T01, T02), the durable
  sync-lifecycle description (T03), and the validation/decision semantics
  (T05, T10).
- `context/sce/shared-context-plan-workflow.md` — the
  clarification/revision continuation contract (T04).
- `context/sce/atomic-commit-workflow.md` — the `sce-commit` package file
  listing and bypass-mode determinism (T08, T09).
- `context/sce/handover-workflow.md` — the writer/loader hardening (T07).
- `context/glossary.md` — glossary entries naming the removed files
  (`commit-contract.yaml`, `commit-message-style.md`,
  `sce-validate/references/sync-report.md`) or describing the old
  plan-task field set.
- A new dated ADR under `context/decisions/` if the task/plan
  synchronization gate judges this cross-cutting change qualifying (it
  changes validation, commit execution, and decision semantics repository-
  wide). This plan does **not** author or supersede
  `context/decisions/2026-07-31-install-time-optional-workflows.md`; T13
  is fully compatible with it.

## Constraints and non-goals

- **In scope:** `config/pkl/base/workflow-{validate,next-task,context-sync,
  change-to-plan,handover,commit}.pkl`, `config/pkl/base/decision-skill.pkl`,
  `config/pkl/base/workflow-catalog.pkl` (read-only reference),
  `config/pkl/renderers/{workflow-composite,generation-contract-check,
  opencode-metadata}.pkl`, the plan template these workflows share, and
  ephemeral regeneration verification.
- **Out of scope:** the Rust `sce` CLI (`cli/**`) — no task in this plan
  changes CLI code; non-workflow Pkl (`sce-config-schema.pkl`,
  `opencode.pkl` plugin registration); which workflows are core vs.
  optional; the install-time optional-workflow selection architecture; any
  change to the six-workflow / single-skill-control-flow /
  package-local-reference model beyond what each task explicitly
  authorizes.
- **Constraints:** `config/pkl/**` is the sole source of truth; never
  hand-edit `.pi/**`, `.claude/**`, or `.opencode/**`; preserve the
  existing package/composite structured-rendering model
  (`workflow-content.pkl`); preserve the sole allowed SCE sibling
  invocation (`sce-decision` from the synchronization gate); keep
  Pi/Claude/OpenCode target-neutral except where a target's runtime
  genuinely requires a difference; do not manually patch generated output
  to make checks pass — fix canonical sources; do not touch or supersede
  `context/decisions/2026-07-31-install-time-optional-workflows.md`.
- **Non-goal:** this plan does not redesign workflow architecture beyond
  the 13 described boundary/redundancy fixes; it does not add or remove
  any catalog workflow; it does not make any optional workflow's
  generation, permissions, or artifact paths conditional on install-time
  selection; it does not change the CLI's actual Git/database behavior —
  only the generated agent instructions describing how an executing agent
  should use Git/state.

## Assumptions

- "Persist" in T03 means: write the lifecycle state into the plan file's
  Markdown (new task-level and plan-level fields), the same durable medium
  the plan already uses for `(status:todo|done)` and completion evidence —
  not a new database or file format, consistent with the `disposable plan
  lifecycle` policy (`context/glossary.md`).
- T04's "explicit typed structures" are named prose fields inside the
  generated agent instructions (state an executing LLM skill is told to
  track across a same-session wait), not a Pkl `class`/`typealias` runtime
  type — these files author Markdown instructions for an executing agent,
  not compiled state, so there is no runtime consumer for a literal type.
- T06's "review of the mandatory five-root-file pass" is resolved as: keep
  it mandatory for every task (current behavior), and state that choice
  plainly in the execution/context-sync reference text, since the request
  explicitly forbids silently removing it and the pass is cheap,
  deterministic, and already load-bearing for context accuracy.
- T10's "review whether Deprecated and Superseded should be creation-time
  statuses" is resolved during T10's implementation by inspecting
  `decision-skill.pkl`'s current status vocabulary; if collapsing them
  would lose information a downstream reader needs, both statuses are kept
  and made creation-time-only (never mutated after creation) rather than
  merged.
- T13 is scoped exactly per clarification: catalog-derived, unconditional
  OpenCode permissions are preserved as-is; only a generation-time
  artifact-integrity assertion is added. No CLI code changes, no ADR
  supersession, no installed-selection concept at generation time.
- Each of T01–T13 lands as one atomic commit, per the change request's
  stated commit split; the task stack mirrors that split one-to-one.

## Task stack

- [x] T01: `Consolidate sce-validate references (validation.md, context-sync.md, no sync-report.md/validation-result.md)` (status:done)
  - Task ID: T01
  - Goal: Move the validation phase result contract into `references/validation.md` and the plan context-sync result/report contract into `references/context-sync.md`, stop generating `references/sync-report.md` for `sce-validate`, and reduce `references/output.md` to only composite user-visible layouts — including the currently-missing `Context synchronization blocked` and `Completion` headings the command body already cites.
  - Boundaries (in/out of scope): In — `config/pkl/base/workflow-validate.pkl` (`references/output.md` restructuring, the validation result contract, the `referenceDocuments`/`outputDocuments` listings), reading from `workflow-context-sync.pkl`'s existing plan-role report content rather than duplicating it. Out — behavior changes to what validation actually checks (T05), next-task's package (T02).
  - Dependencies: none
  - Done when: generated `sce-validate/references/` contains exactly `validation.md`, `context-sync.md`, `validation-report.md`, `output.md` (no `sync-report.md`, no `validation-result.md`) for Pi, Claude, and OpenCode; `output.md` contains `## Context synchronization blocked` and `## Completion` headings and no validation-phase `validated`/`failed`/`blocked` result variants; every `Render the **X** layout from references/output.md` instruction in generated `sce-validate/SKILL.md` has a matching heading in `output.md`.
  - Verification notes (commands or checks): `nix run .#pkl-generate -- "$(mktemp -d)"`; inspect `sce-validate/references/`; `nix run .#pkl-check-generated`.
  - Implementation evidence: In `config/pkl/base/workflow-validate.pkl` — `references/validation.md` is now `renderValidationSkillBody` concatenated with the `VALIDATION_RESULT` contract (self-cited via new `validationResultSelfRef`; `validation-report.md`'s cross-file citation uses new `validationResultFileRef` = `` `references/validation.md` ``); `references/context-sync.md` is `contextSync.planSkillBody` (package mode) with its `` `references/sync-report.md` `` marker redirected in place to a self-reference, concatenated with `contextSync.planOutputReport`; `references/sync-report.md` is no longer in `referenceDocuments`; `references/output.md` is now the new `VALIDATE_OUTPUT_LAYOUTS` constant carrying only `## Context synchronization blocked` and `## Completion`, matching the two `Render the **X** layout from references/output.md` citations already in the composite `SKILL.md` body (`workflow-content.pkl`'s `validateSkillBody`, unchanged). Also fixed two stale canonical-inventory literals this change made incorrect: `config/pkl/renderers/metadata-coverage-check.pkl`'s `validate` entry in `phaseReferencePathsByWorkflow` (dropped `references/sync-report.md`), and `config/pkl/renderers/generation-contract-check.pkl`'s `expectedArtifactPathCount` (113 -> 110, for the 3 removed per-target files; matches the plan's own T13 note of a net -9 from T01/T08).
  - Verification performed: `nix run .#pkl-generate -- "$(mktemp -d)"` then inspected `config/.claude/skills/sce-validate/references/` for all three targets — contains exactly `validation.md`, `context-sync.md`, `validation-report.md`, `output.md`; `output.md` has exactly `## Context synchronization blocked` and `## Completion` headings and no validation result variants; `grep -rn sync-report` under generated `sce-validate/**` returns nothing; both `Render the **X** layout from references/output.md` citations in generated `SKILL.md` match `output.md` headings. `nix run .#pkl-check-generated` passes (110 files). `nix flake check` passes.

- [x] T02: `Deduplicate next-task sync output` (status:done)
  - Task ID: T02
  - Goal: Remove the literal duplicated `# Context Sync Report` block (all three variants) from `nextTaskOutputReference` in `config/pkl/base/workflow-next-task.pkl`, keeping only workflow gates and terminal layouts in `references/output.md`; keep `references/sync-report.md` as the sole owner of the task context-sync report contract.
  - Boundaries (in/out of scope): In — `workflow-next-task.pkl`'s `nextTaskOutputReference` string and its reference listings. Out — the context-sync phase's actual behavior (`workflow-context-sync.pkl`, T03 territory), validation's package (T01).
  - Dependencies: none
  - Done when: generated `sce-next-task/references/output.md` contains no `# Context Sync Report` heading or report-variant content; generated `sce-next-task/references/sync-report.md` still exists and is unchanged in meaning; the `Context synchronization blocked` workflow gate in `output.md` still exists, distinct from the report itself.
  - Verification notes (commands or checks): `nix run .#pkl-generate -- "$(mktemp -d)"`; `grep -c "# Context Sync Report" <target>/sce-next-task/references/output.md` is `0`; `nix run .#pkl-check-generated`.
  - Implementation evidence: In `config/pkl/base/workflow-next-task.pkl` — removed the entire embedded `# Context Sync Report` section (all three `synced`/`no_context_change`/`blocked` report variants plus the shared **Report rules**) from the `nextTaskOutputReference` string, which previously duplicated `contextSync.taskOutputReport` byte-for-byte. `nextTaskOutputReference` now ends after the **Implementation gate**'s `## Rules` block; the workflow gate layout `## Context synchronization blocked` (distinct, decision-relevant workflow prose) is untouched. The `references/sync-report.md` document binding (`model.makeDocument.apply("references/sync-report.md", contextSync.taskOutputReport)`) was not touched and remains the sole owner of the report contract.
  - Verification performed: `nix run .#pkl-generate -- "$(mktemp -d)"` then, for all three targets (`.pi`, `.claude`, `.opencode`) under `sce-next-task/references/`: `grep -c "# Context Sync Report" output.md` is `0`; `sync-report.md` still exists; `grep -c "^## Context synchronization blocked" output.md` is `1`. `nix run .#pkl-check-generated` passes (110 files, unchanged count from T01's post-fix baseline).

- [x] T03: `Persist context synchronization lifecycle` (status:done)
  - Task ID: T03
  - Goal: Add durable `pending → synced | blocked` lifecycle tracking for both task-level and plan-level context synchronization to the plan template/model, and update `sce-next-task` and `sce-validate` to write `pending` before invoking sync, write `synced`/`blocked` (with blocker/required-action/retry-condition) after, and refuse to proceed (new task, or plan-finish) while unresolved sync debt exists.
  - Boundaries (in/out of scope): In — `references/plan-template.md` (canonical source in `workflow-change-to-plan.pkl`), `workflow-next-task.pkl` (plan-review and task-context-sync steps), `workflow-validate.pkl` (finishability gating), `workflow-context-sync.pkl` only insofar as it reports the same status values it already returns — its own scope stays synchronizing `context/`. Out — inventing a non-plan-file persistence mechanism.
  - Dependencies: T01, T02
  - Done when: the plan template documents new lifecycle field(s) with `pending`/`synced`/`blocked` states plus blocker/required-action/retry-condition for the blocked case; generated `sce-next-task/SKILL.md` states it will not start a new implementation task while an earlier completed task's sync lifecycle is not `synced`; generated `sce-validate/SKILL.md` states it will not treat the plan as finishable while any task's lifecycle is not `synced`; the state is written to the plan file (not only asserted in chat) at each transition; generated text no longer says anything equivalent to "nothing records the skipped synchronization, so it is lost once this session ends."
  - Verification notes (commands or checks): read generated `sce-next-task/SKILL.md`, `sce-validate/SKILL.md`, and the generated `plan-template.md` reference for the new field/gating language; `nix run .#pkl-check-generated`.
  - Context synchronization: synced
  - Completed: 2026-08-12
  - Files changed: `config/pkl/base/workflow-change-to-plan.pkl`, `config/pkl/base/workflow-content.pkl`, `config/pkl/base/workflow-context-sync.pkl`, `config/pkl/base/workflow-next-task.pkl`, `config/pkl/base/workflow-validate.pkl`
  - Evidence: `nix run .#pkl-check-generated` passed; ephemeral generation produced 110 files; generated next-task, validate, context-sync, and plan-template references contain the lifecycle fields, transition rules, and unresolved-debt gates.
  - Notes: Existing T01/T02 records predate the lifecycle fields; missing lifecycle records are intentionally treated as unresolved synchronization debt rather than inferred as synced.

- [x] T04: `Fix change-to-plan clarification continuation` (status:done)
  - Task ID: T04
  - Goal: In `config/pkl/base/workflow-change-to-plan.pkl`, name the initial-clarification continuation explicitly (`original_request`, `clarification_answers`, `loaded_context_brief`) distinct from existing-plan revision (`plan_path`, `correction`, `loaded_context_brief`), so the generated skill never needs to re-request the original change request from the user.
  - Boundaries (in/out of scope): In — step 3/4 prose and any shared continuation description in `workflow-change-to-plan.pkl`. Out — the plan-authoring phase's actual authoring logic, the plan template (T03 territory), the clarification-gate question format.
  - Dependencies: T03
  - Done when: generated `sce-change-to-plan/SKILL.md` names both continuation shapes explicitly and distinctly; it states plainly that the original request is preserved and never re-asked for across a clarification wait.
  - Verification notes (commands or checks): read generated `sce-change-to-plan/SKILL.md` steps 3 and 4; `nix run .#pkl-check-generated`.
  - Context synchronization: synced
  - Completed: 2026-08-12
  - Files changed: `config/pkl/base/workflow-change-to-plan.pkl`
  - Evidence: Ephemeral generation produced the updated `sce-change-to-plan/SKILL.md` for Pi, Claude, and OpenCode; the generated steps 3 and 4 contain both named continuation shapes and the no-reask rule. `nix run .#pkl-check-generated` passed (110 files).
  - Notes: The continuation shapes are explicit Markdown fields in the generated instructions, not runtime Pkl types, as assumed by the plan.

- [x] T05: `Make validation observational` (status:done)
  - Task ID: T05
  - Goal: Remove the scaffolding-deletion step and the `Scaffolding removed` report field from `workflow-validate.pkl`'s successful-path content; require leftover debug flags/temp artifacts to be recorded as failure evidence instead; confirm the existing "validation never repairs application/test/config code" language stays intact and unambiguous.
  - Boundaries (in/out of scope): In — the validation phase reference content placed in `references/validation.md` by T01, the plan-file Validation Report layout, the validation result contract. Out — restructuring the reference tree itself (already done by T01); this task edits content, not file layout.
  - Dependencies: T01
  - Done when: no generated `sce-validate` document instructs deleting/removing scaffolding, debug flags, or temporary artifacts; no successful-validation report field is titled `Scaffolding removed`; a failing check for leftover debug/temp artifacts is recorded as validation failure evidence, not repaired.
  - Verification notes (commands or checks): `grep -i scaffold <target>/sce-validate/references/*.md` for the new wording; `nix run .#pkl-check-generated`.
  - Context synchronization: synced
  - Completed: 2026-08-12
  - Files changed: `config/pkl/base/workflow-validate.pkl`, `config/pkl/base/workflow-context-sync.pkl`
  - Evidence: Removed validation's temporary-scaffolding deletion step and successful-path `Scaffolding removed` fields; leftover debug flags, temporary artifacts, and local scaffolding are recorded under failed checks without deletion or repair; removed the stale scaffolding field from the validation handoff consumed by plan context synchronization. Existing validation boundaries continue to prohibit modifying tests, application code, or configuration to make checks pass.
  - Verification performed: `nix run .#pkl-check-generated` passed; ephemeral generation produced 110 files for all targets, with four `sce-validate/references/` files per target and no scaffolding-removal instructions or successful-validation cleanup field. Generated validation references explicitly classify leftover debug/temp artifacts as failure evidence.

- [ ] T06: `Harden next-task execution handoff` (status:todo)
  - Task ID: T06
  - Goal: In `workflow-next-task.pkl`'s execution phase, make the required handoff fields explicit (resolved plan, task identity, changed files, implementation summary, verification evidence, done-check evidence, plan update, context impact), add a pre-edit Git baseline capture step, compute `files_changed` relative to that baseline (excluding unrelated pre-existing staged/unstaged changes), state deterministic behavior for a stale/invalid/contradictory handoff (including under auto-approval), and explicitly state whether the mandatory five-root-file context pass stays mandatory for every task.
  - Boundaries (in/out of scope): In — the execution phase reference and its handoff contract, the task-context-sync phase's consumption of `files_changed`. Out — creating a new standalone reference file for the contract (keep it with the execution phase, per the change request); the plan template (already extended by T03).
  - Dependencies: T02, T03
  - Done when: the generated execution reference states the pre-edit baseline step, computes `files_changed` from that baseline, states the auto-approval stale-handoff rule, and explicitly states the five-root-file pass's mandatory status with a one-sentence rationale; no new standalone `references/execution-*.md` file is introduced.
  - Verification notes (commands or checks): read the generated execution reference for the new steps; `nix run .#pkl-check-generated`.

- [ ] T07: `Harden handover` (status:todo)
  - Task ID: T07
  - Goal: In `workflow-handover.pkl`, read both `git diff` and `git diff --cached` in writer mode; validate loaded handover sections for real content (not just heading presence) in loader mode and reject placeholder-only handovers; if it reduces control-plane size, extract the persisted handover template into `references/handover-template.md`, keeping `SKILL.md` focused on workflow/control flow, with writer success staying concise (path + continuation instruction) and loader success still exposing full loaded content.
  - Boundaries (in/out of scope): In — `workflow-handover.pkl`'s writer/loader steps and persisted-format body. Out — any other workflow package.
  - Dependencies: T06
  - Done when: generated `sce-handover/SKILL.md` writer-mode step reads both `git diff` and `git diff --cached`; loader-mode step rejects a section present only as an empty/placeholder heading; if extracted, `references/handover-template.md` exists per target and `SKILL.md` reads it before composing; writer success output stays concise; loader success output still surfaces full loaded content.
  - Verification notes (commands or checks): read generated `sce-handover/SKILL.md` (and `references/handover-template.md` if extracted); `nix run .#pkl-check-generated`.

- [ ] T08: `Consolidate commit references` (status:todo)
  - Task ID: T08
  - Goal: In `workflow-commit.pkl`, stop generating `references/commit-contract.yaml` and `references/commit-message-style.md` (and do not introduce `references/commit-contract.md`); merge their procedure, subject/body/issue-reference/plan-citation rules, anti-patterns, result variants, and required/optional result fields into `references/atomic-commit.md`; simplify the internal result contract, dropping fields such as `scope_classification`, `notes`, and `cites_plan` unless the composite commit workflow actually consumes them.
  - Boundaries (in/out of scope): In — `workflow-commit.pkl`'s reference listing and `atomic-commit.md` content. Out — bypass-mode execution mechanics (T09).
  - Dependencies: T07
  - Done when: generated `sce-commit/references/` contains exactly `atomic-commit.md` and `output.md` per target; `atomic-commit.md` contains both commit-message rules and the atomic-commit result contract; no generated file is named `commit-contract.yaml`, `commit-message-style.md`, or `commit-contract.md`.
  - Verification notes (commands or checks): directory listing of generated `sce-commit/references/`; grep `atomic-commit.md` for message-style and result-contract content; `nix run .#pkl-check-generated`.

- [ ] T09: `Make bypass commit execution deterministic` (status:todo)
  - Task ID: T09
  - Goal: In the bypass-mode instructions now living in `atomic-commit.md`, replace multiline-message shell interpolation with a message-file/stdin mechanism (`git commit -F <file>` or equivalent), run `git commit` exactly once, retrieve the resulting hash explicitly from `HEAD` after success (never parsed from human-readable Git output), and state that failure never retries, amends, stages more files, or fabricates a hash; confirm `oneshot` and `skip` stay behaviorally identical.
  - Boundaries (in/out of scope): In — `atomic-commit.md`'s bypass-mode section. Out — regular (proposal-only) commit mode.
  - Dependencies: T08
  - Done when: generated `atomic-commit.md` bypass instructions use a message-file/stdin mechanism, state exactly one `git commit` invocation, state explicit post-success `HEAD` hash retrieval, and state the no-retry/no-amend/no-stage-more/no-fabricated-hash failure rule; `oneshot` and `skip` remain described identically apart from the trigger token.
  - Verification notes (commands or checks): read generated `atomic-commit.md` bypass section; `nix run .#pkl-check-generated`.

- [ ] T10: `Clean up decision semantics` (status:todo)
  - Task ID: T10
  - Goal: In `decision-skill.pkl`, make a nonqualifying invocation return `not_qualified`/`skipped` (never `blocked`) with synchronization continuing normally afterward; state existing ADRs stay immutable regardless of status; state a changed decision always creates a new dated ADR; state only an equivalent *active* ADR may be reused (never a rejected/deprecated one); review whether `Deprecated`/`Superseded` should be creation-time-only statuses and simplify without losing needed information.
  - Boundaries (in/out of scope): In — `decision-skill.pkl` and any status-vocabulary reference it owns. Out — the synchronization phases that invoke it (`workflow-context-sync.pkl`) beyond confirming they already treat a nonqualifying/skip result as non-blocking.
  - Dependencies: T09
  - Done when: generated `sce-decision/SKILL.md` states the nonqualifying result is `not_qualified`/`skipped` and non-blocking; states ADR immutability and active-only reuse explicitly; the status vocabulary is coherent (either both `Deprecated`/`Superseded` kept as creation-time-only, or intentionally simplified, stated plainly either way).
  - Verification notes (commands or checks): read generated `sce-decision/SKILL.md`; confirm `sce-next-task`/`sce-validate` context-sync steps that consume the decision-gate result do not describe a nonqualifying result as blocking; `nix run .#pkl-check-generated`.

- [ ] T11: `Remove misleading generic composite boilerplate` (status:todo)
  - Task ID: T11
  - Goal: In `config/pkl/renderers/workflow-composite.pkl` (the shared `renderSkill` preamble) and any generated command boilerplate that copies its overclaiming, replace wording implying every workflow supports clarification, validation repair, and bootstrap waits with wording equivalent to "Any workflow-defined user wait resumes this same skill in the same session," while workflow-specific wait/resume semantics stay stated in the workflow that actually owns them.
  - Boundaries (in/out of scope): In — the shared preamble text in `workflow-composite.pkl` and matching command-boilerplate text. Out — rewriting each workflow's own wait semantics; this task only removes the generic overclaim.
  - Dependencies: T10
  - Done when: no generated workflow `SKILL.md` preamble claims universal support for clarification, validation repair, or bootstrap waits; every generated `SKILL.md` preamble instead carries the generic "any workflow-defined user wait resumes this same skill in the same session" wording (or an equivalent); each workflow's own wait semantics remain stated where they already were.
  - Verification notes (commands or checks): grep every generated workflow `SKILL.md` for the old and new wording; `nix run .#pkl-check-generated`.

- [ ] T12: `Add semantic generation checks` (status:todo)
  - Task ID: T12
  - Goal: Extend `config/pkl/renderers/generation-contract-check.pkl` with the 9 semantic checks from the change request: (1) every `Render the **X** layout from references/foo.md` instruction has a matching `## X` heading in that file; (2) every package-local referenced path exists; (3) validate never generates `references/sync-report.md` or `references/validation-result.md`; (4) commit never generates `references/commit-contract.yaml`, `references/commit-contract.md`, or `references/commit-message-style.md`; (5) `atomic-commit.md` contains both commit-message rules and the atomic-commit result contract; (6) if next-task emits `sync-report.md`, its `output.md` does not also contain the context-sync report contract; (7) Pi/Claude/OpenCode target-neutral references are identical unless explicitly target-specific; (8) no generated file contains "Nothing records the skipped synchronization, so it is lost once this session ends" (stale after T03); (9) validation contains no instruction to repair implementation during final validation. Add a checked-in negative fixture proving each check fails when violated.
  - Boundaries (in/out of scope): In — `generation-contract-check.pkl` and its negative-fixture set. Out — re-verifying inventory/path-presence checks that already exist; this task adds semantic checks alongside them.
  - Dependencies: T11
  - Done when: `generation-contract-check.pkl` asserts all 9 checks; a corresponding negative fixture exists per check and demonstrably fails evaluation with an actionable diagnostic; `nix run .#pkl-check-generated` passes against the now-compliant generated output.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; evaluate each negative fixture directly (`nix develop -c pkl eval <fixture-path>`) and confirm it fails.

- [ ] T13: `Assert generated OpenCode permission integrity` (status:todo)
  - Task ID: T13
  - Goal: Preserve the current catalog-derived, unconditional OpenCode Code-agent `skill:` permission block exactly as-is, including the inert `sce-brownfield` permission for the optional workflow. Add a generation-time assertion in `generation-contract-check.pkl` that every explicitly allowed `sce-*` permission names a workflow artifact the generator is capable of emitting for that target. Do not consult or require installation status, and do not condition generation, composition, routing, permissions, or artifact paths on `optional`.
  - Boundaries (in/out of scope): In — a new integrity assertion in `generation-contract-check.pkl` covering `opencode-metadata.pkl`'s permission output. Out — any change to `opencode-metadata.pkl`'s permission-derivation logic itself, `workflow-catalog.pkl`'s `optional` flag, install-time selection logic, or `cli/**`. Also updates the literal `expectedArtifactPathCount` in `generation-contract-check.pkl` if this repository's earlier tasks changed the total generated artifact count (net -9 from T01/T08).
  - Dependencies: T12
  - Done when: `opencode-metadata.pkl`'s permission derivation is unchanged (still catalog-derived, unconditional, `sce-brownfield` still present); `generation-contract-check.pkl` asserts every explicitly allowed `sce-*` permission corresponds to an artifact the generator emits for that target; a deliberately-misspelled or dangling-to-a-nonexistent-workflow permission fixture fails that assertion with an actionable diagnostic; no assertion consults an installed/selected-workflow set.
  - Verification notes (commands or checks): read `opencode-metadata.pkl` (confirm unchanged) and the new assertion in `generation-contract-check.pkl`; `nix run .#pkl-check-generated`; `nix flake check`.

## Open questions

None. T13's only real ambiguity — whether "workflows actually installed" could be checked at generation time without reversing the accepted install-time-optionality architecture — was resolved via clarification: permissions stay catalog-derived and unconditional, and T13 narrows to a generation-time integrity assertion. The remaining tasks match the change request's own precise specification (exact target file trees, exact forbidden/required generated paths, exact wording equivalents, an exact 13-task commit split); direct inspection of `workflow-validate.pkl`, `workflow-next-task.pkl`, `workflow-handover.pkl`, `workflow-change-to-plan.pkl`, and `opencode-metadata.pkl` confirmed the premises behind T01, T02, T04, T05, T07, and T13 are real, current conditions rather than already-fixed or misdiagnosed ones. Each remaining task's own `Done when` names the concrete generated artifact it must produce, so a wrong premise in T03, T06, T08–T12 fails visibly at verification time rather than silently.
