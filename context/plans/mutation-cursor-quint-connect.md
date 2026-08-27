# Plan: mutation-cursor-quint-connect

## Change summary

Connects the verified `spec/mutation_cursor.qnt` model to the pure Rust kernel at
`cli/src/services/mutation_trace/protocol.rs` using Quint Connect model-based
testing, so that `protocol.rs` becomes a continuously checked refinement of the
Quint spec instead of a one-time manual translation. This extends the completed
`mutation-cursor-protocol-kernel` work (PR #238, branch `mutation-cursor`, head
`2a097408`) with a new `#[cfg(test)]`-only `mbt/` submodule: a driver that
replays Quint-generated and Quint `run`-scenario traces through the real
`prepare`/`commit`/`taint`/`database_failure`/`abandon`/`recover` functions and
compares projected Rust state against Quint state after every step.

This is a **third revision** of the plan (PR #239), correcting one remaining
MBT-transport issue found in review, on top of two earlier rounds of
correction:

- **Round 1** replaced a driver design that inferred action arguments from
  state with a verification-only `MbtAction` transport type, and kept
  `randomPrepare` a single top-level `step` branch instead of splitting it
  into four.
- **Round 2** pinned CI to a repository Nix check carrying both the Rust
  toolchain and the Quint binary, rather than the GitHub runner's Cargo.
- **Round 3 (this revision)** fixes two remaining `MbtAction` design gaps:
  1. Every argument-carrying `MbtAction` variant must use a record payload
     (even single-field ones), not a bare scalar, to match Quint Connect's
     custom sum-type action decoder (unit variant vs. record variant).
  2. `MbtAction` must record the **invoked operation and its arguments**,
     never whether that invocation changed state. `prepare`, `taint`,
     `databaseFailure`, `abandon`, and `recover` (spec lines
     450/707/734/801/882) all currently fall through to the shared `stutter`
     action on their guarded/no-op path; naively wiring `mbtAction' =
     MbtStutter` into that shared `stutter` action would erase which
     operation was actually invoked, silently stop the MBT from exercising
     Rust's guard behavior on exactly the paths where refinement bugs hide,
     and misuse `MbtStutter` — which must mean only "the explicit top-level
     `stutter` action was selected by `step`."

The corrected pipeline:

```text
semantic action (mutate/prepare/commitAttempt/taint/databaseFailure/abandon/recover)
        ↓ (records its own invocation and arguments, on EVERY branch —
        ↓  including a branch that itself performs no state change)
verification-only MbtAction transport (mbtAction: MbtAction)
        ↓
Quint Connect custom action/nondet trace
        ↓
Rust Driver::step (dispatches on the MbtAction variant, always calling
        ↓          the real protocol.rs function — never skipped because the
        ↓          expected Quint state happened not to change)
real protocol.rs call
        ↓
projected Rust ModelState, compared against Quint state every step
```

This plan does not implement production mutation tracing: no `store.rs`,
`coordinator.rs`, `git_snapshot.rs`, database, Git, filesystem, or hook
integration is added. It is a pure verification harness layered on top of the
already-pure `protocol.rs`.

One documentation-target note from the original plan still applies: the
request names `context/plans/mutation-cursor-quint-connect.md` as the
architecture-doc output, but that path is this plan's own file — see
**Assumptions**.

## Acceptance criteria

- [ ] AC1: `quint-connect` is a dev/test-only dependency of the CLI crate; no
  production dependency changes.
  - Validate: `grep -A3 '^\[dev-dependencies\]' cli/Cargo.toml` lists
    `quint-connect`; it does not also appear under `[dependencies]`;
    `./scripts/run-cli-cargo.sh build --manifest-path cli/Cargo.toml` passes.
- [ ] AC2: every action reachable from Quint's randomized `step` (`init`,
  `randomMutate`, `randomPrepare`, `randomCommit`, `randomTaint`,
  `randomRecover`, `randomDatabaseFailure`, `randomAbandon`, `stutter`) is
  recorded by the verification-only `MbtAction` transport with its concrete
  arguments — on every branch of that action, including branches that produce
  no state change — and the Rust driver dispatches on the `MbtAction` variant
  to call the real `protocol.rs` functions for every transported invocation,
  never reimplementing their logic, never inferring arguments from
  before/after state, and never skipping the call because the expected state
  happened not to change.
  - Validate: inspection of `mbt/driver.rs` — each match arm on `MbtAction`
    unconditionally calls exactly one
    `protocol::{prepare,commit,taint,database_failure,abandon,recover}`
    function with the transported arguments, mutates only `worktree_trees`,
    or is a no-op (`MbtStutter` only); no independent scope/attempt/cursor
    mutation logic exists in the driver, and no conditional skips a protocol
    call based on predicted state change.
- [ ] AC3: both the generated `#[quint_run]` trace and the deterministic
  `#[quint_test]` runs transport concrete action arguments through the same
  `MbtAction` mechanism; a scenario using non-default values demonstrably
  carries them through unchanged.
  - Validate: the T04 smoke scenario built from `mutate(WT1, Tree3)` →
    `prepare(Attempt5, Flush(WT1))` → `commitAttempt(Attempt5)` passes, and
    inspection/logging of the driver's received `MbtAction` values for that
    run shows `WT1`, `Tree3`, `Attempt5`, and `Flush(WT1)` reaching the actual
    `protocol::prepare`/`protocol::commit` calls unchanged.
- [ ] AC4: a Quint-generated random trace, replayed through the real
  `protocol.rs`, matches the Quint model's semantic state after every step
  across the configured sample/step budget, and a failing/generated trace is
  reproducible by `QUINT_SEED`.
  - Validate: `mutation_cursor_generated_traces_refine_rust_protocol`
    (`#[quint_run(max_samples = 500, max_steps = 30)]`) passes under the
    Nix-pinned Quint Connect check (T06); running it twice with the same
    explicit `QUINT_SEED=<value>` reproduces the same outcome.
