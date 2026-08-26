# Plan: mutation-cursor-protocol-kernel

## Change summary

Establish a pure, dependency-free Rust refinement of the verified `spec/mutation_cursor.qnt`
protocol under a new `cli/src/services/mutation_trace` module, split as `mod.rs` (public module
boundary), `types.rs` (state/domain types), `protocol.rs` (pure transition logic), and
`tests.rs`. The module represents the protocol's state (`WorktreeState`, `ScopeState`,
`AttemptState`), its pure transitions (prepare/commit attempts for `Start`/`Advance`/`Close`/
`Flush` boundaries, attribution derivation, snapshot-failure taint, database-failure external
taint, scope abandonment, and recovery), and its result/attribution/mutation-event types, with
deterministic tests that mirror the spec's invariants. This is new behavior: no Rust
implementation of this protocol exists today, and the module performs no Git, database,
filesystem, environment, network, or lock I/O. `coordinator.rs`, `git_snapshot.rs`, and
`store.rs` — the imperative-shell orchestration, isolated Git snapshot capture, and DB-backed
CAS persistence seams in the target end-state architecture — are acknowledged as the layout the
protocol module will grow into, but are not created in this PR; `protocol.rs` is not wired into
any existing hook, command, or database call site. That integration is explicitly out of scope
and left for a later plan.

## Acceptance criteria

- [ ] AC1: The mutation-cursor protocol module has an explicit Rust home under
      `cli/src/services/mutation_trace` with zero Git/DB/filesystem/environment/network/
      async/lock I/O in its pure transition logic.
  - Validate: `grep -RnE "std::(fs|process|env)|tokio|reqwest|turso" cli/src/services/mutation_trace` returns nothing; manual inspection of imports.
- [ ] AC2: `Start`/`Advance`/`Close` hook boundaries transition scope status and worktree
      cursor/revision exactly as `commitAttempt` specifies (`spec/mutation_cursor.qnt:455-661`),
      including CAS freshness rejection (`expectedRevision`/`beforeTree` mismatch) and replay
      rejection via processed `EventKey`s.
  - Validate: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace`
- [ ] AC3: Attribution (`IneligibleUnscoped`/`AiExclusive`/`AiContended`, `spec/mutation_cursor.qnt:285-301`)
      and mutation-event emission match `commitAttempt`'s `changed` gate exactly
      (`observedChange and not needsRebaseline`), including the `Flush` boundary and
      failure/taint/`needsRebaseline` attribution overrides.
  - Validate: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace`
- [ ] AC4: Snapshot-failure taint (`taintHealthy`/`taint`, `spec/mutation_cursor.qnt:663-710`)
      changes only `tainted`/`failureKind`/`revision`; database failure
      (`recordDatabaseFailure`/`databaseFailure`, `spec/mutation_cursor.qnt:712-737`) changes
      only `externalTaint`. Neither ever changes the cursor.
  - Validate: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace`
- [ ] AC5: Abandonment (`abandonLiveScope`/`abandon`, `spec/mutation_cursor.qnt:739-805`) is
      terminal, sets `needsRebaseline`, and never moves the cursor; a terminal scope can never
      be reactivated or abandoned again.
  - Validate: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace`
- [ ] AC6: Recovery (`recoverNeeded`/`recover`, `spec/mutation_cursor.qnt:807-886`) re-baselines
      the cursor and clears taint/`needsRebaseline`/`externalTaint`, abandoning live scopes only
      on the taint/`externalTaint` recovery path and preserving them on the
      `needsRebaseline`-only path.
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
  returns plain domain values.
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
    carry only `scope`/`event` (field-for-field with the Quint constructors); `Flush` is
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

