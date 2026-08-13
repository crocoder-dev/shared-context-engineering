# Plan: remove-validate-context-sync

## Change summary

Canonicalize the currently staged `.pi/` workflow-reference changes in Pkl and
make the generator emit the same behavior for all three ephemeral target
payloads under `config/.pi/`, `config/.claude/`, and `config/.opencode/`. The
repository-root integration directories remain installation destinations owned
by `sce setup`; Pkl generation must not write them directly.

The main workflow change is to remove plan-level context synchronization from
`/validate`: `sce-validate` will no longer generate or read a context-sync
reference, invoke `sce-plan-context-sync`, gate validation completion on sync
lifecycle fields, or claim that durable context was synchronized. It will stop
after recording and reporting validation. Task-level synchronization owned by
`/next-task` remains unchanged.

The staged commit-reference changes are also canonicalized: `/commit` will use a
separate `references/commit-message-style.md`, while `references/atomic-commit.md`
will no longer carry the removed YAML result-contract section. Generation and
coverage checks will be updated so the resulting target payloads stay
consistent and deterministic.

## Acceptance criteria

- [x] AC1: A fresh Pkl generation writes only ephemeral payloads below the
  `config/` generation root, including `.pi`, `.claude`, and `.opencode`, and
  does not create or modify repository-root integration directories; `sce setup`
  remains the documented installer of those root files.
  - Validate: `nix run .#pkl-generate -- "$(mktemp -d)"`; inspect the generated
    `config/.{pi,claude,opencode}` paths and confirm repository-root target
    directories are not generation destinations.
- [x] AC2: Generated `sce-validate` contains no plan-context-sync invocation,
  no `sce-plan-context-sync` handoff, and no package-local `context-sync.md`;
  validation no longer requires completed-task sync fields or writes a
  plan-level sync state before returning `validated`, and its completion output
  does not claim durable context synchronization.
  - Validate: inspect generated `sce-validate/SKILL.md` and its references for
    the removed phase, call, lifecycle gate, and synchronization claim; assert
    the generated `sce-validate/references/` inventory contains only the
    validation and output/report documents defined by the canonical source.
- [x] AC3: Generated `/commit` packages contain
  `references/commit-message-style.md`, and `references/atomic-commit.md`
  delegates message-style rules to that file without the removed
  `references/commit-contract.yaml` return section; all three target packages
  preserve the staged reference content semantically.
  - Validate: generate all targets and inspect each `sce-commit` reference
    inventory and the atomic-commit/style documents; confirm the forbidden YAML
    contract-return section is absent.
- [x] AC4: Generation metadata, package-relative reference checks, and exact
  artifact/path assertions describe the new validate and commit inventories;
  target-neutral workflow documents remain equivalent apart from supported
  frontmatter, and setup can consume the generated payload without requiring
  repository-root generated trees.
  - Validate: `nix develop -c pkl eval
    config/pkl/renderers/metadata-coverage-check.pkl`; `nix run
    .#pkl-check-generated`; inspect the generated-input handoff paths.
- [x] AC5: Durable workflow documentation accurately states that `/next-task`
  still owns task context synchronization while `/validate` performs validation
  only and does not synchronize plan context; the commit reference ownership
  description matches the new split.
  - Validate: inspect the affected `context/sce/` and root context files and
    confirm no current-state document says `/validate` invokes plan context
    synchronization or that the removed commit contract remains generated.
- [x] AC6: The `/validate` catalog description, the plan template's
  context-synchronization lifecycle section, `sce-decision`'s accepted
  callers, and the generation contract's decision-invoking workflow list are
  all validation-only-consistent with the accepted
  `2026-08-13-validate-validation-only` ADR: the catalog description states
  `/validate` records final validation evidence rather than synchronizing
  durable context; the plan template documents only a task-level context
  synchronization lifecycle, with no `Plan context synchronization` field or
  `/validate`-sets-synced-or-blocked claim; `sce-decision` accepts requests
  only from `sce-next-task` task context synchronization; and the generation
  contract treats only `sce-next-task` as decision-invoking, asserting
  generated `sce-validate` documents contain neither a `sce-decision`
  reference nor plan-context-sync wording.
  - Validate: inspect `workflow-catalog.pkl`, `workflow-change-to-plan.pkl`,
    `decision-skill.pkl`, and `generation-contract-check.pkl`; generate all
    three targets and inspect the emitted `/validate` description, plan
    template, and `sce-decision` package; run `nix develop -c pkl eval
    config/pkl/renderers/generation-contract-check.pkl`, `nix run
    .#pkl-check-generated`, and `nix flake check`.