- [ ] AC5: the comparable state includes every Quint variable named in the
  request (`worktrees`, `scopes`, `worktreeTrees`, `externalTaint`,
  `processedEvents`, `attempts`, `mutationEvents`, each `MutationEvent`'s full
  field set, each attempt's full field set) and excludes every
  verification-only history, including `mbtAction` itself (transport
  metadata, not semantic state).
  - Validate: inspection of `mbt/model.rs` DTOs against the included/excluded
    field lists above; `grep -RnE
    "mbtAction|cursorHistory|protocolHistory|scopeHistory|abandonHistory|startHistory|recoveryHistory|taintHistory|evidenceAttempts|scopeStartCount|everTerminal"
    cli/src/services/mutation_trace/mbt/model.rs` returns no matches outside
    comments explaining the exclusion.
- [ ] AC6: at least the eight named deterministic Quint `run` scenarios
  (`testStartObservesBeforeActivation`, `testCloseObservesBeforeDeactivation`,
  `testContendedIntervalsRemainAiContended`,
  `testNoChangeHookReplayCannotStealFutureChange`,
  `testConcurrentObservationsHaveOneWinner`,
  `testTaintInvalidatesPreparedObservation`, `testRecoveryEstablishesBaseline`,
  `testClosedScopeCannotReactivate`) replay successfully through Rust via
  `#[quint_test]`, expressed as the same semantic-action call chains already
  used in the spec (no duplicated scenario logic in Rust).
  - Validate: the Nix-pinned Quint Connect check (T06) passes and the eight
    named test functions exist and are green.
- [ ] AC7: the `MbtAction` instrumentation and the `randomPrepare`
  observability change leave `verifyStep`, every listed pure action
  (`prepare`, `prepareAvailable`, `commitAttempt`, `taint`, `recover`,
  `databaseFailure`, `abandon`), invariant definitions, and existing
  deterministic runs semantically unchanged; `step`'s top-level alternatives
  remain the same eight branches (`randomMutate, randomPrepare, randomCommit,
  randomTaint, randomRecover, randomDatabaseFailure, randomAbandon, stutter`)
  rather than being replaced by four top-level prepare branches; the pure
  Quint check suite stays green.
  - Validate: `nix run .#quint -- typecheck spec/mutation_cursor.qnt`;
    `nix run .#quint -- test spec/mutation_cursor.qnt`; `nix run .#quint --
    run spec/mutation_cursor.qnt --step=verifyStep --invariants SafetyCore
    SafetyAttribution SafetyHistory --max-samples=5000 --max-steps=20`; manual
    diff of `step`'s alternative list before/after this plan's changes shows
    the same eight top-level branch names.
- [ ] AC8: CI runs the Quint Connect suite inside a Nix check that pins both
  the repository Rust toolchain and the repository Quint binary — never the
  GitHub runner's preinstalled Cargo — whenever either the Quint spec or the
  Rust refinement/driver/Cargo files change, without weakening the existing
  pure-Quint checks.
  - Validate: inspection of the new Nix check definition (`craneLib`-based,
    reusing the repository `rustToolchain`, with the Nix `quint` package
    available to the test run) and `.github/workflows/quint.yml` — the
    change-detector regex additionally matches
    `cli/src/services/mutation_trace/mbt/**`, `.../protocol.rs`,
    `.../types.rs`, `cli/Cargo.toml`, `cli/Cargo.lock` (`flake.nix`/
    `flake.lock` are already watched); a dedicated job step invokes the Nix
    check rather than stitching together the Nix Quint binary with the
    runner's own Cargo; the existing typecheck/test/randomized-safety steps
    are present and unchanged in behavior.
- [ ] AC9: no production DB/Git/filesystem/coordinator/hook code is
  introduced; the MBT harness is test-only and `protocol.rs` stays pure.
  - Validate: `grep -RnE "std::(fs|process|env)|tokio|reqwest|turso"
    cli/src/services/mutation_trace` returns no matches;
    `mutation_trace/mod.rs` gates `mod mbt;` behind `#[cfg(test)]`.
- [ ] AC10: the existing ~75 handwritten `mutation_trace` protocol tests,
  Clippy, and formatting all continue to pass unmodified.
  - Validate: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml
    mutation_trace`; `./scripts/run-cli-cargo.sh clippy --manifest-path
    cli/Cargo.toml --all-targets -- -D warnings`; `cargo fmt --manifest-path
    cli/Cargo.toml -- --check`.
- [ ] AC11: every argument-carrying `MbtAction` variant uses a record payload
  compatible with Quint Connect's current custom sum-type action decoder;
  there are no bare-scalar-payload variants used for driver dispatch.
  - Validate: inspection of `MbtAction`'s definition in
    `spec/mutation_cursor.qnt` — every variant with arguments is a record
    (`{ field: Type, ... }`, even single-field), and only truly
    argument-free variants (`MbtInit`, `MbtStutter`) are bare.
- [ ] AC12: `MbtAction` identifies the invoked semantic operation and its
  arguments, never whether that invocation changed state — a guarded/no-op
  `prepare` is still recorded as `MbtPrepare{...}`, a guarded/no-op
  `taint`/`databaseFailure`/`abandon`/`recover`/`commitAttempt` is still
  recorded as its own operation-specific variant, and `MbtStutter` is
  produced only when `step` selects the explicit top-level `stutter` action
  (never as a byproduct of another operation's internal guard branch).
  - Validate: manual trace inspection of at least the two guarded-no-op
    regressions added in T05 (see AC13) confirms the operation-specific
    variant, not `MbtStutter`, appears at the guarded step.
- [ ] AC13: at least two deterministic MBT regressions prove that a guarded
  semantic no-op still invokes the corresponding Rust kernel operation rather
  than being skipped: one `prepare` case (re-preparing an attempt that is no
  longer `Available`) and one other guarded action (`recover` when recovery
  is not needed, or `abandon` on a non-live scope).
  - Validate: the two regression scenarios in T05 pass, each proving the
    Rust driver called `protocol::prepare`/`protocol::recover` (or the
    chosen alternative) on the guarded step and independently produced the
    same no-op state Quint did.
- [ ] AC14: the `stutter` action itself, and every guarded action that used
  to fall through to it (`prepare`, `taint`, `databaseFailure`, `abandon`,
  `recover`, and any analogous guarded path in `commitAttempt`), no longer
  share a single "call `stutter`, which sets `mbtAction' = MbtStutter`"
  implementation — each guarded branch sets its own operation-specific
  `mbtAction` while still leaving all other semantic state unchanged exactly
  as the pre-existing `stutter` action did.
  - Validate: manual review of `spec/mutation_cursor.qnt`'s `prepare`,
    `taint`, `databaseFailure`, `abandon`, `recover`, and `commitAttempt`
    guarded branches (lines given in the Change summary are the pre-revision
    locations; re-check at implementation time) confirms none of them
    invokes the shared top-level `stutter` action directly; AC7's existing
    Quint checks confirm this refactor changed no semantic state assignment.

### Full validation

- `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace`
- `./scripts/run-cli-cargo.sh build --manifest-path cli/Cargo.toml`
- `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings`
- `cargo fmt --manifest-path cli/Cargo.toml -- --check`
- `nix run .#quint -- typecheck spec/mutation_cursor.qnt`
- `nix run .#quint -- test spec/mutation_cursor.qnt`
- `nix run .#quint -- run spec/mutation_cursor.qnt --step=verifyStep --invariants SafetyCore SafetyAttribution SafetyHistory --max-samples=5000 --max-steps=20`
- `nix build .#checks.<system>.mutation-trace-quint-connect` (exact attribute name confirmed in T06) — the Nix-pinned Rust+Quint Quint Connect suite
- `nix flake check` if the new check is wired into normal flake checks
- `nix run .#regenerate-cargo-sources` followed by `git diff --stat packaging/flatpak/cargo-sources.json` (expect no further diff after T01 regenerates it)

### Context sync

- `context/cli/mutation-trace-quint-connect.md` (new — see Assumptions for why
  this replaces the request's literal `context/plans/...` target)
- `context/cli/mutation-trace-protocol.md` (cross-link to the new doc if the
  existing "Target end-state architecture" section should point at it)
- `context/context-map.md` (new domain-file entry)

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** `spec/mutation_cursor.qnt` (adding `MbtAction`/`mbtAction`
  instrumentation with record payloads and operation-identity-preserving
  guarded branches, and making `randomPrepare` observable, without changing
  `step`'s top-level branch count);
  `cli/src/services/mutation_trace/mbt/{mod.rs,model.rs,driver.rs,tests.rs}`;
  the `#[cfg(test)] mod mbt;` registration in `mutation_trace/mod.rs`;
  `cli/Cargo.toml` / `cli/Cargo.lock`; `packaging/flatpak/cargo-sources.json`
  regeneration; a new Nix check pinning Rust+Quint for the MBT suite;
  `.github/workflows/quint.yml`;
  `context/cli/mutation-trace-quint-connect.md`; `context/context-map.md`.
- **Out of scope:** `store.rs`, `coordinator.rs`, `git_snapshot.rs`,
  `worktree_guard.rs`, database migrations, CAS persistence, Git object
  storage, hook wiring, Agent Trace evidence generation, runtime checkout
  materialization, filesystem locking, external-taint marker persistence;
  refactoring the CLI into a library crate; moving the existing ~75 handwritten
  protocol tests; a second, duplicated set of MBT-only scenario definitions
  when the existing deterministic `run` expressions can be reused directly.
- **Constraints:** Quint Connect is a dev-dependency only, never a production
  one; CI must use the repository's pinned Nix `quint` binary and the
  repository's pinned Nix Rust toolchain, never a globally-installed npm
  Quint or the GitHub runner's default Cargo; `prepare`, `prepareAvailable`,
  `commitAttempt`, `taint`, `recover`, `databaseFailure`, `abandon`,
  invariant definitions, verification histories, and `verifyStep` must not
  change semantics; existing Quint invariants/tests must not weaken;
  `mbtAction` must never participate in freshness, lifecycle, attribution,
  revisions, cursor movement, taint, recovery, mutation evidence, or
  invariant truth; `randomPrepare` must remain a single top-level `step`
  alternative; every argument-carrying `MbtAction` variant must be a record,
  never a bare scalar; `mbtAction` must record the invoked operation and its
  arguments on every branch of that operation, including guarded/no-op
  branches — `MbtStutter` is reserved exclusively for the explicit top-level
  `stutter` action selected by `step`, never for another operation's internal
  no-op path.
- **Non-goal:** changing protocol semantics to make Quint Connect easier to
  wire up; growing the MBT harness into a second implementation of
  `protocol.rs`'s logic; changing the randomized simulation's top-level
  action-selection distribution; collapsing a guarded operation's identity
  into `MbtStutter` for convenience.

## Assumptions

- The request's target path for the architecture write-up,
  `context/plans/mutation-cursor-quint-connect.md`, is this SCE plan's own
  file (`context/plans/{plan_name}.md`; `context/context-map.md` records
  `context/plans/` as "active plan execution artifacts, not durable history").
  Writing the architecture doc there would overwrite this plan's task-tracking
  file mid-stack. T06 instead writes it to
  `context/cli/mutation-trace-quint-connect.md`, mirroring the two existing
  sibling docs for this exact module
  (`context/cli/mutation-trace-protocol.md`,
  `context/cli/mutation-trace-revision-refinement.md`) and linking it from
  `context/context-map.md`.
- Adding `quint-connect` as a dev-dependency changes `cli/Cargo.lock`, which
  the source-built Flatpak package vendors via a checked-in, CI-guarded
  (`cargo-sources-parity`) mirror at `packaging/flatpak/cargo-sources.json`
  (per `context/sce/flatpak-distribution-patterns.md` and `flake.nix`). T01
  regenerates that file via `nix run .#regenerate-cargo-sources` even though
  the change request's own check list does not name it, because it is
  required for "existing checks... continue passing" to actually hold in CI.
- `flake.nix` already defines a pinned `rustToolchain`/`craneLib` used by the
  existing `cli-tests`/`cli-clippy`/`cli-fmt` checks (each a thin
  `craneLib.cargoTest`/`cargoClippy`/`cargoFmt` wrapper reusing shared
  `cargoArtifacts`). T06's new Quint Connect check follows this exact
  established pattern — a `craneLib.cargoTest`-style derivation scoped to
  `mutation_trace::mbt` with the Nix `quint` package added to its check
  inputs — rather than assembling a bespoke Rust+Quint environment from
  scratch.
- The exact `quint-connect` crate/package name, current compatible version,
  and its custom action/nondet `Config` API for driver dispatch are resolved
  in T01 against the upstream `quint-co/quint-connect` repository (README and
  `connect/examples/two_phase_commit/mbt.rs`, plus any example closer to a
  Choreo-style action-plus-arguments sum type) at implementation time, per
  the request's own instruction not to assume `0.1.2` or the illustrative
  `Driver`/`State<D>`/`#[quint_run]`/`#[quint_test]`/`switch!`/`Config`
  sketches are still current. This same research step confirms exactly how
  Quint Connect's custom decoder distinguishes a unit variant from a record
  variant, which is the mechanism AC11's record-payload rule depends on.
- Whether `randomPrepare`'s selected boundary needs a dedicated
  `PrepareKind`-style nondet choice, or is already fully observable from the
  `MbtPrepare { attempt, boundary }` variant `mbtAction` records regardless of
  which `any { ... }` branch fired, is decided in T03 by testing the smallest
  option first, per the request's own "prefer the smallest correct solution"
  instruction.
- As of this revision, `prepare` (spec line 448-453), `taint` (~702-711),
  `databaseFailure` (~732-737), `abandon` (~794-805), and `recover`
  (~874-886) each guard their real transition and fall through to the shared
  top-level `stutter` action on the guarded path; `commitAttempt`
  (line 455+) computes its own `fresh`/`accepted`/`changed` logic rather than
  delegating to `stutter`. T02 audits all six (not just the five that
  currently call `stutter`) for a guarded/no-op path and applies the same
  operation-identity-preserving fix wherever one exists; exact line numbers
  are re-checked at implementation time since T02/T03 edit this same file
  before T02's own guard refactor lands.
- PR #238 (`mutation-cursor`) remains the semantic base for this stacked PR.

- PR #239 (`quint-connect`) is now in active implementation. The durable task
  state in this plan is the source of truth for implementation progress:

  - T01 complete: `quint-connect` dev-dependency and packaging synchronization.
  - T02 complete: operation-preserving `MbtAction` instrumentation in Quint.
  - T03 complete: `randomPrepare` remains a single top-level `step` branch;
    no additional `PrepareKind` instrumentation was required.
  - T04 complete: Quint Connect Rust driver, comparable model-state projection,
    finite ID mapping, and the non-default
    `WT1` / `Tree3` / `Attempt5` / `Flush(WT1)` transport smoke replay.
  - T05 is the next pending task.

  Do not encode PR #239's current head SHA or statements such as
  "implementation has not started" as durable assumptions here. The branch head
  is mutable and must be fetched from GitHub when executing or reviewing a task.

  PR #239 continues to target PR #238's `mutation-cursor` branch; verify the
  current base/head relationship from GitHub whenever stack state matters.

## Task stack

- [x] T01: `Pin quint-connect as a CLI dev-dependency` (status:done)
  - Task ID: T01
  - Scope: In — confirm the current `quint-connect` crate name/version and API
    shape against the upstream README and
    `connect/examples/two_phase_commit/mbt.rs`, including its custom
    action/nondet `Config` mechanism for driver dispatch and exactly how its
    decoder distinguishes unit vs. record action variants (needed by T02 and
    T04); add it under `cli/Cargo.toml`'s `[dev-dependencies]`; update
    `cli/Cargo.lock`; regenerate `packaging/flatpak/cargo-sources.json` via
    `nix run .#regenerate-cargo-sources`. Out — any driver/model/test code;
    Quint spec changes.
  - Dependencies: none
  - Done when: `quint-connect` appears only under `[dev-dependencies]`;
    `cli/Cargo.lock` and `packaging/flatpak/cargo-sources.json` reflect the
    new dependency with no further diff after regeneration; the CLI still
    builds.
  - Verify: `./scripts/run-cli-cargo.sh build --manifest-path cli/Cargo.toml`;
    `nix run .#regenerate-cargo-sources` then `git diff --stat
    packaging/flatpak/cargo-sources.json` shows no residual diff.
  - Completed: 2026-08-27
  - Files changed: `cli/Cargo.toml`, `cli/Cargo.lock`,
    `nix/flatpak/cargo-sources.nix`, `packaging/flatpak/cargo-sources.json`
  - Result: Added `quint-connect = "0.1.2"` (confirmed current via crates.io
    API; matches the plan's placeholder guess) under a new
    `[dev-dependencies]` section in `cli/Cargo.toml` — no change under
    `[dependencies]`. `cargo build` regenerated `cli/Cargo.lock` with
    `quint-connect`, `quint-connect-macros`, and transitive deps (`itf`,
    `colored`, `similar`, `jiff`, `rand 0.9`, etc. — coexists fine with the
    crate's own `rand 0.8`). Discovered that `nix run .#regenerate-cargo-sources`
    is a no-op against a stale `Cargo.lock` unless the fixed-output
    derivation's pinned `outputHash` in `nix/flatpak/cargo-sources.nix` is
    bumped first (Nix reuses the existing store path for that hash and never
    re-invokes the generator) — confirmed this is the established convention
    via `git log`/`git show eb5e6154` (the prior Turso 0.7.0 bump did the same
    hash-then-regenerate two-step). Set a dummy `outputHash`, captured the
    real hash from the resulting Nix hash-mismatch error
    (`sha256-p8fzi7KWNltCEopHvXFmswASt9ov7UWxh/XU8mGLgH0=`), wrote that in,
    then regenerated `packaging/flatpak/cargo-sources.json` (351 added
    lines, 6 `quint-connect`/`quint-connect-macros` entries); a second
    regeneration run produced an identical file (idempotent).
    API-shape research for T02/T04 (recorded for handoff, not itself
    verified by T01's done-when): `quint-connect` 0.1.2
    (`github.com/informalsystems/quint-connect`, Apache-2.0) exposes
    `Driver`/`State<D>` traits, a `Config { state, nondet }` struct for
    locating comparable state and nondet-action paths in nested Quint state,
    and `#[quint_test(spec, test)]`/`#[quint_run(spec, max_samples, ...)]` +
    `switch!(step { Variant(args) => ... })` for dispatch. Its generic sum-type
    deserialization (`#[serde(tag = "tag", content = "value")]`) supports
    unit, newtype/tuple, and record/struct variants, so `MbtAction`'s planned
    all-record-payload design (AC11) is plausible — but neither shipped
    example (`two_phase_commit`, `tictactoe`) actually uses a record/struct
    action variant; both only exercise unit and bare-scalar tuple variants
    (e.g. `SpontaneouslyPrepares(node)`, `MoveO(coordinate)`) via `switch!`.
    Separately, the crate's `nondet`-path extraction (`extract_nondet_from_sum_type`
    in `connect/src/trace/mod.rs`) specifically requires the picked value to
    deserialize as a `Record` (or an empty tuple) — flag this for T02/T03 to
    verify directly against a record-payload `MbtAction` before relying on it,
    since it wasn't directly evidenced upstream.
  - Verify outcomes: `./scripts/run-cli-cargo.sh build --manifest-path
    cli/Cargo.toml` — passed (`Finished dev profile`); `nix run
    .#regenerate-cargo-sources` then `git diff --stat
    packaging/flatpak/cargo-sources.json` — passed, shows the new-dependency
    diff with no further change on a second run; `nix build
    .#checks.x86_64-linux.cargo-sources-parity` — passed (no diff reported).
  - Context impact: Root context synchronized.

    Adding `quint-connect` introduced the CLI's first dev-only dependency, so
    the dependency baseline references in:

    - `context/overview.md`
    - `context/architecture.md`
    - `context/glossary.md`

    were updated to distinguish production dependencies from the new
    dev/test-only `quint-connect` dependency.

    Additionally, `context/sce/flatpak-distribution-patterns.md` was updated
    with the fixed-output-hash regeneration procedure discovered while
    refreshing `packaging/flatpak/cargo-sources.json`.

    These are documentation/context synchronization changes only; they
    introduce no new runtime architecture or production interface.
  - Context synchronization: synced

- [x] T02: `Add operation-preserving MBT action-transport instrumentation to the spec` (status:done)
  - Task ID: T02
  - Scope: In — `spec/mutation_cursor.qnt`: define a verification-only
    `MbtAction` sum type where every argument-carrying variant is a record
    payload, even single-field ones (`MbtInit`, `MbtMutate({worktree,
    tree})`, `MbtPrepare({attempt, boundary})`, `MbtCommit({attempt})`,
    `MbtTaint({worktree})`, `MbtDatabaseFailure({worktree})`,
    `MbtAbandon({scope})`, `MbtRecover({worktree})`, and unit `MbtStutter`),
    matching the exact unit-vs-record shape Quint Connect's current custom
    action decoder expects (confirmed in T01); a verification-only
    `mbtAction: MbtAction` state variable, never read by or participating in
    any other rule. For `prepare`, `taint`, `databaseFailure`, `abandon`,
    and `recover` — every one of which currently falls through to the shared
    top-level `stutter` action on its guarded path — and for `commitAttempt`
    if it has an analogous no-op path: refactor so the guarded/no-op branch
    still sets `mbtAction'` to that operation's own variant with its real
    arguments (e.g. `MbtPrepare({attempt, boundary})`) while leaving every
    other semantic state assignment exactly as `stutter` already produces it
    — do not let the guarded branch call the shared top-level `stutter`
    action directly, since that would overwrite `mbtAction'` with
    `MbtStutter` and erase which operation was invoked. Only the explicit
    top-level `stutter` action (the one `step` itself can select) sets
    `mbtAction' = MbtStutter`. Out — driver/Rust code; `randomPrepare`/`step`
    structure (T03); the actual transition logic of `prepare`,
    `prepareAvailable`, `commitAttempt`, `taint`, `recover`,
    `databaseFailure`, `abandon`, invariant definitions, verification
    histories, `verifyStep`.
  - Dependencies: T01
  - Done when: `mbtAction` exists purely as instrumentation with the
    record-payload shape above; every guarded/no-op invocation of `prepare`,
    `taint`, `databaseFailure`, `abandon`, `recover` (and `commitAttempt` if
    applicable) records its own operation-specific `MbtAction` rather than
    `MbtStutter`; `MbtStutter` is reachable only from the explicit top-level
    `stutter` action; none of this affects invariant truth, lifecycle,
    attribution, revisions, cursor movement, taint, recovery, or mutation
    evidence; typecheck, the Quint test suite, and the randomized
    `verifyStep` safety run all stay green with unchanged invariant outcomes.
  - Verify: `nix run .#quint -- typecheck spec/mutation_cursor.qnt`;
    `nix run .#quint -- test spec/mutation_cursor.qnt`; `nix run .#quint --
    run spec/mutation_cursor.qnt --step=verifyStep --invariants SafetyCore
    SafetyAttribution SafetyHistory --max-samples=5000 --max-steps=20`; manual
    diff of a known deterministic run's non-`mbtAction` semantic-variable
    trace before/after this task, confirming no divergence; manual trace
    inspection of one guarded `prepare` call and one guarded `recover` (or
    `abandon`) call, confirming their operation-specific `MbtAction` — not
    `MbtStutter` — is recorded.
  - Completed: 2026-08-27
  - Files changed: `spec/mutation_cursor.qnt`
  - Result: Added a verification-only `MbtAction` sum type (all argument-
    carrying variants are records, even single-field ones — `MbtInit`,
    `MbtMutate({worktree,tree})`, `MbtPrepare({attempt,boundary})`,
    `MbtCommit({attempt})`, `MbtTaint({worktree})`,
    `MbtDatabaseFailure({worktree})`, `MbtAbandon({scope})`,
    `MbtRecover({worktree})`, unit `MbtStutter`) and a new `mbtAction:
    MbtAction` state variable, assigned in every action, read by nothing else.
    Replaced `stutter`'s inline field list with a new shared
    `mbtStutterAs(taken: MbtAction): bool` action — identical to the old
    `stutter` body except it takes the `MbtAction` to record as a parameter;
    `stutter` itself is now `mbtStutterAs(MbtStutter)`. Audited all six
    candidate actions per the plan's Assumptions: `prepare`, `taint`,
    `databaseFailure`, `abandon`, and `recover` each had a guarded/no-op
    branch that called the shared top-level `stutter` directly — each now
    calls `mbtStutterAs(<its own MbtVariant>(...))` instead, so the guarded
    branch never overwrites `mbtAction'` with `MbtStutter`.  `commitAttempt`
    does have an analogous no-op path (the existing `not(accepted)` branch,
    which never called `stutter` — it already inlined its own field list) —
    both its not-accepted and accepted branches now additionally set
    `mbtAction' = MbtCommit({attempt: attempt})`. No other field assignment
    in any action changed (confirmed by `git diff`: every non-`mbtAction'`
    line is unchanged context). `MbtStutter` is now reachable only from the
    explicit top-level `stutter` action. `step`, `randomPrepare`, and every
    other `random*` action were not touched (out of scope, deferred to T03).
    A parameter named `action` in the new shared helper collided with the
    `action` keyword and had to be renamed to `taken` — not itself a design
    decision, just a naming fix required to typecheck.
  - Verify outcomes: `nix run .#quint -- typecheck spec/mutation_cursor.qnt`
    — passed, no errors. `nix run .#quint -- test spec/mutation_cursor.qnt`
    — passed (exit 0, all existing `run` scenarios including the 8 named
    deterministic ones still pass). `nix run .#quint -- run
    spec/mutation_cursor.qnt --step=verifyStep --invariants SafetyCore
    SafetyAttribution SafetyHistory --max-samples=5000 --max-steps=20` —
    "[ok] No violation found" (5000 traces, up to 21 steps). Manual diff of
    the non-`mbtAction` semantic-variable assignments before/after this task
    — `git diff spec/mutation_cursor.qnt` shows only added `mbtAction'`
    lines and `stutter` → `mbtStutterAs(...)` call-site substitutions; no
    other variable's assignment expression changed anywhere in the file, so
    there is no divergence to check trace-by-trace. Manual trace inspection
    of a guarded `prepare` (re-preparing `Attempt0` after it was already
    committed), a guarded `recover` (on `WT0` with no taint/rebaseline
    need), and a guarded `abandon` (on `Scope0` while still `NeverSeen`) —
    added as temporary `run` scenarios appended to the spec, run with `quint
    test --match 'tempTest.*'`, all 4 passed (including a control case
    confirming the explicit top-level `stutter` action still records
    `MbtStutter`), then removed before this commit; the working tree carries
    only the permanent instrumentation, not the temporary scenarios.
  - Context impact: none. Only `spec/mutation_cursor.qnt` changed, and the
    change is purely additive/internal to the spec (a verification-only type,
    state variable, and guarded-branch instrumentation never read by any
    other action, invariant, or Rust code — no driver exists yet). It does
    not alter the pure Rust refinement `context/cli/mutation-trace-protocol.md`
    describes (that doc covers `prepare`/`commit`/etc. semantics, which this
    task explicitly left unchanged and verified unchanged), and does not touch
    the production-vs-dev dependency baseline `context/overview.md`,
    `context/architecture.md`, and `context/glossary.md` already record for
    the in-progress `mutation-cursor-quint-connect` harness (from T01). The
    corrected-pipeline architecture write-up is explicitly deferred to T06 by
    this plan's own scope, not skipped here.
  - Context synchronization: synced

- [x] T03: `Keep randomPrepare a single step alternative while making it Connect-observable` (status:done)
  - Task ID: T03
  - Scope: In — `spec/mutation_cursor.qnt`'s `randomPrepare`/`step` only:
    keep `randomPrepare` as one `step` branch (no four-way top-level split);
    using T02's `mbtAction` output, determine whether the concrete selected
    boundary is already fully visible to Quint Connect via the recorded
    `MbtPrepare{attempt,boundary}` value, and add the smallest additional
    instrumentation (e.g. a `PrepareKind`-style nondet choice, recorded
    alongside `mbtAction` for observability only) only if that alone proves
    insufficient. Out — the `MbtAction` type definition and guarded-branch
    fix (T02, already done); driver code; any change to
    `prepare`/`prepareAvailable`/`commitAttempt`/invariants/`verifyStep`.
  - Dependencies: T02
  - Done when: `step`'s top-level alternatives are structurally the same
    eight branches as the pre-existing baseline (`randomMutate, randomPrepare,
    randomCommit, randomTaint, randomRecover, randomDatabaseFailure,
    randomAbandon, stutter`); all four prepare boundary kinds remain reachable
    from `randomPrepare`; the concrete boundary Quint selected for any given
    `randomPrepare` firing is recoverable by the driver.
  - Verify: `nix run .#quint -- typecheck spec/mutation_cursor.qnt`;
    `nix run .#quint -- test spec/mutation_cursor.qnt`; `nix run .#quint --
    run spec/mutation_cursor.qnt --step=verifyStep --invariants SafetyCore
    SafetyAttribution SafetyHistory --max-samples=5000 --max-steps=20`; a
    manual diff confirming `step`'s alternative list is unchanged from the
    pre-refactor baseline (same eight names, no new top-level branches).
  - Completed: 2026-08-27
  - Files changed: `spec/mutation_cursor.qnt`
  - Result: Confirmed the smallest option (no structural change) is already
    sufficient — no code change to `randomPrepare` or `step`. `prepare`
    (T02) unconditionally sets `mbtAction' = MbtPrepare({attempt, boundary})`
    on both its `prepareAvailable` path and its guarded `mbtStutterAs(...)`
    path, and `boundary` there is always the exact concrete `Boundary` value
    passed to whichever of `randomPrepare`'s five inner `any` alternatives
    fired (`Start`/`Advance`/`Close` with concrete `scope`/`event`, or
    `Flush` with a concrete `WorktreeId`). This means the driver, dispatching
    on the `MbtAction::MbtPrepare` variant, already receives which boundary
    kind was selected and its full concrete arguments — no dedicated
    `PrepareKind`-style nondet choice is needed. T01's flagged open question
    about Quint Connect's `extract_nondet_from_sum_type` requiring a
    `Record`-shaped value applies to the top-level nondet-picked action type
    (`MbtAction`, already all-record per AC11/T02), not to `Boundary` as a
    nested field inside `MbtPrepare`'s record — decoding a nested field with
    mixed record/bare-scalar variants (`Flush(WorktreeId)`) is an ordinary
    serde adjacently-tagged-enum concern for T04's DTOs, unrelated to
    Connect's nondet-extraction mechanism. Added a five-line comment above
    `randomPrepare` documenting this conclusion (why it stays a single
    branch with no extra instrumentation) so a future change doesn't
    reintroduce a four-way split or redundant `PrepareKind` field. `step`'s
    eight top-level alternatives and `randomPrepare`'s inner `any` block are
    otherwise byte-for-byte unchanged (confirmed by `git diff`).
  - Verify outcomes: `nix run .#quint -- typecheck spec/mutation_cursor.qnt`
    — passed (exit 0, no errors). `nix run .#quint -- test
    spec/mutation_cursor.qnt` — passed (exit 0). `nix run .#quint -- run
    spec/mutation_cursor.qnt --step=verifyStep --invariants SafetyCore
    SafetyAttribution SafetyHistory --max-samples=5000 --max-steps=20` —
    "[ok] No violation found" (5000 traces, up to 21 steps). Manual diff
    (`git diff spec/mutation_cursor.qnt`) confirms the only change is the
    added comment; `step`'s alternative list and `randomPrepare`'s body are
    unchanged from the pre-task baseline.
  - Context impact: none. Only `spec/mutation_cursor.qnt` changed, and the
    change is a documentation-only comment above `randomPrepare` recording a
    design conclusion — no state variable, action, invariant, or semantic
    behavior was added or altered. `step`'s structure, `mbtAction`'s shape,
    and every action's transition logic are exactly as T02 left them. No
    driver or Rust code exists yet (T04), so there is nothing downstream to
    resynchronize.
  - Context synchronization: synced

- [x] T04: `Build the MBT driver, ID mapping, and comparable model state` (status:done)
  - Task ID: T04
  - Scope: In —
    `cli/src/services/mutation_trace/mbt/{mod.rs,model.rs,driver.rs}`;
    `#[cfg(test)] mod mbt;` registration in `mutation_trace/mod.rs`;
    `MutationCursorDriver` (`protocol: ProtocolState` +
    `worktree_trees: BTreeMap<WorktreeId, TreeId>`); Quint Connect's custom
    action/nondet configuration wired so the driver dispatches on `MbtAction`
    variants (never on before/after state diffing); exact-Quint-`init` state
    construction (both worktrees, all four scopes, all six attempts, matching
    the request's literal initial values); the finite
    WT/Scope/Tree/Event/Attempt ID mapping; the full `MbtAction` →
    `protocol::*` call mapping (`MbtInit`, `MbtMutate` touching only
    `worktree_trees`, `MbtPrepare`, `MbtCommit`, `MbtTaint`,
    `MbtDatabaseFailure`, `MbtAbandon`, `MbtRecover`, `MbtStutter` as a
    no-op) — every arm unconditionally calling its `protocol::*` function
    with the transported arguments, including when the Quint side is
    expected to stutter, since replaying the guarded call and comparing the
    resulting no-op state against Quint is the point of the regressions in
    T05; `ModelState`/`MutationEvent`/`Attempt` comparable DTOs
    (`BTreeMap`/`BTreeSet`) covering worktrees/scopes/worktreeTrees/
    externalTaint/processedEvents/attempts/mutationEvents and excluding
    `mbtAction` plus every other verification-only history;
    `impl State<MutationCursorDriver> for ModelState`; one deterministic
    `#[quint_test]` smoke replay in `mbt/tests.rs` built from non-default
    values (`mutate(WT1, Tree3)` → `prepare(Attempt5, Flush(WT1))` →
    `commitAttempt(Attempt5)`) that demonstrably transports `WT1`, `Tree3`,
    `Attempt5`, and `Flush(WT1)` from the Quint trace into the real
    `protocol::prepare`/`protocol::commit` calls, proving the driver never
    guesses arguments from defaults. Out — the remaining deterministic
    scenario replays, the two guarded-no-op regressions, and the generated
    simulation (T05).
  - Dependencies: T01, T02, T03
  - Done when: the driver never reimplements protocol semantics, never
    infers action arguments from state, and never skips a `protocol::*` call
    because the expected Quint state is unchanged; the comparable state
    matches AC5; the WT1/Tree3/Attempt5 smoke scenario passes and is
    inspectable as proof of real argument transport.
  - Verify: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml
    mutation_trace::mbt` with the Nix `quint` binary on `PATH` (interim, ahead
    of T06's dedicated check); `./scripts/run-cli-cargo.sh clippy
    --manifest-path cli/Cargo.toml --all-targets -- -D warnings`; `cargo fmt
    --manifest-path cli/Cargo.toml -- --check`.
  - Completed: 2026-08-27
  - Files changed: `spec/mutation_cursor.qnt`,
    `cli/src/services/mutation_trace/mod.rs`,
    `cli/src/services/mutation_trace/mbt/mod.rs` (new),
    `cli/src/services/mutation_trace/mbt/model.rs` (new),
    `cli/src/services/mutation_trace/mbt/driver.rs` (new),
    `cli/src/services/mutation_trace/mbt/tests.rs` (new)
  - Result: Added `#[cfg(test)] mod mbt;` to `mutation_trace/mod.rs`. Built
    the `mbt` submodule: `model.rs` defines ITF-wire mirror types (`Wire*`)
    for every Quint identity/enum/record type reachable from the comparable
    state — confirmed against the vendored `quint-connect` 0.1.2 and `itf`
    0.4.0 crate sources (available locally via the Nix store, since both are
    dev-dependencies) that sum types deserialize as `{tag, value}` via
    `#[serde(tag = "tag", content = "value")]` (README "Enums" section),
    that unit-only sum types need no `content` attribute, and that
    `quint-connect`'s nondet-pick extraction (`extract_nondet_from_sum_type`)
    accepts a `Value::Record` directly — resolving T01's flagged uncertainty
    about record-payload `MbtAction` variants: they work exactly as AC11
    designed, no fallback needed. Each `Wire*` type converts via `From` into
    this crate's existing domain types (`types.rs`, untouched). `ModelState`
    is `#[serde(from = "WireModelState")]`-deserializable and holds exactly
    AC5's field list (`worktrees`, `scopes`, `worktree_trees`,
    `external_taint`, `processed_events`, `attempts`, `mutation_events`);
    `mbtAction` has no field anywhere in the wire types, so it is silently
    dropped by serde's default unknown-field handling when the full
    top-level state record deserializes — the mechanism that keeps it out of
    the compared state. `driver.rs` defines `MutationCursorDriver { protocol:
    ProtocolState, worktree_trees: BTreeMap<WorktreeId, TreeId> }`, an
    `init()` matching Quint's `init` exactly (both worktrees Tree0/rev0/
    healthy, all four scopes `NeverSeen` via `scopeActor`'s fixed partition,
    all six attempts `Available`/`Flush(WT0)`/rev0/Tree0/Tree0), and
    `Driver::step` dispatching via `switch!` on every `MbtAction` variant
    (`Config { nondet: &["mbtAction"], state: &[] }`, confirmed correct
    against `quint-connect`'s `extract_from_sum_type` path since `mbtAction`
    is a plain top-level var, not Quint's builtin `mbt::actionTaken`). Every
    arm unconditionally calls its `protocol::*` function with the
    transported, converted arguments (`MbtMutate` touches only
    `worktree_trees`; `MbtStutter` calls a dedicated no-op `mbt_stutter`
    method rather than being inlined, so it isn't a bare `()` statement);
    `boundary_worktree` (already `pub` in `types.rs`) resolves a
    `prepare`/`recover` boundary's worktree from the driver's own `scopes`
    map, which always agrees with Quint's static `scopeWorktree` partition
    since both are seeded identically at `init` and a scope's `worktree_id`
    never changes afterward. Added the one non-default-values smoke scenario
    the task specifies as a new named `run` in `spec/mutation_cursor.qnt`
    (`testMbtDriverTransportsNonDefaultArguments`, appended after the last
    existing `run`) — required because `#[quint_test]` replays a
    spec-defined `run` by name, and no existing named scenario used this
    exact `mutate(WT1, Tree3)` → `prepare(Attempt5, Flush(WT1))` →
    `commitAttempt(Attempt5)` chain; this one small, task-specified addition
    was necessary to satisfy AC3/the task's own Done-when text, not a scope
    expansion. `mbt/tests.rs` wires it via `#[quint_test(spec =
    "../spec/mutation_cursor.qnt", test =
    "testMbtDriverTransportsNonDefaultArguments")]` (relative to `cli/`,
    `cargo test`'s working directory). Deviations from the gate's Approach:
    none material — `mbt_commit`/`mbt_taint`/`mbt_database_failure`/
    `mbt_abandon`/`mbt_recover` take `&AttemptId`/`&WorktreeId`/`&ScopeId`
    references rather than owned values (clippy `needless_pass_by_value`,
    since they only borrow); `BTreeSet::new()` used in place of
    `Default::default()` (clippy `default_trait_access`); both are
    ordinary, reversible local implementation choices.
  - Verify outcomes: `./scripts/run-cli-cargo.sh test --manifest-path
    cli/Cargo.toml mutation_trace::mbt` (Nix `quint` 0.32.0 on `PATH`) —
    passed: `mutation_cursor_transports_non_default_arguments` generated and
    replayed 100 traces of the named scenario, `[OK]`, `1 passed; 0 failed`;
    running the full `mutation_trace` suite together
    (`./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml
    mutation_trace`) shows `76 passed; 0 failed` — the pre-existing ~75
    handwritten tests plus this one new MBT test, confirming no regression.
    `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml
    --all-targets -- -D warnings` — passed after fixing doc-markdown,
    default-trait-access, needless-pass-by-value, unused-self, and a
    macro-generated `no_effect` lint (the original bare `MbtStutter => ()`
    case expanded to a `();` statement inside `switch!`; replaced with an
    explicit `mbt_stutter` method call). `cargo fmt --manifest-path
    cli/Cargo.toml -- --check` — passed (ran `cargo fmt` once to fix import
    ordering/line-wrap, then the check was clean). Additionally (not in this
    task's own Verify list, but touched `spec/mutation_cursor.qnt`): `nix
    run .#quint -- typecheck spec/mutation_cursor.qnt` passed; `nix run
    .#quint -- test spec/mutation_cursor.qnt --match
    '^testMbtDriverTransportsNonDefaultArguments$'` passed (`1 passing`);
    `nix run .#quint -- test spec/mutation_cursor.qnt` (full suite, no
    `--match`) exited 0 with no failure output. `grep -RnE
    "std::(fs|process|env)|tokio|reqwest|turso" cli/src/services/mutation_trace`
    — no matches (AC9); `mod mbt;` confirmed behind `#[cfg(test)]` in
    `mutation_trace/mod.rs`.
  - Context impact: none. Only test-only files changed
    (`cli/src/services/mutation_trace/mbt/*`, gated behind `#[cfg(test)]`)
    plus one new named `run` scenario in `spec/mutation_cursor.qnt` (no
    state/action/invariant change). No production dependency, public
    interface, CLI surface, or architecture changed; `protocol.rs`/
    `types.rs` are unmodified and untouched by this task, so
    `context/cli/mutation-trace-protocol.md` still accurately describes
    them. The corrected-pipeline architecture write-up
    (`context/cli/mutation-trace-quint-connect.md`) remains explicitly
    deferred to T06 by this plan's own scope, as T02/T03 already noted.
  - Context synchronization: synced

- [ ] T05: `Wire deterministic scenario replays, guarded-no-op regressions, and the generated Quint Connect simulation` (status:todo)
  - Task ID: T05
  - Scope: In — `mbt/tests.rs`: `#[quint_test]` functions for the remaining
    named scenarios (`testCloseObservesBeforeDeactivation`,
    `testContendedIntervalsRemainAiContended`,
    `testNoChangeHookReplayCannotStealFutureChange`,
    `testConcurrentObservationsHaveOneWinner`,
    `testTaintInvalidatesPreparedObservation`,
    `testRecoveryEstablishesBaseline`, `testClosedScopeCannotReactivate`), and
    any other existing `run test...` declaration Quint Connect can wire
    without duplicating scenario logic in Rust; two new deterministic
    guarded-no-op regressions proving `MbtAction` transport survives a
    guarded operation: (1) `init.then(prepare(Attempt0,
    Start(...))).then(prepare(Attempt0, Advance(...)))`, where the second
    `prepare` call guards because `Attempt0` is no longer `Available`,
    asserting the driver still calls `protocol::prepare` a second time and
    independently reaches the same no-op outcome; (2) one non-`prepare`
    guarded case — `recover` on a worktree that does not need recovery —
    asserting the driver still calls `protocol::recover` and independently
    reaches the same no-op outcome; `mutation_cursor_generated_traces_refine_rust_protocol`
    (`#[quint_run(max_samples = 500, max_steps = 30)]`) comparing `ModelState`
    after every generated step, using the same `MbtAction` transport as the
    deterministic runs; `QUINT_SEED` reproduction confirmed. Out —
    driver/model changes (T04, already done); CI wiring and documentation
    (T06).
  - Dependencies: T04
  - Done when: all eight named scenarios (plus any further existing ones
    wired) pass through Rust via `#[quint_test]`, each comparing the same
    fields as T04's smoke replay; both guarded-no-op regressions pass,
    proving the Rust driver called the corresponding `protocol::*` function
    on the guarded step rather than skipping it; the generated simulation
    passes at the configured sample/step budget; re-running it with an
    explicit fixed `QUINT_SEED=<value>` reproduces the same outcome twice.
  - Verify: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml
    mutation_trace::mbt` with the Nix `quint` binary on `PATH`; the generated
    simulation test re-run twice with the same explicit `QUINT_SEED=<value>`,
    comparing outcomes.
  - Context synchronization: pending

- [ ] T06: `Add a Nix-pinned Rust+Quint CI check and document the architecture` (status:todo)
  - Task ID: T06
  - Scope: In — a dedicated Nix check (e.g. `mutation-trace-quint-connect`)
    following the existing `cli-tests`/`cli-clippy`/`cli-fmt` `craneLib`
    pattern in `flake.nix` — reusing the repository's pinned `rustToolchain`/
    `craneLib`/`cargoArtifacts` and adding the Nix `quint` package as a check
    input — that runs `cargo test --manifest-path cli/Cargo.toml
    mutation_trace::mbt`, reusing the existing CLI generated-input mechanism
    (`scripts/produce-cli-generated-input.sh` / the `cliGeneratedInput` Nix
    derivation) rather than bypassing it, and prints `rustc --version` /
    `cargo --version` / `quint --version` at least while stabilizing the
    check; `.github/workflows/quint.yml` updated to invoke that check instead
    of stitching together the Nix Quint binary with the runner's own Cargo,
    with its change-detector regex extended to also match
    `cli/src/services/mutation_trace/mbt/**`, `.../protocol.rs`,
    `.../types.rs`, `cli/Cargo.toml`, `cli/Cargo.lock` (`flake.nix`/
    `flake.lock` are already watched, so no change needed there);
    `context/cli/mutation-trace-quint-connect.md` documenting the corrected
    architecture (semantic action → `mbtAction` transport → Quint Connect →
    Rust driver), the compared/excluded fields (including why `mbtAction` is
    excluded), the `randomPrepare` single-branch decision, the
    operation-identity-vs-stutter distinction and why it matters (with the
    guarded-no-op regressions as the proof), ID mapping, the `u64` revision
    limitation, the generated-simulation configuration, deterministic runs
    wired, seed reproduction, the Nix-pinned CI command, and non-goals — see
    Assumptions for why this replaces the request's literal
    `context/plans/...` target; a new entry in `context/context-map.md`.
    Out — any change to the existing pure-Quint typecheck/test/
    randomized-safety steps beyond the detector's watched-path list.
  - Dependencies: T05
  - Done when: the new Nix check runs the MBT suite under repository-pinned
    Rust and Quint, printing their versions at least while stabilizing;
    `.github/workflows/quint.yml` invokes it without installing a separate
    Rust toolchain for this job; the change-detector triggers on Rust-only
    `mutation_trace` changes as well as spec changes; the context doc exists,
    covers the required topics, and is linked from `context/context-map.md`.
  - Verify: `nix build .#checks.<system>.mutation-trace-quint-connect` (or
    `nix flake check` if wired into normal checks) run locally; manual diff
    review of `.github/workflows/quint.yml`; `cat
    context/cli/mutation-trace-quint-connect.md`; the plan's full `Full
    validation` command list run end-to-end.
  - Context synchronization: pending

## Open questions

None. The genuine ambiguities found while planning — the requested
architecture-doc path colliding with this plan's own file, the exact
`quint-connect` version/custom-action API, and whether `commitAttempt` has a
guarded no-op path analogous to the other five actions — resolve cleanly by
repository convention and by deferring live verification to T01/T02, both
recorded under Assumptions rather than blocking authoring, since none of them
changes scope or acceptance criteria. Whether `randomPrepare`'s boundary
needs a dedicated `PrepareKind` beyond `mbtAction` is an implementation-time
decision scoped explicitly into T03's Done-when criteria, not a
planning-time unknown.
