# Plan: compact-doctor-output

## Change summary

Improve the default human-readable `sce doctor` report so it is a compact,
domain-oriented health summary instead of an inventory of every successful file
check. The change is a presentation redesign over the existing doctor report:
diagnosis remains complete and read-only, while healthy paths, IDs, hashes, and
individual integration files are suppressed in text mode. Failed or warning
nodes retain the relevant path, state, diagnostic, and remediation details
already produced by the doctor checks.

The existing `--format json` report and `sce doctor --fix` repair behavior remain
available. This plan does not add a new CLI flag: there is no existing doctor
verbose/debug mode, and JSON already provides the complete machine-readable
detail needed for troubleshooting and automation. A future verbose text mode
can be added independently if operators demonstrate a need for expanded
successful checks.

## Current architecture

- `cli/src/cli_schema.rs` defines `doctor { --fix, --format text|json }`; no
  `--verbose` or debug output mode exists. Clap conversion is in
  `cli/src/services/parse/command_runtime.rs`, and
  `cli/src/services/doctor/command.rs` is the thin runtime adapter.
- `cli/src/services/doctor/mod.rs` resolves the repository root, creates a
  repo-scoped context, invokes the shared `lifecycle_providers(true)` catalog,
  adapts lifecycle results into doctor-owned problems, builds a report, and
  delegates text/JSON rendering.
- `cli/src/services/lifecycle.rs` owns provider-neutral `HealthProblem` data:
  kind, category, severity, fixability, summary, remediation, and next action.
  Config, local DB, auth DB, Agent Trace DB, and Git-hook providers produce
  these results. Their checks must not become terminal-rendering logic.
- `cli/src/services/doctor/types.rs` owns `HookDoctorReport`, location and
  identity facts, hook health, flat `IntegrationGroupHealth` records, flat
  integration child records, and doctor problem/result enums.
- `cli/src/services/doctor/inspect.rs` gathers the report facts and performs
  integration inventory checks. Integration groups are currently emitted as
  flat labels such as `ClaudeCode skills`, with one child for every embedded
  installed asset. Existing optional-workflow filtering and content/merge
  validation are the source of truth and must remain unchanged.
- `cli/src/services/doctor/render.rs` currently renders the fixed sections
  `Environment`, `Configuration`, `Repository`, `Git Hooks`, and
  `Integrations`; every successful path-backed row, identity value, and
  integration child path is printed. It also renders existing problem counts,
  fix results, and the complete JSON payload.
- Paths and metadata originate in the report facts collected by `inspect.rs`
  (`state_root`, config locations, repository/hooks paths, checkout identity,
  Agent Trace DB identity/path, and integration child paths), while failure
  paths and messages also originate in provider `HealthProblem.summary` and
  `remediation` strings. Integration child state supplies missing, mismatch,
  and read-error classification.

## Proposed output model

### Domain hierarchy

Use a render-only diagnostic tree with this text-mode order:

```text
SCE doctor

Environment
  [STATUS] State
  [STATUS] Configuration
  [STATUS] Repository identity

Repository
  [STATUS] Git repository
  [STATUS] Git hooks

Integrations
  Claude Code
    [STATUS] Plugins
    [STATUS] Agents
    [STATUS] Commands
    [STATUS] Skills

  OpenCode
    [STATUS] Plugins
    [STATUS] Agents
    [STATUS] Commands
    [STATUS] Skills

  Pi
    [STATUS] Extensions
    [STATUS] Prompts
    [STATUS] Skills
```

Only configured/detected integration targets are rendered, preserving the
existing target-resolution behavior. The implementation should use typed target
and area metadata for integration groups rather than recovering hierarchy by
parsing labels such as `ClaudeCode skills`. Existing terminology is normalized
for display (`Claude Code`, `Plugins`, `Extensions`) without changing the
underlying setup asset paths or target IDs.