### Full validation

- `nix run .#pkl-check-generated`
- `nix flake check`

### Context sync

- `context/sce/shared-context-code-workflow.md`
- `context/sce/context-workflow-rules.md`
- `context/sce/atomic-commit-workflow.md`
- `context/overview.md`
- `context/architecture.md`
- `context/patterns.md`
- `context/glossary.md`
- `context/context-map.md` when workflow ownership or a decision link changes
- A new or superseding ADR if synchronization determines that removing the
  plan-context-sync lifecycle or changing the commit-reference contract is a
  qualifying system-wide compatibility/ownership decision.

## Constraints and non-goals

- **In scope:** Canonical Pkl workflow sources for validate and commit;
  workflow-composite integration where required; metadata and generation
  contract checks; ephemeral generated payload verification; and durable
  workflow documentation.
- **Out of scope:** Direct generation into `.pi/`, `.claude/`, or `.opencode/`
  at repository root; changes to `sce setup` installation mechanics; Rust CLI
  behavior; `/next-task` task-context synchronization behavior; and deletion of
  the shared task-context-sync implementation.
- **Constraints:** `config/pkl/**` is the canonical source; root integration
  files are installed from generated assets by `sce setup`; generated payloads
  are inspected in temporary/config-derived locations; preserve validation's
  full acceptance-criteria and repository-check behavior; preserve commit
  mode routing and staged-diff semantics; use Nix-owned generation and checks.
- **Non-goal:** Replacing plan synchronization with another automatic context
  synchronization mechanism. This change intentionally leaves `/validate`
  without a plan-context-sync phase; any future replacement requires a separate
  contract.

## Assumptions

- The user's clarification means `/validate` ends after the validation phase
  writes its Validation Report and returns `validated`, `failed`, or `blocked`;
  it no longer produces a downstream context-impact handoff or completion claim
  that context is synchronized.
- The staged `.pi/` commit-reference edits are intentional desired behavior and
  must be represented in canonical Pkl rather than copied into generated target
  trees by hand.
- Task-level context synchronization remains required for successful
  `/next-task` execution and is not removed by this plan.
- The existing `sce setup` asset pipeline consumes the ephemeral generated
  payload and installs the corresponding root target files, so no new installer
  behavior is needed.

## Task stack

- [x] T01: `Remove plan context synchronization from the validate workflow` (status:complete)
  - Task ID: T01
  - Goal: Remove the plan-context-sync phase, invocation, package-local reference,
    lifecycle gate, context-impact handoff, and synchronization completion claim
    from the canonical `/validate` workflow while retaining validation and
    Validation Report behavior.
  - Boundaries (in/out of scope): In — `workflow-validate.pkl`, its composed
    package/reference inventory, validation result and completion layouts, and
    any directly required shared-model wiring. Out — task context synchronization
    in `workflow-context-sync.pkl` and `/next-task` behavior.
  - Dependencies: none
  - Done when: generated validate skills contain no context-sync call or
    `sce-plan-context-sync` reference; validation no longer blocks on task/plan
    sync lifecycle state; validated completion reports final validation and the
    report path without claiming durable context synchronization; all validation
    statuses remain defined.
  - Verification notes (commands or checks): direct Pkl evaluation of
    `workflow-validate.pkl`; temporary generation; focused searches over all
    three generated `sce-validate` packages; package-relative inventory review.
  - Implementation evidence: removed plan-level context synchronization from
    `workflow-validate.pkl` and the composed validate workflow in
    `workflow-content.pkl`; retained validation, report writing, and all three
    terminal statuses.
  - Verification evidence: `nix develop -c pkl eval
    config/pkl/base/workflow-validate.pkl` passed; temporary generation emitted
    only `validation.md`, `validation-report.md`, and `output.md` under each
    target's `sce-validate/references/`; focused forbidden-content searches
    passed across `.pi`, `.claude`, and `.opencode`.
  - Context synchronization: synced (manually confirmed; durable context
    realigned by T04)

