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

- [ ] AC1: A fresh Pkl generation writes only ephemeral payloads below the
  `config/` generation root, including `.pi`, `.claude`, and `.opencode`, and
  does not create or modify repository-root integration directories; `sce setup`
  remains the documented installer of those root files.
  - Validate: `nix run .#pkl-generate -- "$(mktemp -d)"`; inspect the generated
    `config/.{pi,claude,opencode}` paths and confirm repository-root target
    directories are not generation destinations.
- [ ] AC2: Generated `sce-validate` contains no plan-context-sync invocation,
  no `sce-plan-context-sync` handoff, and no package-local `context-sync.md`;
  validation no longer requires completed-task sync fields or writes a
  plan-level sync state before returning `validated`, and its completion output
  does not claim durable context synchronization.
  - Validate: inspect generated `sce-validate/SKILL.md` and its references for
    the removed phase, call, lifecycle gate, and synchronization claim; assert
    the generated `sce-validate/references/` inventory contains only the
    validation and output/report documents defined by the canonical source.
- [ ] AC3: Generated `/commit` packages contain
  `references/commit-message-style.md`, and `references/atomic-commit.md`
  delegates message-style rules to that file without the removed
  `references/commit-contract.yaml` return section; all three target packages
  preserve the staged reference content semantically.
  - Validate: generate all targets and inspect each `sce-commit` reference
    inventory and the atomic-commit/style documents; confirm the forbidden YAML
    contract-return section is absent.
- [ ] AC4: Generation metadata, package-relative reference checks, and exact
  artifact/path assertions describe the new validate and commit inventories;
  target-neutral workflow documents remain equivalent apart from supported
  frontmatter, and setup can consume the generated payload without requiring
  repository-root generated trees.
  - Validate: `nix develop -c pkl eval
    config/pkl/renderers/metadata-coverage-check.pkl`; `nix run
    .#pkl-check-generated`; inspect the generated-input handoff paths.
- [ ] AC5: Durable workflow documentation accurately states that `/next-task`
  still owns task context synchronization while `/validate` performs validation
  only and does not synchronize plan context; the commit reference ownership
  description matches the new split.
  - Validate: inspect the affected `context/sce/` and root context files and
    confirm no current-state document says `/validate` invokes plan context
    synchronization or that the removed commit contract remains generated.

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

## Open questions

None. The clarification explicitly removes plan-level context synchronization
and its invocation from `/validate`, while retaining task-level synchronization
and the existing ephemeral-generation/setup boundary. The staged commit
reference edits are treated as the requested canonicalization target.