`State` summarizes state-root and local/auth database readiness. `Configuration`
summarizes global and local config validation. `Repository identity` summarizes
checkout identity and repository-scoped Agent Trace identity/database health.
`Git hooks` summarizes the effective hooks directory plus the required hook
rollout. This removes the current standalone `Git Hooks` section while keeping
the same checks and failure facts.

### Status rules

- `[PASS]` means the node and all descendants are healthy.
- `[WARN]` means the node has a non-blocking warning and no blocking failure.
  This makes existing warning severity visible without pretending it is a
  successful check.
- `[FAIL]` means a blocking error or failed validation exists below the node.
- `[MISS]` remains the leaf status for a required file/check that is absent;
  its parent is `[FAIL]` because the missing asset blocks readiness.
- Parent status is the worst descendant status in the order `FAIL`, `MISS`,
  `WARN`, `PASS`. Readiness and exit-code classification continue to use the
  existing problem severity/readiness model rather than the renderer's status
  token.
- Color behavior remains the shared style policy: pass is green, warning is
  yellow if a warning style exists or otherwise unstyled, and fail/miss are
  red; non-TTY and `NO_COLOR` output contains deterministic plain tokens.

### Collapse and expansion rules

- A healthy top-level node renders one concise row and no details. Do not show
  absolute paths, UUIDs, repository IDs, repository state-directory hashes,
  canonical identities, configured remote names, or implementation metadata on
  successful text rows.
- A healthy integration area renders one row (`[PASS] Skills`) and never lists
  its installed files.
- A warning or failure expands only the affected branch. The affected group
  row is followed by the child asset/check rows needed to locate the problem;
  healthy sibling groups remain collapsed.
- Integration children are projected from the existing flat child facts into a
  generic relative-path tree. For `skills`, the first workflow directory is a
  child node and its files are nested beneath it, so one missing `SKILL.md` can
  render as `Skills -> sce-commit -> SKILL.md`. Other asset groups use the
  meaningful relative asset path without assuming a fixed number of files.
- A failing child shows the existing state-specific context: `Missing: <path>`
  for absent files, `Path: <path>` plus content-mismatch information for stale
  files, and the stored read error for unreadable files. The affected group also
  renders the matching existing problem summary and remediation, deduplicated
  by the typed problem/child association rather than by substring matching.
- A failing top-level node renders the relevant existing problem summary and
  remediation, including absolute paths and expected/actual or invalid-value
  information where the diagnostic already supplies it. No filesystem or Git
  check is added to the renderer.
- Fix-mode text keeps the same report tree and appends the existing `Fix
  results` section. Fix details remain detailed because they describe actions
  taken, not healthy state.

### Information hidden versus retained

On success, hide all path and implementation metadata listed above, including
individual integration asset paths and checkout/Agent Trace IDs. On warning or
failure, retain the checked path, missing/stale/read state, provider summary,
remediation, and any diagnostic value already present in the report. JSON keeps
its current complete path/identity/problem fields, so this text compaction does
not remove machine-readable troubleshooting data.

## Implementation approach

### Data model and ownership

Keep diagnosis and lifecycle providers presentation-neutral. The collapse
algorithm belongs in the doctor text presentation layer because it changes only
what is shown, not what is checked, what is considered ready, or how repairs are
selected. Do not move filesystem checks, content hashing, optional-workflow
selection, or severity calculation into `render.rs`.

Extend the doctor-owned report model only to preserve typed relationships needed
by the renderer:

- Add typed integration target/area metadata to `IntegrationGroupHealth` (or an
  equivalent doctor-owned group key) and derive display labels from it. Keep
  `IntegrationChildHealth` as the source of relative path and content state.
- Add a render-only node/status/detail representation in `doctor/types.rs` or a
  focused private section of `doctor/render.rs`. It should accept a completed
  `HookDoctorReport` and never access the filesystem.