- [x] T02: `Canonicalize staged commit reference ownership in Pkl` (status:complete)
  - Task ID: T02
  - Goal: Make the canonical commit package emit the staged
    `commit-message-style.md` reference and remove the obsolete YAML result
    contract section from `atomic-commit.md`, while preserving commit mode
    routing, staged truth, result branching, and human-visible output layouts.
  - Boundaries (in/out of scope): In — `workflow-commit.pkl`, its package and
    composite reference documents, and narrowly required workflow-model changes.
    Out — changing commit execution semantics, message wording beyond the staged
    style guide, Rust commit behavior, or generated root target trees.
  - Dependencies: none
  - Done when: all target packages contain the split style reference; the
    atomic-commit reference points to it and no longer instructs a YAML result
    section; no generated document names or requires the removed
    `commit-contract.yaml` file; regular and bypass paths remain unchanged.
  - Verification notes (commands or checks): direct Pkl evaluation of
    `workflow-commit.pkl`; generate temporary target payloads; compare affected
    Pi package content with the staged baseline and normalize supported target
    frontmatter for Claude/OpenCode.
  - Implementation evidence: updated `workflow-commit.pkl` to emit the staged
    commit-message guide as a package-local reference for all targets, point the
    atomic-commit phase at it, and remove the obsolete YAML result-contract
    section while preserving commit routing and result branching.
  - Verification evidence: `nix develop -c pkl eval
    config/pkl/base/workflow-commit.pkl` passed; temporary generation emitted
    `atomic-commit.md`, `commit-message-style.md`, and `output.md` under each
    target's `sce-commit/references/`; generated references were target-neutral,
    included the split guide, and contained no result-contract section or
    `commit-contract.yaml` reference.
  - Context synchronization: synced (manually confirmed; durable context
    realigned by T04)

- [x] T03: `Align generation contracts with the new workflow inventories` (status:complete)
  - Task ID: T03
  - Goal: Update renderer metadata coverage, generation-contract assertions, and
    any producer/check expectations so removal of validate context-sync output and
    addition of commit-message-style output are exact, deterministic, and
    target-parity checked.
  - Boundaries (in/out of scope): In — Pkl coverage/contract modules, focused
    negative fixtures or check registration required by the changed inventories,
    and generated-input inspection. Out — implementing workflow behavior already
    covered by T01/T02 or changing setup installation code.
  - Dependencies: T01, T02
  - Done when: exact generated paths and package-local references match the
    canonical renderers; forbidden validate context-sync and commit-contract
    artifacts are rejected; target-neutral references agree; ephemeral producer
    output passes its inventories and remains consumable by setup.
  - Verification notes (commands or checks): `nix develop -c pkl eval
    config/pkl/renderers/metadata-coverage-check.pkl`; `nix run
    .#pkl-check-generated`; targeted negative-fixture evaluations; inspect a
    producer handoff under a temporary directory.
  - Implementation evidence: updated metadata coverage and generation contracts
    for the validate and commit inventories; added explicit forbidden-path and
    atomic-reference ownership assertions; updated negative fixtures and check
    diagnostics; retained target-neutral and producer inventory validation.
  - Verification evidence: `nix develop -c pkl eval
    config/pkl/renderers/metadata-coverage-check.pkl` passed; `nix run
    .#pkl-check-generated` passed with 107 generated files and all registered
    negative fixtures failing as expected; temporary producer output contained
    all three `config/.{pi,claude,opencode}` roots, validate references only
    `output.md`, `validation.md`, and `validation-report.md`, and commit references
    included `commit-message-style.md`; `git diff --check` passed.
  - Context synchronization: synced (manually confirmed; durable context
    realigned by T04)

- [x] T04: `Realign durable workflow ownership documentation` (status:complete)
  - Task ID: T04
  - Goal: Update current-state context to describe validation-only `/validate`,
    task-only context synchronization ownership, ephemeral Pkl target payloads,
    setup-installed root targets, and split commit-reference ownership.
  - Boundaries (in/out of scope): In — the files listed under Context sync and
    any required decision-record cross-reference. Out — historical plan files,
    generated target trees, implementation code, and unrelated context cleanup.
  - Dependencies: T03
  - Done when: current context contains no contradictory claim that `/validate`
    invokes plan context synchronization or that the removed commit contract is
    generated; the ephemeral `config/` generation boundary and setup ownership
    remain accurately documented; changed context files stay within repository
    hygiene limits.
  - Verification notes (commands or checks): focused context searches; read
    affected context files against generated output and canonical Pkl; verify
    links and line counts; run `git diff --check` for authored files.
  - Implementation evidence: realigned root context and workflow ownership
    documents with validation-only `/validate`, task-only synchronization,
    ephemeral target payloads, setup-owned installation, and split commit
    references; updated the overlap/dedup ownership indexes and context map.
  - Verification evidence: temporary `nix run .#pkl-generate -- <tmp>` emitted
    only the ephemeral `config/.{opencode,claude,pi}` payload roots; generated
    validate references were `validation.md`, `validation-report.md`, and
    `output.md`, while commit references included `commit-message-style.md`;
    metadata and generation contract evaluations passed; focused stale-claim
    searches, relative-link checks, line-count checks, `git diff --check`, and
    `nix run .#pkl-check-generated` passed (107 files).
  - Context synchronization: synced (manually confirmed; this task performed
    the durable-context realignment directly)