- [ ] T02: `Implement prepare/commit attempt transition for Start/Advance/Close boundaries` (status:todo)
  - Task ID: T02
  - Scope: In — in `protocol.rs`, pure transition function(s) refining
    `prepareAvailable`/`prepare`/`commitAttempt` (`spec/mutation_cursor.qnt:417-661`) for
    `Start`/`Advance`/`Close` boundaries: CAS freshness check (`expectedRevision ==
    worktree.revision` and `beforeTree == worktree.cursorTree`), replay rejection via the
    processed `EventKey` set, the boundary-specific `observes` rule (Start requires
    `NeverSeen`; Advance requires live; Close accepts `NeverSeen` or live), scope lifecycle
    transition (`NeverSeen`→`Active` on Start, →`Closed` on Close), cursor advancement gated by
    `observes` and worktree `needsRebaseline`, and attempt status transitions
    (`Available`→`Prepared`→`Committed`/`Rejected`); tests land in `tests.rs`. Out — `Flush`
    boundary and attribution/mutation-event emission (T03); taint/database-failure/abandon/
    recovery actions (T04-T06).
  - Dependencies: T01
  - Done when: a state-sequence test proves prepare→commit accepts a fresh Start, rejects a
    stale-revision or stale-`beforeTree` attempt without mutating worktree/scope/cursor state,
    rejects a replayed `EventKey`, and transitions scope status correctly for Start and Close.
  - Verify: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace`.
  - Context synchronization: pending

- [ ] T03: `Implement attribution and mutation-event emission` (status:todo)
  - Task ID: T03
  - Scope: In — in `protocol.rs`, a pure function refining `attributionFor`
    (`spec/mutation_cursor.qnt:285-301`) computing `IneligibleUnscoped`/`AiExclusive(scope)`/
    `AiContended` from live scopes plus worktree `failureKind`/`externalTaint`/
    `needsRebaseline`; wire mutation-event construction (refining `mkMutationEvent`,
    `spec/mutation_cursor.qnt:303-323`) into the T02 commit transition, gated by `changed`
    (`observedChange and not needsRebaseline`) exactly as `commitAttempt` computes it,
    including the `Flush` boundary special-casing and the no-op exclusion (`beforeTree ==
    afterTree` emits nothing); tests land in `tests.rs`. Out — taint/failure/abandon/recovery
    state changes (T04-T06).
  - Dependencies: T02
  - Done when: tests prove zero/one/multiple live scopes map to the three attribution
    variants, an unhealthy `failureKind`/external taint/`needsRebaseline` forces
    `IneligibleUnscoped` even with active scopes, a no-op tree change emits no mutation event,
    and a real change emits exactly one event carrying the correct attribution/boundary/
    revision.
  - Verify: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace`.
  - Context synchronization: pending

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
  - Dependencies: T03
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
    records the scope as terminal, and is a guarded no-op for a non-live scope or an externally
    tainted worktree; tests land in `tests.rs`. Out — recovery (T06).
  - Dependencies: T04
  - Done when: tests prove abandoning a live scope sets `Abandoned`+`needsRebaseline` without
    moving the cursor, abandoning an already-terminal (`Closed`/`Abandoned`) scope is
    rejected/no-op (never reactivates a terminal scope), and abandoning on an externally
    tainted worktree is a no-op.
  - Verify: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace`.
  - Context synchronization: pending

- [ ] T06: `Implement recovery` (status:todo)
  - Task ID: T06
  - Scope: In — in `protocol.rs`, a pure transition refining `recoverNeeded`/`recover`
    (`spec/mutation_cursor.qnt:807-886`): re-baselines `cursorTree` to the current observed
    worktree tree, clears `tainted`/`failureKind`/`needsRebaseline`/`externalTaint`, advances
    revision, and abandons every live scope on the worktree only when recovering from `tainted`
    or external taint — a healthy worktree with only `needsRebaseline` set preserves its live
    scopes; guarded no-op when the worktree is healthy, not externally tainted, and does not
    need rebaseline; tests land in `tests.rs`. Out — none remaining; this completes the action
    set.
  - Dependencies: T05
  - Done when: tests prove taint/external-taint recovery abandons every live scope on that
    worktree while a `needsRebaseline`-only recovery preserves them, both paths clear
    `externalTaint`/`tainted`/`failureKind`/`needsRebaseline` and rebaseline the cursor to the
    current tree, and recovery is a no-op on an already-healthy worktree with no rebaseline
    need.
  - Verify: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace`.
  - Context synchronization: pending

- [ ] T07: `Add cross-action state-sequence and invariant tests, and the Quint refinement matrix` (status:todo)
  - Task ID: T07
  - Scope: In — in `tests.rs`, complete state-machine sequence tests spanning multiple actions
    (concurrent scopes producing `AiContended` evidence then reverting to `AiExclusive`;
    taint→recover; database-failure→recover; abandon→`needsRebaseline`→recover-with-preserved-
    survivors; replay of a committed `EventKey`; stale-attempt rejection never advancing
    revision or emitting evidence); invariant-style tests named to mirror the Quint invariants
    this module refines (`CursorRevisionConsistent`, `FailureKindMatchesTaint`,
    `TerminalScopesStayTerminal`, `DatabaseFailureDoesNotMutateDurableProtocolState`,
    `ExternalTaintNeverStrengthensAttribution`, `RecoveryClearsExternalTaintOnlyAfterBaseline`,
    `NoNoopMutationEvents`, `AiExclusiveRequiresExactlyOneActiveScope`,
    `AiContendedRequiresMultipleActiveScopes`, `RejectedAttemptsDoNotCommitEvidence` —
    `spec/mutation_cursor.qnt:1041-1274`); in `mod.rs`, a module-level rustdoc refinement matrix
    mapping every Quint action/result/invariant this module refines to its Rust counterpart,
    plus a short note on the `coordinator.rs`/`git_snapshot.rs`/`store.rs` seams this layout
    leaves for later PRs. Out — none; this is the closing task.
  - Dependencies: T06
  - Done when: the named invariant tests exist and pass, at least three multi-action sequence
    tests exist and pass, and the module doc comment contains a refinement matrix a reviewer
    can audit against `spec/mutation_cursor.qnt`.
  - Verify: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace`; `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings`; `cargo fmt --manifest-path cli/Cargo.toml -- --check`.
  - Context synchronization: pending

## Open questions

None. The request pre-authorizes following the current `spec/mutation_cursor.qnt` over its own
illustrative examples, which resolves the one substantive doubt (see Assumptions); the module's
value and scope are otherwise well-specified and not duplicated by any existing code. The
`coordinator.rs`/`git_snapshot.rs`/`store.rs` roadmap is recorded as context for later plans
rather than as work here, consistent with this plan's own non-goals.
