# Plan: mutation-cursor-protocol-kernel

## Change summary

Establish a pure, dependency-free Rust refinement of the verified `spec/mutation_cursor.qnt`
protocol under a new `cli/src/services/mutation_trace` module, split as `mod.rs` (public module
boundary), `types.rs` (state/domain types), `protocol.rs` (pure transition logic), and
`tests.rs`. The module represents the protocol's state as an explicit `ProtocolState` aggregate
(`worktrees`, `scopes`, `external_taint`, `processed_events`, `attempts`, `mutation_events`) over
the existing leaf types (`WorktreeState`, `ScopeState`, `AttemptState`), its pure transitions
(prepare/commit evaluation for `Start`/`Advance`/`Close`/`Flush` boundaries, attribution
derivation, snapshot-failure taint, database-failure external taint, scope abandonment, and
recovery — the last two taking the currently observed tree as an explicit input rather than
reading Git themselves), and its result/attribution/mutation-event types, with deterministic
tests that mirror the spec's invariants. This is new behavior: no Rust implementation of this
protocol exists today, and the module performs no Git, database, filesystem, environment,
network, or lock I/O. `coordinator.rs`, `git_snapshot.rs`, and `store.rs` — the imperative-shell
orchestration, isolated Git snapshot capture, and DB-backed CAS persistence seams in the target
end-state architecture — are acknowledged as the layout the protocol module will grow into, but
are not created in this PR; `protocol.rs` is not wired into any existing hook, command, or
database call site. That integration is explicitly out of scope and left for a later plan.

This revision reshapes the T02-T07 task stack after a review of the first pass: it makes the
`accepted`/`observes`/`observedChange`/`changed`/`advancesRevision` distinction in `commitAttempt`
explicit per task (they are not equivalent — an accepted-but-non-observing hook still advances
the revision without moving the cursor), moves all four boundary kinds' commit evaluation
(including `Flush`) into one task instead of splitting `Flush` semantics across tasks, adds the
`externalTaint` freshness guard explicitly, requires attribution/mutation-event materialization
to use the *pre-transition* live-scope set exactly as `commitAttempt` computes it (before
`nextScope` is applied), and replaces "at least three" multi-action sequence tests with a
requirement to cover every named scenario. It does not change the module's scope, file layout, or
non-goals.

## Acceptance criteria

- [ ] AC1: The mutation-cursor protocol module has an explicit Rust home under
      `cli/src/services/mutation_trace` with zero Git/DB/filesystem/environment/network/
      async/lock I/O in its pure transition logic, and operates over an explicit `ProtocolState`
      aggregate (`worktrees`/`scopes`/`external_taint`/`processed_events`/`attempts`/
      `mutation_events`) rather than free-floating leaf values.
  - Validate: `grep -RnE "std::(fs|process|env)|tokio|reqwest|turso" cli/src/services/mutation_trace` returns nothing; manual inspection of imports and the `ProtocolState` type.
- [ ] AC2: `Start`/`Advance`/`Close` hook boundaries and the non-hook `Flush` boundary compute
      `accepted`/`observes`/`observedChange`/`changed`/`advancesRevision` and transition scope
      status and worktree cursor/revision exactly as `commitAttempt` specifies
      (`spec/mutation_cursor.qnt:455-661`), including CAS freshness rejection
      (`expectedRevision`/`beforeTree` mismatch), the `externalTaint` freshness guard, and replay
      rejection via processed `EventKey`s. An accepted-but-non-observing hook (for example a
      fresh `Start` on an already-`Active` scope, or an invalid `Advance`/`Close`) still advances
      the revision and records the event as processed while the cursor and scope remain
      unchanged; `Flush`'s `advancesRevision` requires `observedChange`, unlike hook boundaries
      whose `advancesRevision` follows from `accepted` alone. `prepare` takes the currently
      observed tree as an explicit input parameter rather than obtaining it itself.
  - Validate: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace`
- [ ] AC3: Attribution (`IneligibleUnscoped`/`AiExclusive`/`AiContended`,
      `spec/mutation_cursor.qnt:285-301`) and mutation-event emission match `commitAttempt`'s
      `changed` gate exactly (`observedChange and not needsRebaseline`), computed from the
      *pre-transition* live-scope set exactly as `commitAttempt` computes `live`/`attribution`
      before applying `nextScope` — a `Start` boundary's emitted event never attributes the
      mutation to the scope it is about to activate, and a `Close` boundary's emitted event still
      attributes to the scope it is about to close — including the `Flush` boundary and
      failure/taint/`needsRebaseline` attribution overrides.
  - Validate: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace`
- [ ] AC4: Snapshot-failure taint (`taintHealthy`/`taint`, `spec/mutation_cursor.qnt:663-710`)
      changes only `tainted`/`failureKind`/`revision`; database failure
      (`recordDatabaseFailure`/`databaseFailure`, `spec/mutation_cursor.qnt:712-737`) changes
      only `externalTaint`. Neither ever changes the cursor.
  - Validate: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace`
- [ ] AC5: Abandonment (`abandonLiveScope`/`abandon`, `spec/mutation_cursor.qnt:739-805`) is
      terminal, sets `needsRebaseline`, never moves the cursor, and preserves the scope's
      `actor_kind` and `worktree_id` (scope identity stability); a terminal scope can never be
      reactivated or abandoned again, and abandoning a `NeverSeen`, `Closed`, or `Abandoned`
      (non-live) scope is a no-op.
  - Validate: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace`
- [ ] AC6: Recovery (`recoverNeeded`/`recover`, `spec/mutation_cursor.qnt:807-886`), given the
      currently observed tree as an explicit input rather than reading Git itself, re-baselines
      the cursor to that observed tree and clears taint/`needsRebaseline`/`externalTaint`,
      abandoning live scopes only on the taint/`externalTaint` recovery path and preserving them
      on the `needsRebaseline`-only path.
  - Validate: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace`
- [ ] AC7: No rejected or stale attempt ever advances the revision, moves the cursor, or emits
      mutation evidence, across multi-action sequences.
  - Validate: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace` (sequence/invariant tests from T07)
- [ ] AC8: The formal specification stays untouched and green, and no existing production code
      path calls the new module.
  - Validate: `git diff --stat spec/mutation_cursor.qnt` is empty; `grep -rn "mutation_trace" cli/src/services/hooks cli/src/services/agent_trace.rs` finds no call sites; `nix run .#quint -- typecheck spec/mutation_cursor.qnt && nix run .#quint -- test spec/mutation_cursor.qnt`

