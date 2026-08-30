# Plan: mutation-cursor-external-taint

## Change summary

Add the external durability boundary for the mutation cursor: a worktree-local
filesystem marker at `<git-dir>/sce/mutation-cursor-tainted`, armed write-ahead
at the start of mutation-cursor boundary processing, that a later invocation
reads as the external signal that the previous invocation never proved a
trustworthy durable completion. This is the concrete runtime refinement of the
already-verified abstract `ProtocolState.external_taint` / `databaseFailure` /
`recover` semantics — it changes no protocol semantics, adds no database state,
and requires no migration.

The fence must begin at the mutation-cursor runtime boundary, **before**
repository Agent Trace DB acquisition — not after an already-open
`RepositoryAgentTraceDb` has been handed in. The current
`coordinate(repository_root, &db, boundary)` signature arms the fence too late:
if the hook runtime's DB open fails, `coordinate()` is never entered, no marker
is armed, and a later invocation that opens the DB successfully sees no
inherited marker and can treat a lost `A → C` interval as trustworthy evidence.
This plan reshapes the outer `coordinate()` entrypoint to own the whole
protected operation: it resolves `git_dir`, acquires the `WorktreeLock`, arms
the marker, and only then acquires the DB (through a caller-supplied provider),
runs the snapshot / recovery / protocol / CAS pipeline, and clears the marker
only on complete success. The lower-level `coordinate_boundary` /
`coordinate_with_db` pipeline (the #244 snapshot/protocol/store logic) still
receives an already-open `&RepositoryAgentTraceDb` internally;
`MutationTraceStore` and `protocol.rs` never open or resolve a DB.

Target ordering the plan establishes:

```text
RuntimeBoundary
  → resolve git_dir
  → acquire WorktreeLock
  → inspect inherited marker
  → persist marker
  → get/create checkout ID  (WorktreeId)
  → open RepositoryAgentTraceDb  (caller-supplied provider)
  → capture + pin one snapshot
  → if inherited taint: overlay database_failure, recover against that snapshot
  → prepare / commit the triggering boundary
  → DB CAS
  → complete success
  → clear marker
```

Safety invariant: **no failure after a mutation-cursor boundary enters its
protected runtime section — including failure to open the Agent Trace DB
itself, or to resolve checkout identity — can disappear without leaving a
worktree-local external-taint signal for the next invocation.**

All work is confined to `cli/src/services/mutation_trace/runtime/` (a new
`external_taint.rs` primitive, entrypoint reshaping plus arming and
inherited-taint recovery wiring in `coordinator.rs`, and updated integration
tests in `runtime/tests.rs`) and the durable context/spec docs that describe
the coordinator. It extends the existing `mutation-cursor-runtime-coordinator`
work and preserves that coordinator's current lock, snapshot, DB-backed
`SnapshotFailure`, and CAS-retry behavior. No harness or command wiring is part
of this plan, and `coordinate()` stays reachable only from within `runtime`.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the
check that proves it. `/validate` runs these checks; no task in the stack
performs final validation.

- [ ] AC1: A marker is stored beneath the caller-supplied worktree-specific Git
  directory (`<git-dir>/sce/mutation-cursor-tainted`), survives reconstruction
  of the marker handle, and two distinct `git_dir` inputs derive independent
  marker paths and marker state. (Actual linked-worktree independence over a
  real `git worktree add` pair is AC12/T04.)
  - Validate: `runtime::external_taint::tests::marker_is_worktree_scoped` and
    `runtime::external_taint::tests::marker_persists_until_explicitly_cleared`
- [ ] AC2: `persist` then `persist` then `clear` then `clear` all succeed;
  `clear` on an absent marker is success.
  - Validate: `runtime::external_taint::tests::persist_and_clear_are_idempotent`
- [ ] AC3: A public `coordinate()` call arms its marker internally and clears it
  only after a successful `CoordinateOutcome`.
  - Validate: `runtime::tests::successful_coordinate_clears_external_taint_marker`
- [ ] AC4: After marker arming, any coordinator error leaves the marker present —
  proven for the snapshot-failure path and at least one deterministic
  non-snapshot failure path.
  - Validate: `runtime::tests::snapshot_failure_leaves_external_taint_marker`
    plus one deterministic non-snapshot failure test
- [ ] AC5: Given durable cursor A, an active scope S, an inherited marker, and a
  current worktree C, the next invocation rebaselines to C, emits no A→C
  mutation evidence, abandons S, then processes its triggering boundary.
  - Validate: `runtime::tests::inherited_external_taint_recovers_before_boundary`
- [ ] AC6: External-taint recovery and triggering-boundary processing use the
  same `observed_tree`; no second Git snapshot occurs.
  - Validate: coordinator unit test with a call-counting `SnapshotCapture`
    asserting exactly one `capture`
- [ ] AC7: A losing recovery CAS re-injects external taint on reload and
  recomputes recovery until `Applied` or retry exhaustion; the marker remains
  present throughout the retry loop.
  - Validate: coordinator unit test racing a recovery CAS conflict and asserting
    re-injection
- [ ] AC8: A failure injected after recovery has committed but before the
  triggering boundary completes leaves recovery durable, the boundary
  incomplete, and the marker still present; a later invocation recovers
  conservatively again.
  - Validate: `runtime::tests` marker-clear/boundary-incomplete scenario
- [ ] AC9: A marker that survives an invocation which never materialized a
  worktree row causes the next successful invocation to establish a baseline
  with no evidence for the unknown interval.
  - Validate: coordinator unit test plus `runtime::tests` first-ever-failure
    scenario
- [ ] AC10: A worktree with live scopes and inherited external taint persists
  those scopes as `Abandoned` during recovery and never treats them as eligible
  exclusive attribution after the unknown interval.
  - Validate: coordinator unit test asserting `Abandoned` scope status after
    inherited-taint recovery
- [ ] AC11: End to end, no `MutationEvent` treats an interval spanning an
  incomplete/failed invocation (baseline A, scope start, edit→B, failed
  invocation, edit→C, next successful invocation) as one trustworthy
  AI-attributable A/B→C interval.
  - Validate: `runtime::tests` end-to-end no-evidence-across-gap scenario
- [ ] AC12: An external-taint marker in linked worktree A does not trigger
  recovery in linked worktree B, even with a shared repository Agent Trace DB.
  - Validate: `runtime::tests` linked-worktree independence scenario
- [ ] AC13: When marker inspection or marker persistence cannot be established,
  `coordinate()` returns a distinct external-taint marker error before any
  checkout-identity, DB, snapshot, or protocol processing.
  - Validate: coordinator unit test injecting marker inspect/persist I/O failure
- [ ] AC14: DB acquisition is inside the external-taint fence. Once
  mutation-cursor boundary processing has acquired the `WorktreeLock` and armed
  the marker, a failure to resolve or open the repository Agent Trace DB (the
  caller-supplied provider returns `Err`) makes `coordinate()` return an error
  with the marker still present, even though no `RepositoryAgentTraceDb` was
  ever available and the lower-level coordinator pipeline was never entered. The
  follow-up invocation, given a working DB provider, snapshots the current tree,
  runs external-taint recovery, and produces no evidence across the lost
  interval.
  - Validate: `runtime::tests` DB-provider-returns-`Err` scenario (marker armed
    → provider `Err` → error returned → marker still present) plus its
    follow-up-invocation recovery assertion; and a coordinator unit test with a
    fake failing DB provider

### Full validation

Repository-wide checks `/validate` runs after the last task, regardless of
which criterion they map to.

- `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::mutation_trace::runtime::`
- `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml`
- `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings`
- `./scripts/run-cli-cargo.sh fmt --manifest-path cli/Cargo.toml -- --check`
- `nix flake check`
- `nix run .#pkl-check-generated`
- Confirm the existing Quint Connect model-based-testing harness
  (`checks.cli-tests` / `checks.mutation-trace-quint-connect`) stays green;
  no `spec/mutation_cursor.qnt` behavior change is expected.

### Context sync

- `context/cli/mutation-trace-runtime-coordinator.md` — document the
  `ExternalTaintMarker` primitive and its worktree-scoped path; the reshaped
  `coordinate()` entrypoint that owns `WorktreeLock`, marker arming, the
  caller-supplied DB provider, the snapshot/pipeline, and marker clear; the
  arm-before-DB-acquisition ordering and the safety invariant; the
  inherited-vs-armed distinction and the invocation-local `external_taint_pending`
  overlay onto `database_failure`; the new marker-I/O and
  DB-provider-failure `CoordinateError` variants and their fail-closed
  semantics; and the added `<git-dir>/sce/` on-disk-layout entry.
- `context/cli/mutation-trace-protocol.md` — note that the concrete runtime
  refinement of `ProtocolState.external_taint` is the stale worktree-local
  marker, armed write-ahead before DB acquisition and promoted to protocol
  external taint only when inherited by a later invocation.
- `context/context-map.md` — refresh the coordinator domain-file annotation.
- `spec/mutation_cursor.md` — record that the abstract `externalTaint` marker's
  concrete refinement is `<git-dir>/sce/mutation-cursor-tainted`, armed
  write-ahead at the start of the protected runtime section (before Agent Trace
  DB acquisition) and becoming protocol external taint only when inherited.
- Verify-only pass over `context/overview.md`, `context/architecture.md`,
  `context/glossary.md`, `context/patterns.md`.

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** `cli/src/services/mutation_trace/runtime/external_taint.rs`
  (new), `cli/src/services/mutation_trace/runtime/mod.rs`,
  `cli/src/services/mutation_trace/runtime/coordinator.rs` (including reshaping
  the public `coordinate()` signature and updating its existing call sites in
  `runtime/tests.rs`), `cli/src/services/mutation_trace/runtime/tests.rs`;
  docs-only edits to `context/cli/mutation-trace-runtime-coordinator.md`,
  `context/cli/mutation-trace-protocol.md`, `context/context-map.md`, and
  `spec/mutation_cursor.md`.
- **Out of scope:** ref reconciliation; harness adapters and any
  hook/command/`diff_traces` wiring; a `pub(crate)` re-export of `coordinate()`;
  redesigning Agent Trace storage resolution beyond, at most, a narrow split
  that lets the marker be armed before the existing DB-open primitive runs;
  changes to `protocol.rs`, `spec/mutation_cursor.qnt`, or the Quint refinement
  matrix; any `store.rs` / SQL / migration change; a daemon; cross-machine
  locking; snapshot-ref reclamation; new Agent Trace evidence formats.
- **Constraints:**
  - No new Cargo dependencies unless an unavoidable platform issue is discovered.
  - The DB CAS stays the protocol linearization point.
  - The outer `coordinate()` entrypoint owns `WorktreeLock`,
    `ExternalTaintMarker`, the DB-provider call, the snapshot service, the
    coordinator pipeline, and the marker clear. DB acquisition is supplied to
    `coordinate()` as a caller-provided `FnOnce` provider (or equivalent seam),
    not resolved by `coordinate()` itself — repository identity and remote come
    from config the coordinator does not read.
  - `MutationTraceStore` and `protocol.rs` never open or resolve a DB; the
    lower-level `coordinate_boundary` / `coordinate_with_db` pipeline still
    receives an already-open `&RepositoryAgentTraceDb`.
  - The marker is never armed before the `WorktreeLock` is held, and
    same-worktree marker state is inspected/persisted/cleared only while that
    lock is held.
  - The filesystem marker is never authoritative for normal cursor state.
  - No RAII/`Drop`-based marker deletion; only a successful `CoordinateOutcome`
    clears the marker.
  - `WorktreeProjection::into_protocol_state()` still returns an empty
    `external_taint`; the filesystem overlay is applied by runtime code only.
  - Marker durability follows the existing
    `checkout::persist_checkout_id_inner` style (`fsync` the marker file,
    best-effort `#[cfg(unix)]` parent-directory `sync_all`).
- **Non-goal:** introducing a new protocol `FailureKind`; making
  `WorktreeProjection::into_protocol_state()` read filesystem state; opening the
  DB before arming the marker (that would leave DB-open failure uncovered, which
  is the specific failure this plan exists to close); making the protocol or
  store layer responsible for opening or resolving the DB; broadening the
  durability claim to host power loss / filesystem crash.

## Assumptions

The user's change request states "exact names may change", "the exact type is
flexible", and "this exact API is NOT mandatory"; the following are recorded
local choices, not new requirements.

- `coordinate()` is reshaped to
  `coordinate(repository_root: &Path, boundary: &RuntimeBoundary, open_db: impl FnOnce() -> anyhow::Result<RepositoryAgentTraceDb>) -> Result<CoordinateOutcome, CoordinateError>`.
  The private `coordinate_inner(.., on_lock_contention, open_db)` test seam is
  kept. The lower-level pipeline function is renamed/kept as
  `coordinate_boundary` (or `coordinate_with_db`) taking `&RepositoryAgentTraceDb`
  plus `inherited_external_taint`. A future harness adapter builds the provider
  closure around `agent_trace_storage::resolve_agent_trace_storage_for_hook_runtime`
  and hands `coordinate()` the resulting `.db`.
- Marker I/O failures surface as a dedicated `CoordinateError` variant, e.g.
  `ExternalTaintMarker { operation: ExternalTaintOperation, source: anyhow::Error }`
  with `ExternalTaintOperation { Inspect, Persist, Clear }`. A DB-provider
  failure after the marker is armed surfaces as a separate variant, e.g.
  `CoordinateError::AgentTraceDbUnavailable(anyhow::Error)`, and intentionally
  leaves the marker in place.
- The new primitive lives at `runtime/external_taint.rs` as
  `ExternalTaintMarker`, backed by an empty file at
  `<git-dir>/sce/mutation-cursor-tainted`; its existence is the entire state.
- Marker durability protects against process error, non-graceful process exit,
  `SIGKILL`, and normal runtime restart: `persist()` creates and `fsync`s the
  marker file, `clear()` removes it, and both do a best-effort `#[cfg(unix)]`
  parent-directory `sync_all` whose error is not propagated (mirroring
  `checkout::persist_checkout_id_inner`). The plan does not claim durability
  across host power loss or a filesystem-level crash, because that
  parent-directory sync is best-effort.
- Test module and function names follow the paths named in the acceptance
  criteria (`runtime::external_taint::tests::*`, `runtime::tests::*`).

## Task stack

- [x] T01: `Add the external-taint marker primitive` (status:done)
  - Task ID: T01
  - Completed: 2026-08-30
  - Files changed:
    - `cli/src/services/mutation_trace/runtime/external_taint.rs` (new) —
      `ExternalTaintMarker` with `new`/`exists`/`persist`/`clear`, local
      `SCE_RUNTIME_DIR` / `MARKER_FILE` consts, checkout-identity-style
      durability, cfg-gated best-effort parent-dir sync, inline `#[cfg(test)]
      mod tests`.
    - `cli/src/services/mutation_trace/runtime/mod.rs` — `mod external_taint;`.
  - Result: New primitive backed by an empty file at
    `<git-dir>/sce/mutation-cursor-tainted`; existence is the entire state.
    `persist()` does `create_dir_all` + non-truncating `create` open
    (`write(true).create(true).truncate(false)` — marker contents carry no
    meaning) + `sync_data()` + best-effort `#[cfg(unix)]` parent-dir `sync_all`
    (error swallowed); `clear()` does `remove_file` (`NotFound` → `Ok`) + the
    same best-effort dir sync; `exists()` via `symlink_metadata`. No `Drop`
    deletion. Module carries `#![allow(dead_code)]` (per `services/capabilities.rs`
    precedent) since nothing wires it in until T02. No `coordinator.rs`,
    protocol, store, or error-type change.
  - Verify (re-run after the PR #245 review fixes — `truncate` removal, AC1 wording):
    - `test ...runtime::external_taint` — PASS (3 passed: `marker_is_worktree_scoped`,
      `marker_persists_until_explicitly_cleared`, `persist_and_clear_are_idempotent`).
    - `test ...runtime::` — PASS (38 passed, 0 failed).
    - `clippy --all-targets -- -D warnings` — PASS (clean).
    - `fmt -- --check` — PASS (clean).
  - Context impact: docs-update-needed. Introduces a new runtime module
    (`ExternalTaintMarker`) and a new durable on-disk artifact
    (`<git-dir>/sce/mutation-cursor-tainted`). Affected durable context:
    `context/cli/mutation-trace-runtime-coordinator.md` (primitive + on-disk
    layout entry), `spec/mutation_cursor.md` (concrete refinement of the
    abstract `externalTaint` marker). Matches the plan's Context sync section;
    no behavior reaches the coordinator or protocol yet.
  - Scope: In — new `cli/src/services/mutation_trace/runtime/external_taint.rs`
    defining `ExternalTaintMarker` with `new(git_dir)`, `exists()`, `persist()`,
    `clear()`; empty marker file at `<git-dir>/sce/mutation-cursor-tainted`;
    checkout-identity-style durability (`fsync` the file on create, best-effort
    `#[cfg(unix)]` parent-dir `sync_all` on create and remove); `NotFound` on
    clear treated as success; no `Drop` deletion; registration in
    `runtime/mod.rs`. Out — any `coordinator.rs` change, any protocol/store/
    error-type change, harness wiring.
  - Dependencies: none
  - Done when: the type compiles and is registered under the module's existing
    `#[allow(dead_code)]` precedent; `persist`/`clear` are idempotent; the path
    is derived from the caller-supplied worktree `git_dir`; inline
    `#[cfg(test)] mod tests` (unique `std::env::temp_dir()` paths, per
    `context/patterns.md`) covers persistence across marker-value reconstruction,
    idempotent persist, idempotent clear, and two `git_dir`s resolving to
    independent marker paths.
  - Verify: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::mutation_trace::runtime::external_taint`;
    `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings`;
    `./scripts/run-cli-cargo.sh fmt --manifest-path cli/Cargo.toml -- --check`.
  - Context synchronization: synced

- [x] T02: `Arm the write-ahead fence around the full runtime boundary, DB acquisition included` (status:done)
  - Task ID: T02
  - Completed: 2026-08-30
  - Files changed:
    - `cli/src/services/mutation_trace/runtime/coordinator.rs` — reshaped the
      public `coordinate()` to take a caller-supplied
      `open_db: impl FnOnce() -> anyhow::Result<RepositoryAgentTraceDb>` provider
      instead of `&RepositoryAgentTraceDb`; `coordinate_inner` now resolves
      `git_dir` → acquires `WorktreeLock` → constructs `ExternalTaintMarker` →
      reads `inherited_external_taint = marker.exists()?` → `marker.persist()?` →
      runs the new `coordinate_protected` (checkout identity → `open_db()` →
      `GitSnapshotService` → `coordinate_boundary`) → `marker.clear()?` only on
      `Ok`; added `ExternalTaintOperation { Inspect, Persist, Clear }` and the
      `CoordinateError::ExternalTaintMarker { operation, source }` and
      `CoordinateError::AgentTraceDbUnavailable(_)` variants plus their `Display`
      arms; `coordinate_boundary` gained an unused `_inherited_external_taint:
      bool` param (consumed in T03); updated the `two_threads_…_serialize`
      `coordinate_inner` call site and added six inline tests
      (`public_coordinate_clears_marker_on_success`,
      `public_coordinate_leaves_marker_after_a_snapshot_failure`,
      `public_coordinate_leaves_marker_after_a_non_snapshot_failure`,
      `public_coordinate_fails_closed_when_the_marker_cannot_be_armed`
      (`ExternalTaintOperation::Persist` path),
      `public_coordinate_fails_closed_when_marker_inspection_fails`
      (`ExternalTaintOperation::Inspect` path — holds the runtime lock, swaps
      `<git-dir>/sce` from a directory to a regular file at the worker's
      lock-contention point so `marker.exists()` hits a deterministic `ENOTDIR`
      with no permission changes, asserts the DB provider is never called),
      `public_coordinate_leaves_marker_when_the_db_provider_fails`).
    - `cli/src/services/mutation_trace/runtime/tests.rs` — updated all five
      `coordinate()` call sites to pass a provider closure
      (`|| RepositoryAgentTraceDb::open_for_hooks_without_migrations_at(&path)`).
  - Result: The external-taint fence now spans the whole protected runtime
    section. `coordinate()` arms the worktree-local marker after `WorktreeLock`
    acquisition and before checkout-identity, DB-provider, snapshot, and
    protocol work; a successful `CoordinateOutcome` clears it; every failure
    after arming (snapshot failure, DB provider `Err`, revision exhaustion, CAS
    exhaustion, scope conflict, DB read/write, unexpected) returns with the
    marker present; both the `ExternalTaintOperation::Inspect` and
    `ExternalTaintOperation::Persist` marker-I/O failure paths fail closed with
    the `CoordinateError::ExternalTaintMarker` error before any checkout,
    DB-provider, snapshot, or protocol work — each proven by a dedicated
    deterministic regression test (AC13). `MutationTraceStore` and
    `protocol.rs` are untouched; `coordinate_boundary` still receives an
    already-open `&RepositoryAgentTraceDb`. Inherited-taint recovery mapping is
    still T03 (`_inherited_external_taint` is threaded but unused).
  - Verify:
    - `test ...services::mutation_trace::runtime::coordinator` — PASS (21 passed,
      0 failed; `public_coordinate_fails_closed_when_marker_inspection_fails`
      also re-run 5× for determinism).
    - `test ...services::mutation_trace::runtime::` — PASS (44 passed, 0 failed).
    - `clippy --all-targets -- -D warnings` — PASS (clean).
    - `fmt -- --check` — PASS (clean).
  - Context impact: docs-update-needed. Reshapes the public `coordinate()`
    entrypoint (caller-supplied DB provider, `WorktreeLock`/marker ownership,
    arm-before-DB-acquisition ordering, marker clear on success only) and adds
    two `CoordinateError` variants with fail-closed semantics. Affected durable
    context: `context/cli/mutation-trace-runtime-coordinator.md` (reshaped
    entrypoint, marker arming, DB provider, new error variants, safety
    invariant), `context/cli/mutation-trace-protocol.md` (write-ahead marker as
    concrete refinement armed before DB acquisition), `context/context-map.md`
    (coordinator annotation refresh), `spec/mutation_cursor.md` (write-ahead
    timing of the concrete `externalTaint` refinement). Matches the plan's
    Context sync section. No protocol semantics, DB state, or migration changed.
  - Scope: In — `runtime/coordinator.rs`: reshape the public `coordinate()` so
    it no longer receives an already-open `&RepositoryAgentTraceDb` but a
    caller-supplied DB provider (`impl FnOnce() -> anyhow::Result<RepositoryAgentTraceDb>`
    or equivalent seam); reorder the outer path to resolve `git_dir` → acquire
    `WorktreeLock` → construct `ExternalTaintMarker` → read
    `inherited_external_taint = marker.exists()?` → `marker.persist()?` →
    `get_or_create_checkout_id` (`WorktreeId`) → invoke the DB provider →
    construct `GitSnapshotService` → run the lower-level pipeline
    (`coordinate_boundary` / `coordinate_with_db`, taking `&RepositoryAgentTraceDb`
    plus `inherited_external_taint`, unused until T03) → `marker.clear()?` only
    on `Ok`, leaving the marker on every `Err`; add a distinct marker-I/O
    `CoordinateError` variant (inspect/persist failure returns before checkout,
    DB, snapshot, or protocol work; clear failure after a successful boundary
    returns `Err` with the marker left in place) and a distinct
    DB-provider-failure variant that also leaves the marker; update the existing
    `coordinate()` call sites in `runtime/tests.rs` to pass a provider closure.
    Out — mapping inherited taint into recovery (T03); new integration scenarios
    (T04).
  - Dependencies: T01
  - Done when: every `coordinate()` call arms the marker after `WorktreeLock`
    acquisition and before checkout-identity, DB-provider, snapshot, and
    protocol work; a successful `CoordinateOutcome` clears it; every path after
    arming leaves it present — snapshot failure, DB provider returning `Err`,
    checkout-identity failure, DB read/write failure, CAS exhaustion,
    scope-identity conflict, unexpected error; marker inspect/persist failure
    returns the new marker error before any of that work; the private
    `coordinate_inner` test seam and `on_lock_contention` closure are preserved;
    `coordinator.rs` inline tests prove success→cleared, the snapshot-failure
    path and one deterministic non-snapshot failure path both retaining the
    marker, the fail-closed inspect/persist error path, and a fake DB provider
    returning `Err` after arming leaving the marker present.
  - Verify: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::mutation_trace::runtime::coordinator`;
    `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::mutation_trace::runtime::`;
    `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings`;
    `./scripts/run-cli-cargo.sh fmt --manifest-path cli/Cargo.toml -- --check`.
  - Context synchronization: synced

- [x] T03: `Map inherited marker into protocol recovery` (status:done)
  - Task ID: T03
  - Completed: 2026-08-30
  - Files changed:
    - `cli/src/services/mutation_trace/runtime/coordinator.rs` — renamed
      `coordinate_boundary`'s `_inherited_external_taint` param to
      `inherited_external_taint` and split the body into a 5-arg
      `coordinate_boundary` wrapper plus a private
      `coordinate_boundary_inner(.., after_load: FnMut(u32), after_recovery: FnMut(u32))`
      test seam (mirroring the existing `coordinate_inner` /
      `run_taint_retry_loop_inner` seam precedent; production passes no-ops).
      Seeded `let mut external_taint_pending = inherited_external_taint;` before
      the CAS loop; after each `load_worktree` the loop overlays
      `state = protocol::database_failure(&state, worktree_id)` while
      `external_taint_pending`, so the existing `needs_recovery` /
      `protocol::recover` path runs against the one already-captured
      `observed_tree`. On recovery-CAS `Applied` the flag is cleared (and
      `after_recovery` fires); on `Conflict` the existing `continue` keeps it set
      so the next reload re-injects the overlay. Added four inline tests
      (`inherited_external_taint_recovers_once_before_the_boundary`,
      `inherited_external_taint_with_no_worktree_row_baselines_without_evidence`,
      `a_losing_recovery_cas_reinjects_external_taint_until_it_applies`,
      `a_landed_recovery_clears_the_flag_so_a_boundary_cas_retry_does_not_re_recover`).
  - Result: An inherited external-taint marker (T02's `inherited_external_taint`)
    now drives protocol recovery. `external_taint_pending` is invocation-local
    and seeded from the inherited flag; while set, `protocol::database_failure`
    is overlaid onto every freshly loaded projection before the recovery check,
    so `protocol::recover` performs exactly one conservative recovery (cursor :=
    observed tree, revision += 1, external taint cleared, live scopes →
    `Abandoned`, no `MutationEvent` for the fenced interval) against the single
    captured snapshot, then the triggering boundary is processed against the
    recovered state. The overlay is never persisted (`DurableTransition` ignores
    `external_taint`; `into_protocol_state()` always returns it empty). A losing
    recovery CAS re-injects the overlay on the next reload and recomputes until
    `Applied` or retry exhaustion; once recovery lands, the flag is clear so a
    later boundary-CAS retry in the same invocation does not re-trigger recovery.
    A first-ever inherited marker with no durable worktree row is baselined
    against the observed tree by the existing `initialize_worktree`, then
    conservatively recovered once. The filesystem marker is never touched here —
    `coordinate_inner`'s success path still owns clearing it. No `protocol.rs`,
    `store.rs`, SQL, or migration change.
  - Verify:
    - `test ...services::mutation_trace::runtime::coordinator` — PASS (25 passed,
      0 failed).
    - `test ...services::mutation_trace::runtime::` — PASS (48 passed, 0 failed).
    - `clippy --all-targets -- -D warnings` — PASS (clean).
    - `fmt -- --check` — PASS (clean, after `cargo fmt`).
  - Context impact: docs-update-needed. Adds the invocation-local
    `external_taint_pending` overlay onto `database_failure` and its
    inherited-vs-armed recovery semantics to the coordinator pipeline; no
    protocol semantics, DB state, error variants, or public signatures changed
    beyond T02's already-recorded reshape. Affected durable context:
    `context/cli/mutation-trace-runtime-coordinator.md` (inherited-taint overlay
    onto `database_failure`, one-recovery-per-inherited-marker, flag lifecycle
    across recovery-CAS conflict/apply), `context/cli/mutation-trace-protocol.md`
    (the stale marker becomes protocol external taint only when inherited by a
    later invocation), `spec/mutation_cursor.md` (same inherited-only promotion).
    Matches the plan's Context sync section.
  - Deviations: added the private `coordinate_boundary_inner` `after_load` /
    `after_recovery` test seams — consistent with the existing
    `coordinate_inner(on_lock_contention)` and
    `run_taint_retry_loop_inner(after_load)` precedent — because deterministically
    forcing a recovery-CAS conflict, and a boundary-CAS conflict after a landed
    recovery, is otherwise only reachable through non-deterministic thread races.
  - Scope: In — `runtime/coordinator.rs`: invocation-local
    `external_taint_pending`, seeded from `inherited_external_taint` and threaded
    into the lower-level pipeline; when set, overlay `protocol::database_failure`
    onto each freshly loaded projection before the existing `needs_recovery`
    check, so `protocol::recover` runs against the single already-captured
    `observed_tree`; keep `external_taint_pending` set across a recovery-CAS
    `Conflict` and clear the in-memory flag only on recovery-CAS `Applied`;
    never touch the filesystem marker here (T02's success path owns clearing);
    first-ever inherited marker with no durable worktree row initializes the
    worktree against the observed tree, then recovers. Out — filesystem marker
    writes; new integration scenarios (T04).
  - Dependencies: T02
  - Done when: an inherited marker forces exactly one recovery transition
    (cursor := observed tree, revision += 1, `tainted`/external taint cleared,
    active scopes → `Abandoned`, no `MutationEvent` for the skipped interval)
    before the triggering boundary is processed against the recovered state;
    recovery and boundary share one snapshot (call-counting `SnapshotCapture`
    proves a single `capture`); a recovery-CAS conflict re-injects
    `database_failure` on reload and recomputes; after recovery-CAS `Applied`
    the flag is clear so a later boundary-CAS retry in the same invocation does
    not re-trigger recovery; a first-ever inherited marker with no worktree row
    produces no evidence for the unknown interval; `coordinator.rs` inline tests
    cover each case.
  - Verify: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::mutation_trace::runtime::coordinator`;
    `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings`;
    `./scripts/run-cli-cargo.sh fmt --manifest-path cli/Cargo.toml -- --check`.
  - Context synchronization: synced

- [ ] T04: `Add restart/failure integration tests through coordinate()` (status:todo)
  - Task ID: T04
  - Scope: In — `runtime/tests.rs` cross-module tests driving only the public
    `coordinate()` API against real `git init` / `git worktree add` repositories
    and real temp-file `RepositoryAgentTraceDb`s (unique-temp-path precedent),
    with the DB passed through the provider closure: successful invocation
    leaves no marker; a failed invocation leaves the marker; a **DB provider
    returning `Err` after the marker is armed** leaves the marker present and
    the follow-up invocation (working provider) rebaselines to the current tree
    with no evidence across the lost interval; a stale marker on the next
    invocation rebaselines to the current tree, emits no evidence across the
    gap, abandons prior live scopes, then processes its boundary; a first-ever
    failed invocation that never materialized a worktree row cannot create
    evidence; two linked worktrees keep independent markers over one shared DB
    (a marker in A does not recover B); snapshot-failure interaction leaves both
    a durable `SnapshotFailure` and the marker, and the next invocation performs
    a single conservative recovery; a marker-clear failure after a durable
    boundary keeps the marker for a later conservative re-recovery. Out — any
    production-code change.
  - Dependencies: T03
  - Done when: the listed scenarios pass through the public entrypoint only,
    including the end-to-end "trusted A → failed/incomplete invocation (DB-open
    failure included) → filesystem changes → next successful invocation
    rebaselines to C with no A/B→C evidence" story; `runtime::tests` and the
    full CLI test suite pass.
  - Verify: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::mutation_trace::runtime::tests`;
    `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml`.
  - Context synchronization: pending

## Open questions

None. The entry-contract question raised in the prior draft is resolved by this
plan: PR #245 establishes an outer mutation-cursor runtime boundary that arms
external taint before Agent Trace DB acquisition, and future harness adapters
must call that protected boundary (`harness adapter → coordinate() → marker →
DB → snapshot / protocol / CAS`) rather than opening the DB themselves and
calling a lower-level coordinator. Whether a later change also splits
`agent_trace_storage` resolution into "resolve identity/path" and "open DB"
halves is left to implementation — the plan only requires the narrowest seam
that puts DB acquisition inside the fence, and the caller-supplied provider
closure achieves that without touching `agent_trace_storage` at all.