- Reuse `ProblemKind`, `ProblemSeverity`, `DoctorProblem.summary`, and
  `DoctorProblem.remediation` for top-level and group failure details. If a
  typed association is needed to avoid fragile summary matching, add a small
  doctor-owned problem scope/key during report construction; do not parse
  human-readable summaries to determine status.
- Do not change lifecycle `HealthProblem` semantics unless implementation
  proves an existing failure detail cannot be associated with a node. If a
  detail extension is unavoidable, make it structured and provider-neutral,
  copy it through the existing doctor/lifecycle adapters, and add it
  additively to JSON only with an explicit compatibility review.

### Renderer changes

Refactor `doctor/render.rs` so text and JSON are separate contracts:

- Build the new text tree from the existing report facts/problems, compute
  worst-descendant status, render concise healthy rows, and recursively render
  only unhealthy branches.
- Render the default diagnose header as `SCE doctor`; retain an explicit,
  deterministic fix-mode header that identifies repair mode without restoring
  the old `diagnose` inventory wording.
- Keep `render_report_json` field names and values unchanged unless a narrowly
  justified additive field is required by the existing contract. In particular,
  do not apply text redaction rules to JSON.
- Keep shared TTY/`NO_COLOR` styling and stdout payload ownership unchanged.

`doctor/inspect.rs` should only change to provide typed group keys or structured
associations required by the renderer. Existing inventory checks, optional
workflow filtering, content-state classification, and problem generation must
remain the same. `doctor/mod.rs`, the provider modules, the parser, and exit
code handling should not change unless the typed adapter change requires it.

### Architectural decisions and trade-offs

1. **Presentation collapse, not diagnostic collapse.** Keeping every leaf fact
   in the report preserves correctness and JSON/debuggability, while a
   render-only tree gives the default text UX the desired healthy-collapse /
   failure-expansion behavior. Aggregating in providers would risk hiding facts
   from JSON, fix mode, or future renderers.
2. **Typed group keys instead of label parsing.** This adds a small semantic
   result-model seam, but avoids coupling hierarchy to display spelling and
   supports future target/asset types without duplicating checks.
3. **No `--verbose` in this change.** No such mechanism exists today. Adding it
   would create a second text contract and a new compatibility surface; JSON is
   already the stable full-detail route. Revisit only if compact text cannot
   serve operators who need successful-file inventories.
4. **Warnings become visible.** Existing warning problems such as optional
   OpenCode asset health should not silently appear as `[PASS]`. `[MISS]` is
   retained for absent required leaves, while blocking parent nodes remain
   `[FAIL]`.

## Backward compatibility

- Preserve process exit-code semantics: successful report generation remains
  exit code `0` even when the report says `not_ready`, and parse/validation/
  runtime/dependency failures retain the existing class mapping (`2/3/4/5`).
- Preserve `sce doctor --fix` behavior, provider ordering, repair ownership,
  idempotence, and fix-result outcomes.
- Preserve `--format json` field names, values, problem records, path/identity
  detail, and machine-readable readiness. Text layout is intentionally a
  human-facing contract change; scripts should use JSON rather than parse the
  compact text hierarchy.
- Preserve non-TTY and CI behavior: no ANSI sequences when output is not a TTY
  or `NO_COLOR` is set, deterministic ordering of domains/groups/children, and
  no extra stdout/stderr streams.
- Update existing exact text-contract documentation/tests because the current
  approved section order and `SCE doctor diagnose` header will change. No parser
  or help compatibility change is needed because no new option is introduced.
- There is no existing verbose/debug mechanism to preserve. The current fully
  expanded success inventory remains available through the complete JSON report,
  not through a new text flag.

## Testing strategy

Add pure renderer/view-model tests with synthetic `HookDoctorReport` fixtures,
plus focused integration-state tests where existing inspection helpers are the
best source of truth. Assert exact plain-text output with color disabled and
assert that JSON remains unchanged for representative fields.

Required cases:

- **Everything passes:** only the compact Environment, Repository, and selected
  target/group rows appear; no successful path, UUID, repository ID, canonical
  identity, or integration file path appears.
- **One top-level check fails:** the parent becomes `[FAIL]`, its failure
  summary/remediation and checked path remain visible, and unrelated healthy
  domains stay collapsed.
- **One deeply nested integration file fails:** the target and area expand to
  the affected asset/workflow and file; the missing/mismatch/read-error path
  and existing diagnostic are visible while healthy siblings remain concise.
- **Several failures in one domain:** one domain status is emitted with all
  affected child branches, deterministic ordering, and no duplicated summary
  lines.
- **Failures across multiple domains:** each affected domain expands
  independently; no failure detail is lost or attached to the wrong target.
- **Unusual paths/spaces:** paths containing spaces, parentheses, quotes, and
  non-ASCII characters remain intact as path values and do not break hierarchy
  construction or detail rendering. Use `PathBuf`/structured fields rather than
  splitting rendered strings.
- **Warnings:** a non-blocking warning renders `[WARN]`, while a missing
  required child renders `[MISS]` and its parent renders `[FAIL]`.
- **Fix mode:** the compact report is followed by the existing fix-result
  vocabulary/details; no repair is triggered by rendering.
- **JSON/CI:** JSON remains parseable and retains existing path/identity/problem
  detail; plain text from a non-TTY contains no ANSI sequences and keeps stable
  status tokens.
- **No verbose mode:** parser/help tests continue to reject no newly implied
  option, and `--format json` is documented/tested as the full-detail route.

## Example outputs

### Completely healthy installation

```text
SCE doctor

Environment
  [PASS] State
  [PASS] Configuration
  [PASS] Repository identity

Repository
  [PASS] Git repository
  [PASS] Git hooks

Integrations
  Claude Code
    [PASS] Plugins
    [PASS] Agents
    [PASS] Commands
    [PASS] Skills

  OpenCode
    [PASS] Plugins
    [PASS] Agents
    [PASS] Commands
    [PASS] Skills

  Pi
    [PASS] Extensions
    [PASS] Prompts
    [PASS] Skills

Summary: 0 blocking problem(s), 0 warning(s)
```

### Nested integration failure

```text
SCE doctor

Environment
  [PASS] State
  [PASS] Configuration
  [PASS] Repository identity

Repository
  [PASS] Git repository
  [PASS] Git hooks

Integrations
  Claude Code
    [PASS] Plugins
    [PASS] Agents
    [PASS] Commands
    [FAIL] Skills
      [PASS] sce-change-to-plan
      [MISS] sce-commit
        Missing: /home/user/project/.claude/skills/sce-commit/SKILL.md
        Problem: ClaudeCode skills required file(s) are missing.
        Remediation: Reinstall repo-root Claude assets, then rerun 'sce doctor'.
      [PASS] sce-handover

  OpenCode
    [PASS] Plugins
    [PASS] Agents
    [PASS] Commands
    [PASS] Skills

  Pi
    [PASS] Extensions
    [PASS] Prompts
    [PASS] Skills

Summary: 1 blocking problem(s), 0 warning(s)
```

The concrete problem line must be rendered from the existing structured
`DoctorProblem` summary/remediation and child state; the example is not a new
hardcoded diagnostic.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the
check that proves it. `/validate` runs these checks; no task in the stack
performs final validation.

- [ ] AC1: Default text output uses the Environment/Repository/Integrations hierarchy and groups integration areas beneath typed Claude Code, OpenCode, and Pi target nodes without listing every healthy file.
  - Validate: exact plain-text renderer tests for an all-pass report and selected-target ordering.
- [ ] AC2: Healthy rows hide absolute paths, IDs, hashes, canonical identities, remote names, and individual integration paths, while all current checks still execute and readiness is unchanged.
  - Validate: all-pass report assertions plus existing inspection/lifecycle tests and JSON field assertions.