### Full validation

- `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml`
- `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings`
- `cargo fmt --manifest-path cli/Cargo.toml -- --check`
- `nix run .#quint -- typecheck spec/mutation_cursor.qnt`
- `nix run .#quint -- test spec/mutation_cursor.qnt`

### Context sync

- `context/context-map.md` (new domain-file entry for the mutation-cursor protocol module)
- `context/cli/mutation-trace-protocol.md` (new domain file: `mutation_trace` module
  responsibility and file layout, Quint refinement scope, explicit "not yet wired into
  production" status, and the target end-state directory layout — `coordinator.rs`,
  `git_snapshot.rs`, `store.rs` as the seams later PRs will fill in — so later plans have a
  recorded architecture to build against instead of rediscovering it)
- `context/overview.md` (brief mention: new pure protocol module exists, not yet integrated)

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition** beside the
  status. Never infer `synced` from conversation history; write every lifecycle transition to
  the plan file.

## Constraints and non-goals

- **In scope:** `cli/src/services/mutation_trace/` (new module: `mod.rs`, `types.rs`,
  `protocol.rs`, `tests.rs`), `cli/src/services/mod.rs` (module registration only).
- **Out of scope:** `spec/mutation_cursor.qnt` / `spec/mutation_cursor.md` (read-only,
  authoritative — no edits); `cli/src/services/agent_trace.rs`,
  `cli/src/services/agent_trace_db/`, `cli/src/services/hooks/**` (no behavior changes — these
  remain untouched); `coordinator.rs`, `git_snapshot.rs`, `store.rs` (the imperative-shell
  orchestration, isolated Git snapshot capture, and DB-backed CAS persistence seams of the
  target architecture — acknowledged in context but not created); any Git, SQLite, filesystem,
  or hook wiring; a Quint-driven property-based testing harness; the `SCE_LAST_ACCEPTED_COMMIT`
  file or any cursor-persistence adapter.
- **Constraints:** no new Cargo dependencies (`cli/Cargo.toml` has no `proptest`/`quickcheck`
  today, and the request itself discourages adding dependency churn for this PR); the protocol
  core stays synchronous with no `tokio`, `Arc<Mutex<_>>`, `RwLock<_>`, or file locks; follow
  repository domain-type and test-module conventions (see `cli/src/services/patch.rs`: plain
  enums/structs, `#[cfg(test)] mod tests;` sibling file) and the `#[allow(dead_code)]`
  precedent for modules not yet consumed by production call sites (`agent_trace_export`,
  `bash_policy`, `repository_identity` in `cli/src/services/mod.rs`); `types.rs` and
  `protocol.rs` stay free of any reference to `coordinator.rs`/`git_snapshot.rs`/`store.rs`
  concerns (Git objects, DB rows, CAS transactions) — the protocol layer only ever receives and
  returns plain domain values, including the observed-tree inputs to `prepare` and `recover`,
  which are plain `TreeId` values supplied by the caller.
- **Non-goal:** wiring this protocol into any hook, command, or database layer; Git/SQLite/
  filesystem adapters for its inputs; a full Rust-vs-Quint model/PBT test harness; implementing
  `coordinator.rs`, `git_snapshot.rs`, or `store.rs`.

## Assumptions

- The request's illustrative Quint-concept examples (generation capture/consumption,
  `AI`/`Human`/`SkipConcurrent`/`AlreadyProcessed`/`NoMutation`/`PrePublishFailure`/
  `PublishFailure`/`AnchorFailure`/`CursorFailure` result branches, `Begin`/`End` agent scopes,
  base/next cursor boundaries) describe an earlier or hypothetical shape of the protocol. The
  current `spec/mutation_cursor.qnt` on `main` models a different, more evolved state machine:
  `WorktreeId`/`ScopeId`/`TreeId` CAS-based cursor commits (`worktrees.revision`/`cursorTree`),
  a `NeverSeen`/`Active`/`Closed`/`Abandoned` scope lifecycle, `tainted`/`externalTaint`/
  `needsRebaseline` failure/recovery state, and `IneligibleUnscoped`/`AiExclusive`/
  `AiContended` attribution. Per the request's own instruction ("If the Quint file has changed
  since this prompt was written, follow the current file"), this plan is authored against the
  actual current spec, and every task below cites concrete `spec/mutation_cursor.qnt` line
  ranges rather than the request's illustrative names.
- `cli/src/commands/hooks/event_processing.rs`, named in the request as an inspection target,
  does not exist in this repository — there is no `cli/src/commands` directory at all. Hook
  dispatch instead lives under `cli/src/services/hooks/` (e.g. `mod.rs`, `command.rs`,
  `codex/`). This plan treats that directory as the equivalent inspection target and leaves it
  unmodified per the non-goals above.
- No `proptest`/`quickcheck`-family dependency exists in `cli/Cargo.toml` today. Per the
  request's own guidance not to add dependency/infrastructure churn to claim PBT coverage, this
  plan shapes the API for later property testing (plain functions over explicit state/input/
  outcome types) without adding a new test dependency now.
- Module location and name: `cli/src/services/mutation_trace/` (not `mutation_cursor`, per
  user correction), registered in `cli/src/services/mod.rs` with `#[allow(dead_code)]`,
  matching the existing precedent for modules not yet consumed by production call sites. The
  spec file itself stays `spec/mutation_cursor.qnt`/`.md` — only the Rust module is renamed;
  the plan's line-range citations against the spec are unaffected. Internal PR-1 file split is
  `mod.rs` / `types.rs` / `protocol.rs` / `tests.rs`, matching the user-supplied target
  architecture; `coordinator.rs` (imperative shell: DB load, Git snapshot, call protocol,
  CAS/retry, persist), `git_snapshot.rs` (isolated Git object store / temporary index / tree
  capture and diff), and `store.rs` (protocol persistence interface: cursor/revision, scopes,
  processed events, mutation evidence, CAS transaction) are the later-PR seams this layout
  leaves room for, and are recorded in the new context file but not created here.