- [x] T05: `Fix the /validate catalog description to state validation-only evidence recording` (status:complete)
  - Task ID: T05
  - Goal: Update the `/validate` workflow record's `description` in
    `config/pkl/base/workflow-catalog.pkl` from claiming it "synchronize[s]
    its durable context" to stating it records final validation evidence,
    then regenerate and verify the Pi/Claude/OpenCode `/validate` skill
    frontmatter.
  - Boundaries (in/out of scope): In — `workflow-catalog.pkl`'s `/validate`
    record `description` field and the generated frontmatter it drives. Out —
    any other catalog field, other workflow descriptions, or `/validate`
    behavior itself (already validation-only per T01).
  - Dependencies: none
  - Done when: the catalog description no longer says "synchronize its
    durable context"; generated `/validate` command/skill frontmatter across
    all three targets reflects the corrected description; lightweight
    post-task verification passes.
  - Verification notes (commands or checks): direct Pkl evaluation of
    `workflow-catalog.pkl`; temporary generation; grep generated `/validate`
    frontmatter for the old and new wording across `.pi`/`.claude`/
    `.opencode`; `nix run .#pkl-check-generated`; `nix flake check`.
  - Implementation evidence: changed the `["validate"]` record's `description`
    field in `config/pkl/base/workflow-catalog.pkl` from "Validate one
    completed SCE plan and synchronize its durable context" to "Validate one
    completed SCE plan and record final validation evidence".
  - Verification evidence: `nix develop -c pkl eval
    config/pkl/base/workflow-catalog.pkl` passed; temporary generation
    confirmed the old wording is absent and the new wording appears in all
    six affected generated files (`config/.pi/prompts/validate.md`,
    `config/.pi/skills/sce-validate/SKILL.md`,
    `config/.claude/commands/validate.md`,
    `config/.claude/skills/sce-validate/SKILL.md`,
    `config/.opencode/command/validate.md`,
    `config/.opencode/skills/sce-validate/SKILL.md`); `nix run
    .#pkl-check-generated` passed (107 files); `nix flake check` passed (all
    checks green).
  - Context synchronization: synced
  - Context synchronization handoff: Plan path:
    context/plans/remove-validate-context-sync.md; Task ID: T05; Task title:
    Fix the /validate catalog description to state validation-only evidence
    recording; Changed files: config/pkl/base/workflow-catalog.pkl;
    Implementation summary: changed the `["validate"]` record's `description`
    field from "Validate one completed SCE plan and synchronize its durable
    context" to "Validate one completed SCE plan and record final validation
    evidence"; Verification: `nix develop -c pkl eval
    config/pkl/base/workflow-catalog.pkl` passed; temporary generation
    confirmed old wording absent and new wording present in all six affected
    generated files across `.pi`/`.claude`/`.opencode`; `nix run
    .#pkl-check-generated` passed (107 files); `nix flake check` passed; Done
    checks: all three done-when criteria met (old wording removed, new
    wording present across all three targets, verification passed); Context
    impact: local — this only corrects a workflow-catalog description string
    that root context files already describe accurately in validation-only
    terms (per T04's realignment); no root or domain context edit expected.

- [x] T06: `Remove the plan-level context-sync lifecycle from the plan template` (status:complete)
  - Task ID: T06
  - Goal: In `config/pkl/base/workflow-change-to-plan.pkl`, replace the
    `## Context synchronization lifecycle` section's `Plan context
    synchronization` field and its `/validate`-sets-synced-or-blocked claim
    with a `## Task context synchronization lifecycle` section stating only
    the per-task `pending | synced | blocked` lifecycle `/next-task` owns,
    including the sync-before-next-task-or-handoff rule and the `blocked`
    retained fields; apply the fix everywhere the template text is rendered
    so package and composite output agree.
  - Boundaries (in/out of scope): In — the plan-template text/rendering
    functions in `workflow-change-to-plan.pkl` (`changeToPlanPlanTemplate`
    and `renderPlanTemplate`), and the generated `references/plan-template.md`
    / inlined template they produce. Out — the completion-record block's
    per-task `Context synchronization: pending | synced | blocked` field,
    which already matches the task-only lifecycle and stays unchanged;
    `/next-task`'s own canonical source.
  - Dependencies: none
  - Done when: no generated plan-template document or inlined template
    mentions `Plan context synchronization` or claims `/validate` sets a
    synced/blocked plan-level state; a `## Task context synchronization
    lifecycle` section states the task-only lifecycle in all three targets;
    lightweight post-task verification passes.
  - Verification notes (commands or checks): direct Pkl evaluation of
    `workflow-change-to-plan.pkl`; temporary generation; grep generated
    `/change-to-plan` package/template output for `Plan context
    synchronization` (expect none) across all targets; `nix run
    .#pkl-check-generated`; `nix flake check`.
  - Implementation evidence: in both `changeToPlanPlanTemplate` and
    `renderPlanTemplate` in `config/pkl/base/workflow-change-to-plan.pkl`,
    renamed `## Context synchronization lifecycle` to `## Task context
    synchronization lifecycle`, removed the `Plan context synchronization`
    field and its `/validate`-sets-synced-or-blocked claim, and kept the
    `Task context synchronization` sync-before-next-task-or-finish rule and
    the `blocked` retained fields (Blocker, Required action, Retry
    condition) unchanged.
  - Verification evidence: `nix develop -c pkl eval
    config/pkl/base/workflow-change-to-plan.pkl` passed; temporary generation
    via `nix run .#pkl-generate -- <tmp>` produced `references/plan-template.md`
    under `.pi`, `.claude`, and `.opencode`, each containing `## Task context
    synchronization lifecycle` and none containing `Plan context
    synchronization`; `nix run .#pkl-check-generated` passed (107 files); `nix
    flake check` passed (all checks green).
  - Context synchronization: synced
  - Context synchronization handoff: Plan path:
    context/plans/remove-validate-context-sync.md; Task ID: T06; Task title:
    Remove the plan-level context-sync lifecycle from the plan template;
    Changed files: config/pkl/base/workflow-change-to-plan.pkl; Implementation
    summary: replaced the `## Context synchronization lifecycle` section's
    `Plan context synchronization` field and `/validate`-sets-state claim
    with a `## Task context synchronization lifecycle` section stating only
    the task-only lifecycle, in both `changeToPlanPlanTemplate` and
    `renderPlanTemplate`; Verification: `nix develop -c pkl eval
    config/pkl/base/workflow-change-to-plan.pkl` passed; temporary generation
    confirmed the new heading present and `Plan context synchronization`
    absent across all three targets' `plan-template.md`; `nix run
    .#pkl-check-generated` passed (107 files); `nix flake check` passed; Done
    checks: all three done-when criteria met (no `Plan context
    synchronization` mention or plan-level-state claim in any generated
    plan-template document, `## Task context synchronization lifecycle`
    present in all three targets, verification passed); Context impact:
    possibly local — this only removes stale plan-level context-sync wording
    from the plan-template document that plans copy when authored; root
    context files already describe validation-only `/validate` and
    task-only synchronization per T04's realignment, but context-sync should
    confirm no root/domain document still describes a plan-level
    `Plan context synchronization` field.

- [x] T07: `Narrow sce-decision's accepted callers to task context synchronization` (status:complete)
  - Task ID: T07
  - Goal: In `config/pkl/base/decision-skill.pkl`, restrict the accepted
    decision-request source to `sce-next-task` task context synchronization
    only, removing `sce-validate`/plan-context-synchronization wording and
    the "implementation or validation evidence" phrasing in favor of
    "implementation / task-verification evidence", then regenerate and verify
    all three `sce-decision` target packages.
  - Boundaries (in/out of scope): In — `decision-skill.pkl`'s Purpose/Input/
    Boundaries prose. Out — the decision gate criteria, ADR template, and
    `sce-next-task`'s own invocation of `sce-decision` (unchanged).
  - Dependencies: none
  - Done when: generated `sce-decision/SKILL.md` on all three targets accepts
    requests only from `sce-next-task`/task context synchronization, contains
    no `sce-validate` or plan-context-synchronization caller wording, and
    uses "implementation / task-verification evidence"; lightweight
    post-task verification passes.
  - Verification notes (commands or checks): direct Pkl evaluation of
    `decision-skill.pkl`; temporary generation; grep generated
    `sce-decision/SKILL.md` across targets for `sce-validate` and `plan
    context` (expect none); `nix run .#pkl-check-generated`; `nix flake
    check`.
  - Implementation evidence: in `config/pkl/base/decision-skill.pkl`, changed
    the Purpose paragraph from "successful task or plan context
    synchronization" to "successful task context synchronization"; changed
    the Input section's accepted-caller sentence from "`sce-next-task` or
    `sce-validate` context synchronization" to "`sce-next-task` task context
    synchronization"; changed the evidence bullet from "The implementation or
    validation evidence" to "The implementation / task-verification
    evidence"; changed the Boundaries bullet from "Run outside successful
    task or plan context synchronization" to "Run outside successful task
    context synchronization". The decision gate criteria, ADR template, and
    `sce-next-task`'s own invocation were left unchanged.
  - Verification evidence: `nix develop -c pkl eval
    config/pkl/base/decision-skill.pkl` passed (exit 0); temporary generation
    via `nix run .#pkl-generate -- <tmp>` emitted `sce-decision/SKILL.md`
    under `.pi`, `.claude`, and `.opencode`; a grep for `sce-validate` and
    `plan context` across all three generated files returned no matches; a
    grep confirmed "implementation / task-verification evidence" and
    "`sce-next-task` task context" present in all three; `nix run
    .#pkl-check-generated` passed (107 files, inventory sha256
    afed23f3c581a761518b0a83b46c96ec185e898e84f5b773e897df0e071b5d33); `nix
    flake check` passed (all checks green).
  - Context synchronization: synced
  - Context synchronization handoff: Plan path:
    context/plans/remove-validate-context-sync.md; Task ID: T07; Task title:
    Narrow sce-decision's accepted callers to task context synchronization;
    Changed files: config/pkl/base/decision-skill.pkl; Implementation
    summary: restricted the accepted decision-request source to
    `sce-next-task` task context synchronization only in
    `decision-skill.pkl`'s Purpose, Input, and Boundaries prose, removing
    `sce-validate`/plan-context-synchronization wording and replacing
    "implementation or validation evidence" with "implementation /
    task-verification evidence"; Verification: `nix develop -c pkl eval
    config/pkl/base/decision-skill.pkl` passed; temporary generation
    confirmed `sce-validate`/plan-context wording absent and the new
    evidence/caller wording present across all three targets' generated
    `sce-decision/SKILL.md`; `nix run .#pkl-check-generated` passed (107
    files); `nix flake check` passed; Done checks: the single done-when
    criterion met (generated `sce-decision/SKILL.md` on all three targets
    accepts requests only from `sce-next-task`/task context synchronization,
    contains no `sce-validate` or plan-context-synchronization caller
    wording, and uses "implementation / task-verification evidence";
    verification passed); Context impact: possibly local — this only narrows
    prose in the standalone `sce-decision` package describing its accepted
    callers; root context files already describe task-only synchronization
    per T04's realignment, but context-sync should confirm no root/domain
    document still describes `sce-validate` as an accepted `sce-decision`
    caller.