- [ ] AC3: Blocking failures and non-blocking warnings retain actionable details, including paths, missing/stale/read state, existing summaries, remediations, and nested integration context; healthy siblings remain collapsed.
  - Validate: renderer tests covering top-level, nested, same-domain, cross-domain, warning, and unusual-path fixtures.
- [ ] AC4: Parent/domain statuses are deterministic summaries of descendants, using `[PASS]`, `[WARN]`, `[FAIL]`, and `[MISS]` according to the documented severity rules.
  - Validate: status aggregation unit tests for every status combination and exact output assertions.
- [ ] AC5: `--format json`, fix behavior, stream routing, non-TTY styling, and existing exit-code semantics remain compatible.
  - Validate: JSON regression tests, fix-mode rendering tests, parser/app contract tests, and `NO_COLOR`/non-TTY renderer tests.
- [ ] AC6: The updated doctor text contract and CLI documentation describe the new hierarchy, detail policy, JSON full-detail route, and lack of a verbose flag.
  - Validate: inspection of the updated durable context files against the renderer and `nix run .#pkl-check-generated`/`nix flake check` where applicable.

### Full validation

Repository-wide checks `/validate` runs after the last task, regardless of which
criterion they map to.

- `nix run .#pkl-check-generated`
- `nix flake check`

### Context sync

- `context/sce/doctor-human-text-contract.md` — replace the old flat text layout,
  status/header rules, and integration row contract with the compact hierarchy
  and failure-expansion contract.
- `context/sce/agent-trace-hook-doctor.md` — update the approved operator-health
  text-mode and output-shape description while preserving readiness, repair, and
  JSON contracts.
- `context/cli/cli-command-surface.md` — update the current doctor output
  description and the statement that text rows expose path details.
- `context/overview.md` and `context/architecture.md` — update only if their
  current doctor text claims are no longer accurate after implementation.

## Constraints and non-goals

- **In scope:** Rust doctor report/view-model and text renderer changes,
  typed integration grouping metadata, doctor-focused tests, and the durable
  doctor/CLI text-contract updates listed under Context sync.
- **Out of scope:** changing diagnostic checks, health taxonomy, optional
  workflow selection, setup/install behavior, repair logic, JSON field semantics,
  exit codes, top-level CLI parsing, or integration asset contents.
- **Constraints:** use existing lifecycle/report facts; keep renderer pure and
  filesystem-free; preserve deterministic ordering; use the shared style/TTY
  policy; keep stdout payload ownership in the app layer; run repository checks
  through Nix.
- **Non-goal:** introduce a generic health-dashboard framework or a new
  `--verbose`/debug text mode. The render-only tree is specific to doctor text
  output and must not become a second diagnostic engine.

## Assumptions

- The user accepts an intentional human-text contract change from
  `SCE doctor diagnose` plus separate `Git Hooks` to the compact `SCE doctor`
  hierarchy shown here; JSON and process semantics remain the compatibility
  boundary.
- `[WARN]` is acceptable for existing non-blocking warning problems, while
  `[MISS]` remains useful for required missing leaves. This follows the request
  to define PASS/WARN/FAIL behavior and the existing warning severity model.
- Existing summaries/remediations contain sufficient failure detail for the
  first implementation; a structured detail field is added only if typed
  association cannot be achieved without parsing strings.

## Task stack