- `ProtocolState` (T02) is a plain aggregate of the existing leaf types keyed by their identity
  newtypes (`BTreeMap<WorktreeId, WorktreeState>`, `BTreeMap<ScopeId, ScopeState>`,
  `BTreeSet<WorktreeId>` for `external_taint`, `BTreeSet<EventKey>` for `processed_events`,
  `BTreeMap<AttemptId, AttemptState>` for `attempts`, `BTreeSet<MutationEvent>` for
  `mutation_events`), mirroring the Quint state machine's top-level `worktrees`/`scopes`/
  `externalTaint`/`processedEvents`/`attempts`/`mutationEvents` variables
  (`spec/mutation_cursor.qnt:2-14`). Quint's verification-only histories (`cursorHistory`,
  `protocolHistory`, `scopeHistory`, `abandonHistory`, `startHistory`, `recoveryHistory`,
  `taintHistory`, `evidenceAttempts`, `scopeStartCount`, `everTerminal`) are not represented in
  `ProtocolState`; T07's refinement matrix records them as verification-only.
  `BTreeMap`/`BTreeSet` are chosen over `HashMap`/`HashSet` for deterministic iteration order in
  tests, matching the repository's existing `BTreeMap` usage in `cli/src/services/patch.rs`.
- `prepare` and `recover` take the currently observed tree as an explicit `TreeId` parameter
  (`prepare(state, attempt, boundary, observed_tree)`, `recover(state, worktree, observed_tree)`)
  rather than reading it internally, because the pure kernel must not perform Git I/O; the
  observed tree corresponds to Quint's `worktreeTrees.get(worktree)`, which a future
  `git_snapshot.rs`/`coordinator.rs` adapter will supply from real Git state.