- [x] T08: `Tighten the generation contract's decision-invoking workflow boundary` (status:complete)
  - Task ID: T08
  - Goal: In `config/pkl/renderers/generation-contract-check.pkl`, narrow
    `isDecisionInvokingWorkflowDocument` to match only `sce-next-task`, and
    add a negative assertion that every generated `sce-validate` document
    contains neither a `sce-decision` reference nor plan-context-sync
    wording, so the new task/validate decision boundary is enforced as a
    generation invariant rather than resting on prose alone.
  - Boundaries (in/out of scope): In — `generation-contract-check.pkl`'s
    decision-invoking predicate and its assertions, plus any required
    negative fixture. Out — other generation-contract assertions,
    `metadata-coverage-check.pkl`, and workflow behavior itself (fixed in
    T05-T07).
  - Dependencies: T05, T06, T07
  - Done when: `isDecisionInvokingWorkflowDocument` matches only
    `/skills/sce-next-task/`; a new assertion fails generation when a
    generated `sce-validate` document contains `sce-decision` or
    plan-context-sync wording and passes on current output; `nix develop -c
    pkl eval config/pkl/renderers/generation-contract-check.pkl` passes;
    `nix run .#pkl-check-generated` and `nix flake check` are green.
  - Verification notes (commands or checks): direct Pkl evaluation of the
    contract module; temporary generation; a controlled negative fixture
    proving the new assertion fails as expected; `nix run
    .#pkl-check-generated`; `nix flake check`.
  - Implementation evidence: in `config/pkl/renderers/generation-contract-check.pkl`,
    narrowed `isDecisionInvokingWorkflowDocument` to
    `path.contains("/skills/sce-next-task/")` only (removed the
    `sce-validate` disjunct); added `hidden assertValidateExcludesDecisionAndPlanSync`,
    which fails when any generated `sce-validate` document contains
    `sce-decision`, `plan-context-sync`, `plan context sync`, or `Plan
    context synchronization`, and registered it in `contractChecks` under
    `validate-decision-sync-boundary`; added the negative fixture
    `config/pkl/renderers/fixtures/validate-decision-sync-boundary-check.pkl`,
    which injects a forbidden `sce-decision`/`plan-context-sync` string into
    a fake `sce-validate` document and asserts the new check throws; wired
    the fixture into `config/pkl/check-generated.sh` via
    `expect_pkl_fixture_failure` with the exact expected diagnostic.
  - Verification evidence: `pkl eval
    config/pkl/renderers/generation-contract-check.pkl` passed, printing
    `["validate-decision-sync-boundary"] = "sce-validate package: no
    sce-decision reference or plan-context-sync wording"` alongside all
    other checks including `["decision-invocation"] = "generated decision
    invocation: synchronization-only"`; `pkl eval
    config/pkl/renderers/fixtures/validate-decision-sync-boundary-check.pkl`
    failed with exactly the expected diagnostic; the full
    `config/pkl/check-generated.sh` (equivalent to `nix run
    .#pkl-check-generated`) passed, reporting "Ephemeral Pkl generation
    passed: 107 files"; `pkl eval
    config/pkl/renderers/metadata-coverage-check.pkl` passed; `nix flake
    check` passed after staging the new fixture file (Nix only sees
    git-tracked sources) — `checks.x86_64-linux.pkl-generated` passed
    directly (`nix build .#checks.x86_64-linux.pkl-generated -L`), a
    transient unrelated flaky failure in
    `services::agent_trace_export::tests::read_parts_after_limit_truncates_and_follow_up_continues`
    (a SQLite `UNIQUE constraint failed: parts.id` under concurrent
    agent-trace DB writes, unrelated to this task's scope) was confirmed
    flaky by an isolated rebuild of `checks.x86_64-linux.cli-tests` passing
    334/334, and a full rerun of `nix flake check` then printed "all checks
    passed!" with exit 0.
  - Context synchronization handoff: Plan path:
    context/plans/remove-validate-context-sync.md; Task ID: T08; Task title:
    Tighten the generation contract's decision-invoking workflow boundary;
    Changed files: config/pkl/renderers/generation-contract-check.pkl,
    config/pkl/check-generated.sh,
    config/pkl/renderers/fixtures/validate-decision-sync-boundary-check.pkl;
    Implementation summary: narrowed `isDecisionInvokingWorkflowDocument` to
    match only `/skills/sce-next-task/`; added a `sce-validate`-scoped
    negative assertion (`assertValidateExcludesDecisionAndPlanSync`)
    rejecting `sce-decision` references and plan-context-sync wording,
    registered it in `contractChecks`, added a negative fixture proving it
    fails on injected forbidden content, and wired the fixture into
    `check-generated.sh`; Verification: `pkl eval
    config/pkl/renderers/generation-contract-check.pkl` passed with the new
    check present; the new negative fixture failed with the exact expected
    diagnostic; `config/pkl/check-generated.sh` (`nix run
    .#pkl-check-generated` equivalent) passed (107 files); `nix flake check`
    passed ("all checks passed!") after confirming an unrelated transient
    test flake was not caused by this change; Done checks: all four
    done-when criteria met (predicate narrowed, new assertion fails on
    injected content and passes on current output, contract module
    evaluation passes, `pkl-check-generated`/`nix flake check` green);
    Context impact: possibly local — this only strengthens a generation-time
    invariant (`sce-validate` excludes `sce-decision`/plan-context-sync
    content) that root context files already describe behaviorally via
    T04-T07's realignment; context-sync should confirm no root/domain
    document still needs updating to describe this contract-level
    enforcement, and that none contradicts the narrowed decision-invoking
    predicate.
  - Context synchronization: synced

## Open questions

None. The clarification explicitly removes plan-level context synchronization
and its invocation from `/validate`, while retaining task-level synchronization
and the existing ephemeral-generation/setup boundary. The staged commit
reference edits are treated as the requested canonicalization target. This PR
review's follow-up fixes (T05-T08) close the remaining gap between the
accepted `2026-08-13-validate-validation-only` ADR and the canonical Pkl
sources, without reopening any design question the ADR already settled.

## Validation Report

**Status:** validated  
**Date:** 2026-08-13

### Commands run

- `nix run .#pkl-check-generated` -> exit 0 (Ephemeral Pkl generation passed: 107 files, inventory sha256 afed23f3c581a761518b0a83b46c96ec185e898e84f5b773e897df0e071b5d33)
- `nix flake check` -> exit 0 (all checks passed!)
- `nix run .#pkl-generate -- "$(mktemp -d)"` -> exit 0 (wrote only `config/.{pi,claude,opencode}` and `config/{optional-workflows.json,schema/}` under the temp dir; no repository-root `.pi`/`.claude`/`.opencode` paths were created or modified)
- `nix develop -c pkl eval config/pkl/renderers/metadata-coverage-check.pkl` -> exit 0
- `nix develop -c pkl eval config/pkl/renderers/generation-contract-check.pkl` -> exit 0 (includes `["validate-decision-sync-boundary"] = "sce-validate package: no sce-decision reference or plan-context-sync wording"`)
- `nix develop -c pkl eval config/pkl/renderers/fixtures/validate-decision-sync-boundary-check.pkl` -> exit 1, as required (fixture asserts the new check throws on injected forbidden content; failed with the expected `assertValidateExcludesDecisionAndPlanSync` diagnostic)

### Success-criteria verification

- [x] AC1: A fresh Pkl generation writes only ephemeral payloads below `config/`, not repository-root integration directories -> temp-dir generation via `nix run .#pkl-generate -- "$(mktemp -d)"` produced only `<tmp>/config/.{pi,claude,opencode}` and no root-level `.pi`/`.claude`/`.opencode` paths; the existing repository-root `.pi`/`.claude`/`.opencode` directories (installed previously by `sce setup`) were untouched by the run.
- [x] AC2: Generated `sce-validate` has no plan-context-sync invocation, handoff, or package-local `context-sync.md`, and its completion output does not claim synchronization -> `sce-validate/references/` inventory across `.pi`/`.claude`/`.opencode` contains exactly `validation.md`, `validation-report.md`, `output.md`, `SKILL.md`; a grep for `sce-plan-context-sync|context-sync.md|plan-context-sync|sce-decision|synchronize durable context` across all three generated `sce-validate` packages and `/validate` command/prompt files returned only the expected negation statements ("Do not: Synchronize durable context under `context/`." and "Validation does not synchronize durable context.").
- [x] AC3: Generated `/commit` packages contain `references/commit-message-style.md`; `atomic-commit.md` delegates to it without a YAML result-contract section -> `commit-message-style.md` present in all three targets' `sce-commit/references/`; `atomic-commit.md` references `references/commit-message-style.md` for subject/body wording; a grep for `commit-contract.yaml|## Result contract|result contract` across all three `sce-commit` packages returned no matches.
- [x] AC4: Generation metadata and generation-contract checks describe the new validate/commit inventories -> `metadata-coverage-check.pkl` and `pkl-check-generated` both passed (107 files, matching inventory hash).
- [x] AC5: Durable workflow documentation states `/next-task` owns task context synchronization while `/validate` is validation-only -> `context/sce/shared-context-code-workflow.md`, `context/sce/context-workflow-rules.md`, `context/architecture.md`, and `context/glossary.md` each state `/validate` does not invoke or persist plan-level context synchronization; `context/sce/atomic-commit-workflow.md` and `context/glossary.md` state no `commit-contract.yaml` artifact is generated; no matches found for stale claims that `/validate` invokes plan-context synchronization.
- [x] AC6: Catalog description, plan template, `sce-decision` callers, and generation-contract decision-invoking list are validation-only-consistent with `2026-08-13-validate-validation-only` -> `workflow-catalog.pkl`'s `["validate"]` record `description` reads "Validate one completed SCE plan and record final validation evidence"; `workflow-change-to-plan.pkl` emits `## Task context synchronization lifecycle` (no `## Context synchronization lifecycle` or `Plan context synchronization` field) in both `changeToPlanPlanTemplate` and `renderPlanTemplate`; `decision-skill.pkl` accepts requests only from "`sce-next-task` task context synchronization"; `generation-contract-check.pkl`'s `isDecisionInvokingWorkflowDocument` matches only `/skills/sce-next-task/`, and its `assertValidateExcludesDecisionAndPlanSync` check (registered under `validate-decision-sync-boundary`) passed on current generated output and the paired negative fixture failed as required.

### Failed checks and follow-ups

- None.

### Residual risks

- None identified.