- [x] T01: `Add typed doctor display grouping and status projection seams` (status:done)
  - Task ID: T01
  - Scope: In — extend `cli/src/services/doctor/types.rs` and the integration-group construction in `doctor/inspect.rs` with typed target/area keys and render-only node/status/detail helpers; preserve all existing health facts and checks. Out — changing rendered output, lifecycle provider behavior, JSON serialization, or CLI options.
  - Dependencies: none
  - Done when: the completed report can be projected into a deterministic domain/group/asset tree without parsing display labels or consulting the filesystem, and existing inspection tests still describe the same expected assets/states.
  - Verify: targeted doctor Rust tests covering typed group keys, optional-workflow filtering, and deterministic child ordering.
  - Completed: 2026-08-17
  - Files changed: `cli/src/services/doctor/types.rs`, `cli/src/services/doctor/inspect.rs`, `cli/src/services/doctor/render.rs`
  - Result: Added typed integration target/area keys with derived display labels, filesystem-free display node/status/detail projection helpers, and coverage for typed grouping, optional-workflow filtering, and deterministic child ordering without changing rendered output or health checks.
  - Done checks: Report projection is deterministic and filesystem-free (done); existing inspection assets/states remain covered (done).
  - Context impact: local — doctor report/view-model and inspection seams changed; durable doctor text-contract context remains unchanged until the rendering tasks.
  - Context synchronization: synced

- [ ] T02: `Render compact healthy doctor domains and integration groups` (status:todo)
  - Task ID: T02
  - Scope: In — refactor text rendering in `cli/src/services/doctor/render.rs` to emit the new header, Environment/Repository/Integrations hierarchy, concise pass rows, target-scoped group rows, and deterministic status/color handling; add all-pass, target-selection, non-TTY, and no-integration fixtures. Out — failure-detail expansion, diagnostic check changes, JSON shape changes, and fix execution.
  - Dependencies: T01
  - Done when: a healthy report contains only the compact rows from the approved output model, selected targets remain the only rendered integration targets, and no successful path/ID/file inventory leaks into text mode.
  - Verify: exact plain-text renderer tests with color disabled and non-TTY/`NO_COLOR` policy assertions.
  - Context synchronization: pending

- [ ] T03: `Expand warnings and failures with nested diagnostic details` (status:todo)
  - Task ID: T03
  - Scope: In — implement recursive unhealthy-branch expansion in `doctor/render.rs`, associate existing `DoctorProblem` details with top-level/group nodes, render nested integration asset/workflow failures, add `[WARN]`/`[FAIL]`/`[MISS]` aggregation, preserve fix-result detail, and add all required failure/path test cases. Make only the minimal `doctor/inspect.rs` or doctor-owned model adjustment needed for typed associations. Out — new checks, provider logic, repair behavior, verbose mode, and JSON redesign.
  - Dependencies: T02
  - Done when: every failed/warned node exposes enough existing context to troubleshoot immediately, healthy siblings stay collapsed, multiple failures across one or more domains render deterministically, and paths with spaces/unusual characters remain intact.
  - Verify: exact renderer tests for top-level failure, deeply nested integration failure, multiple same-domain failures, cross-domain failures, warnings, missing/mismatch/read errors, unusual paths, and fix mode.
  - Context synchronization: pending

- [ ] T04: `Lock compatibility and update the doctor text contract` (status:todo)
  - Task ID: T04
  - Scope: In — add JSON regression assertions and parser/app compatibility coverage as needed, verify unchanged exit/stream/fix semantics, and update `context/sce/doctor-human-text-contract.md`, `context/sce/agent-trace-hook-doctor.md`, and `context/cli/cli-command-surface.md` to match the implemented output. Update root context claims only when they are stale. Out — a final validation-only task, new CLI flags, unrelated documentation, and application behavior outside doctor presentation.
  - Dependencies: T03
  - Done when: the new text contract is documented once, JSON and command compatibility are covered, and the implementation leaves a complete actionable plan for `/validate` without a trailing cleanup task.
  - Verify: focused doctor/app tests plus documentation-to-code inspection; repository-wide checks are listed under Full validation for `/validate`.
  - Context synchronization: pending

## Open questions

None. The request specifies the required UX outcome and explicitly permits a
proposed hierarchy. The main trade-offs (render-only aggregation, typed group
keys, warning token, and no verbose flag) are resolved above without changing
diagnostic correctness or machine-readable compatibility.