- **Runtime scope materialization is an adapter/store responsibility, not a protocol
  transition.** The Quint model's `SCOPES` universe is finite and every `ScopeState` entry is
  created by `init` (`scopes' = SCOPES.mapBy(scope => { status: NeverSeen, actorKind:
  scopeActor(scope), worktreeId: scopeWorktree(scope) })`), so `scopeActor`/`scopeWorktree`
  already resolve for any `ScopeId` before any boundary is evaluated. This refinement's
  `ScopeId` is an unbounded runtime string (see the identity-refinement assumption above), so
  `ProtocolState.scopes` cannot be prepopulated the way `init` does. Before the surrounding
  coordinator/store projection calls `prepare` with a hook boundary (`Start`/`Advance`/`Close`)
  referencing a `ScopeId`, it must ensure that scope already exists in `ProtocolState.scopes`
  with its durable identity — `status: NeverSeen`, the correct `actor_kind`, and the correct
  `worktree_id`. Once a `ScopeId` exists, its `actor_kind`/`worktree_id` association is
  immutable for the lifetime of that scope: the pure protocol never invents, remaps, or
  implicitly materializes scope identity, and a future adapter that observes an existing
  `ScopeId` with a conflicting `actor_kind`/`worktree_id` must treat that as an
  identity/protocol error rather than silently overwriting the record. A missing `ScopeId` is
  therefore not equivalent to a `NeverSeen` one: a missing entry means identity has not been
  materialized (invalid/unresolved protocol input, which `prepare`/`commit` already handle as a
  no-op — see T02's Result), while an existing `ScopeState { status: NeverSeen, .. }` is a known,
  materialized scope identity that simply has not yet had an accepted `Start`. Once built,
  `coordinator.rs` receives hook/session identity, resolves the scope's actor/worktree identity,
  asks `store.rs` to load or materialize the scope, obtains a `ProtocolState`, and calls the
  pure protocol; `store.rs` loads durable scope records and atomically creates a new one as
  `NeverSeen` when appropriate, but never remaps `actor_kind`/`worktree_id` for an existing
  `ScopeId`; `protocol.rs` assumes referenced scopes are already represented and only validates/
  transitions lifecycle state (see `context/cli/mutation-trace-protocol.md`, "Runtime scope
  materialization", for the full contract). Future `coordinator.rs`/`store.rs` integration tests
  (not implemented by this plan) must cover: a new scope's materialization producing exactly
  `{ status: NeverSeen, actor_kind, worktree_id }`; idempotent re-materialization of an
  already-known identical `(ScopeId, actor_kind, worktree_id)` never resetting an `Active`/
  `Closed`/`Abandoned` scope back to `NeverSeen`; rejection of a conflicting `actor_kind` for an
  existing `ScopeId`; rejection of a conflicting `worktree_id` for an existing `ScopeId`; and
  preservation of lifecycle status across identity materialization/checking in every case.

## Task stack

- [x] T01: `Establish mutation-trace module skeleton and pure domain types` (status:done)
  - Task ID: T01
  - Scope: In — create `cli/src/services/mutation_trace/` with `mod.rs` (public module
    boundary) and `types.rs`; register the module in `cli/src/services/mod.rs` with
    `#[allow(dead_code)]`; define Rust types in `types.rs` refining `WorktreeId`, `ScopeId`,
    `TreeId`, `EventId`, `AttemptId`, `EventKey`, `FailureKind`, `ScopeStatus`, `AttemptStatus`,
    `Attribution`, `Boundary`, `WorktreeState`, `ScopeState`, `AttemptState`, and
    `MutationEvent` (`spec/mutation_cursor.qnt:2-117`); pure constructors/accessors mirroring
    `scopeWorktree`, `scopeActor`, `isLive`, `isTerminal`, `boundaryWorktree`, `boundaryScope`,
    `boundaryEvent`, `boundaryEventKey`, `isHook`/`isStart`/`isAdvance`/`isClose`/`isFlush`
    (`spec/mutation_cursor.qnt:151-245`); add a `tests.rs` skeleton wired via
    `#[cfg(test)] mod tests;` in `mod.rs`. Out — commit/prepare transition logic (T02),
    attribution/mutation-event derivation (T03), taint/failure/abandon/recovery actions
    (T04-T06), `coordinator.rs`/`git_snapshot.rs`/`store.rs` (not this plan).
  - Dependencies: none
  - Done when: the module compiles, exposes the listed types with no Git/DB/FS/env/network/
    lock/async imports, and each type/free function carries a doc comment naming its Quint
    counterpart.
  - Verify: `./scripts/run-cli-cargo.sh build --manifest-path cli/Cargo.toml`; `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace`.
  - Context synchronization: synced
  - Completed: 2026-08-26
  - Files changed:
    - `cli/src/services/mod.rs` (registered `pub mod mutation_trace;` with `#[allow(dead_code)]`)
    - `cli/src/services/mutation_trace/mod.rs` (new; module doc comment corrected post-review, see below)
    - `cli/src/services/mutation_trace/types.rs` (new; corrected post-review, see below)
    - `cli/src/services/mutation_trace/tests.rs` (new; corrected post-review, see below)
  - Result: Added the `mutation_trace` module skeleton with all state/domain types
    (`WorktreeId`/`ScopeId`/`TreeId`/`EventId`/`AttemptId`/`EventKey`/`ActorKind`/
    `FailureKind`/`ScopeStatus`/`AttemptStatus`/`Attribution`/`Boundary`/`WorktreeState`/
    `ScopeState`/`AttemptState`/`MutationEvent`) and pure accessors
    (`is_live`/`is_terminal`/`ScopeState::scope_worktree`/`ScopeState::scope_actor`/
    `boundary_worktree`/`boundary_scope`/`boundary_event`/`boundary_event_key`/`is_hook`/
    `is_start`/`is_advance`/`is_close`/`is_flush`), each with a doc comment naming its Quint
    counterpart and line range. No production call site references the module. One approved
    local design decision remains (recorded in Assumptions below): identity types
    (`WorktreeId`/`ScopeId`/`TreeId`/`EventId`/`AttemptId`) are opaque `String`-wrapping
    newtypes rather than the Quint model's fixed enums.
    **Post-review correction 1 (PR #238 review):** the original `Boundary` shape gave
    `Start`/`Advance`/`Close` an independent `worktree: WorktreeId` field so
    `boundary_worktree` could stay a pure function of `Boundary` alone. This was unfaithful:
    the Quint `Boundary` type (`spec/mutation_cursor.qnt:31-35`) never stores a worktree for a
    hook boundary — `boundaryWorktree` always derives it from `scopeWorktree(data.scope)` — and
    `spec/mutation_cursor.qnt:418,458` confirm `commitAttempt`/`prepareAvailable` resolve
    `worktree` from the boundary before any scope lookup, so a stored worktree that could
    diverge from the boundary's own scope would let a caller act on the wrong worktree's state,
    a state the Quint type cannot represent. Fixed: `Boundary::Start`/`Advance`/`Close` now
    carry only `scope`/`event`, field-for-field with the Quint constructors; `Flush` is
    unchanged.
    **Post-review correction 2 (PR #238 review):** correction 1's
    `boundary_worktree(boundary, scope: Option<&ScopeState>)` still let a caller pass an
    arbitrary `ScopeState` alongside the boundary with no proof it belonged to the boundary's
    own `scope` — a test literally constructed `Boundary::Start { scope: scope0, .. }` alongside
    an unrelated `ScopeState { worktree_id: wt1, .. }` and got `wt1` back. Fixed: the signature
    is now `boundary_worktree(boundary, scopes: &BTreeMap<ScopeId, ScopeState>)`; for
    `Start`/`Advance`/`Close` it reads the `ScopeId` out of the boundary and looks up that exact
    key in `scopes` (returning `None` when the key is absent), so the boundary's own scope is
    the only key ever used — no parameter lets a caller inject a worktree independent of it.
    `Flush` is unaffected (it carries its own worktree and ignores `scopes`). `tests.rs` was
    rewritten: the misleading `boundary_worktree_reflects_the_scopes_own_worktree_not_a_guess`
    test (which had proved the bug) was replaced with tests keyed off a two-scope map spanning
    two worktrees, proving each hook boundary resolves via its own `scope` regardless of what
    else is in the map, that a missing scope yields `None`, and that `Flush` ignores the map
    entirely. `mod.rs`'s module doc comment was also reworded to drop task/plan references
    (repository convention: source comments should not cite the current task, fix, or PR).
  - Verify outcomes:
    - `./scripts/run-cli-cargo.sh build --manifest-path cli/Cargo.toml` — passed.
    - `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace` — passed,
      14/14 tests (10 original + 3 from correction 1 + 1 net add from correction 2's rewrite).
    - `grep -RnE "std::(fs|process|env)|tokio|reqwest|turso" cli/src/services/mutation_trace` —
      no matches (AC1 spot-check).
    - `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings` — passed, no warnings.
    - `cargo fmt --manifest-path cli/Cargo.toml -- --check` — passed, no diff.
    - `nix run .#quint -- typecheck spec/mutation_cursor.qnt` — passed.
    - `nix run .#quint -- test spec/mutation_cursor.qnt` — passed.
    - `git diff -- spec/mutation_cursor.qnt spec/mutation_cursor.md` — empty (spec untouched).
  - Context impact: Classification: minor. A new, currently-unreferenced module exists under
    `cli/src/services/`; no existing behavior, hook, or command changed. Context synchronized
    in the same session as implementation: `context/cli/mutation-trace-protocol.md` (new domain
    file), `context/context-map.md` (new entry), and `context/overview.md` (one-sentence
    mention) were all updated at T01 completion; this correction pass updated the domain file's
    description of the `boundary_worktree` refinement to match the keyed-lookup design.

- [x] T02: `Implement the protocol aggregate state, explicit observation inputs, and prepare/commit evaluation for every boundary` (status:done)
  - Task ID: T02
  - Scope: In —
    - In `types.rs`, define the `ProtocolState` aggregate (`worktrees`, `scopes`,
      `external_taint`, `processed_events`, `attempts`, `mutation_events`) described in
      Assumptions, mirroring the Quint state machine's top-level state variables
      (`spec/mutation_cursor.qnt:2-14`). Verification-only Quint histories are not represented.
    - In `protocol.rs`, `prepare` refining `prepareAvailable`/`prepare`
      (`spec/mutation_cursor.qnt:417-453`), taking the currently observed tree as an explicit
      `TreeId` input (e.g. `prepare(state, attempt, boundary, observed_tree)`) rather than
      reading it internally; `observed_tree` corresponds to Quint's
      `worktreeTrees.get(worktree)` at the boundary's resolved worktree.
    - In `protocol.rs`, a commit-evaluation function refining `commitAttempt`
      (`spec/mutation_cursor.qnt:455-661`) for **all four** boundary kinds (`Start`, `Advance`,
      `Close`, `Flush`) in one pass: compute `accepted` (`fresh`: prepared status, worktree not
      in `external_taint`, `expectedRevision == revision`, `beforeTree == cursorTree`, and for
      hook boundaries the `EventKey` not already in `processed_events`), `observes` (`Start`
      requires `NeverSeen`; `Advance` requires live; `Close` accepts `NeverSeen` or live;
      `Flush` is always `true`), `observedChange` (`accepted and observes and beforeTree !=
      afterTree`), `changed` (`observedChange and not needsRebaseline` — expose this as a
      computed flag but do **not** construct a `MutationEvent` from it; that is T03's job), and
      `advancesRevision` (`accepted and (not isFlush(boundary) or observedChange)`); apply scope
      lifecycle transitions (`NeverSeen`→`Active` on accepted `Start`, →`Closed` on accepted
      `Close`), cursor advancement (`afterTree` when `observes and not needsRebaseline`,
      otherwise unchanged), attempt status transitions (`Prepared`→`Committed` on `accepted`,
      →`Rejected` otherwise), and processed-`EventKey` recording for accepted hook boundaries.
    - Tests land in `tests.rs`.
  - Out — attribution derivation and `MutationEvent` materialization (T03); taint/
    database-failure/abandon/recovery actions (T04-T06).
  - Dependencies: T01
  - Done when:
    - a state-sequence test proves prepare→commit accepts a fresh `Start`, rejects a
      stale-revision or stale-`beforeTree` attempt without mutating worktree/scope/cursor
      state, rejects a replayed `EventKey`, rejects an attempt whose worktree is in
      `external_taint` even with a fresh revision/`beforeTree`, and transitions scope status
      correctly for `Start` and `Close`;
    - a test proves an accepted-but-non-observing hook — a fresh `Start` on an already-`Active`
      scope, and the equivalent invalid `Advance`/`Close` cases — results in: attempt →
      `Committed`, event key → processed, revision → increments, cursor → unchanged, scope →
      unchanged, and `changed` is `false`;
    - a test proves `Flush`'s `advancesRevision` is `false` when `observedChange` is `false`
      (a no-op tree), distinguishing it from hook boundaries whose `advancesRevision` follows
      from `accepted` alone.
  - Verify: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace`.
  - Context synchronization: synced
  - Completed: 2026-08-26
  - Files changed:
    - `cli/src/services/mutation_trace/types.rs` (added `ProtocolState`; added `Ord`/`PartialOrd`
      derives to `FailureKind`, `Attribution`, `Boundary`, `MutationEvent` so `MutationEvent` can
      live in a `BTreeSet`)
    - `cli/src/services/mutation_trace/protocol.rs` (new; `prepare`, `commit`, `CommitEvaluation`,
      `CommitOutcome`, and the private `ResolvedAttempt` helper)
    - `cli/src/services/mutation_trace/mod.rs` (registered `pub mod protocol;`; module doc comment
      updated to reflect that transition logic now exists)
    - `cli/src/services/mutation_trace/tests.rs` (11 new tests for `prepare`/`commit`; added
      `attempt_id`/`healthy_worktree`/`scope_with_status` builders)
  - Result: Implemented `ProtocolState` (`worktrees`/`scopes`/`external_taint`/`processed_events`/
    `attempts`/`mutation_events`) and, in `protocol.rs`, `prepare` (refining
    `prepareAvailable`/`prepare`, taking the observed tree as an explicit `TreeId` parameter) and
    `commit` (refining `commitAttempt`) for all four boundary kinds in one pass. `commit` is split
    into a private `ResolvedAttempt` helper (`resolve`/`evaluate`/`apply`) to stay under Clippy's
    line-count lint; `evaluate` returns a `CommitEvaluation` (`accepted`/`observes`/
    `observed_change`/`changed`/`advances_revision`) that `apply` and T03 both consume, so
    `changed` is exposed as a computed flag without constructing a `MutationEvent` — `commit`
    leaves `mutation_events` untouched, as scoped. The scope-transition guards for `Start`/`Close`
    reuse `evaluate`'s own `observes` flag rather than re-deriving the same `NeverSeen`/live check
    a second time, since the two are provably identical per `commitAttempt`'s own definition of
    `observes`. `prepare` and `commit` are no-ops (state unchanged, evaluation flags all `false`)
    when the boundary's worktree cannot be resolved (an unregistered scope for a hook boundary) or
    has no durable state — an attempt only reaches `commit` via a successful `prepare`, which
    already refuses to prepare against an unresolvable worktree, so this is a defensive default
    rather than a path any required test exercises. No production call site references the
    module.
    **Post-review clarification (PR #238 review):** `prepare(Start/Advance/Close)` requires the
    referenced `ScopeId` to already exist in `ProtocolState.scopes`; unknown scopes are not
    materialized by the protocol. This is intentional, not a gap — runtime scope materialization
    is an adapter/store responsibility, not a protocol transition (see Assumptions: "Runtime
    scope materialization is an adapter/store responsibility, not a protocol transition"). No
    code changed for this clarification: the existing `prepare`/`commit` no-op behavior for an
    unresolvable worktree (an unregistered scope for a hook boundary), described above, already
    matches this contract exactly; only the contract itself was made explicit.
  - Verify outcomes:
    - `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace` — passed,
      25/25 tests (14 from T01 + 11 new: fresh `Start` activation, `Close` scope transition, stale
      revision/`beforeTree` rejection, replay rejection, external-taint rejection, three
      accepted-but-non-observing cases — `Start` on `Active`, `Advance` on `NeverSeen`, `Close` on
      `Abandoned` — and two `Flush` cases proving `advancesRevision` requires `observedChange`
      unlike hook boundaries).
    - `./scripts/run-cli-cargo.sh build --manifest-path cli/Cargo.toml` — passed.
    - `grep -RnE "std::(fs|process|env)|tokio|reqwest|turso" cli/src/services/mutation_trace` — no
      matches (AC1 spot-check).
    - `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings`
      — passed after splitting `commit` (`too_many_lines`), allowing
      `clippy::struct_excessive_bools` on `CommitEvaluation` (precedented at
      `cli/src/services/setup/mod.rs:194`), and replacing a redundant closure with a method
      reference.
    - `cargo fmt --manifest-path cli/Cargo.toml -- --check` — passed, no diff (after running
      `cargo fmt`).
    - `nix run .#quint -- typecheck spec/mutation_cursor.qnt` — passed.
    - `nix run .#quint -- test spec/mutation_cursor.qnt` — passed.
    - `git diff --stat -- spec/mutation_cursor.qnt spec/mutation_cursor.md` — empty (spec
      untouched).
    - `grep -rn "mutation_trace" cli/src/services/hooks cli/src/services/agent_trace.rs` — no
      matches (AC8 spot-check).
  - Context impact: Classification: domain. `protocol.rs` is new pure transition logic added to an
    already-unreferenced module; no existing behavior, hook, or command changed. Synchronized in
    the same session as implementation: `context/cli/mutation-trace-protocol.md` (Current
    state/Module layout/target-architecture sections updated to describe `protocol.rs`'s
    `prepare`/`commit`), `context/context-map.md` (summary line updated), and `context/overview.md`
    (one-line mention corrected) — all three were stale, describing the module as types-only.
    `context/architecture.md`, `context/glossary.md`, and `context/patterns.md` were verified and
    found not contradicted; no edit needed. No qualifying architecture decision.

- [x] T03: `Implement attribution and mutation-event materialization from pre-transition live scopes` (status:done)
  - Task ID: T03
  - Scope: In — in `protocol.rs`, a pure function refining `attributionFor`
    (`spec/mutation_cursor.qnt:285-301`) computing `IneligibleUnscoped`/`AiExclusive(scope)`/
    `AiContended` from live scopes plus worktree `failureKind`/`externalTaint`/
    `needsRebaseline`; wire `MutationEvent` construction (refining `mkMutationEvent`,
    `spec/mutation_cursor.qnt:303-323`) into T02's commit evaluation, gated by the `changed`
    flag T02 already computes. Compute `live`/`attribution` from the **pre-transition** scope
    set exactly as `commitAttempt` computes them — before `nextScope` is applied
    (`spec/mutation_cursor.qnt:484-485` precede the `nextScope` `val` at line 530): a `Start`
    boundary's emitted event never attributes the mutation to the scope it is about to
    activate (that scope is not yet counted as live), and a `Close` boundary's emitted event
    still attributes to the scope it is about to close (that scope is still counted as live).
    Tests land in `tests.rs`. Out — taint/failure/abandon/recovery state changes (T04-T06); no
    new commit boundary or `Flush` semantics (already covered by T02).
  - Dependencies: T02
  - Done when: tests prove zero/one/multiple pre-transition live scopes map to the three
    attribution variants; an unhealthy `failureKind`/external taint/`needsRebaseline` forces
    `IneligibleUnscoped` even with active scopes; a no-op tree change emits no mutation event; a
    real change emits exactly one event carrying the correct attribution/boundary/revision; a
    `Start` on a `NeverSeen` scope that also observes a change emits an event whose attribution
    excludes the newly-activated scope; a `Close` on the sole live scope that also observes a
    change emits an event whose attribution still counts that scope as live.
  - Verify: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace`.
  - Context synchronization: synced
  - Completed: 2026-08-26
  - Files changed:
    - `cli/src/services/mutation_trace/protocol.rs` (added `live_scopes_on`, `attribution_for`;
      wired `MutationEvent` materialization into `ResolvedAttempt::apply`, gated by
      `evaluation.changed`; updated `commit`/`CommitEvaluation` doc comments to reflect that
      mutation-event materialization is now implemented, and dropped stale `T03`-referencing
      comment text per repository convention)
    - `cli/src/services/mutation_trace/tests.rs` (11 new tests: `live_scopes_on` filtering;
      `attribution_for`'s zero/one/multiple-live-scope and
      unhealthy/externally-tainted/needs-rebaseline-forces-`IneligibleUnscoped` cases; `commit`
      emitting no event on a no-op tree change, exactly one event with correct
      attribution/boundary/revision on a real change via `Advance` (which does not itself alter
      the scope set), `Start` on `NeverSeen` excluding the newly-activated scope from
      attribution, and `Close` on the sole live scope still counting it as live)
  - Result: Added `live_scopes_on(state, worktree)` (refining `liveScopesOn`,
    `spec/mutation_cursor.qnt:265-269`, filtering the known `state.scopes` map by
    `worktree_id`/`is_live()` since this refinement has no fixed `SCOPES` universe to filter) and
    `attribution_for(state, worktree)` (refining `attributionFor`,
    `spec/mutation_cursor.qnt:285-301`) to `protocol.rs`, both `pub` so `tests.rs` can exercise
    them directly as pure functions. Wired `MutationEvent` construction (refining
    `mkMutationEvent`, `spec/mutation_cursor.qnt:303-323`) into `ResolvedAttempt::apply`,
    inserted into `next.mutation_events` when `evaluation.changed`, computed by calling
    `live_scopes_on`/`attribution_for` against `apply`'s own `state: &ProtocolState` parameter —
    the pre-transition state `commit` passes through unmutated, since `apply` clones it into
    `next` and only ever mutates `next` — matching `commitAttempt`'s own `live`/`attribution`
    computation at `spec/mutation_cursor.qnt:484-485`, which precedes the `nextScope` `val` at
    line 530. No new fields were added to `CommitEvaluation`; `changed` (already computed by T02)
    is the sole gate, per the task's own scope. No production call site references the module.
  - Verify outcomes:
    - `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace` — passed,
      36/36 tests (25 from T01+T02 + 11 new).
    - `./scripts/run-cli-cargo.sh build --manifest-path cli/Cargo.toml` — passed.
    - `grep -RnE "std::(fs|process|env)|tokio|reqwest|turso" cli/src/services/mutation_trace` —
      no matches (AC1 spot-check).
    - `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings`
      — passed, no warnings.
    - `cargo fmt --manifest-path cli/Cargo.toml -- --check` — passed, no diff (after running
      `cargo fmt`).
    - `nix run .#quint -- typecheck spec/mutation_cursor.qnt` — passed.
    - `nix run .#quint -- test spec/mutation_cursor.qnt` — passed.
    - `git diff --stat -- spec/mutation_cursor.qnt spec/mutation_cursor.md` — empty (spec
      untouched, AC8 spot-check).
    - `grep -rn "mutation_trace" cli/src/services/hooks cli/src/services/agent_trace.rs` — no
      matches (AC8 spot-check).
  - Context impact: Classification: domain. `attribution_for`/`live_scopes_on`/mutation-event
    materialization are new pure logic added to an already-unreferenced module; no existing
    behavior, hook, or command changed.

- [ ] T04: `Implement snapshot-failure taint and database-failure external-taint actions` (status:todo)
  - Task ID: T04
  - Scope: In — in `protocol.rs`, pure transitions refining `taintHealthy`/`taint`
    (`spec/mutation_cursor.qnt:663-710`) and `recordDatabaseFailure`/`databaseFailure`
    (`spec/mutation_cursor.qnt:712-737`): taint sets `tainted=true`,
    `failureKind=SnapshotFailure`, advances revision, leaves `cursorTree`/`needsRebaseline`
    untouched, and is a guarded no-op when already tainted or externally tainted; database
    failure adds the worktree to `externalTaint` only, touching no other durable worktree/scope
    field, and is a guarded no-op when already externally tainted; tests land in `tests.rs`.
    Out — abandonment (T05), recovery (T06).
  - Dependencies: T02 (taint/database-failure semantics do not depend on attribution or
    mutation-event materialization)
  - Done when: tests prove `taint` changes exactly `tainted`/`failureKind`/`revision` and
    nothing else, `databaseFailure` changes exactly `externalTaint` and leaves every other
    durable worktree/scope field equal to before, and both actions are no-ops on an
    already-tainted/already-externally-tainted worktree.
  - Verify: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace`.
  - Context synchronization: pending

- [ ] T05: `Implement scope abandonment` (status:todo)
  - Task ID: T05
  - Scope: In — in `protocol.rs`, a pure transition refining `abandonLiveScope`/`abandon`
    (`spec/mutation_cursor.qnt:739-805`): transitions a live scope to `Abandoned`, sets the
    owning worktree's `needsRebaseline=true`, advances revision, leaves `cursorTree` untouched,
    records the scope as terminal, preserves the scope's `actor_kind` and `worktree_id`
    unchanged (scope identity stability), and is a guarded no-op for any non-live scope —
    Quint's guard is "not live", so `NeverSeen`, `Closed`, and `Abandoned` all stutter — or for
    a live scope on an externally tainted worktree; tests land in `tests.rs`. Out — recovery
    (T06).
  - Dependencies: T04
  - Done when: tests prove abandoning a live scope sets `Abandoned`+`needsRebaseline` without
    moving the cursor and without changing `actor_kind`/`worktree_id`; abandoning a `NeverSeen`
    scope is a no-op; abandoning an already-terminal (`Closed`/`Abandoned`) scope is a no-op
    (never reactivates a terminal scope); abandoning a live scope on an externally tainted
    worktree is a no-op.
  - Verify: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace`.
  - Context synchronization: pending

- [ ] T06: `Implement recovery with an explicit observed-tree input` (status:todo)
  - Task ID: T06
  - Scope: In — in `protocol.rs`, a pure transition refining `recoverNeeded`/`recover`
    (`spec/mutation_cursor.qnt:807-886`), taking the currently observed tree as an explicit
    `TreeId` input (e.g. `recover(state, worktree, observed_tree)`) rather than obtaining it
    itself — the core must not ambiguously "get the current tree"; that is the future Git
    adapter's responsibility. Re-baselines `cursorTree` to `observed_tree`, clears
    `tainted`/`failureKind`/`needsRebaseline`/`externalTaint`, advances revision, and abandons
    every live scope on the worktree only when recovering from `tainted` or external taint — a
    healthy worktree with only `needsRebaseline` set preserves its live scopes; guarded no-op
    when the worktree is healthy, not externally tainted, and does not need rebaseline; tests
    land in `tests.rs`. Out — none remaining; this completes the action set.
  - Dependencies: T05
  - Done when: tests prove taint/external-taint recovery abandons every live scope on that
    worktree while a `needsRebaseline`-only recovery preserves them, both paths clear
    `externalTaint`/`tainted`/`failureKind`/`needsRebaseline` and rebaseline the cursor to
    `observed_tree`, and recovery is a no-op on an already-healthy worktree with no rebaseline
    need.
  - Verify: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace`.
  - Context synchronization: pending

- [ ] T07: `Add cross-action state-sequence and invariant tests, and the Quint refinement matrix` (status:todo)
  - Task ID: T07
  - Scope: In — in `tests.rs`, complete state-machine sequence tests spanning multiple actions;
    every scenario below is required, not "at least three":
    1. Two scopes `Active` on the same worktree, with a mutation observed while both are live
       producing `AiContended` evidence; then a `Close` boundary reduces live scopes to one —
       because `Close`'s own commit computes `live`/`attribution` from the **pre-close** live
       set, if that same `Close` also observes a mutation it still emits `AiContended` evidence,
       not `AiExclusive`; a subsequent mutation observed after the close, with exactly one live
       scope remaining, is where `AiExclusive` evidence first appears.
    2. Taint → recover (abandons live scopes, rebaselines cursor).
    3. Database failure → recover (clears external taint, rebaselines cursor).
    4. Abandon → `needsRebaseline` → recover-with-preserved-survivors (a second, still-live
       scope on the same worktree survives a `needsRebaseline`-only recovery).
    5. Replay of a committed `EventKey` (rejected, no state change).
    6. Stale-attempt rejection (stale revision or `beforeTree`) never advances revision, moves
       the cursor, or emits evidence.
    7. Competing prepared attempts (a real CAS race, not a manually constructed stale
       `AttemptState`): `prepare` two attempts `A`/`B` against the same worktree at revision 0;
       `commit(A)` is accepted (`Committed`, revision → 1); `commit(B)` is then rejected because
       its `expected_revision` (0) no longer matches the worktree's revision (1) — revision stays
       1, the cursor is unchanged by `B`, and `B` emits no evidence.
    8. Taint invalidates a prepared attempt: `prepare(A)` at revision `R`; `taint(worktree)`
       advances the worktree to revision `R + 1`; `commit(A)` is then rejected as stale (its
       `expected_revision` no longer matches) — no cursor movement and no evidence from `A`. This
       is a cross-action consequence of Quint's snapshot-failure taint semantics, not something a
       single-action test can show.
    Invariant-style tests named to mirror the Quint invariants this module refines
    (`CursorRevisionConsistent`, `FailureKindMatchesTaint`, `TerminalScopesStayTerminal`,
    `DatabaseFailureDoesNotMutateDurableProtocolState`,
    `ExternalTaintNeverStrengthensAttribution`, `RecoveryClearsExternalTaintOnlyAfterBaseline`,
    `NoNoopMutationEvents`, `AiExclusiveRequiresExactlyOneActiveScope`,
    `AiContendedRequiresMultipleActiveScopes`, `RejectedAttemptsDoNotCommitEvidence` —
    `spec/mutation_cursor.qnt:1041-1274`); in `mod.rs`, a module-level rustdoc refinement matrix
    classifying every relevant Quint action/result/invariant this module refines — including
    `ScopeActorIdentityIsStable`, `ScopeStartedAtMostOnce`, `MutationEventsHavePositiveRevision`,
    `MutationEventUniquePerWorktreeRevision`, `MutationFailureKindMatchesTaint`,
    `AttributionMatchesObservedScopes`, `NeedsRebaselineSuppressesAttribution`,
    `StartDoesNotAbandonExistingScopes`.

    **The matrix must classify Quint verification variables/checkpoint ledgers separately from
    the semantic invariants stated over them — a verification-only data structure is not the
    same thing as a verification-only invariant.** Model instrumentation may be verification-only;
    the protocol property proved with that instrumentation may still be a required production
    invariant, so a property's classification is never inferred solely from the fact that Quint
    happens to state it using a history variable. Two categories, classified independently:

    - **Verification-only model instrumentation** — the concrete Quint checkpoint types and
      history/counter variables that exist only to state or prove properties, with no Rust
      production equivalent: `CursorCheckpoint`, `ProtocolCheckpoint`, `ScopeCheckpoint`,
      `AbandonCheckpoint`, `StartCheckpoint`, `RecoveryCheckpoint`,
      `DurableProtocolCheckpoint`, and the variables `cursorHistory`/`protocolHistory`/
      `scopeHistory`/`abandonHistory`/`startHistory`/`recoveryHistory`/`taintHistory`/
      `evidenceAttempts`/`scopeStartCount`/`everTerminal`. Each of these is classified
      `verification-only / intentionally omitted` in the matrix, since `ProtocolState` does not
      materialize Quint's histories, unless a future production adapter turns out to need a
      direct equivalent for another reason.
    - **Semantic properties expressed using that instrumentation** — classified independently,
      by what the Rust code actually does, into: implemented directly, enforced by Rust type,
      preserved by transition tests, verification-only / intentionally omitted, or external
      adapter responsibility. At minimum:
      - `TerminalScopesStayTerminal` (Quint mechanism: `everTerminal`) — **preserved by
        transition tests**: `Closed`/`Abandoned` are terminal, and no Rust transition may move
        either back to `NeverSeen` or `Active`. Require a test that reaches a terminal state
        through real transitions and then exercises a later boundary against it to prove it
        cannot reactivate — not merely constructing a terminal `ScopeState` and inspecting it.
      - `ScopeStartedAtMostOnce` (Quint mechanism: `scopeStartCount`) — **preserved by
        transition tests**: only `Start` on `NeverSeen` activates a scope; require a sequence
        proving a second `Start` (fresh `EventKey`) on an already-`Active` scope is
        accepted-but-non-observing and the scope remains `Active`, and that `Start` on a
        terminal (`Closed`/`Abandoned`) scope never reactivates it.
      - `RejectedAttemptsDoNotCommitEvidence` (Quint mechanism: `evidenceAttempts`) —
        **preserved by transition tests**: rejected, stale, replayed, and external-taint-rejected
        attempts must not emit `MutationEvent` evidence; tested once T03 exists.
      - `StartDoesNotAbandonExistingScopes` (Quint mechanism: `startHistory`/`scopeHistory`) —
        **preserved by transition tests**: starting one scope must not alter another
        already-active scope; require an actual multi-scope sequence, not an isolated single-scope
        test.
      - `RecoveryClearsExternalTaintOnlyAfterBaseline` (Quint mechanism:
        `recoveryHistory`/`cursorHistory`) — **preserved by transition tests**: recovery
        establishes `observed_tree` as the new cursor baseline in the same pure transition that
        clears `external_taint`; the histories are omitted, but the semantic ordering/effect
        remains a required test.
      - `DatabaseFailureDoesNotMutateDurableProtocolState` (Quint mechanism:
        `taintHistory`/`durableProtocolStateFor`) — **preserved by transition tests**:
        `database_failure` changes only `external_taint` and leaves every other durable
        worktree/scope field unchanged.
      - `ScopeActorIdentityIsStable` — **preserved by transition tests + external adapter
        responsibility** (already established, preserved here): no transition in `protocol.rs`
        ever mutates `actor_kind`/`worktree_id`, and future scope materialization must reject a
        conflicting identity rather than overwrite it.

      Quint's finite `SCOPES` universe and its `init`-time `ScopeState` population classify as
      **external adapter responsibility**: the Rust refinement's unbounded `ScopeId` space means
      scope identity is materialized at runtime by the future coordinator/store layer rather than
      at protocol startup (see Assumptions: "Runtime scope materialization").

    Make the matrix auditable: for each entry, name the Quint element, whether it is
    instrumentation or a semantic property, its Rust counterpart (if any), its classification,
    and the concrete test or enforcement mechanism that backs a non-verification-only
    classification — a markdown table is one reasonable way to do this, but any rustdoc layout
    that carries the same information per entry satisfies the requirement. Add a short note on
    the `coordinator.rs`/`git_snapshot.rs`/`store.rs` seams this layout leaves for later PRs.
    Out — none; this is the closing task.
  - Dependencies: T06
  - Done when:
    1. all eight named multi-action sequence tests exist and pass;
    2. every named semantic invariant has an explicit Rust enforcement classification
       (implemented directly, enforced by Rust type, preserved by transition tests, external
       adapter responsibility, or verification-only / intentionally omitted) backed by a named
       test or mechanism;
    3. verification-only model instrumentation (the checkpoint types and history/counter
       variables listed above) is classified separately from the semantic invariants stated
       using it;
    4. no semantic property is classified verification-only merely because Quint states it using
       a history/checkpoint variable — a property lands there only when it truly has no
       production semantic meaning;
    5. the module doc comment contains a refinement matrix that names the concrete Rust test or
       enforcement mechanism for each production-semantic invariant, auditable against
       `spec/mutation_cursor.qnt`, including Quint's finite `SCOPES`/`init` population as
       external adapter responsibility and `ScopeActorIdentityIsStable` as jointly preserved by
       transition tests and external adapter responsibility.
  - Verify: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace`; `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings`; `cargo fmt --manifest-path cli/Cargo.toml -- --check`.
  - Context synchronization: pending

## Open questions

None. The request pre-authorizes following the current `spec/mutation_cursor.qnt` over its own
illustrative examples, which resolves the one substantive doubt (see Assumptions); the module's
value and scope are otherwise well-specified and not duplicated by any existing code. The
`coordinator.rs`/`git_snapshot.rs`/`store.rs` roadmap is recorded as context for later plans
rather than as work here, consistent with this plan's own non-goals. This revision's task
reshaping (T02 absorbing `Flush` commit evaluation, T03's pre-transition live-scope requirement,
T04's dependency correction, T05's `NeverSeen` no-op case, T06's explicit `observed_tree`
parameter, and T07's full-scenario/refinement-matrix requirements) was fully specified by the
user, leaving nothing to ask.
