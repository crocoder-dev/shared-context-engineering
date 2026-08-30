# Plan: mutation-cursor-runtime-coordinator

## Change summary

Adds the runtime (imperative-shell) layer that connects the verified,
already-implemented mutation-cursor protocol kernel (`protocol.rs`) and its
persistence layer (`store.rs`) to a real Git worktree: a concurrency-safe
checkout-identity primitive, an OS-backed per-worktree advisory lock for the
coordinator's own critical section (`runtime/worktree_lock.rs`), an isolated Git
snapshot service that captures the current worktree state as a durable
`TreeId` into the repository's normal object database and protects it from
GC/prune with an SCE-owned ref, never touching the caller's real index
(`runtime/git_snapshot.rs`), and a runtime coordinator (`runtime/coordinator.rs`) that
derives `WorktreeId` from checkout identity, materializes worktree/scope
rows, runs recovery when the durable state requires it, drives
`prepare`/`commit`, and retries on CAS conflict — including CAS conflict
while persisting a snapshot-failure taint — while reusing the single Git
snapshot captured for that invocation. This extends existing work —
`protocol.rs` and `store.rs` are unmodified — and completes the `read
durable state → speculative snapshot → derive transition → DB CAS → commit
or reject/retry` boundary the Quint model and
`context/cli/mutation-trace-protocol.md` already describe as future work.
The coordinator remains standalone: nothing in this PR wires it into a
harness hook, a CLI command, or `diff_traces`.

This revision (PR #244 review) replaces the original plan's private,
alternate-backed object store with the repository's normal object database
plus SCE-owned refs (a real durability defect in the original design — see
Design decisions), makes `checkout::get_or_create_checkout_id` itself
concurrency-safe instead of only protecting the coordinator's own call to it
(closing the race for every caller, including `agent_trace_storage`),
correctly retries snapshot-failure taint persistence under CAS conflict
instead of claiming it succeeded unconditionally, and documents the
`(ScopeId, EventId)` replay-identity contract `RuntimeBoundary` places on
future harness adapters.

A second revision (further PR #244 review) makes three additional
corrections. First, checkout-identity persistence is now crash-safe, not
merely concurrency-safe: `get_or_create_checkout_id` writes through a
unique temporary file and an atomic rename rather than writing the canonical
`checkout-id` path in place, so a process crash mid-write can never expose a
partially written identity to a later caller. Second, the coordinator no
longer bases its snapshot-failure taint-vs-bootstrap decision on durable
state read *before* the Git snapshot attempt — that read was stale by
construction and could miss a worktree row another caller materializes
concurrently; the decision now always comes from a fresh read taken *after*
the failure. Third, this plan corrects its own earlier claim that
create-only ref pinning produces "bounded" storage growth — growth is
unbounded over the repository's lifetime without a reconciliation pass, and
the Follow-up PR section now sequences that reconciliation pass and the
still-deferred filesystem external-taint marker as required runtime
completion work *before* any harness adapter may become a production
consumer of this coordinator.

A third revision (further PR #244 review) corrects two remaining Git-level
issues in the snapshot mechanism itself, with the surrounding architecture
otherwise unchanged. First, the unborn-`HEAD` path now explicitly runs
`git read-tree --empty` to initialize a genuinely valid empty index, rather
than assuming a freshly reserved, never-created temp-index path was already
one — verified experimentally that a bare zero-byte file at that path is not
a valid Git index and makes `git add -A -- .` fail deterministically. The
temporary-index RAII guard is now explicit that it only reserves a unique
path and never creates a file there itself, so `git read-tree` is always
what first touches it. Second, the plan's `git gc`/`git prune` durability
test design is corrected: because Git is content-addressed, "an identical
unpinned tree" is a contradiction — pinning one copy of identical content
pins all of it — so the negative control now captures a second,
distinct-content tree in the same repository, deliberately unreachable from
any ref, and proves that one, not a same-content decoy, is what `git gc
--prune=now`/`git prune --expire=now` actually reclaims.

A fourth revision (pre-implementation structural correction, before T02 or
any later task began) moves every planned imperative-runtime file under a
new `cli/src/services/mutation_trace/runtime/` module instead of adding them
flat alongside `protocol.rs`/`store.rs`/`types.rs`. `worktree_lock.rs`,
`git_snapshot.rs`, and `coordinator.rs` become
`runtime/worktree_lock.rs`, `runtime/git_snapshot.rs`, and
`runtime/coordinator.rs`; the planned `runtime_tests.rs` becomes
`runtime/tests.rs`, declared as `runtime`'s own `#[cfg(test)] mod tests`.
This is a pure module-boundary correction with no change to protocol,
persistence, or task-ordering semantics: it separates the pure, verified
protocol kernel and its durable persistence (`protocol.rs`, `store.rs`,
`types.rs`, `mbt/`) from the imperative shell that drives Git subprocesses,
filesystem locks, and checkout identity around them, so the dependency
direction — `runtime` depends on `protocol`/`store`/`types` and on
`services::checkout`, never the reverse — is structurally visible rather
than merely a documented convention. `checkout/` remains its own top-level
service, unmoved, since other Agent Trace storage paths already depend on it
independently of the mutation-cursor runtime.

## Acceptance criteria

- [x] AC1: A worktree observed for the first time establishes the currently
  observed tree as its cursor baseline on its first boundary and emits no
  mutation evidence for filesystem changes that predate that observation.
  - Validate: `runtime::coordinator::tests::first_observation_establishes_baseline_without_evidence` via `nix build .#checks.<system>.cli-tests`
- [x] AC2: An edit made between a `Start` and a subsequent `Advance` on the
  same scope commits as exactly one `AiExclusive` mutation event whose
  `before_tree`/`after_tree` match the Start baseline and the post-edit
  snapshot.
  - Validate: `runtime::coordinator::tests::exclusive_edit_between_start_and_advance_commits_one_event`
- [x] AC3: Re-processing the identical `(scope, event)` boundary a second time
  produces no duplicated mutation evidence.
  - Validate: `runtime::coordinator::tests::replaying_the_same_scope_event_key_does_not_duplicate_evidence`
- [x] AC4: A mutation made just before `Close` is still attributed using the
  scope set as it existed immediately before `Close`'s own scope transition.
  - Validate: `runtime::coordinator::tests::close_boundary_attributes_using_pre_close_scope_set`
- [x] AC5: Two concurrently active scopes on one worktree yield `AiContended`
  attribution for a subsequent `Advance`, independent of whether the two
  scopes share an `ActorKind`.
  - Validate: `runtime::coordinator::tests::contended_scopes_yield_ai_contended_same_and_different_actor`
- [x] AC6: Capturing a Git snapshot never mutates the caller's real index,
  staged changes, or working tree, correctly reflects staged, unstaged,
  untracked, and deleted state, excludes ignored files, and — on an unborn
  `HEAD` — starts from an explicitly initialized, genuinely valid empty
  Git index (`git read-tree --empty`), never from a bare, freshly created
  file the coordinator merely assumes Git will treat as empty, and produces
  a correct tree even when the unborn repository has no files at all.
  - Validate: `runtime::git_snapshot::tests::*` (index preservation, ignored files, deletion, unborn HEAD with a file, unborn HEAD with no files)
- [x] AC7: A snapshot's `TreeId` remains resolvable through `diff_trees`
  after the process that captured it has exited, its temporary index file no
  longer exists, **and after a `git gc --prune=now` / `git prune
  --expire=now` pass has run against the repository** — because it is
  reachable from an SCE-owned ref in the repository's normal refs namespace,
  not merely present in an isolated object store Git's own reachability
  analysis knows nothing about.
  - Validate: `runtime::git_snapshot::tests::snapshot_survives_a_fresh_process_and_temp_index_deletion`, `runtime::git_snapshot::tests::pinned_snapshot_survives_git_gc_prune_now`, `runtime::git_snapshot::tests::pinned_snapshot_survives_git_prune_expire_now`
- [x] AC8: When two invocations race to commit from the same durable
  revision, exactly one succeeds, the other reloads durable state and
  recomputes its transition using its own originally captured snapshot, and
  no second Git snapshot is taken for that invocation.
  - Validate: `runtime::coordinator::tests::cas_conflict_reloads_and_recomputes_without_a_second_snapshot`
- [x] AC9: Two coordinator invocations targeting the same worktree cannot
  execute their critical sections concurrently. Invocations on two different
  worktrees, including linked worktrees of the same repository, are not
  serialized against each other, derive distinct `WorktreeId`s, and persist
  independently into the same caller-supplied repository-scoped Agent Trace
  DB. `coordinate()` resolves `git_dir`, checkout identity, and `WorktreeId`
  from its `repository_root` argument; the `RepositoryAgentTraceDb` is
  supplied by the caller, not resolved by the coordinator.
  - Validate: `runtime::worktree_lock::tests::*` (contention, distinct-path independence); `runtime::tests::linked_worktrees_have_independent_locks_and_worktree_ids`
- [x] AC10: A worktree whose durable state is `SnapshotFailure`-tainted or
  `needs_rebaseline` is recovered exactly once, using the same snapshot
  captured for the triggering boundary, before that boundary is processed;
  `needs_rebaseline` recovery preserves live scopes, taint recovery abandons
  them.
  - Validate: `runtime::coordinator::tests::recovers_from_needs_rebaseline_preserving_live_scopes`, `runtime::coordinator::tests::recovers_from_snapshot_failure_taint_abandoning_live_scopes`
- [x] AC11: A Git snapshot failure against an already-materialized worktree
  durably persists a `SnapshotFailure` taint via `protocol::taint`, retried
  under the same bounded semantic CAS-retry policy the main coordinator loop
  uses when another committer's transition wins the race; `persisted_taint`
  is `true` **only once a taint transition has actually been `Applied`**,
  never merely attempted. A snapshot failure with no prior worktree row
  makes no durable write and never fabricates a `TreeId`. The
  bootstrap-vs-taint decision, and every taint-retry iteration, reads
  durable worktree state fresh **after** the failure has already occurred —
  never state read earlier in the same invocation — so a worktree another
  caller materializes concurrently, while this invocation's own Git snapshot
  is still being captured, is still found and correctly tainted.
  - Validate: `runtime::coordinator::tests::snapshot_failure_taints_an_existing_worktree`, `runtime::coordinator::tests::snapshot_failure_taint_survives_a_losing_cas_and_commits_on_retry`, `runtime::coordinator::tests::snapshot_failure_taint_reports_not_persisted_after_retries_are_exhausted`, `runtime::coordinator::tests::snapshot_failure_before_any_baseline_makes_no_durable_write`, `runtime::coordinator::tests::snapshot_failure_taints_a_worktree_materialized_concurrently_during_capture`
- [x] AC12: All concurrent first-time callers of
  `checkout::get_or_create_checkout_id` for one physical checkout — whether
  through the coordinator, `agent_trace_storage`, or any other caller —
  converge on exactly one checkout ID, and the on-disk `checkout-id` file
  ends up containing that same value.
  - Validate: `checkout::tests::concurrent_first_time_callers_converge_on_one_checkout_id`, `runtime::tests::agent_trace_storage_and_coordinator_observe_the_same_checkout_id`
- [x] AC13: For cooperating SCE processes, the canonical `checkout-id` path
  is at every observable point either absent or contains exactly one
  complete, valid checkout ID — never a partially written or truncated
  value — including immediately after a process is interrupted between
  creating its temporary identity file and renaming it into place, and an
  orphaned temporary file left behind by such an interruption never
  prevents a later call from converging on the canonical ID.
  - Validate: `checkout::tests::interruption_before_rename_leaves_the_canonical_path_absent`, `checkout::tests::completed_rename_leaves_the_canonical_path_with_a_complete_id`, `checkout::tests::an_orphaned_temp_file_does_not_block_convergence_on_the_canonical_id`

### Full validation

- `nix flake check`
- `nix run .#pkl-check-generated`

### Context sync

- `context/cli/mutation-trace-protocol.md` — "Target end-state architecture"
  currently states `coordinator.rs`/`git_snapshot.rs` "remain future work";
  update to reflect their existence and responsibilities at their actual
  location, `cli/src/services/mutation_trace/runtime/`.
- `context/cli/mutation-trace-store.md` — "Non-goals" currently states "no
  `coordinator.rs` or `git_snapshot.rs` exists yet"; update once they exist
  under `mutation_trace/runtime/`.
- `context/cli/checkout-identity.md` — currently documents
  `get_or_create_checkout_id` as a plain "reuses an existing ID or writes a
  new one"; update to document the identity-creation lock and the
  convergence guarantee it now provides to every caller.
- `context/overview.md` — the `mutation_trace` module description should
  mention the new runtime coordinator layer while preserving the accurate
  "not yet wired into any hook or command" framing.
- `context/context-map.md` — the `mutation-trace-protocol.md` line
  annotation names the `coordinator.rs`/`git_snapshot.rs` seams (now
  `mutation_trace/runtime/coordinator.rs`/`mutation_trace/runtime/git_snapshot.rs`)
  as not-yet-created; update once T06 lands.
- A new domain file (e.g. `context/cli/mutation-trace-runtime-coordinator.md`)
  documenting the lock, snapshot, ref-durability, and coordinator design
  decisions below is likely warranted; leave the exact filename to task
  context synchronization.

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** `cli/src/services/checkout/mod.rs` (behavior change:
  `get_or_create_checkout_id` becomes concurrency-safe),
  `cli/src/services/mutation_trace/runtime/mod.rs` (new),
  `cli/src/services/mutation_trace/runtime/worktree_lock.rs` (new),
  `cli/src/services/mutation_trace/runtime/git_snapshot.rs` (new),
  `cli/src/services/mutation_trace/runtime/coordinator.rs` (new),
  `cli/src/services/mutation_trace/runtime/tests.rs` (new),
  `cli/src/services/mutation_trace/mod.rs` (module registration only, to
  declare `pub(crate) mod runtime;`).
- **Out of scope:** Claude/Codex/OpenCode/Pi hook translation, Bash
  `PreToolUse`/`PostToolUse` wiring, final Agent Trace `diff_traces`
  insertion, commit attribution, auto-sync/remote sync, control-plane
  changes, redesigning the verified Quint protocol, changing existing
  mutation attribution semantics, general Git abstraction refactors
  (`capabilities::GitOps`) unrelated to this protocol, a daemon/background
  process, cross-machine locking, the filesystem `externalTaint`/`TAINTED`
  marker (see Design decisions — Requirement 10), and a repository-wide
  `refs/sce/mutation-cursor/**` reconciliation/pruning pass (see Design
  decisions — snapshot durability, and Follow-up PR). Both deferred items
  are required runtime-completion work before any harness adapter becomes a
  production consumer of this coordinator — see Follow-up PR's "Runtime
  completion sequence" — but neither is implemented by this PR's own task
  stack. The `get_or_create_checkout_id` fix is a narrow, internal-locking
  and internal-persistence change to an existing function's body — it
  changes neither its signature nor the checkout-ID format, and is not the
  "broad checkout identity redesign" this plan's brief separately excludes
  (for example, changing how `WorktreeId` is derived, or introducing a
  different identity scheme).
- **Constraints:** no new Cargo dependencies (`std::fs::File::lock`/
  `try_lock`/`unlock`, stabilized in Rust 1.89 and confirmed to compile and
  run under this repository's pinned `1.95.0` toolchain, cover every OS
  advisory lock this plan needs; `uuid` already provides `v4` for
  `AttemptId` generation); no new database migration (`store.rs`'s existing
  schema and API are sufficient); `protocol.rs`/`types.rs` are not modified;
  `cargo clippy` runs with `pedantic`/`warnings` denied workspace-wide.
  `git update-ref`/`git for-each-ref` (validated experimentally, see Design
  decisions) are the only new Git plumbing commands beyond the previously
  validated `read-tree`/`add`/`write-tree`/`diff`.
- **Non-goal:** implementing `protocol::database_failure` end to end. The
  coordinator never calls it in this PR (see Design decisions — Requirement
  10); wiring it is the immediate follow-up PR's first concern.

## Task stack

- [x] T01: `Make checkout-identity creation concurrency-safe and crash-safe` (status:done)
  - Task ID: T01
  - Scope: In — `cli/src/services/checkout/mod.rs`:
    `get_or_create_checkout_id` gains an internal, dedicated
    `<git-dir>/sce/checkout-id.lock` acquired only on the slow path (the
    existing lock-free `read_checkout_id` fast path is unchanged for the
    already-created case), re-checking for an existing ID under the lock
    before generating one; the write itself goes through a unique
    `<git-dir>/sce/checkout-id.tmp-<checkout-id>` file
    (`OpenOptions::create_new(true)`), a data sync (`File::sync_data()`),
    and an atomic `std::fs::rename` into the canonical `checkout-id` path,
    with a best-effort `#[cfg(unix)]` parent-directory sync after the rename
    for additional crash-durability hardening. Out — the mutation-cursor
    runtime lock (T02); any change to `resolve_git_dir`,
    `read_checkout_id`'s public signature, or the checkout-ID file format;
    any change to `agent_trace_storage` (it benefits automatically, with no
    caller-side change needed); any cleanup pass for orphaned
    `checkout-id.tmp-*` files (unnecessary — see Design decisions).
  - Dependencies: none
  - Done when: concurrent first-time callers on the same `git_dir` — from
    any call site — converge on exactly one checkout ID, and the on-disk
    file contains that value; an already-created checkout ID is still read
    without ever acquiring the new lock; the canonical `checkout-id` path is
    never observable as partially written, at any interruption point in the
    write sequence; an orphaned `checkout-id.tmp-*` file left by an
    interrupted attempt never blocks a later call from creating the
    canonical file.
  - Verify: `cargo test -p shared-context-engineering checkout::` (via `./scripts/run-cli-cargo.sh test`)
  - Completed: 2026-08-29
  - Files changed: `cli/src/services/checkout/mod.rs`
  - Result: `get_or_create_checkout_id` now acquires a dedicated
    `<git-dir>/sce/checkout-id.lock` (blocking `std::fs::File::lock()`, no
    timeout) only on the slow path, re-checking `read_checkout_id` under the
    lock before generating a new ID; the previously-created case still
    returns via the unchanged, lock-free `read_checkout_id` fast path. The
    write itself now goes through a unique
    `checkout-id.tmp-<checkout-id>` file created with
    `OpenOptions::create_new(true)`, `write_all`, `File::sync_data()`, then
    an atomic `std::fs::rename` into the canonical `checkout-id` path, with a
    best-effort `#[cfg(unix)]` sync of the parent `sce/` directory handle
    afterward. No public signature, checkout-ID file format, or caller
    (including `agent_trace_storage`) changed. Added a `#[cfg(test)] mod
    tests` covering concurrent first-time convergence, the fast path never
    touching the lock, a completed rename leaving a complete ID, a simulated
    crash before rename leaving the canonical path absent, and an orphaned
    temp file not blocking a later call.

    PR #244 review follow-up: the crash-safety test originally reimplemented
    the create-temp/write/sync/(no rename) sequence inline instead of
    exercising production code, so a regression to an unsafe direct write
    could still have passed it. The temp-file/write/sync/rename/directory-sync
    sequence is now factored out of `get_or_create_checkout_id` into a
    private `persist_checkout_id(checkout_dir, checkout_id)` helper, whose
    name and signature describe only the production responsibility (persist
    one checkout ID crash-safely) with no test vocabulary in it. It delegates
    to a lower-level private `persist_checkout_id_inner(checkout_dir,
    checkout_id, before_rename)`, where `before_rename` runs after the temp
    file is written and synced but before the rename;
    `persist_checkout_id` itself calls the inner helper with a no-op
    `before_rename`, so `get_or_create_checkout_id` never mentions the test
    seam at all, while both crash-safety tests call `persist_checkout_id_inner`
    directly to reach it. `completed_rename_leaves_the_canonical_path_with_a_complete_id`
    calls the public-shaped `persist_checkout_id` (no injected interruption)
    and asserts the canonical file exists, contains exactly the generated ID,
    and parses as a valid UUID. `interruption_before_rename_leaves_the_canonical_path_absent`
    calls `persist_checkout_id_inner` directly and injects a `before_rename`
    that asserts the temp file already contains the complete ID and the
    canonical path is still absent, then returns an error to abort before the
    rename runs; the test then asserts the canonical path stays absent and
    `read_checkout_id` returns `None`. Crash-safety is now tested through the
    actual, shared production persistence implementation with an injected
    pre-rename failure, not through filesystem steps duplicated in the test.
  - Verify (actual): `./scripts/run-cli-cargo.sh test --manifest-path
    cli/Cargo.toml checkout::` → 5/5 passed. Additionally ran
    `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml
    --all-targets -- -D warnings` → clean (pedantic/warnings denied
    workspace-wide, per Constraints), and `./scripts/run-cli-cargo.sh fmt
    --manifest-path cli/Cargo.toml -- --check` → clean. Additionally ran
    `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml
    agent_trace_storage::` → 14/14 passed (no regression in the other,
    unmodified caller).
  - Context impact:
    - Updated `context/cli/checkout-identity.md` to document the
      identity-creation lock, the crash-safe temp-file/rename write, the
      convergence guarantee across every caller, and the file's now-present
      inline unit test coverage (previously documented as absent).
    - Updated `context/patterns.md`'s "Unit testing in Nix sandbox" section
      to document the filesystem-touching inline-unit-test pattern this
      task's tests use (unique `std::env::temp_dir()` paths, Nix-sandbox-safe
      via `TMPDIR`) as an established, code-verified pattern rather than the
      stale "integration tests only" rule it stated before.
  - Context synchronization: synced

- [x] T02: `Add the per-worktree OS advisory runtime lock` (status:done)
  - Task ID: T02
  - Scope: In — `runtime/worktree_lock.rs`: `WorktreeLock::acquire(git_dir, timeout)`,
    RAII release, bounded polling acquisition. Out — wiring into
    `runtime/coordinator.rs` (T05); resolving `git_dir` itself (caller-supplied);
    the distinct checkout-identity-creation lock (T01, already landed by the
    time this task depends on nothing from it).
  - Dependencies: none
  - Done when: `WorktreeLock::acquire` opens/creates
    `<git_dir>/sce/mutation-cursor.lock`, blocks a second acquirer on the same
    path until the first releases (via `Drop`) or the bounded timeout elapses,
    returns a distinct, matchable error on timeout, and never treats the
    lock file's mere existence as ownership.
  - Verify: `cargo test -p shared-context-engineering runtime::worktree_lock::` (via `./scripts/run-cli-cargo.sh test`)
  - Completed: 2026-08-30
  - Files changed: `cli/src/services/mutation_trace/runtime/mod.rs` (new),
    `cli/src/services/mutation_trace/runtime/worktree_lock.rs` (new),
    `cli/src/services/mutation_trace/mod.rs` (added `pub(crate) mod runtime;`)
  - Result: Added `runtime/mod.rs`, declaring `mod worktree_lock;` (private —
    only `coordinator`'s future public entrypoints will be reachable from
    outside `runtime`, per the plan's module-privacy design), and registered
    `pub(crate) mod runtime;` in `mutation_trace/mod.rs`. This scaffolding was
    not explicitly named in T02's own Scope line but was necessary for
    `worktree_lock.rs` to compile and for its tests to run — recorded as a
    reviewed assumption, not a deviation from scope.

    `WorktreeLock::acquire(git_dir: &Path, timeout: Duration) ->
    Result<WorktreeLock, WorktreeLockError>` opens/creates
    `<git_dir>/sce/mutation-cursor.lock` via `OpenOptions` (matching T01's
    file-creation convention), then polls `File::try_lock()` every 100ms
    (`LOCK_POLL_INTERVAL`) against the caller-supplied bounded `timeout`
    rather than calling the blocking `lock()` directly — never treating the
    lock file's mere existence as ownership, only a successful OS-level
    `try_lock()`. On timeout it returns a distinct `WorktreeLockError::TimedOut
    { path, timeout }` variant (matchable independently of the `Io` variant
    wrapping other failures), matching the plan's design decision that this
    lock is bounded, unlike the checkout-identity lock. `WorktreeLock`
    implements `Drop`, releasing the OS lock (`self.file.unlock()`,
    best-effort) when the guard is dropped — RAII release, matching the
    Done-when requirement. `WorktreeLockError` implements `Display` and
    `std::error::Error`, and derives `Debug`, consistent with an ordinary
    matchable Rust error type (this task's own error type — not the
    coordinator's later `CoordinateError::LockAcquisition(anyhow::Error)`,
    which T05 will construct by wrapping whatever this function returns).
    Added a `#[cfg(test)] mod tests` covering: a second acquirer blocking
    until the first releases (via a channel-signaled background thread);
    two distinct worktree paths not contending; `acquire` timing out with a
    distinct, matchable `TimedOut` error (asserting the exact `timeout` field)
    when the lock is still held; and a leftover lock file written directly to
    disk, with no `.lock()`/`.try_lock()` ever called against it, not
    blocking a fresh acquirer — proving the "mere existence is not ownership"
    requirement.

    PR #244 review follow-up: two issues were corrected without changing
    locking design or T02 semantics. First, the module doc comment's claim
    that "`File::lock()`/`try_lock()` have no built-in timeout and block
    indefinitely" was inaccurate for `try_lock()`, which is non-blocking and
    returns `TryLockError::WouldBlock` immediately on contention; the comment
    now states that `File::lock()` can block indefinitely while
    `File::try_lock()` is non-blocking but provides no waiting deadline by
    itself, so `WorktreeLock::acquire` polls it at a short interval until
    acquisition succeeds or the caller-supplied deadline expires. The plan's
    own "Blocking vs. timeout" design-decision text and
    `context/cli/mutation-trace-runtime-coordinator.md` were both checked
    against the same claim and found already accurate (they describe
    `File::lock()`'s blocking behavior specifically, never claiming
    `try_lock()` blocks), so neither needed a correction.

    Second, `a_second_acquirer_blocks_until_the_first_releases` previously
    inferred "the worker was blocked" from a 300ms `recv_timeout` on a single
    completion channel — a scheduling false positive, since a merely delayed
    (not blocked) worker thread would produce the same observation without
    proving it ever reached `WorktreeLock::acquire`. The test now uses two
    channels: the worker signals a dedicated `started` channel immediately
    before calling `acquire`, and the main thread waits (with a generous
    5-second bound) for that signal before asserting non-completion — so the
    300ms `recv_timeout` now only bounds the specific assertion "the worker,
    having definitely reached the acquisition attempt, must not complete
    while the first guard is held," rather than standing in for proof the
    worker started. After dropping the first guard, the test now waits for
    the worker's result signal (bounded, not a bare `join()`) before joining
    the thread and asserting success, so the thread is never left detached.
    This strengthened test now fails if `WorktreeLock::acquire` were
    accidentally changed to let a second caller acquire while the first
    guard remained alive.

    Third, at the user's explicit request, every comment in
    `worktree_lock.rs` and `runtime/mod.rs` was removed — including the
    module doc comment whose corrected wording is quoted above and every
    `///`/`//` comment on items, fields, and test steps — since both files
    were authored entirely during this task and its review follow-up. This
    superseded the module-doc-comment correction above at the text level
    without reopening its substance: the inaccurate `try_lock()` claim no
    longer exists in either corrected or original form, because no doc
    comment remains. Production behavior, the public API, and all four
    tests are unchanged; `WorktreeLock`, `WorktreeLockError`, and their
    documented semantics above remain the authoritative description of this
    module's behavior now that the code carries no comments of its own.

    Fourth, the "Second" fix above still had a scheduler false positive:
    signaling "started" immediately before calling `WorktreeLock::acquire`
    proved only that the worker reached the call site, not that it actually
    executed `try_lock()` and observed contention — a descheduled-before-call
    worker would have produced the same passing result. `acquire`'s body
    moved into a new private `acquire_inner(git_dir, timeout, on_contention:
    impl FnOnce())` that `acquire` calls with a no-op closure
    (`acquire_inner(git_dir, timeout, || {})`), keeping the public API and
    every other behavior (deadline calculation, poll interval, error
    variants, RAII release, stale-lock-file handling, per-worktree
    independence) unchanged. The `on_contention` callback fires at most once,
    only from inside the already-existing `Err(TryLockError::WouldBlock)`
    arm, strictly after that branch is reached and before the existing
    deadline check/sleep — it observes contention, it does not create or
    influence it. The contention test now calls `acquire_inner` directly with
    a closure that signals a dedicated channel, and waits on that channel
    (bounded only as a deadlock guard) before asserting non-completion and
    dropping the first guard. The contention test observes the actual
    `TryLockError::WouldBlock` branch before the first guard is released,
    then proves the second acquirer succeeds after `Drop` — a real
    happens-before proof of OS-level contention, not an inference from
    timing. This test now fails if the `WouldBlock` branch were no longer
    reached while the first lock is held. Re-ran the contention test 8
    consecutive times with no flakiness.
  - Verify (actual): `./scripts/run-cli-cargo.sh test --manifest-path
    cli/Cargo.toml runtime::worktree_lock::` → 4/4 passed (re-run after the
    "Fourth" `acquire_inner` fix above; the contention test was additionally
    run 8 consecutive times in isolation with no flakiness). Additionally ran
    `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml
    --all-targets -- -D warnings` → clean (pedantic/warnings denied
    workspace-wide, per Constraints), and `./scripts/run-cli-cargo.sh fmt
    --manifest-path cli/Cargo.toml -- --check` → clean (after running
    `cargo fmt` once to apply formatting corrections, on the original T02
    implementation; every subsequent PR #244 review follow-up change,
    including the `acquire_inner` seam, was already fmt-clean).
  - Context impact: `domain` — introduced a new architectural element
    (`WorktreeLock`, the mutation-cursor runtime lock) not owned by any
    existing context file. Added `context/cli/mutation-trace-runtime-coordinator.md`
    describing it, linked it from `context/context-map.md`, and cross-linked
    it from `context/cli/checkout-identity.md`'s "See also" line. The five
    root context files (`overview.md`, `architecture.md`, `glossary.md`,
    `patterns.md`, `context-map.md`) were verified against this change;
    `context-map.md` was edited to add the new domain file's index entry,
    the other four remained accurate and were not edited. No qualifying
    system-wide architecture decision was introduced by this task — the
    lock's design (two distinct locks, bounded polling, std-only locking)
    was already established in the plan's own Design decisions section
    during plan authoring, not originated by this task's execution.
  - Context synchronization: synced

- [x] T03: `Add the isolated Git snapshot service with ref-pinned durability` (status:done)
  - Task ID: T03
  - Scope: In — `runtime/git_snapshot.rs`: `GitSnapshotService` (resolves
    `--git-dir` once; a unique, never-pre-created private temp index path
    under `<git-dir>/sce/tmp/`, reserved by the RAII guard but left for Git
    itself to create via `read-tree`; writes tree/blob objects into the
    repository's normal, shared object database — no private object store,
    no `GIT_OBJECT_DIRECTORY`/`GIT_ALTERNATE_OBJECT_DIRECTORIES` overrides),
    `capture_tree` (branches on HEAD's existence between `git read-tree
    HEAD` and `git read-tree --empty`, never a bare/absent index file),
    `pin_tree` (creates `refs/sce/mutation-cursor/<worktree-id>/<tree-sha>`),
    `diff_trees`. Out — wiring `capture_tree`/`pin_tree` into
    `runtime/coordinator.rs` (T04); locking (T02/T05 own that); any ref
    deletion/reconciliation logic (deferred, see Design decisions and
    Follow-up PR — this PR's pins are create-only).
  - Dependencies: none
  - Done when: `capture_tree` never mutates the real index/working
    tree/staged diff, respects `.gitignore`, captures modifications,
    additions, deletions, and an unborn `HEAD` — with and without existing
    files — by explicitly initializing a valid empty index via
    `git read-tree --empty` rather than relying on a bare temp-index file,
    and produces an opaque `TreeId` whose length is never assumed; a tree
    `pin_tree` protects remains resolvable via `diff_trees` after
    `git gc --prune=now` and `git prune --expire=now`, while a distinct,
    genuinely unreachable control tree captured and left unpinned in the
    same repository is reclaimed by that same aggressive pass; `pin_tree` is
    idempotent for the same `(worktree_id, tree)` pair.
  - Verify: `cargo test -p shared-context-engineering runtime::git_snapshot::` (via `./scripts/run-cli-cargo.sh test`)
  - Completed: 2026-08-30
  - Files changed: `cli/src/services/mutation_trace/runtime/git_snapshot.rs` (new),
    `cli/src/services/mutation_trace/runtime/mod.rs` (added `mod git_snapshot;`)
  - Result: Added `runtime/git_snapshot.rs` with a `pub struct GitSnapshotService`
    (private module, matching `worktree_lock`'s module-privacy convention —
    reachable from sibling modules under `runtime`, not outside it).
    `GitSnapshotService::new(repository_root)` resolves `--git-dir` once via
    `git rev-parse --git-dir` (the same resolution logic as
    `checkout::resolve_git_dir`, duplicated locally rather than imported,
    since `checkout`'s function is scoped to that module's own concerns and
    this plan's Constraints exclude broad Git-abstraction refactors) and
    stores both the resolved `git_dir` and `repository_root`.

    `capture_tree`: reserves a unique `<git-dir>/sce/tmp/index-<uuid>` path
    via a private `TempIndexGuard` (constructs the path only, creates the
    `tmp/` directory, never touches the index file itself, best-effort
    removes it on `Drop`); probes `HEAD` via
    `git rev-parse --verify --quiet HEAD`; runs `git read-tree HEAD` when
    `HEAD` exists or `git read-tree --empty` on an unborn `HEAD`, both with
    only `GIT_DIR`/`GIT_INDEX_FILE` set and `cwd = repository_root`; then
    `git add -A -- .`; then `git write-tree`, returning the raw trimmed SHA
    wrapped in `TreeId`, never assuming a fixed length. `pin_tree`
    (`git update-ref refs/sce/mutation-cursor/<worktree_id>/<tree-sha>
    <tree-sha>`, only `GIT_DIR` set) and `diff_trees` (`git diff --binary
    --full-index --no-ext-diff --no-textconv <before> <after>`, only
    `GIT_DIR` set, returning the raw `String`) match the plan's validated
    command sequences exactly. No `GIT_OBJECT_DIRECTORY`/
    `GIT_ALTERNATE_OBJECT_DIRECTORIES` environment variable is set anywhere.

    Added a `#[cfg(test)] mod tests` (real, per-test temporary Git
    repositories via `git init`, following T01/T02's unique-temp-dir
    convention) covering: staged/unstaged/untracked/deleted state captured
    correctly while the real index, `git status --porcelain`, `git diff`,
    and `git diff --cached` stay byte-identical before and after capture;
    `.gitignore` exclusion; a committed-file deletion reflected as absent;
    unborn `HEAD` with an untracked file present and with no files at all
    (asserting an empty `ls-tree`); a captured tree remaining resolvable via
    `git cat-file` after the temp index file is gone; a `pin_tree`-protected
    tree surviving both `git gc --prune=now` and `git prune --expire=now`
    while a distinct-content, genuinely unreachable tree captured and left
    unpinned in the same repository is reclaimed by the same pass (run as
    two separate tests, one per pruning command, both using distinct-content
    trees per the plan's corrected experimental design — never a
    same-content decoy); `pin_tree` idempotency for the same
    `(worktree_id, tree)` pair; `diff_trees` producing `git diff --git`
    formatted, parseable output; and a best-effort SHA-256 repository case
    (`git init --object-format=sha256`) that skips gracefully when the local
    Git build lacks the flag, asserting `TreeId` needs no length assumption.

    PR #244 review follow-up: two correctness issues were fixed without
    changing the snapshot design. First, HEAD-absence and HEAD-probe-failure
    semantics were conflated: `capture_tree` previously ran `git rev-parse
    --verify --quiet HEAD` through the shared `run_git` helper and used
    `.is_ok()` to pick between `read-tree HEAD`/`read-tree --empty`, so *any*
    Git failure — a corrupted repository, a missing `.git/HEAD`, an
    unexpected fatal error — was silently treated as "unborn HEAD" and
    produced a false empty-baseline snapshot instead of surfacing an error.
    A dedicated `head_exists(&self) -> Result<bool>` now runs that probe
    directly and inspects `output.status.code()`: exit status `0` (HEAD
    resolves) → `Ok(true)`; exit status `1` (`--verify --quiet`'s documented
    "does not resolve" signal — verified experimentally against this
    repository's Git as the exit status for a genuinely unborn HEAD) →
    `Ok(false)`; every other status (verified experimentally: a corrupted or
    missing `.git/HEAD` produces exit status `128`, "fatal: not a git
    repository") → `Err`, propagated by `capture_tree` via `?` before ever
    reaching `read-tree --empty`. HEAD absence is a normal Git state; HEAD
    probe failure is a snapshot failure — the two are no longer conflated.

    Second, `resolve_git_dir` used `git rev-parse --git-dir` and manually
    joined a relative result onto `repository_root` when not already
    absolute — but that join only produces a genuinely absolute path when
    `repository_root` itself is absolute. Given a relative
    `repository_root`, the stored `git_dir` stayed relative, and since every
    later Git subprocess runs with `cwd = repository_root` (also relative)
    and `GIT_DIR = self.git_dir`, the relative `GIT_DIR` was resolved by the
    child process against its own (already `repository_root`-joined) `cwd`,
    double-joining the path and pointing at the wrong directory.
    `resolve_git_dir` now runs `git rev-parse --absolute-git-dir` instead —
    verified experimentally to return a canonicalized absolute path for both
    a normal clone and a linked worktree — eliminating the manual
    absolute/relative branch entirely, with a `debug_assert!` documenting the
    invariant. `GitSnapshotService` now always stores an absolute `git_dir`,
    regardless of whether the caller's `repository_root` was relative or
    absolute; `repository_root` itself is unchanged (still used only as
    `Command::current_dir`, which is resolution-agnostic).

    Added two regression tests:
    `an_unexpected_head_probe_failure_propagates_instead_of_using_read_tree_empty`
    (deletes `.git/HEAD` after constructing the service — a real Git failure
    distinct from unborn HEAD — and asserts `capture_tree()` returns `Err`
    rather than a false empty-baseline capture) and
    `resolves_an_absolute_git_dir_from_a_relative_repository_root` (computes
    a genuinely relative `repository_root` from the real, unmutated process
    `cwd` via a small test-local `relative_path_from` helper — no
    `std::env::set_current_dir`, so the test is parallel-safe — and proves
    `GitSnapshotService::new`, `capture_tree`, `pin_tree`, and `diff_trees`
    all succeed with `git_dir` resolved absolute). The existing unborn-HEAD
    tests (`capture_on_unborn_head_with_a_file_produces_a_valid_tree`,
    `capture_on_unborn_head_with_no_files_produces_an_empty_tree`) continue
    to prove the genuinely-unborn path still produces a valid `read-tree
    --empty` snapshot; the new failure test proves the two paths no longer
    share a code path they should not share.
  - Verify (actual): `./scripts/run-cli-cargo.sh test --manifest-path
    cli/Cargo.toml runtime::git_snapshot::` → 11/11 passed (pre-follow-up);
    13/13 passed (post-follow-up, including the two new regression tests).
    Additionally ran `./scripts/run-cli-cargo.sh test --manifest-path
    cli/Cargo.toml runtime::` → 20/20 passed (pre-follow-up), 22/22 passed
    (post-follow-up; no regression in `worktree_lock`'s existing tests).
    Additionally ran `./scripts/run-cli-cargo.sh clippy --manifest-path
    cli/Cargo.toml --all-targets -- -D warnings` → clean both times
    (pedantic/warnings denied workspace-wide, per Constraints; fixed two
    pedantic findings — `uninlined_format_args` and `map_unwrap_or` — during
    the original implementation; the follow-up introduced none), and
    `./scripts/run-cli-cargo.sh fmt --manifest-path cli/Cargo.toml -- --check`
    → clean both times.
  - Context impact: `domain` — introduces a new architectural element
    (`GitSnapshotService`, the isolated Git snapshot/ref-pinning service)
    under the mutation-cursor-runtime-coordinator domain T02 already
    established a domain file for. Updated
    `context/cli/mutation-trace-runtime-coordinator.md` to document
    `GitSnapshotService` (`capture_tree`/`pin_tree`/`diff_trees`), the
    updated on-disk layout (`sce/tmp/index-<uuid>`, the
    `refs/sce/mutation-cursor/**` namespace), its test coverage, and the
    revised Status section. Corrected two now-stale statements this task's
    landing directly falsified: `context/cli/mutation-trace-protocol.md`'s
    "Target end-state architecture" said `coordinator.rs`/`git_snapshot.rs`
    "remain future work" — updated its prose, Mermaid diagram, and
    per-seam bullets to mark `runtime/git_snapshot.rs` implemented while
    `coordinator.rs` remains future work; `context/cli/mutation-trace-store.md`'s
    "Non-goals" said "no `coordinator.rs` or `git_snapshot.rs` exists yet" —
    corrected to note `runtime/git_snapshot.rs` now exists as a sibling
    module, not yet wired to the store. Updated `context/context-map.md`'s
    entries for both files accordingly. No repository-wide behavior,
    architecture, or terminology changed, so `overview.md`, `architecture.md`,
    `glossary.md`, and `patterns.md` were verified against the change and
    left unedited. No qualifying system-wide architecture decision was
    introduced by this task — the ref-pinning/normal-object-database design
    was already established in the plan's own Design decisions section
    during plan authoring, not originated by this task's execution.

    PR #244 review follow-up: corrected `context/cli/mutation-trace-runtime-coordinator.md`'s
    `git_snapshot.rs` bullet, which the follow-up fix directly falsified —
    it said `GitSnapshotService::new` "resolves `--git-dir` once" (now
    `--absolute-git-dir`, with the absolute-path invariant and the reason it
    matters explained inline) and described the HEAD branch without
    distinguishing probe failure from genuine absence (now documents the
    `head_exists` exit-status distinction). Also extended the Testing
    boundary section's `git_snapshot.rs` test-coverage summary to name the
    two new regression tests. No other context file was affected by this
    follow-up: it is an internal correctness fix with no public-interface,
    behavior-contract, or architecture change beyond what the two
    corrections above capture.
  - Context synchronization: synced

- [x] T04: `Add the coordinator's core protocol-integration pipeline` (status:done)
  - Task ID: T04
  - Scope: In — `runtime/coordinator.rs`: `RuntimeBoundary` (with its
    `(ScopeId, EventId)` replay-identity contract documented on the type),
    `CoordinateOutcome`, `CoordinateError`, `SnapshotCapture` trait
    (dependency-injection seam covering `capture` and `pin`), worktree/scope
    materialization ordering (no worktree-existence read before Git snapshot
    capture — that decision is made exactly once, fresh, inside the
    snapshot-failure handler, never earlier), the load → recover-if-needed →
    `prepare`/`commit` sequence with its bounded CAS-conflict retry loop
    reusing one captured and already-pinned `TreeId`, and snapshot-failure
    taint handling whose own bounded CAS-conflict retry loop's first
    iteration is also where the bootstrap-vs-taint branch is decided. An
    internal, generic-over-`SnapshotCapture` entrypoint is the unit under
    test here; the public, lock-wrapped `coordinate()` entrypoint is T05.
    Out — actual `WorktreeLock` acquisition (T05); checkout-id resolution
    (T05, and now concurrency-safe and crash-safe by construction of T01
    regardless of ordering); ref deletion/reconciliation (deferred);
    cross-module integration tests (T06).
  - Dependencies: T03 (uses `GitSnapshotService` as the production
    `SnapshotCapture` implementation; tests use a fake).
  - Done when: every scenario in AC1–AC5, AC8, AC10, AC11 passes against the
    internal generic entrypoint using a real temp-file `RepositoryAgentTraceDb`
    and either the real `GitSnapshotService` or a fake, call-counting
    `SnapshotCapture`; the fake proves both "exactly one `capture`" and "at
    most one `pin`" per invocation, across CAS retries; a test using an
    injected snapshot-capture failure and controlled store sequencing proves
    a worktree row materialized *during* a failing capture attempt (not
    before it) is still found and tainted by that same failing invocation.
  - Verify: `cargo test -p shared-context-engineering runtime::coordinator::` (via `./scripts/run-cli-cargo.sh test`)
  - Completed: 2026-08-30
  - Files changed: `cli/src/services/mutation_trace/runtime/coordinator.rs` (new),
    `cli/src/services/mutation_trace/runtime/mod.rs` (added `mod coordinator;`)
  - Result: Added `runtime/coordinator.rs` with `RuntimeBoundary` (`Start`/
    `Advance`/`Close` carrying `{ scope, event, actor_kind }`, `Flush` carrying
    nothing — its `WorktreeId` is always the invocation's already-resolved
    worktree, never caller-supplied — with the `(ScopeId, EventId)`
    replay-identity contract from `spec/mutation_cursor.md`'s "Event identity"
    section documented on the type's doc comment), `CoordinateOutcome`
    (`worktree_id`/`observed_tree`/`revision`/`evaluation`/`mutation_event`,
    exactly the plan's sketch), `CoordinateError` (`SnapshotFailure`/
    `ScopeIdentityConflict`/`CasConflictExhausted`/`LockAcquisition`/`Other`,
    with `Display`/`std::error::Error` impls; `LockAcquisition` is reserved for
    T05 and never constructed here — `#[allow(dead_code)]` on
    `pub mod mutation_trace;` in `services/mod.rs` already covers it), and the
    `SnapshotCapture` trait (`capture(&self) -> Result<TreeId>`,
    `pin(&self, worktree_id: &WorktreeId, tree: &TreeId) -> Result<()>` —
    taking no `repository_root` parameter, since `GitSnapshotService`
    (T03) already binds it at construction; a reviewed deviation from the
    plan's earlier trait sketch, not from `GitSnapshotService`'s actual
    signature), implemented for `GitSnapshotService` by direct delegation to
    `capture_tree`/`pin_tree`.

    The private `coordinate_boundary<C: SnapshotCapture>(db, capture,
    worktree_id, boundary: &RuntimeBoundary)` is the internal,
    generic-over-`SnapshotCapture` entrypoint the plan calls for: it captures
    and pins the one Git snapshot for the invocation; on failure, delegates to
    `handle_snapshot_failure`/`run_taint_retry_loop_inner` and returns
    `SnapshotFailure` without ever reaching the pipeline below; on success,
    calls `initialize_worktree` (idempotent) and, for hook boundaries only,
    `register_scope` (mapping `ScopeIdentityConflict` errors), then runs the
    bounded (`MAX_CAS_RETRY_ATTEMPTS = 5`, no backoff) `load_worktree` →
    recover-if-needed (its own `DurableTransition`/CAS commit, retried via
    `continue` on `Conflict`) → `prepare`/`commit` the triggering boundary (a
    second `DurableTransition`/CAS commit) loop, reusing the one captured
    `observed_tree` and the same `AttemptId` generation
    (`Uuid::new_v4()`) every iteration; a `None` `DurableTransition` (a
    stale/rejected/replayed attempt) is a successful no-op return, not an
    error. `run_taint_retry_loop_inner` implements the plan's exact
    snapshot-failure pseudocode: a fresh `load_worktree(worktree, None, None)`
    on every iteration including the first, always evaluated after the
    triggering failure; `None` → `persisted_taint: false` (bootstrap, no
    durable write); an already-tainted no-op `taint()` transition → reads back
    the current `tainted` flag rather than assuming success; otherwise commits
    the taint transition and retries on `Conflict`; exhaustion after 5
    attempts → `persisted_taint: false`. `run_taint_retry_loop` (production,
    used by `handle_snapshot_failure`) is a thin wrapper over
    `run_taint_retry_loop_inner` with a no-op `after_load` hook; the hook
    itself (`after_load: impl FnMut(u32)`, fired after each iteration's load
    and before its own commit) is a deterministic-testing seam mirroring
    `worktree_lock.rs`'s own `acquire_inner`/`on_contention` pattern (T02),
    used only by this task's own tests to inject a competing commit at a
    precise point without needing real thread synchronization.

    Added a `#[cfg(test)] mod tests` with 13 tests (11 exactly matching the
    plan's own AC1–AC5/AC8/AC10/AC11 "Validate" function names, plus the
    `assert_contended_attribution` helper counted under AC5's single test
    function, plus internal coverage): a `FakeSnapshotCapture` (scripted
    `Succeed`/`Fail` outcome queue, falling back to a fixed default tree, with
    `capture_call_count`/`pin_call_count`) proves AC1 (Flush at first
    observation stays at revision 0, no evidence, exactly one capture/pin),
    AC2 (Start-then-Advance commits one `AiExclusive` event with the correct
    before/after trees), AC3 (replaying the identical `(scope, event)`
    boundary is a no-op — the same revision, no duplicated event — proven via
    two sequential `coordinate_boundary` calls with the same boundary), AC4
    (Close still attributes to the scope it is about to close, then confirms
    the scope reaches `Closed`), and AC5 (`AiContended` for two live scopes,
    exercised for both same-`ActorKind` and different-`ActorKind` pairs via
    one test calling a shared `assert_contended_attribution` helper twice).
    AC8 (`cas_conflict_reloads_and_recomputes_without_a_second_snapshot`) uses
    3 real OS threads racing distinct `Flush` boundaries against one shared
    on-disk DB (separate `RepositoryAgentTraceDb` handles per thread, via
    `open_for_hooks_without_migrations_at`, matching `store.rs`'s own
    two-writer-race test convention) — deterministically bounded, since at
    most `WRITERS − 1 = 2` retries are ever needed against the
    `MAX_CAS_RETRY_ATTEMPTS = 5` bound — and asserts every writer captured
    exactly once, pinned exactly once, and landed at a distinct revision.
    AC10 has two tests: `recovers_from_needs_rebaseline_preserving_live_scopes`
    (directly commits a `protocol::abandon` transition to force
    `needs_rebaseline`, then proves a subsequent `Advance` recovers first —
    clearing the flag, rebaselining the cursor to the recovering invocation's
    own observed tree, emitting no evidence for the discarded interval — while
    the untouched live scope stays `Active`) and
    `recovers_from_snapshot_failure_taint_abandoning_live_scopes` (same
    pattern via a direct `protocol::taint` commit, proving the live scope
    instead becomes `Abandoned`). AC11 has five tests: taints an existing
    worktree; survives one losing CAS via the `after_load` hook injecting one
    competing (revision-only, non-taint) commit on the first iteration, then
    succeeds on retry; exhausts all 5 attempts via the hook injecting a
    competing commit on every iteration (`persisted_taint: false`, worktree
    stays untainted); a bootstrap failure with no prior worktree row makes no
    durable write; and a `HookedFailingCapture` (single-shot failing capture
    running an injected closure — here, a direct `initialize_worktree` call —
    immediately before returning its failure) proves the fresh, post-failure
    `load_worktree` still finds and taints a worktree materialized
    concurrently during this invocation's own (failing) capture.

    PR #244 review follow-up: corrected a real correctness hole in the
    recovery step, and a stale sketch of the `SnapshotCapture` signature this
    record itself had already superseded but the plan's own "Dependency
    injection for deterministic tests" design-decision text still carried.

    `protocol::recover` is a guarded no-op — among other guards — when the
    worktree's revision is already `u64::MAX` and cannot be advanced (see
    `protocol.rs`'s own `next_revision` headroom guard). Before this
    follow-up, `coordinate_boundary`'s recovery step treated that no-op
    identically to "recovery was not needed": `DurableTransition::between`
    returning `None` simply fell through to evaluating the triggering
    boundary against the un-recovered, still-tainted-or-needing-rebaseline
    state — silently processing a boundary the coordinator's own contract
    ("if durable state requires recovery, recovery must complete successfully
    before the triggering boundary is processed") forbids. A worktree stuck
    at `revision: u64::MAX` while tainted or needing rebaseline could
    therefore have a hook boundary attributed and committed against stale,
    un-recovered state.

    The fix adds `CoordinateError::RevisionExhausted { worktree_id: WorktreeId,
    revision: u64 }` (with a `Display` arm) and changes the recovery step's
    `if let Some(transition) = ... { ... }` (silently falling through on
    `None`) to a `let Some(transition) = ... else { return
    Err(RevisionExhausted { .. }) };` — the triggering boundary is now
    reached only when recovery's own `DurableTransition` genuinely exists.
    The coordinator does not itself re-check `revision == u64::MAX`: given
    `needs_recovery` is already true, `protocol::recover`'s own no-op guards
    for "already healthy" and "no durable state" cannot be the cause of a
    `None` transition, so a `None` here is only reachable via `recover`'s
    revision-headroom guard — `protocol::recover` remains the sole authority
    on whether recovery is possible, per the brief's explicit instruction not
    to duplicate that check. The `revision` field is populated by reading the
    already-loaded (pre-recovery) state, not by a separate query.

    Added `mandatory_recovery_that_cannot_advance_revision_rejects_the_triggering_boundary`,
    driven by a shared `assert_recovery_at_revision_exhaustion_is_rejected`
    helper covering both reachable `needs_recovery` reasons: a worktree row
    inserted directly (via raw SQL, bypassing `initialize_worktree`, which
    always starts at revision 0) at `revision: u64::MAX` with `tainted: true`
    and, separately, at `revision: u64::MAX` with `needs_rebaseline: true`.
    Each asserts `coordinate_boundary` (given a `Start` boundary and a
    succeeding `SnapshotCapture`) returns `RevisionExhausted { revision:
    u64::MAX, .. }`, then reloads the worktree/scope and asserts nothing
    about the pre-existing durable state moved: `revision` is still exactly
    `u64::MAX` (no wrap), `tainted`/`needs_rebaseline` and `cursor_tree` are
    byte-identical to what was inserted, the scope is still `NeverSeen` (the
    triggering `Start` never transitioned it), and `processed_events` is
    empty (the triggering event was never recorded) — proving the triggering
    boundary was never evaluated, not merely that it produced no visible
    event. The two existing recovery tests
    (`recovers_from_needs_rebaseline_preserving_live_scopes`,
    `recovers_from_snapshot_failure_taint_abandoning_live_scopes`) are
    unchanged and still pass, confirming ordinary (non-exhausted) recovery
    still completes and the triggering boundary still runs against the
    recovered state.

    Separately, this record's own "Dependency injection for deterministic
    tests" reference and this task's `SnapshotCapture` bullet already
    correctly stated `fn capture(&self) -> Result<TreeId>` (no
    `repository_root` parameter, since `GitSnapshotService::new` binds it at
    construction), but the plan's "Dependency injection for deterministic
    tests" Design decisions section still described `fn capture(&self,
    repository_root: &Path) -> Result<TreeId>` — a stale sketch this
    record's own implementation had already superseded without ever
    correcting that other section. That section now matches the implemented
    signature and states why `capture` takes no `repository_root` parameter.

    At the user's explicit request, every comment (`//!`/`///`/`//`,
    including on the newly added `RevisionExhausted` variant and every
    pre-existing item) was removed from `coordinator.rs`, matching T02's
    established precedent (`worktree_lock.rs`, `runtime/mod.rs`) for files
    "authored entirely during this task and its review follow-up." No
    production behavior, public API shape (beyond the new
    `RevisionExhausted` variant itself), or test coverage was changed by the
    comment removal.
  - Verify (actual): `./scripts/run-cli-cargo.sh test --manifest-path
    cli/Cargo.toml runtime::coordinator::` → 13/13 passed (pre-follow-up);
    14/14 passed (post-follow-up, including the new
    `mandatory_recovery_that_cannot_advance_revision_rejects_the_triggering_boundary`
    test). Additionally ran `./scripts/run-cli-cargo.sh clippy
    --manifest-path cli/Cargo.toml --all-targets -- -D warnings` → clean both
    times (pedantic/warnings denied workspace-wide, per Constraints; the
    original implementation fixed `match_same_arms`, `needless_pass_by_value`,
    two `needless_continue`, `elidable_lifetime_names`, and
    `items_after_statements` findings; the follow-up introduced none), and
    `./scripts/run-cli-cargo.sh fmt --manifest-path cli/Cargo.toml -- --check`
    → clean both times (after running `cargo fmt` once per pass). Additionally
    ran `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml` (full
    suite) → 819/819 passed (pre-follow-up), 820/820 passed (post-follow-up),
    no regression.
  - Context impact: `domain` — introduces new architectural elements
    (`RuntimeBoundary`, `CoordinateOutcome`, `CoordinateError`,
    `SnapshotCapture`, and the internal `coordinate_boundary` pipeline) under
    the mutation-cursor-runtime-coordinator domain T02/T03 already established
    a domain file for (`context/cli/mutation-trace-runtime-coordinator.md`).
    Affected areas for context synchronization to verify/update:
    `context/cli/mutation-trace-runtime-coordinator.md` (document the new
    types and the pipeline's load → recover-if-needed → prepare/commit and
    taint-retry design); `context/cli/mutation-trace-protocol.md`'s "Target
    end-state architecture" section, which currently states `coordinator.rs`
    "remain[s] future work" — now stale, since the file and its internal
    pipeline exist (the public, lock-wrapped `coordinate()` entrypoint remains
    T05); `context/context-map.md`'s `coordinator.rs` annotation. Reason: this
    is the first task to create `runtime/coordinator.rs` and its core
    pipeline types, directly falsifying the "future work" framing multiple
    existing context files use for it.
  - Context synchronization: synced

- [x] T05: `Wire the worktree lock and checkout identity into coordinate()` (status:done)
  - Task ID: T05
  - Scope: In — the public `coordinate(repository_root, db, boundary)`
    entrypoint: resolve `git_dir`, acquire `WorktreeLock`, resolve checkout
    identity via the now-concurrency-safe `get_or_create_checkout_id` (T01),
    derive `WorktreeId`, delegate to T04's internal pipeline for the rest of
    the critical section. Out — the internal pipeline logic itself (T04,
    unchanged here).
  - Dependencies: T01, T02, T04
  - Done when: two threads targeting the same worktree cannot run their
    critical sections concurrently (one observably blocks until the other's
    `WorktreeLock` drops); `coordinate()`'s public signature matches
    `Result<CoordinateOutcome, CoordinateError>`.
  - Verify: `cargo test -p shared-context-engineering runtime::coordinator::tests::two_threads_on_the_same_worktree_serialize` (via `./scripts/run-cli-cargo.sh test`)
  - Completed: 2026-08-30
  - Files changed: `cli/src/services/mutation_trace/runtime/coordinator.rs`,
    `cli/src/services/mutation_trace/runtime/worktree_lock.rs`
  - Result: Added the public `coordinate(repository_root: &Path, db:
    &RepositoryAgentTraceDb, boundary: &RuntimeBoundary) ->
    Result<CoordinateOutcome, CoordinateError>` entrypoint to
    `runtime/coordinator.rs`. It resolves `git_dir` via
    `checkout::resolve_git_dir(repository_root)` (error → `CoordinateError::Other`),
    acquires the `WorktreeLock` (`WORKTREE_LOCK_TIMEOUT`, 10s) into an
    RAII guard held for the whole critical section, resolves checkout identity
    via T01's now concurrency-/crash-safe `checkout::get_or_create_checkout_id(&git_dir)`
    (error → `CoordinateError::Other`), wraps the result as
    `WorktreeId(checkout_id)` — no caller-supplied `WorktreeId` or `Boundary`
    is ever accepted — constructs `GitSnapshotService::new(repository_root)`
    (which keeps its own internal `--absolute-git-dir` resolution, unchanged),
    and delegates to T04's unchanged internal `coordinate_boundary(db,
    &snapshot, &worktree_id, boundary)` for the rest of the critical section.
    `coordinate()` is a one-line delegation to a private
    `coordinate_inner(.., on_lock_contention: impl FnOnce())` (see the test
    paragraph below); production passes a no-op closure, so the code path is
    identical. Both `WorktreeLockError` variants (`TimedOut`, `Io`) are mapped
    to `CoordinateError::LockAcquisition(anyhow::Error)` via a small private
    `lock_acquisition` helper. `WORKTREE_LOCK_TIMEOUT` is a new module-level
    `const Duration = Duration::from_secs(10)`, matching the plan's "Blocking
    vs. timeout" design decision (bounded 10s deadline, polled at a 100ms
    interval). `worktree_lock.rs`'s existing private `acquire_inner` seam is
    widened to `pub(super)` (no behavior change); `WorktreeLock::acquire`'s
    public signature and semantics are untouched. The T04 pipeline,
    `RuntimeBoundary`, `CoordinateOutcome`, and `CoordinateError` are
    otherwise unchanged.

    Deviations from the pre-implementation gate summary, both reversible and
    matching surrounding style: (1) `coordinate` is a plain `pub fn` (not
    `pub(crate)`), mirroring `git_snapshot.rs`'s `pub fn capture_tree` and
    `worktree_lock.rs`'s `pub fn acquire` in the same private `runtime`
    module tree; (2) `runtime/mod.rs` was left unchanged — a private `mod
    coordinator;` is already reachable from `runtime`'s own future
    `#[cfg(test)] mod tests` (T06) as `super::coordinator::coordinate`, so no
    visibility change to `mod.rs` was needed. The dead-code warning for the
    not-yet-called entrypoint is already covered by the existing
    `#[allow(dead_code)]` on `pub mod mutation_trace;` in `services/mod.rs`
    (same coverage T04 relied on for the unused `LockAcquisition` variant).

    `runtime::coordinator::tests::two_threads_on_the_same_worktree_serialize`
    (the plan's exact Verify function name) proves the critical-section
    serialization through a real happens-before ordering rather than a
    scheduling window. `coordinate()` delegates to a private
    `coordinate_inner(repository_root, db, boundary, on_lock_contention:
    impl FnOnce())`, which acquires the lock via
    `worktree_lock::acquire_inner` (T02's existing seam, its visibility
    widened from private to `pub(super)` so `coordinator` can reach it —
    `acquire_inner`'s `on_contention` callback still fires exactly once, only
    from inside the real `Err(TryLockError::WouldBlock)` arm of the same
    `try_lock()` poll loop). `coordinate()` itself is
    `coordinate_inner(.., .., .., || {})`, so production takes the identical
    code path and its public signature, `WORKTREE_LOCK_TIMEOUT`, and every
    lock/CAS/snapshot behavior are unchanged; no fake locking implementation
    exists. The test creates a real `git init` temp repository and a real
    temp-file `RepositoryAgentTraceDb`, holds a real first `WorktreeLock`
    (acquired via `acquire_inner(.., || {})`), spawns a worker calling
    `coordinate_inner(.., Flush, move || contention_tx.send(()))`, and blocks
    on `contention_rx` (5s deadlock guard only). Receiving the contention
    signal is the primary proof: the worker's `coordinate()` reached the real
    `try_lock()` loop and observed `WouldBlock` while the first guard was
    provably still alive (the main thread cannot drop it until after the
    `recv`). A secondary `result_rx.recv_timeout(300ms).is_err()` asserts the
    same invocation has not completed while the guard is held; then the first
    guard is dropped and the same worker invocation is required to acquire the
    lock and return `Ok` with `revision == 0` (first-observation flush) — no
    restart, no manual retry. This mirrors `worktree_lock.rs`'s own corrected
    T02 contention test (`try_lock()` → `WouldBlock` → contention observer)
    and makes the same causal claim; it fails if `coordinate()` were changed
    to enter its critical section without contending on the worktree lock. The
    earlier revision of this test signalled a pre-call `started` channel and
    asserted only non-completion for 500ms — a scheduler false positive (a
    worker descheduled between the signal and the `coordinate()` call would
    pass without ever reaching `try_lock()`); that framing, and any claim it
    was equivalent to the `worktree_lock.rs` contention test, is superseded.
  - Verify (actual): `./scripts/run-cli-cargo.sh test --manifest-path
    cli/Cargo.toml runtime::coordinator::tests::two_threads_on_the_same_worktree_serialize`
    → 1/1 passed, re-run 5 consecutive times in isolation with no flakiness
    after the PR #244 review follow-up (the strengthened `WouldBlock`-observed
    proof). Additionally ran `./scripts/run-cli-cargo.sh test
    --manifest-path cli/Cargo.toml runtime::` → 37/37 passed (no regression
    in `worktree_lock`, `git_snapshot`, or the T04 coordinator tests);
    `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml
    --all-targets -- -D warnings` → clean (pedantic/warnings denied
    workspace-wide, per Constraints; `cargo fmt` run once to apply the
    follow-up's formatting); `./scripts/run-cli-cargo.sh fmt
    --manifest-path cli/Cargo.toml -- --check` → clean. Full CLI suite
    (`./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml`) → 821
    passed / 0 failed on the baseline-plus-one count (pre-change baseline was
    820/820). This project's full parallel test run is intermittently flaky in
    modules unrelated to this task: across T05's original and follow-up
    verification, three distinct full-suite runs each surfaced a different
    single failure in `sync`, `agent_trace_db::repository`, or
    `agent_trace_export`, every one of which passed immediately when re-run in
    isolation and cleared on the next full run. The targeted
    `two_threads_on_the_same_worktree_serialize` and `runtime::` runs were
    never flaky.
  - Context impact: `domain` — completes the mutation-cursor-runtime
    coordinator by adding its public `coordinate()` entrypoint under the
    domain file T02/T03/T04 established
    (`context/cli/mutation-trace-runtime-coordinator.md`). Affected areas for
    context synchronization to verify/update:
    `context/cli/mutation-trace-runtime-coordinator.md` (document the public
    `coordinate()` entrypoint, its lock-wrapped critical-section ordering —
    resolve `git_dir` → acquire `WorktreeLock` (bounded 10s) → resolve
    checkout identity → derive `WorktreeId` → delegate to the T04 pipeline —
    and the `WorktreeLockError` → `CoordinateError::LockAcquisition` mapping);
    `context/cli/mutation-trace-protocol.md`'s "Target end-state
    architecture", which still frames `coordinator.rs`'s public entrypoint as
    future work — now stale, the file's public API exists (only harness/CLI
    wiring and `diff_traces` insertion remain out of scope);
    `context/cli/mutation-trace-store.md`'s "Non-goals" line about
    `coordinator.rs`; `context/context-map.md`'s `coordinator.rs` annotation
    (the plan's own Context sync note says "update once T06 lands" — T05 is
    the task that creates the public seam, so re-check the wording now);
    `context/overview.md`'s `mutation_trace` description (the runtime
    coordinator now has a usable public entrypoint, still not wired into any
    hook or command). The five root context files must still be verified per
    the mandatory pass. Reason: this is the first task to expose a public,
    lock-wrapped `coordinate()` API, directly falsifying the "public
    entrypoint remains future work" framing multiple context files carry.

    PR #244 review follow-up: strengthened
    `two_threads_on_the_same_worktree_serialize` from a scheduler-window
    non-completion check into a real `TryLockError::WouldBlock` happens-before
    proof (see Result). `context/cli/mutation-trace-runtime-coordinator.md`
    was re-synced accordingly — its coordinator-bullet and Testing-boundary
    paragraphs now describe the `coordinate_inner(.., on_lock_contention)`
    seam and the `worktree_lock::acquire_inner` `pub(super)` widening, and no
    longer imply the superseded pre-call-`started` framing was equivalent to
    `worktree_lock.rs`'s own contention test. No other context file was
    affected: `coordinate_inner` is a private test seam with no
    public-interface, behavior-contract, or architecture change. The five
    root context files were re-verified against the follow-up and remain
    accurate as edited during the original T05 sync.
  - Context synchronization: synced

- [x] T06: `Add cross-module runtime integration tests` (status:done)
  - Task ID: T06
  - Scope: In — `runtime/tests.rs`: end-to-end `coordinate()` calls against
    two real linked Git worktrees given handles to the same caller-supplied
    repository-scoped DB, proving distinct `WorktreeId`s/locks and
    non-serialization across worktrees; an end-to-end
    snapshot-failure-then-recovery cycle across two real sequential
    `coordinate()` calls; a cross-caller assertion that
    `agent_trace_storage`'s own checkout-identity resolution and the
    coordinator's observe the same checkout ID for one worktree. Out — any
    new production code; this task is test-only.
  - Dependencies: T05
  - Done when: AC9's linked-worktree assertions, AC12's cross-caller
    assertion, and an end-to-end failure/recovery cycle pass using only the
    public `coordinate()` API, real `git worktree add`, and a real temp-file
    `RepositoryAgentTraceDb`.
  - Verify: `cargo test -p shared-context-engineering runtime::tests::` (via `./scripts/run-cli-cargo.sh test`)
  - Completed: 2026-08-30
  - Files changed: `cli/src/services/mutation_trace/runtime/tests.rs` (new),
    `cli/src/services/mutation_trace/runtime/mod.rs` (added `#[cfg(test)] mod tests;`)
  - Result: Added `runtime/tests.rs` as `runtime`'s own `#[cfg(test)] mod
    tests` (declared in `runtime/mod.rs`), holding three cross-module,
    end-to-end integration tests that drive only the public
    `coordinator::coordinate()` API against real Git repositories and real
    temp-file `RepositoryAgentTraceDb`s. No production code was added or
    changed; the file carries no comments, matching the established
    precedent for `runtime/` files authored entirely within their own task
    (T02–T05).

    `linked_worktrees_have_independent_locks_and_worktree_ids` (AC9): inits a
    main repo (with an empty commit so `git worktree add` is allowed), adds a
    real linked worktree via `git worktree add`, opens one caller-supplied
    repository-scoped DB path and hands a separate `RepositoryAgentTraceDb`
    handle to each `coordinate()` call (`new_at` for the main,
    `open_for_hooks_without_migrations_at` for the linked, matching the AC8
    two-writer convention — `coordinate()` never resolves the DB itself).
    Asserts `resolve_git_dir` returns distinct worktree-specific git dirs
    (hence distinct lock/identity paths); runs `coordinate(Flush)` on the
    main worktree; then holds the main worktree's real `WorktreeLock`
    (`WorktreeLock::acquire`) across a synchronous `coordinate(Flush)` call
    for the linked worktree. That call returning `Ok` before the main lock
    guard is dropped is the deterministic proof of independent lock paths: a
    regression to one shared lock could not acquire that lock while `held`
    is still alive and would instead return a lock-acquisition timeout. No
    wall-clock assertion is used. Asserts the two
    `CoordinateOutcome.worktree_id`s differ, that both distinct worktree rows
    coexist in the same caller-supplied DB (`MutationTraceStore::load_worktree`),
    and that a tree pinned by the main worktree's coordinator resolves
    through the linked worktree's `GIT_DIR`
    (`GitSnapshotService::new(&linked_root).diff_trees(...)`), proving the
    shared object database / refs namespace.

    `agent_trace_storage_and_coordinator_observe_the_same_checkout_id`
    (AC12): inits a repo with an `origin` remote (enough for
    `agent_trace_storage` identity resolution; no network), then races — via
    a two-party `Barrier` — a background-thread
    `resolve_agent_trace_storage_at_state_root` call against a main-thread
    `coordinate(Flush)` call, both on first-ever checkout-identity creation
    for the same physical checkout. Asserts the storage resolution's
    `checkout_id`, the coordinator outcome's `worktree_id.0`, and the on-disk
    `checkout-id` file (`checkout::read_checkout_id`) are all the identical
    value — proving T01's convergence guarantee holds across the module
    boundary, not only inside `checkout::`'s own suite. (The two callers use
    different databases; the only shared state under contention is the
    `<git-dir>/sce/checkout-id` file.)

    `a_snapshot_failure_then_recovery_cycle_runs_through_the_public_api`
    (end-to-end AC10/AC11): runs a baseline `coordinate(Flush)` to
    materialize the worktree, then injects a real `GitSnapshotService`
    failure by planting a regular file where `capture_tree` expects its
    `<git-dir>/sce/tmp/` temp-index directory (so `capture_tree`'s
    `create_dir_all` fails deterministically while repo detection, lock
    acquisition, and checkout-identity resolution are all untouched). Asserts
    the next `coordinate(Start{..})` returns
    `CoordinateError::SnapshotFailure { persisted_taint: true, .. }` and that
    the durable worktree row is `tainted`. Removes the planted file and runs
    one more `coordinate(Flush)`, asserting it succeeds on the same worktree
    identity and that the `tainted` flag was cleared — i.e. the coordinator
    recovered from the taint before processing the triggering boundary.

    Reviewed assumption (recorded, not a scope deviation): the plan's Scope
    line sketched the snapshot-failure injection loosely ("across two real
    sequential `coordinate()` calls"); the implementation uses a baseline
    call, a failing call, and a recovery call (three sequential public-API
    invocations), and injects the failure by blocking the snapshot service's
    temp-index directory rather than by corrupting `.git/HEAD` (HEAD removal
    breaks Git repo detection for `resolve_git_dir`, which runs before the
    snapshot step, so the failure would surface as `CoordinateError::Other`
    rather than a `SnapshotFailure` taint path — verified empirically during
    implementation).

    PR #244 review follow-up: two test/wording corrections, no production
    change. First, `linked_worktrees_have_independent_locks_and_worktree_ids`
    dropped its `Instant::now()` / `elapsed() < 5s` assertion (and the unused
    `Instant` import): the lock-independence proof is the deterministic
    ordering — the main worktree's `WorktreeLock` stays held across the whole
    synchronous linked `coordinate()` call, and that call returning `Ok`
    before `held` is dropped is only possible if the linked worktree
    acquires a different lock path; a shared lock would return a
    lock-acquisition timeout instead. The wall-clock check added only CI
    timing sensitivity and proved nothing the ordering did not. Second, AC9
    and the surrounding wording (this record, the Scope line, and
    `context/cli/mutation-trace-runtime-coordinator.md`) were corrected to
    stop implying `coordinate()` resolves the repository-scoped Agent Trace
    DB. It does not: `coordinate(repository_root, db, boundary)` derives
    `git_dir`, checkout identity, and `WorktreeId` from `repository_root`,
    but the `RepositoryAgentTraceDb` is supplied by the caller. The
    linked-worktree test hands a separate handle to the one caller-opened DB
    path to each call and proves the two distinct worktree rows coexist in
    that supplied DB — not that each invocation independently resolves it.
  - Verify (actual): `./scripts/run-cli-cargo.sh test --manifest-path
    cli/Cargo.toml runtime::tests::` → 3/3 new tests passed (the filter also
    re-runs 5 unrelated `parse::command_runtime::tests` that share the
    `runtime::tests` substring; all passed). Additionally ran
    `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml
    services::mutation_trace::runtime::` → 35/35 passed (no regression in the
    T02/T03/T04/T05 `worktree_lock`, `git_snapshot`, and `coordinator`
    tests); `services::checkout::` → 5/5; `services::agent_trace_storage::`
    → 14/14 (the two cross-called modules, unchanged, still green).
    `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml
    --all-targets -- -D warnings` → clean (pedantic/warnings denied
    workspace-wide, per Constraints); `./scripts/run-cli-cargo.sh fmt
    --manifest-path cli/Cargo.toml -- --check` → clean (after one `cargo fmt`
    pass on the new file).

    PR #244 review follow-up re-verification: after removing the
    elapsed-time assertion and correcting the DB-ownership wording,
    `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml
    services::mutation_trace::runtime::tests::` → 3/3 passed;
    `services::mutation_trace::runtime::` → 35/35 passed; the full
    `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml` suite →
    824/824 passed / 0 failed; `clippy --all-targets -- -D warnings` → clean;
    `fmt -- --check` → clean (after one `cargo fmt` pass re-wrapping the
    edited `open_for_hooks_without_migrations_at` line).
  - Context impact: `domain` — this task adds only tests; it introduces no
    new architectural element, public interface, behavior contract, or
    terminology. The mutation-cursor-runtime-coordinator domain file
    (`context/cli/mutation-trace-runtime-coordinator.md`) already carries a
    "Testing boundary" section; task context synchronization should extend
    its `runtime/tests.rs` bullet to name the three landed cross-module
    integration tests (linked-worktree independence, cross-caller
    checkout-identity convergence, public-API failure/recovery cycle) and
    flip any "T06 not yet landed" / "runtime/tests.rs planned" framing to
    present-tense. Per the plan's own Context sync notes, T06 is the trigger
    for finalizing the `context/context-map.md`
    `coordinator.rs`/`git_snapshot.rs` seam annotations (drop the
    "not-yet-created" wording now that the full `runtime/` module including
    its integration suite exists) and for a last pass over
    `context/cli/mutation-trace-protocol.md`,
    `context/cli/mutation-trace-store.md`, and `context/overview.md` to
    confirm no stale "future work" framing about the runtime layer remains.
    The five root context files must still be verified per the mandatory
    pass. No qualifying system-wide architecture decision was introduced by
    this task.

    PR #244 review follow-up: the synced context edits were corrected to
    stop implying `coordinate()` resolves the repository-scoped Agent Trace
    DB. `context/cli/mutation-trace-runtime-coordinator.md`'s `coordinate()`
    bullet now states it takes an already-resolved `RepositoryAgentTraceDb`
    from its caller and never resolves/opens one (identity chain:
    `repository_root → git_dir → WorktreeLock → checkout ID → WorktreeId`,
    with the DB not on that chain), and its "Testing boundary" paragraph
    describes the linked-worktree proof as the held-lock ordering (no
    wall-clock timing) with both worktree rows coexisting in the one
    caller-supplied DB. AC9's own wording in this plan was corrected to
    match. No other context file needed a change; the five root files remain
    accurate as verified in the original sync.
  - Context synchronization: synced

## Design decisions

### Checkout-identity concurrency: fixed at the primitive, not at each caller

`cli/src/services/checkout/mod.rs:109`'s `get_or_create_checkout_id` is
confirmed read-then-write with no lock: `read_checkout_id` returns `None`,
then a fresh UUIDv7 is generated and written. Two concurrent first-time
callers on the same `git_dir` can each observe `None`, each generate a
*different* UUIDv7, and the second `std::fs::write` wins — the two callers
then disagree about this checkout's identity for the remainder of their
respective processes.

The original version of this plan protected only the coordinator's own call
to this function, by acquiring the mutation-cursor runtime lock first. That
was insufficient: `cli/src/services/agent_trace_storage/mod.rs:168` calls
the same function independently, with no lock at all, during every storage
resolution — a race between an `agent_trace_storage` resolution and a
coordinator invocation (or two concurrent `agent_trace_storage`
resolutions, with no coordinator involved at all) could still produce two
different checkout IDs for the same physical worktree.

This revision fixes `get_or_create_checkout_id` itself (T01) instead:

```text
read_checkout_id(git_dir)   -- unchanged, lock-free fast path
if Some(id): return id      -- the common case after the first invocation ever
else:
    acquire <git-dir>/sce/checkout-id.lock (blocking; see "Blocking vs. timeout" below)
    re-read_checkout_id(git_dir)   -- another caller may have won while we waited
    if Some(id): return id
    generate UUIDv7
    -- crash-safe write, see "Crash-safe persistence" below --
    write the ID through checkout-id.tmp-<uuid>, then atomically
    rename it into checkout-id
    (lock released on Drop)
    return the generated id
```

Every caller of `get_or_create_checkout_id` — the coordinator, and
`agent_trace_storage`'s existing call, unmodified — now converges on exactly
one ID, with no caller-side change required. This is the "smallest correct
implementation" the brief asked for: it changes one function's body, not its
signature, not the checkout-ID file format, and not any caller.

An alternative considered and rejected: skip the lock and instead write with
`OpenOptions::new().create_new(true)` directly on `checkout-id`, having a
losing writer read back the winner's value. This is tempting because it
needs no separate lock file, but it has a real correctness gap: `create_new`
only makes the *creation* atomic, not the subsequent `write_all` — a losing
reader could observe the winning writer's file after creation but before
its content is fully written, and would need to distinguish "empty/partial,
retry" from "genuinely corrupt" with no reliable signal. The lock-based
design avoids this for *concurrent readers*: a losing caller blocks on the
*same* lock the winner holds, and by the time it can re-read, the winner has
already finished writing. It does **not**, by itself, protect against a
*crashed* writer — the lock's own release on process death (or on `Drop`
after a successful write) says nothing about whether the write it was
guarding actually completed. That is a distinct problem, addressed next.

### Crash-safe persistence: temp file plus atomic rename

The lock closes the *concurrency* race; it does not by itself close the
*crash* race. `get_or_create_checkout_id`'s write step was, in the plan's
prior revision, effectively `std::fs::write(checkout-id, uuid)` — a single
call that is not atomic with respect to a process crash partway through: a
process can be killed after the OS has created/truncated `checkout-id` but
before the full UUID has been written to it. The advisory lock is released
automatically when the process dies, so a *later*, otherwise-correct caller
can then observe an empty or truncated `checkout-id` file through the
existing lock-free fast path — `read_checkout_id` already rejects this as
corruption (empty content, or a UUID parse failure) rather than recognizing
it as a recoverable incomplete write, and there was no mechanism to make
that state unreachable in the first place.

The fix eliminates this state by construction, using a unique temporary
file plus an atomic rename rather than writing the canonical path in place:

```text
generate UUIDv7 (call it `id`)
tmp_path = <git-dir>/sce/checkout-id.tmp-<id>
file = OpenOptions::new().write(true).create_new(true).open(tmp_path)   -- fails loudly on
                                                                          -- the essentially
                                                                          -- impossible case
                                                                          -- of a name collision;
                                                                          -- `id` is a fresh
                                                                          -- UUIDv7 embedded in
                                                                          -- the temp filename,
                                                                          -- so no other attempt,
                                                                          -- past or concurrent,
                                                                          -- ever reuses this
                                                                          -- exact path
file.write_all(id.as_bytes())
file.sync_data()   -- flush content to disk before the rename; see rationale below
drop(file)          -- close the handle
std::fs::rename(tmp_path, <git-dir>/sce/checkout-id)   -- atomic on the same filesystem
#[cfg(unix)] best-effort: open <git-dir>/sce/ as a File and call .sync_all()
             (durability hardening for the rename's directory-entry update;
              Windows has no equivalent operation and does not need one — see below)
```

Verified experimentally (`create_new` + `write_all` + `sync_data` +
`std::fs::rename`, reproducing the sequence above against a real
filesystem): the canonical path does not exist at any point before the
`rename` call; immediately after `rename`, it contains the complete,
untruncated ID; the temporary path no longer exists after a successful
rename. A directory handle opened with `File::open(dir_path)` on Unix
accepts `.sync_all()` without error, confirming the best-effort
parent-directory sync is implementable with no new dependency.

- **Why `create_new(true)` for the temp file:** guarantees the temp file
  itself starts empty and is never silently reused; combined with the fresh
  UUID embedded in its name, a name collision is not just unlikely but
  requires generating the identical UUIDv7 twice, which this design treats
  as unreachable rather than something to retry around.
- **Why `sync_data()`, not `sync_all()`:** the content (the UUID bytes) is
  what must survive a crash; the file's metadata (for example its mtime) is
  not load-bearing for anything this plan reads, so the slightly cheaper
  `sync_data()` is sufficient and is what is recommended, though
  `sync_all()` would also be correct.
- **Why `std::fs::rename`, not an in-place write:** POSIX `rename(2)` is
  atomic with respect to concurrent readers of the destination path — a
  reader observes either the old state (absent, for a fresh checkout) or the
  complete new file, never an intermediate state, because the rename only
  ever changes a directory entry, not file content in place.
- **Windows:** `std::fs::rename` on Windows is implemented over
  `MoveFileExW` with `MOVEFILE_REPLACE_EXISTING`, so replacing (or, here,
  creating) the destination succeeds the same way it does on POSIX for this
  single-writer, lock-protected sequence; this is the same cross-platform
  rename-based atomic-replace pattern used throughout the Rust ecosystem
  (for example by the `tempfile` crate's `persist()`) for exactly this
  purpose. Windows has no direct analogue to a POSIX directory `fsync`, so
  the best-effort parent-directory sync step above is `#[cfg(unix)]`-only;
  it is a durability hardening addition, not a correctness requirement (the
  rename itself, once it returns, already guarantees the *content*
  atomicity property AC13 requires — the directory sync only narrows the
  already-vanishingly-small window between "rename returned" and "the
  rename survives an immediate, otherwise-unrelated full machine power
  loss").
- **Orphaned `checkout-id.tmp-*` files are harmless and need no cleanup
  pass.** Each temp filename embeds a freshly generated UUIDv7 never reused
  by any other attempt, so an orphan left behind by a crash between
  `sync_data()` and `rename` can never collide with, block, or be confused
  for a later attempt's own (differently named) temp file. Because the
  lock-free fast path only ever reads the canonical `checkout-id` path,
  never the `sce/` directory's full contents, an orphaned temp file is
  simply inert leftover state — a few dozen bytes, at most once per
  checkout, only in the already-rare case of a crash landing in this narrow
  window. This plan implements no cleanup pass for these files, matching the
  same "harmless, not self-cleaning, no reconciliation in this PR" judgment
  already made for orphaned `refs/sce/mutation-cursor/**` pins (see Snapshot
  durability, below) — for the identical reason: getting cleanup wrong by
  deleting something still needed is worse than never cleaning up something
  genuinely inert.

### Checkout-identity lock vs. the mutation-cursor runtime lock: two locks, two invariants

These are deliberately separate locks, at separate paths, guarding separate
invariants — conflating them was the original plan's mistake:

- `<git-dir>/sce/checkout-id.lock` (T01): guards only "this checkout has at
  most one durable identity." Acquired only on the rare slow path (identity
  not yet created), held only across a few filesystem syscalls, and used by
  every caller of `get_or_create_checkout_id`, not just the coordinator.
- `<git-dir>/sce/mutation-cursor.lock` (T02, unchanged from the original
  plan): guards the coordinator's entire runtime critical section — snapshot
  capture, worktree/scope materialization, recovery, and the CAS retry loop.
  Held on every `coordinate()` call, not just the rare first one, and used
  only by the coordinator.

Because T01 makes checkout-identity creation safe for every caller
independent of the mutation-cursor lock, the coordinator's *ordering*
between "acquire the mutation-cursor lock" and "resolve checkout identity"
no longer has any bearing on this race (T05 still resolves identity after
acquiring the runtime lock, for the unrelated reason that it is the natural
first fact the rest of the critical section needs — not because it is what
closes the race, which it no longer needs to be).

### Locking implementation: Rust std, not a new dependency

Verified by direct compilation against this repository's pinned toolchain
(`rustc 1.95.0`): `std::fs::File::lock()`, `try_lock()` (returning
`Result<(), TryLockError>` with `TryLockError::{WouldBlock, Error(io::Error)}`),
and `unlock()` all compile and behave as expected (a second handle's
`try_lock()` on an already-locked file returns `WouldBlock`). This API was
stabilized in Rust 1.89, well before this project's pin, is documented as
cross-platform (`flock(2)` on Unix, `LockFileEx` on Windows), and is
advisory and exclusive by default — exactly what both locks need. No
`fd-lock`/`fs2`-style dependency is added.

### Blocking vs. timeout, for each lock

- **Mutation-cursor runtime lock (T02):** `File::lock()` has no built-in
  timeout and blocks indefinitely. A hook invocation is a short-lived CLI
  process whose caller (the harness) is typically waiting on it; if a prior
  `sce` process hangs while holding this lock (a slow Git operation, a slow
  DB retry, or a genuine bug), every subsequent hook invocation on that
  worktree would otherwise queue up and block forever, effectively
  deadlocking the harness's own tool-calling loop. `WorktreeLock::acquire`
  therefore polls `try_lock()` on a short interval (100ms) against a bounded
  deadline (10s) rather than calling the blocking `lock()` directly, and
  returns a distinct `CoordinateError::LockAcquisition` on timeout with
  actionable guidance.
- **Checkout-identity lock (T01):** blocks indefinitely (plain `lock()`, no
  polling, no timeout). This is a deliberate asymmetry, not an oversight:
  the critical section it guards is a handful of local filesystem syscalls
  with no DB round-trip and no Git subprocess in it, so it cannot
  meaningfully hang the way the runtime lock's much larger critical section
  can. Adding timeout/polling machinery here would be complexity with no
  corresponding risk to justify it.

Both are internal constants, not a new `policies.database_retry`-style
config surface — this is a correctness/optimization boundary, not a
DB-facing concern, and there is no existing consumer to justify
config-driven tuning yet.

### Snapshot durability: normal Git object database + SCE-owned refs, not a private store

The original version of this plan proposed writing snapshot trees into a
private `<common-dir>/sce/mutation-objects/` store, with
`GIT_ALTERNATE_OBJECT_DIRECTORIES` pointing at the repository's normal
object directory so unchanged blobs could still resolve. **This was not a
sufficient durability guarantee.** A persisted `TreeId` in that design could
reference blobs that exist only in the repository's *normal* object
database; Git's own GC/prune reachability analysis has no knowledge that an
externally-stored SCE tree depends on them, so a routine `git gc` on the
user's repository could delete a blob a durable, already-committed
`MutationEvent` row still names — silently producing an unreadable
historical snapshot. The original plan's AC7 durability claim did not hold
by construction.

Verified experimentally (reproducible; commands below match what was run,
as two separate trials — never as a same-repository comparison between two
copies of *identical* content, since Git's content-addressing would make
those the same object and any such comparison meaningless; see "Isolated
Git snapshot" below for the corrected same-repository design this plan
actually specifies, using two *distinct*-content trees): a tree object with
**no** ref pointing at it (directly or transitively) is reclaimed by
`git gc --prune=now`; a tree object **with** a ref pointing at it survives
both `git gc --prune=now` and `git prune --expire=now` intact and fully
resolvable via `git cat-file`/`git ls-tree`. This is exactly Git's ordinary
reachability contract, and it is the only mechanism in Git that actually
prevents pruning — an alternate-object directory participates in
*resolution*, never in *reachability*.

Two designs were compared:

- **A. Normal object database + SCE-owned refs pinning durable snapshot
  trees.** Objects are written with plain `git write-tree` against the
  repository's own object database (no `GIT_OBJECT_DIRECTORY` override at
  all). Each tree that becomes durable evidence is made reachable by
  creating one ref under a dedicated SCE namespace. Git's own GC naturally
  preserves everything reachable from that ref — including every blob it
  transitively references, whether newly written by this snapshot or
  already shared with the rest of the repository's history — with no
  alternate-object bookkeeping and no closure-copying logic to get right.
  `diff_trees` needs no special object environment at all: once a tree is
  written into the normal store, it resolves like any other Git object.
- **B. Fully self-contained private object store.** To honor the brief's
  own framing ("must remain resolvable without depending on objects Git may
  later prune from the normal store"), a private store cannot merely borrow
  unchanged blobs through an alternate — reachability through an alternate
  is not durability. It would have to *copy* the complete object closure
  each durable snapshot tree depends on into the private store: for every
  tree, walk it recursively, and for every blob/subtree not already present
  in the private store, duplicate it in. This is real, non-trivial logic
  (a closure walk plus copy, run on every snapshot, since a later `git gc`
  could reclaim the source blob between snapshots), provides no benefit
  design A does not already provide, and actively duplicates on-disk storage
  for every unchanged blob a snapshot touches — where design A's
  content-addressing means an unchanged blob referenced by many snapshots is
  stored exactly once, in the one place the rest of the repository already
  keeps it.

**Recommendation: A**, as the brief's own stated default preference, and
confirmed by this comparison: it is simultaneously the simplest
implementation, the only one that is GC-safe by construction rather than by
a separately-maintained copying invariant, and the only one that needs no
extra object-directory environment variables for either writing or reading.
This also means `runtime/git_snapshot.rs` does **not** need to resolve
`--git-common-dir` at all (a further simplification and deviation from the
original plan, which planned to add `resolve_git_common_dir` to
`checkout.rs`): verified experimentally from inside a real linked worktree,
running `git write-tree`/`git update-ref` with only `GIT_DIR` set to the
worktree's own `--git-dir` (no object-directory override) already resolves
objects and refs through the shared commondir automatically, and a ref
created from the linked worktree's `GIT_DIR` is immediately visible when
queried from the main checkout's `GIT_DIR`, and vice versa — Git's own
worktree/commondir mechanism already gives every worktree of one repository
a shared object database and a shared default ref namespace with no help
from this plan.

**Ref namespace:** `refs/sce/mutation-cursor/<worktree-id>/<tree-sha>` — one
ref per distinct `(worktree_id, tree)` pair. `worktree_id` scopes cleanup
and any future audit tooling to one worktree without a repository-wide scan;
`tree-sha` makes the ref path itself content-addressed, so creating the same
ref twice (for example across CAS retries that reuse one captured tree, or
two invocations that happen to observe identical content) is a harmless,
idempotent `git update-ref` — verified experimentally.

**Pin lifecycle: create-only in this PR — and that growth is unbounded, not
bounded, until reconciliation exists.** The coordinator (T04) calls
`pin_tree(worktree_id, observed_tree)` exactly once per invocation,
immediately after `capture_tree` succeeds and *before* any DB operation —
whether or not that invocation's boundary ultimately commits, is rejected,
or is a no-op. It never deletes a pin. This is a deliberate simplification,
not an oversight: safely deleting a pin requires confirming, under the
worktree lock, that the pinned tree is neither the worktree's current
`cursor_tree` nor referenced by any historical `mutation_trace_events` row —
a repository-wide-aware check this PR's narrow, per-invocation coordinator
has no efficient way to perform (a single invocation only knows about the
one worktree revision it just read, not the full DB history of every tree
that worktree has ever pinned). Getting cleanup wrong in the unsafe
direction — deleting a ref for a tree still needed by durable evidence — is
strictly worse than never deleting one, so this PR accepts the resulting
storage cost and defers actual reconciliation to a follow-up maintenance
pass (see Follow-up PR).

An earlier revision of this plan described that cost as "bounded, linear"
growth. That was imprecise and is corrected here: growth is **unbounded
over the repository's lifetime**, in direct proportion to hook-invocation
volume — every distinct tree a `coordinate()` invocation ever observes
leaves one permanent ref and its protected objects, for as long as the
repository exists and this PR's create-only policy remains unmodified.
"Linear in invocation count" is not "bounded"; it only stays small in
absolute terms for as long as invocation volume itself stays small, which
this standalone PR's lack of any production caller happens to guarantee for
now, but which the moment a harness adapter starts driving high-frequency
hook traffic through `coordinate()` would no longer hold. This is exactly
why the Follow-up PR section below sequences a reconciliation pass as
required runtime-completion work *before* that wiring, not as an optional
later enhancement.

This directly answers "when can refs safely be deleted": not from inside
this PR's per-invocation coordinator at all; only from a pass with
visibility into the full set of durable references a ref might still be
protecting. The follow-up reconciliation pass's retained-roots contract —
precise enough to bound its architectural shape without prescribing its
algorithm, which this PR does not implement:

- **Retained roots** (a tree a reconciliation pass must never unpin while
  any of these still reference it): every worktree's current
  `mutation_trace_worktrees.cursor_tree`, and every historical
  `mutation_trace_events.before_tree`/`after_tree`. These three columns are
  the complete set of durably-referenced `TreeId`s in `store.rs`'s schema —
  no other table stores one.
- **Conceptual shape:** read every durably-referenced `TreeId` from the
  three roots above, list the full `refs/sce/mutation-cursor/**` namespace,
  delete any ref whose tree appears in neither set, and let Git's own,
  already-existing GC reclaim the now-unreachable objects on its own normal
  schedule — this PR adds no GC invocation of its own, only the refs GC
  already knows how to act on.
- **Concurrency safety is the hard part, and is explicitly out of this PR's
  scope to solve, only to flag:** a reconciliation pass must never delete a
  ref for a tree a coordinator has *just* pinned but has not yet committed
  durably — deleting it out from under an in-flight `coordinate()` call
  would reintroduce exactly the durability defect this revision fixes. Two
  viable strategies for the follow-up to choose between: reconciling one
  worktree at a time under that worktree's own `mutation-cursor.lock` (true
  synchronization, at the cost of contending with live coordinator traffic),
  or a conservative minimum-age threshold on candidate refs (for example,
  never consider a ref for deletion until it is meaningfully older than any
  realistic single `coordinate()` call could still be running, sidestepping
  lock contention entirely at the cost of a small reclaim delay). This plan
  does not choose between them — only states that whichever the follow-up
  picks, it must not be able to delete a ref protecting a tree that could
  still become durable.

**Crash ordering.** Pin creation is a **blocking precondition** to the DB
CAS, not a follow-up step after it — this ordering is what makes the
brief's specific worry ("DB commit succeeds but ref persistence fails")
structurally impossible rather than merely unlikely:

```text
write Git objects (git write-tree, into the normal object database)
        ↓
pin_tree(worktree_id, observed_tree)   -- git update-ref, before any DB read/write
        ↓ (on failure: treated identically to a snapshot-capture failure — see below;
        ↓  the coordinator never attempts the DB CAS without a successful pin)
store.load_worktree / recover-if-needed / prepare / commit  (the retry loop; reuses the
        already-pinned observed_tree across every iteration, no re-pinning needed)
```

- **Crash before the DB CAS runs** (after a successful pin): the pin exists,
  no DB row references its tree yet, or an older DB row still stands. This
  is harmless — an orphaned pin protecting a tree that may never become
  durable, reclaimable later by the deferred reconciliation pass, with no
  correctness impact in the meantime.
- **Pin creation itself fails:** the coordinator does not proceed to the DB
  CAS at all (same branch as a `capture_tree` failure — see "Snapshot
  failure" below), so there is no window in which a durable DB row could
  ever reference an unprotected tree.
- **DB CAS succeeds:** by construction, the tree it now references has been
  ref-protected since *before* the CAS was even attempted — there is no
  "commit succeeded, pin still pending" state to reason about, because
  pinning is never deferred past the commit.
- **DB CAS conflicts (ordinary retry, not a crash):** the pin remains in
  place (harmless, and still correct to reuse — the same `observed_tree` is
  reused verbatim across retries), so no re-pinning is needed on retry.

The DB CAS remains the mutation protocol's linearization point, unchanged;
the ref is durability infrastructure that exists entirely to make what the
CAS already decided stay resolvable, not a second decision-making mechanism.

**Linked worktrees.** Since objects and the default ref namespace are
already shared across all worktrees of one repository (verified above), two
linked worktrees' coordinators pin into the *same* underlying object
database and ref namespace, scoped apart only by the `worktree_id` path
segment — no special-casing is needed for the linked-worktree case beyond
what worktree-scoped `WorktreeId` derivation already provides.

**Why SCE refs do not interfere with user-visible Git operations.**
Verified experimentally: `refs/sce/mutation-cursor/**` never appears in
`git branch`, `git tag`, or plain `git log`/`git status` output (those only
enumerate `refs/heads/*`/`refs/tags/*` or the current `HEAD`); a plain
`git clone` of the repository does not transfer it (clone/fetch only
transfer refs matching the configured refspec, by default
`refs/heads/*`/`refs/tags/*`, not arbitrary custom namespaces); and a plain
`git push` only pushes the refspecs the user names, never "every local
ref." A tool enumerating with `git log --all`/`git rev-list --all` would see
these refs, matching how other established Git-based tooling (for example
`git notes`, GitHub's `refs/pull/*`) already uses dedicated `refs/`
namespaces for the same reason — this is a well-precedented, low-cost
pattern, not a new category of interference.

Realized on-disk layout:

```text
<worktree-git-dir>/sce/
├── checkout-id                 (existing, unmodified)
├── checkout-id.lock            (new, T01 — identity-creation lock, distinct from the runtime lock)
├── mutation-cursor.lock        (new, T02 — per-worktree runtime lock)
└── tmp/
    └── index-<uuid>            (new, T03 — per-worktree, ephemeral)

<repository's normal, shared object database>       (no SCE-specific directory; T03 writes here directly)
<repository's normal, shared refs namespace>
└── refs/sce/mutation-cursor/<worktree-id>/<tree-sha>   (new, T03/T04 — one ref per pinned tree, create-only this PR)
```

### Isolated Git snapshot: temporary index, and validated commands

Validated experimentally against this repository's own Git version
(reproduced by the commands below):

```text
env:
  GIT_DIR=<git-dir>                                   # resolves the correct worktree's HEAD, shared objects, and shared refs
  GIT_INDEX_FILE=<git-dir>/sce/tmp/index-<uuid>        # private, per-invocation, never the real index
cwd: <repository_root>                                 # the real working tree, for `add -A` to see real files

1. `git rev-parse --verify --quiet HEAD` — unborn-HEAD probe (exit 1 = unborn)
2a. HEAD exists: `git read-tree HEAD`
2b. HEAD unborn: `git read-tree --empty`   — explicitly initializes a valid,
                                              empty Git index; see "Unborn
                                              HEAD" below for why this
                                              replaced an earlier, incorrect
                                              design
3. `git add -A -- .`         — stages the full current working-tree reality
                                (modified, added, untracked, deleted;
                                respects `.gitignore`) into the private index
4. `git write-tree`          — writes the resulting tree into the
                                repository's normal, shared object database;
                                unchanged blob content already present is
                                never duplicated (Git checks object existence
                                before writing, exactly as it already does
                                for a normal `git add`/`git commit`)
5. `git update-ref refs/sce/mutation-cursor/<worktree-id>/<tree-sha> <tree-sha>`
                              — pins the resulting tree (see "Snapshot
                                durability" above)
```

No `GIT_OBJECT_DIRECTORY`/`GIT_ALTERNATE_OBJECT_DIRECTORIES` env vars are
set at any point — this is a further simplification over the original
plan's private-store design, made possible once objects are written
straight into the normal store.

**Unborn HEAD: `git read-tree --empty`, not a bare temp-index file.** An
earlier revision of this plan skipped step 2 entirely on an unborn `HEAD`
and assumed a freshly created, empty (zero-byte) temp-index file was
equivalent to an empty Git index. Verified experimentally: it is not. A
zero-byte file at the path named by `GIT_INDEX_FILE` is an invalid index —
`git add -A -- .` against it fails deterministically with `fatal: <path>:
index file smaller than expected`, because Git's index format has a
required header a zero-length file cannot satisfy. There are two ways to
avoid ever presenting Git with such a file, and this plan adopts the more
explicit of the two: `git read-tree --empty`, run in place of `git read-tree
HEAD` when the unborn-HEAD probe fails, initializes a genuinely valid empty
index at the `GIT_INDEX_FILE` path (verified: Git creates it correctly,
`git add -A -- .` against it succeeds). Verified equivalent, and preferred
for that explicitness, is the alternative of never creating anything at the
index path at all and letting `git add -A -- .` run directly against a
genuinely nonexistent `GIT_INDEX_FILE` path (Git treats a missing index file
as "start from empty" for `add`, exactly as `read-tree --empty` does
explicitly) — both were confirmed experimentally to produce byte-identical
resulting trees for the same working-tree content, and an unborn repository
with no files at all produces a valid, zero-entry tree either way, whose
object ID this plan never hardcodes (Git computes it; the test only asserts
zero `ls-tree` entries). `git read-tree --empty` is nonetheless the design
this plan specifies, because it makes "start from empty" an explicit,
self-documenting step in the command sequence rather than an implicit
consequence of a file happening not to exist yet — the same reasoning
Requirement 2's brief itself gives (works independently of object format;
represents the correct base state for a repository with no commits without
relying on an unstated invariant about file-absence semantics).

This is also why the temporary-index lifecycle (below) must never
pre-create an empty file at the reserved path: doing so would reintroduce
exactly the invalid-index failure this fix removes, on the unborn-HEAD path
specifically, since step 2b's `git read-tree --empty` needs to be the first
thing to ever touch that path, not a second write on top of an
already-existing zero-byte file.

Confirmed by direct experiment: with only `GIT_DIR`/`GIT_INDEX_FILE` set,
the real repository's `.git/index`, `git status --porcelain`, `git diff`,
and `git diff --cached` are byte-identical before and after a snapshot
capture that includes staged, unstaged, and untracked changes
simultaneously; an ignored file is correctly absent from the resulting
tree's `ls-tree`; a deletion of a previously-committed file is correctly
reflected as absent from the resulting tree; `git update-ref` on the same
`(worktree_id, tree)` pair twice is a no-op; within one repository, a
distinct, genuinely unreachable tree (unique content, no ref, no branch, no
tag, no reflog entry — reflogs only ever record commit-ref history, and
this plan's loose trees are never committed or ref'd except via the SCE
pin, so they have no reflog entry to be protected by regardless) is
reclaimed by `git gc --prune=now` followed by `git prune --expire=now`,
while a second, differently-content tree pinned via
`refs/sce/mutation-cursor/**` survives the identical pass. `TreeId` stays
the opaque `String` `types.rs` already declares it as — no code depends on a
40-character SHA-1 length, so a SHA-256 repository
(`git init --object-format=sha256`) needs no special handling; the
git_snapshot tests include a best-effort SHA-256 case that skips gracefully
if the local Git build lacks the flag.

Filenames with unusual characters and symlinks need no special handling:
`git add -A` already captures them correctly (symlinks as `120000` blobs;
unusual filenames via Git's own path handling). Submodules also need no
special handling: `git add -A` on a path Git recognizes as a nested
repository stages it as a single `160000` gitlink entry (the submodule's
current commit SHA), matching normal Git tree semantics — the mutation
cursor's tree snapshot correctly captures "this submodule was pinned to
commit X," not the submodule's own file contents, which is the same
information any real Git tree carries.

Temporary-index cleanup uses an RAII guard around the unique
`<git-dir>/sce/tmp/index-<uuid>` path — the guard only *reserves and tracks*
this unique path (generating the name, ensuring the parent `tmp/` directory
exists) and performs best-effort removal on drop, tolerant of an
already-removed or never-created file; it never creates the index file
itself. The path stays genuinely absent until the first `git read-tree`
call (`HEAD` or `--empty`) creates it, which is what makes both branches of
step 2 above produce a valid index rather than risking the invalid
zero-byte state described under "Unborn HEAD." This RAII structure still
fully serves its lifecycle purpose: a mid-capture panic or early return
still triggers cleanup of whatever Git did create at that path, and the
guard's own removal never being guaranteed under a hard process kill is
acceptable because the temp index is pure write-time scratch state — the
durable artifact is the pinned tree/blob objects already written to the
repository's normal object database, not the index.

### `diff_trees`'s output type

`diff_trees(before, after)` runs
`git diff --binary --full-index --no-ext-diff --no-textconv <before> <after>`
with only `GIT_DIR` set (no object-directory environment at all, since both
trees already live in the normal store once pinned) and returns the raw
`Result<String>` output. Git's `--binary` unified-diff format is
base85-encoded text, so the output is always valid UTF-8 even for binary
file changes. This is the smallest abstraction that satisfies Requirement 3:
the existing `cli/src/services/patch.rs::parse_patch(input: &str,
session_id: Option<&str>)` already parses exactly this `diff --git`
formatted text, so the next PR's Agent Trace integration can hand
`diff_trees`'s output straight to the parser this repository already has,
with no new parsing code and no intermediate structured-diff type invented
here.

### `WorktreeId` derivation and `RuntimeBoundary`

The coordinator, not the caller, derives `WorktreeId`: `coordinate()`
resolves `git_dir` from `repository_root`, acquires `WorktreeLock`, then
calls `get_or_create_checkout_id(&git_dir)` (T01's now concurrency-safe
version) and wraps the result as `WorktreeId(checkout_id)`. No
caller-supplied `WorktreeId` or `Boundary` value is ever accepted,
satisfying Requirement 4's "runtime callers should not be allowed to invent
a `WorktreeId`."

A separate `RuntimeBoundary` type is warranted and lives in
`runtime/coordinator.rs`: `types::Boundary` intentionally carries no `ActorKind`
(that is a scope-*registration* concern, not a pure protocol transition
concern) and its `Flush` variant carries an explicit `worktree: WorktreeId`
the coordinator must not let a caller supply. `RuntimeBoundary` matches the
brief's sketch exactly — `Start`/`Advance`/`Close` each carry
`{ scope: ScopeId, event: EventId, actor_kind: ActorKind }`, `Flush` carries
nothing — and the coordinator converts it into a `types::Boundary` (dropping
`actor_kind`, which is consumed earlier for `register_scope`) plus the
internally-derived `WorktreeId` before calling `protocol::prepare`.

### Runtime identity contract: `(ScopeId, EventId)` namespace requirements

`spec/mutation_cursor.md`'s "Event identity" section already states the
production requirement precisely: "Hook replay identity is scoped by
`ScopeId` and `EventId` through `EventKey`. The real implementation must
provide an equivalent uniqueness guarantee. If hook IDs are not unique per
scope, the database key must include the actual delivery namespace, such as
worktree, harness, session, and hook ID." `types::EventKey { scope_id,
event_id }` is exactly this replay key, unmodified — and it has no
`actor_kind` field at all, so `ActorKind` is, by construction, never part of
replay identity; it flows only into `register_scope`.

`RuntimeBoundary` inherits this contract as a documented, load-bearing
precondition rather than something the coordinator can verify or enforce
for its caller — the coordinator has no way to know whether a given
`ScopeId`/`EventId` pair is genuinely unique to one logical hook delivery,
since that fact depends entirely on the harness's own ID scheme. The
contract, to be stated on `RuntimeBoundary`'s doc comment and in a future
harness-adapter's own documentation:

- `RuntimeBoundary` accepts already-canonical runtime identities.
  `(ScopeId, EventId)` must uniquely identify exactly one logical hook
  delivery, for the lifetime of that scope, for replay purposes.
- A harness adapter must not blindly forward a raw native hook/event ID
  unless the harness already guarantees that ID is unique within the given
  scope. If it does not, the adapter — not the coordinator, not
  `protocol.rs`, not `store.rs` — owns constructing a canonicalized ID, for
  example conceptually `ScopeId("<harness>:<native-session-id>")` and, if
  the harness's own event IDs are not already scope-unique,
  `EventId("<harness>:<delivery-namespace>:<native-event-id>")`. This plan
  does not choose or implement an encoding, since no harness adapter exists
  yet to canonicalize for.
- Two different logical hook deliveries colliding under the same
  `(ScopeId, EventId)` is invalid adapter behavior, not a case the protocol
  or coordinator can detect or correct — `store.rs`'s replay-dedup logic
  will (correctly, given its own contract) treat them as the same delivery.
- Replaying the same logical delivery — the harness genuinely re-sending an
  event, for example after a retry — must always produce the identical
  canonical `(ScopeId, EventId)` pair, or replay-dedup silently breaks.

### Worktree/scope materialization ordering

Matches `context/cli/mutation-trace-protocol.md`'s "Runtime scope
materialization"/"Runtime worktree materialization" sections and
Requirement 5, updated for the pin-before-DB-write ordering above:

```text
resolve WorktreeId under the lock
        ↓
capture the ONE Git snapshot for this invocation → observed_tree
        │
        ├── failure → see "Snapshot failure" below: loads durable state
        │             fresh, AFTER this failure — never a state read
        │             earlier in this diagram, because there is none
        │
        └── success
                ↓
        pin_tree(worktree, observed_tree)   (blocking precondition to everything below;
        │                                    a pin failure is handled identically to a
        │                                    capture failure — see "Snapshot failure")
        ↓
store.initialize_worktree(worktree, observed_tree)   (idempotent; no-op if the row already exists)
        ↓
store.register_scope(scope, worktree, actor_kind)    (only for Start/Advance/Close; idempotent; Err on identity conflict)
        ↓
retry loop: load → recover-if-needed → prepare/commit
```

This revision removes an earlier version of this diagram's leading
`existing_worktree = store.load_worktree(...)` probe, taken *before*
snapshot capture, whose only purpose was deciding the snapshot-failure
branch below. That probe was itself a race: another caller could
materialize the worktree row *after* the probe ran but *before* (or during)
this invocation's own capture attempt — a caller that bypasses the advisory
`WorktreeLock` entirely is exactly the scenario the lock cannot see, which
is why CAS remains the authoritative correctness mechanism throughout this
plan. A failing invocation basing its bootstrap-vs-taint decision on that
stale `None` would then durably under-report the failure
(`persisted_taint: false`) even though a worktree row existed by the time
the decision was actually made. The corrected flow makes exactly one durable
read on the failure path, taken after the failure, described in full under
"Snapshot failure" below; the success path never reads durable worktree
state before `initialize_worktree`, whose own idle-insert semantics already
handle "does a row exist yet" correctly without a separate probe.

Capturing the snapshot *before* `initialize_worktree` is what makes AC1's
"first observation" guarantee fall out of the pure protocol with no special
case: `initialize_worktree` sets `cursor_tree = observed_tree`, and this
same `observed_tree` becomes the `Start` attempt's `after_tree`
(`prepare`'s `observed_tree` parameter) — `before_tree == after_tree`, so
`protocol.rs`'s existing `tree_changed` gate is already `false` and no
`MutationEvent` is materialized. No coordinator-side "is this the first
observation" branch exists or is needed.

A scope-identity conflict (`register_scope` returning `Err` because an
existing scope's `worktree_id`/`actor_kind` disagrees with this boundary) is
propagated as `CoordinateError::ScopeIdentityConflict` before the retry loop
runs; the coordinator never overwrites a scope's identity. Per the
create-only pin policy above, the pin already created for this invocation's
`observed_tree` is left in place even on this abort path.

### Protocol execution and `AttemptId`

The coordinator calls `protocol::prepare`/`protocol::commit` exactly as
`protocol.rs` exposes them, and reproduces none of their lifecycle logic.
`AttemptId` is generated fresh (`Uuid::new_v4()`) on each retry-loop
iteration and never persisted: `store::WorktreeProjection::into_protocol_state`
always sets `attempts: BTreeMap::new()` on load (`store.rs`'s own
documented non-goal — no `mutation_trace_attempts` table exists), so
`prepare`'s "already underway" guard (`state.attempts.get(&attempt)`) always
sees `None` regardless of whether the ID is reused or freshly generated each
iteration; generating fresh each iteration is simplest and avoids any doubt
about cross-iteration interaction.

### CAS retry: what it protects, and why it's still needed under the lock

`MutationTraceStore::commit`'s own `execute_transactional_cas_batch` already
retries `Busy`/`BusySnapshot` (SQLite/Turso transient contention) internally,
via `resolve_query_retry_policy`, completely opaque to the coordinator — the
coordinator never sees or reacts to that class of failure directly. What the
coordinator's own retry loop reacts to is `CasResult::Conflict`: a settled,
non-error `Ok` outcome meaning the worktree's guard `UPDATE ... WHERE
revision = ?` affected zero rows, because some other committer already
advanced the revision this attempt was prepared against.

Under `WorktreeLock`, no other `coordinate()` invocation on this exact
worktree can be mid-flight, so in ordinary single-host operation a `Conflict`
at this point should not occur. The retry loop exists anyway because CAS,
not the lock, is the protocol's actual correctness mechanism (per the
brief's own framing) — the lock is real-world defense-in-depth against a
class of writer the lock cannot see: OS advisory locking limitations on some
filesystems, a caller that bypasses `WorktreeLock` entirely, or (out of
scope here, but worth naming) a future multi-host deployment where the lock
is host-local but the DB file is not. The whole `coordinate()` pipeline —
from the fresh `store.load_worktree` at the top of each retry-loop iteration
through the recovery check and the boundary's own `prepare`/`commit` — is
wrapped in one bounded loop (`MAX_CAS_RETRY_ATTEMPTS = 5`, no backoff: a
`Conflict` under the lock is already anomalous, and reload-and-recompute is
cheap, so sleeping before retrying buys nothing). Every iteration reuses the
exact same, already-pinned `observed_tree: TreeId` captured once at the top
of `coordinate()` — the loop only re-reads durable state and re-derives the
transition from it, never re-captures or re-pins the worktree. Exhausting
the bound returns `CoordinateError::CasConflictExhausted { attempts }`.

The snapshot-failure taint path below reuses this exact policy
(`MAX_CAS_RETRY_ATTEMPTS`, no backoff) for its own, narrower retry loop —
one shared constant, one shared rationale, applied in two places.

### Recovery: which triggers are reachable, and why it's two CAS transactions

`spec/mutation_cursor.md`'s "Failure and durability boundary" section states
recovery as one action producing one durable transition:
"snapshot current worktree → establish current tree as the new cursor
baseline → produce no evidence → abandon every active scope (only for
taint/externalTaint) → commit recovery to DB → clear the taint/failure state."
`protocol::recover` (already implemented, unmodified) is exactly this one
pure transition.

`WorktreeProjection::into_protocol_state` always sets `external_taint:
BTreeSet::new()` on load — `store.rs`'s documented, deliberate non-goal,
because `external_taint` is never DB-authoritative. This means, within this
PR's runtime path, recovery can only ever be triggered by the two
DB-observable flags: `tainted` (`SnapshotFailure`) or `needs_rebaseline`.
`external_taint`-triggered recovery is unreachable code in this PR — it
becomes live only once the follow-up PR wires `protocol::database_failure`
and a filesystem external-taint marker (see Requirement 10, below). Both
reachable modes are implemented and tested in T04: `needs_rebaseline`
recovery preserves live scopes (only the skipped interval is ambiguous),
taint recovery abandons them (per `protocol::recover`'s own guarded
behavior, unmodified).

Recovery and the triggering boundary are **two sequential DB CAS
transactions**, not one, and not a single combined `DurableTransition`:
`DurableTransition::between` hard-`bail!`s unless the worktree's revision
advances by *exactly one* between `before` and `after`
(`cli/src/services/mutation_trace/store.rs:304-310`). Recovery alone
advances revision by one; the triggering boundary, if accepted, advances it
by one more. A `before`-to-after-both-steps diff would require an advance of
two, which `between` explicitly rejects — so recovery must be persisted
(`store.commit`) before the triggering boundary's own `prepare`/`commit` is
even evaluated against the resulting state. Both transactions happen inside
the same retry-loop iteration, under the same lock hold, and both reuse the
one captured, already-pinned `observed_tree`: recovery uses it as its
rebaseline target, and because the triggering boundary is then prepared
against the *already* just-rebaselined cursor (which now equals
`observed_tree`), a boundary processed in the same invocation that just
recovered naturally observes `before_tree == after_tree` and emits no
evidence for the just-discarded interval — again, no special-case
coordinator logic, purely a consequence of reusing one snapshot for both
steps.

If recovery's own CAS conflicts, the whole retry-loop iteration restarts
(reload → re-check recovery need → recompute), governed by the same
`MAX_CAS_RETRY_ATTEMPTS` bound as boundary-CAS conflicts — they are the same
phenomenon (stale reload needed) and share one counter.

### Snapshot failure: bootstrap case, and correct CAS retry for the taint transition

`SnapshotCapture::capture` or `SnapshotCapture::pin` returning `Err` are
treated identically (see "Crash ordering" above — pinning is a blocking
precondition, so a pin failure is a snapshot failure in every way that
matters here). Unlike an earlier revision of this plan, the coordinator does
**not** consult any durable state read before this failure — see
"Worktree/scope materialization ordering" above for why a pre-capture probe
is itself a race. Instead, the very first thing the failure handler does is
its own fresh read, and that single read decides both the bootstrap-vs-taint
branch *and* doubles as the first iteration of the taint-retry loop:

```text
loop (bounded MAX_CAS_RETRY_ATTEMPTS):
    state = store.load_worktree(worktree, None, None)   -- fresh every iteration,
                                                          -- including the first, and
                                                          -- always AFTER the capture/pin
                                                          -- failure, never before it
    match state {
        None =>
            // No durable worktree row exists, even now, freshly checked
            // after the failure. There is no durable state to taint and no
            // baseline TreeId to fabricate one from — protocol::taint itself
            // guards on worktree existence and would be a no-op anyway.
            return SnapshotFailure { persisted_taint: false, source }
            // No durable write is made. Per the brief: do not fabricate a TreeId.
        Some(loaded) => {
            tainted_state = protocol::taint(&loaded, &worktree)
            match DurableTransition::between(&loaded, &tainted_state, &worktree)? {
                None =>
                    // taint() itself no-opped: the worktree was already
                    // tainted (durable state already reflects a failure),
                    // or is already at revision u64::MAX. Either way there
                    // is nothing new to persist, and the *current* durable
                    // state already answers "is this worktree tainted" —
                    // read it back directly rather than trusting a stale
                    // local flag.
                    return SnapshotFailure {
                        persisted_taint: loaded.worktrees[&worktree].tainted,
                        source,
                    }
                Some(transition) => match store.commit(&transition)? {
                    CasResult::Applied =>
                        return SnapshotFailure { persisted_taint: true, source }
                    CasResult::Conflict => continue   -- reload next iteration; this is
                                                       -- exactly the "another caller
                                                       -- materialized/advanced the
                                                       -- worktree concurrently" case
                }
            }
        }
    }
// loop exhausted without ever reaching Applied, an already-tainted no-op, or None
return SnapshotFailure { persisted_taint: false, source }
```

Because the `None` branch is evaluated fresh on *every* iteration — not only
the first — this also directly answers the race the review specifically
named: a worktree that did not exist when this invocation began its Git
snapshot capture, but that another caller materializes concurrently while
that capture is still in flight (for example a caller that bypasses
`WorktreeLock`, since the lock only serializes cooperating callers against
each other), is still found and correctly tainted, because the failure
handler's read happens strictly after the capture attempt has already
failed, giving any such concurrent writer every opportunity to have already
landed its row.

`persisted_taint` therefore reports **whether the durable worktree state is
(now) tainted**, not merely "did this exact call perform a write" — this is
both simpler to reason about (one boolean, one meaning) and more useful to a
caller than distinguishing "we tainted it" from "someone else already had,"
since both mean the same thing for the caller's purposes: the durable state
honestly reflects the failure. `persisted_taint` is `false` in exactly two
cases: the bootstrap case (no row exists on the fresh, post-failure read),
and taint-CAS exhaustion (every attempt in the bounded loop conflicted).
Both report through the same `CoordinateError::SnapshotFailure` variant,
distinguished only by the `source` error chain's message — a caller only
needs to know "was the failure durably recorded," and a second public enum
case to distinguish *why* it was not would leak an implementation detail
(this codebase's existing `RepositoryIdentityResolutionError` precedent for
typed-error precision is about *distinguishable outcomes a caller must react
to differently*, and bootstrap-vs-exhaustion do not differ in the required
reaction: "the taint was not recorded, treat this worktree as if the
model's `database_failure` case applied").

DB busy/transient retry inside this loop's own `store.commit` call stays
exactly where it already lives — inside `execute_transactional_cas_batch`,
via `resolve_query_retry_policy` — never duplicated here, matching the main
retry loop's own separation of concerns.

The future filesystem external-taint marker remains responsible for making
the still-deferred DB-unavailable case (Requirement 10, below) crash-safe;
this fix only corrects the already-in-scope `SnapshotFailure`/CAS-conflict
interaction, which does not depend on that marker existing.

### Requirement 10: database unavailability and external taint stay deferred

`spec/mutation_cursor.md` distinguishes snapshot failure (recorded via
`taint`, a DB-backed field) from database unavailability
(`databaseFailure`, which changes only the abstractly-modeled
`externalTaint` — explicitly "not a database row"). `store.rs` already
established, in the prior plan, that this distinction is real in the Rust
implementation too: `DurableTransition::between` returns `Ok(None)` for a
`database_failure`-only transition, so `store.commit` is never even called
for it, and no `external_taint` column or table exists.

Building the filesystem marker `externalTaint` conceptually models would be
new, non-trivial infrastructure (a durability signal that must itself
survive process crashes and be readable before the DB is even opened) that
nothing in the existing code makes unavoidable: when `store.load_worktree`
or `store.commit` returns a genuine `Err` after the DB layer's own retries
are exhausted (this applies both to the main boundary-processing loop and
to the snapshot-failure taint-retry loop above), `coordinate()` simply
propagates it as `CoordinateError::Other`, leaving no trace anywhere. This
is a correct, if intentionally incomplete, subset of the model's behavior —
"durable DB protocol state remains unchanged" holds; "externalTaint
contains the worktree" does not yet. `protocol::database_failure` is not
called anywhere in this PR, matching its current (unmodified) unwired
status. This is recorded as the follow-up PR's first concern (see below),
exactly as the brief anticipated as the likely outcome.

### `CoordinateOutcome` and `CoordinateError`

```rust
pub struct CoordinateOutcome {
    pub worktree_id: WorktreeId,
    pub observed_tree: TreeId,
    pub revision: u64,
    pub evaluation: protocol::CommitEvaluation,
    pub mutation_event: Option<types::MutationEvent>,
}

pub enum CoordinateError {
    SnapshotFailure { persisted_taint: bool, source: anyhow::Error },
    ScopeIdentityConflict(anyhow::Error),
    CasConflictExhausted { attempts: u32 },
    LockAcquisition(anyhow::Error),
    Other(anyhow::Error),
}
```

Unchanged from the original plan except in the meaning of
`SnapshotFailure.persisted_taint`, corrected above. This is the smallest
return/error shape that satisfies every requirement without leaking store
internals (`DurableTransition`, `CasResult`, SQL row shapes never appear in
either type). `revision`/`evaluation`/`mutation_event` all come directly
from the *triggering boundary's* own `protocol::commit` result
(`CommitOutcome`), not from any intermediate recovery step.

### Dependency injection for deterministic tests

`SnapshotCapture` (`fn capture(&self) -> Result<TreeId>`, `fn pin(&self,
worktree_id: &WorktreeId, tree: &TreeId) -> Result<()>`) is the one seam this
plan introduces for determinism. `capture` takes no `repository_root`
parameter: `GitSnapshotService::new(repository_root)` binds that context once
at construction, so `capture()` does not receive it repeatedly on every call.
`GitSnapshotService` implements the trait for production use, and T04's tests use
a fake, call-counting implementation to prove "exactly one `capture`, at
most one `pin`, per invocation, even across CAS retries" (AC8) without
needing real concurrent Git processes. The lock is *not* faked —
`runtime::worktree_lock::tests`/`checkout::tests` and T05's contention test exercise
real `std::fs::File` locks against real temp directories, because proving
actual OS-level exclusion is the point. The DB is not faked either, matching
`store.rs`'s own established precedent
(`RepositoryAgentTraceDb::new_at(&temp_path)` against a real temp-file
database, not an in-memory or mocked store).

Proving the taint-CAS-retry path (AC11) deterministically follows the same
pattern the existing `store.rs` test suite already uses for CAS-conflict
scenarios (for example `competing_prepared_attempts_the_second_to_commit_is_rejected_by_cas`):
start from one loaded `ProtocolState`, commit a *second*, independently
derived transition directly through `store.commit` first (simulating "some
other committer already advanced this worktree"), then run the taint-retry
logic starting from the now-stale first state — its first commit attempt
observably conflicts, forcing the loop to reload and retaint, without
needing thread synchronization. Proving exhaustion uses a background thread
that keeps advancing the worktree's revision via direct, independent
`store.commit` calls for the duration of the bounded loop, guaranteeing
every attempt conflicts.

### What changes vs. what's new

`cli/src/services/checkout/mod.rs` is now genuinely **modified**, not
purely additive: `get_or_create_checkout_id`'s body changes (T01), though
its signature and the checkout-ID file format do not. `protocol.rs`,
`types.rs`, `store.rs`, `repository_identity/`, and `agent_trace_storage/`
are all read but not modified — `agent_trace_storage` benefits from T01
automatically, with no change of its own required. No schema migration is
added: `store.rs`'s existing five tables and its
`initialize_worktree`/`register_scope`/`load_worktree`/`commit` API are
sufficient for everything this coordinator needs.

## Files

New:

- `cli/src/services/mutation_trace/runtime/mod.rs` — declares the runtime
  submodules and exposes only what the rest of the crate needs (the
  `coordinate()` entrypoint and its outcome/error types); Git snapshot and
  lock internals stay unexposed outside `runtime`.
- `cli/src/services/mutation_trace/runtime/worktree_lock.rs` — per-worktree
  runtime OS advisory lock (T02).
- `cli/src/services/mutation_trace/runtime/git_snapshot.rs` — isolated Git
  snapshot capture, ref pinning, and `diff_trees` (T03).
- `cli/src/services/mutation_trace/runtime/coordinator.rs` — `RuntimeBoundary`,
  `CoordinateOutcome`, `CoordinateError`, `SnapshotCapture`, the internal
  protocol-integration pipeline, and the public lock-wrapped `coordinate()`
  entrypoint (T04, T05). This is the composition point: the only module that
  combines `protocol`/`store`/`types` with `runtime::git_snapshot`,
  `runtime::worktree_lock`, and `services::checkout`.
- `cli/src/services/mutation_trace/runtime/tests.rs` — cross-module
  linked-worktree, cross-caller checkout-identity, and end-to-end
  failure/recovery integration tests (T06), declared as `runtime`'s own
  `#[cfg(test)] mod tests`.

Modified:

- `cli/src/services/checkout/mod.rs` — `get_or_create_checkout_id` gains an
  internal identity-creation lock plus a crash-safe temp-file-and-rename
  write sequence (T01); no signature change. `checkout/` remains its own
  top-level service, not moved under `mutation_trace`.
- `cli/src/services/mutation_trace/mod.rs` — add `pub(crate) mod runtime;`
  alongside the existing `pub mod protocol; pub mod store; pub mod types;`
  and private `mod mbt;`, consistent with the module's existing
  `#[allow(dead_code)]` precedent for code not yet wired to a command/hook.
  `runtime/mod.rs` itself declares `mod git_snapshot; mod worktree_lock; mod
  coordinator; #[cfg(test)] mod tests;`, keeping the Git-snapshot and lock
  modules private to `runtime` — only `coordinator`'s public entrypoints are
  reachable from outside it.

Docs (context sync, not authored in this plan's tasks):

- `context/cli/mutation-trace-protocol.md`, `context/cli/mutation-trace-store.md`,
  `context/cli/checkout-identity.md`, `context/overview.md`,
  `context/context-map.md` — see "Context sync" above.

Cargo/dependency changes: none.

## Testing strategy

- **Pure unit tests** (no filesystem/DB/lock): none new — `protocol.rs`'s
  existing pure-function tests already cover `prepare`/`commit`/`recover`/
  `taint`/`abandon`; this plan only adds imperative-shell code around them.
- **Git integration tests** (`runtime/git_snapshot.rs`, real temp Git repos): index
  preservation under staged+unstaged+untracked simultaneously, ignored-file
  exclusion, tracked-file deletion; unborn `HEAD` — both with a file present
  (asserting the resulting tree contains it) and with the working tree
  completely empty (asserting a valid, zero-entry tree, its object ID never
  hardcoded) — driving the real `capture_tree` command sequence against a
  freshly `git init`ed repository with no commit, proving `git read-tree
  --empty` (not a bare temp-index file) is what makes `git add -A -- .`
  succeed; snapshot durability after temp-index deletion; **pinned-tree
  survival across `git gc --prune=now` and `git prune --expire=now`**,
  proven with a negative control that captures a *second, distinct-content*
  tree in the same repository, deliberately leaves it unpinned and
  unreachable from any ref/branch/tag/reflog, and asserts that same
  aggressive `gc`/`prune` pass reclaims it while the first, pinned tree
  survives — never a same-repository comparison between two copies of
  identical content, since Git's content-addressing would make those the
  same object and unpinning one would (correctly) leave it pinned by the
  other; `pin_tree` idempotence; `diff_trees` correctness; best-effort
  SHA-256 case. Prove: `capture_tree` faithfully represents the current
  worktree without ever touching the real index, always starts from a valid
  Git index regardless of whether `HEAD` exists, and a pinned snapshot's
  durability holds against Git's own reachability analysis (proven against a
  genuinely unreachable control object, not against "the temp index is
  gone" or an object the test itself cannot actually make unreachable).
- **Store/DB integration tests** (`runtime/coordinator.rs`, real temp-file
  `RepositoryAgentTraceDb`): AC1–AC5, AC10, AC11 — first observation,
  exclusive mutation, no-op observation, replay, close attribution,
  contention, both recovery modes, snapshot-failure with immediate success,
  snapshot-failure taint surviving a losing CAS and committing on retry,
  snapshot-failure taint exhaustion never claiming `persisted_taint: true`,
  and the bootstrap snapshot-failure case. Prove: the coordinator drives the
  existing store/protocol APIs to produce exactly the durable outcomes the
  Quint model specifies, including under contention on the taint path.
- **Concurrency tests** (`checkout::mod.rs` T01, `runtime/worktree_lock.rs` T02,
  `runtime/coordinator.rs` T05): concurrent first-time `get_or_create_checkout_id`
  callers on one `git_dir` converge on one ID; a second `try_lock()` on the
  same worktree-lock path observably blocks/fails while the first holds it
  and succeeds immediately after `Drop`; two real threads calling
  `coordinate()` against the same worktree serialize (the second's critical
  section provably starts only after the first's ends). Prove: both locks
  are OS-level exclusion, not lockfile-existence checking, and the
  checkout-identity fix actually closes the race for every caller, not just
  the coordinator's.
- **Checkout-identity crash-safety tests** (`checkout::mod.rs` T01): the
  canonical `checkout-id` path remains absent through every step of the
  write sequence up to and including a successful `sync_data()`, and only
  becomes present, with complete content, once `std::fs::rename` returns —
  exercised by driving the internal persistence helper up to (but not past)
  the rename call and asserting the canonical path's absence, then letting
  it complete and asserting the canonical path's complete content; an
  abandoned `checkout-id.tmp-*` file (simulating an interruption after
  `sync_data()` but before `rename`) does not prevent, or alter the outcome
  of, a subsequent `get_or_create_checkout_id` call, which still converges
  on one complete, valid ID. Prove: AC13 — the canonical path is never
  observable as partially written, and orphaned temp files are inert.
- **Linked-worktree tests** (`runtime/tests.rs`, real `git worktree add`):
  two linked worktrees derive distinct `checkout_id`/`WorktreeId` and
  distinct runtime-lock paths — proven by holding one worktree's
  `WorktreeLock` across a synchronous `coordinate()` call for the other and
  observing that call succeed before the held guard is dropped, not by any
  wall-clock timing — their distinct worktree rows coexist in the one
  caller-supplied repository-scoped DB handed to both calls (`coordinate()`
  does not resolve the DB), and a tree pinned from one worktree resolves
  correctly when queried via the other worktree's `GIT_DIR`. Prove: AC9 end
  to end through the public API.
- **Cross-caller checkout-identity test** (`runtime/tests.rs`): a direct
  `agent_trace_storage` resolution and a `coordinate()` call against the
  same `repository_root`, run concurrently on first-ever resolution, observe
  the identical checkout ID. Prove: AC12's convergence guarantee holds
  across module boundaries, not only within `checkout::mod.rs`'s own test
  suite.
- **Failure/recovery tests** (`runtime/coordinator.rs`, `runtime/tests.rs`): CAS
  conflict via two prepared-but-not-yet-committed states from the same
  revision (proving reload+recompute+no-second-snapshot, and no re-pin);
  snapshot failure with and without a prior worktree row; snapshot-failure
  taint retried through a losing CAS to a successful one, and exhausted
  entirely; snapshot failure against a worktree row that a second, direct
  `store.initialize_worktree` call materializes only *after* the injected
  capture failure begins (using a fake `SnapshotCapture` whose `capture`
  performs that materialization itself, immediately before returning `Err`),
  proving the failure handler's fresh, post-failure read finds and taints it
  rather than basing its decision on any earlier state; a full two-invocation
  failure-then-recovery cycle via the public API. Prove: AC8, AC10, AC11 in
  both unit and end-to-end form.
- **Lock staleness**: `runtime::worktree_lock::tests` documents (via a test that
  opens, writes bytes to, and closes the lock file *without* ever calling
  `.lock()`, then proves a subsequent real `WorktreeLock::acquire` on the
  same path succeeds immediately) that a leftover lock file with no active
  OS lock held against it never blocks a future caller — the OS lock, not
  the file's existence, is the ownership signal. The same property is
  exercised for `checkout-id.lock` in `checkout::mod.rs`'s own tests.

## Failure matrix

| Failure | Durable state mutation? | External marker? | Retry? | Result |
| --- | --- | --- | --- | --- |
| checkout-identity lock (T01) | No | No | No (blocking, no timeout — see rationale above) | Blocks until the current holder releases; cannot itself fail except on genuine I/O error, surfaced as `Err` |
| process crash between `checkout-id.tmp-*` creation and rename (T01) | No — the canonical `checkout-id` path is unaffected; either absent (fresh checkout) or unchanged (identity already existed) | No | No — not a retryable operation from the crashed process's perspective; a later, fresh call retries the whole sequence from scratch | Orphaned `checkout-id.tmp-*` file, permanently harmless (see Design decisions); the next `get_or_create_checkout_id` call on this `git_dir` succeeds normally |
| mutation-cursor lock acquisition (timeout) | No | No | No (single bounded polling attempt per call) | `CoordinateError::LockAcquisition`, no writes attempted |
| Git snapshot capture or pin, worktree row exists on the fresh post-failure read | Yes — a `protocol::taint` transition, retried under the shared bounded CAS-retry policy until `Applied` or exhaustion | No | Yes — up to `MAX_CAS_RETRY_ATTEMPTS`, reload/retaint/recommit each time, each iteration re-reading durable state fresh | `CoordinateError::SnapshotFailure { persisted_taint: true }` once `Applied`, or `{ persisted_taint: false }` after exhaustion |
| Git snapshot capture or pin, no worktree row on the fresh post-failure read (bootstrap, or not yet materialized by any concurrent writer) | No | No | No | `CoordinateError::SnapshotFailure { persisted_taint: false }` |
| DB busy/transient (`Busy`/`BusySnapshot`) | N/A until resolved | No | Yes — inside `execute_transactional_cas_batch`, opaque to the coordinator; applies identically to the main loop and the taint-retry loop | Transparent success, or `Err` propagated after the DB layer's own retries are exhausted |
| CAS conflict (semantic, guard affected 0 rows), main loop | No | No | Yes — coordinator's bounded retry loop, reloads state, reuses the same captured and already-pinned snapshot | Eventually `Applied`, or `CoordinateError::CasConflictExhausted` |
| DB unavailable (`Err` surviving DB-layer retries) | No | No (out of scope this PR) | No | `CoordinateError::Other`; `external_taint` not recorded (deferred, see follow-up PR) |
| recovery's own CAS conflict | No | No | Yes — same outer loop, whole iteration re-executes reusing the same captured and already-pinned snapshot | Eventually `Applied`, or `CoordinateError::CasConflictExhausted` |
| scope identity conflict (`register_scope`) | No | No | No | `CoordinateError::ScopeIdentityConflict`, no protocol transition attempted; this invocation's pin (if already created) is left in place per the create-only policy |

## Validation

- `nix flake check` — runs `checks.<system>.cli-tests` (all new tests
  above), `checks.<system>.cli-clippy` (pedantic/warnings-denied), `checks.<system>.cli-fmt`,
  `checks.<system>.mutation-trace-quint-connect` (confirms the unmodified
  `protocol.rs`/`types.rs` still refine `spec/mutation_cursor.qnt` — this PR
  changes neither, so this should pass unchanged), `checks.<system>.pkl-generated`,
  and `checks.<system>.workflow-actionlint`.
- `nix run .#pkl-check-generated` — lightweight post-task baseline per
  `context/patterns.md`; this PR touches no generated-config surface, so this
  confirms no accidental drift.
- No new or modified Quint model file, so no separate `nix run .#... quint`
  verification step is needed beyond the existing `mutation-trace-quint-connect`
  check above.

## Open questions

None. Every architecture question the brief enumerated, plus every
correctness issue raised in the PR #244 review that produced this revision,
is resolved above with a concrete, code-grounded (and, for the Git-level
claims, experimentally verified) decision. See "Final quality check" below
for a direct, itemized accounting.

## Final quality check

1. **Can `checkout-id` ever be exposed half-written after an SCE process
   crashes during creation?** No, once T01 lands. The canonical path is
   written by generating the ID, writing it in full to a uniquely named
   temp file, syncing that file's content, and only then atomically
   renaming it into place — a reader of the canonical path observes either
   its prior state (absent, for a fresh checkout) or the complete value,
   never a partial one.
2. **What exact atomic persistence sequence prevents that?**
   `OpenOptions::create_new(true)` on `checkout-id.tmp-<uuid>` →
   `write_all` the complete ID → `File::sync_data()` → `std::fs::rename`
   into `checkout-id` → (Unix only, best-effort) sync the parent directory.
   Verified experimentally: the canonical path is absent at every point
   before the rename call and contains the complete value immediately after
   it; an orphaned temp file left by an interruption before the rename does
   not affect the canonical path's content.
3. **Can concurrent checkout-ID callers still return different IDs?** No,
   once T01 lands — for any caller, not only the coordinator.
   `get_or_create_checkout_id` itself acquires `<git-dir>/sce/checkout-id.lock`,
   re-checks for an existing ID under the lock, and only then generates and
   writes one via the atomic sequence above — the fix lives in the
   primitive, so every caller (including `agent_trace_storage`, unmodified)
   benefits automatically.
4. **Does snapshot failure use any DB state loaded before the snapshot
   attempt?** No, once T04 lands. An earlier revision of this plan read
   worktree existence before capture, purely to decide the bootstrap-vs-taint
   branch on failure; that read has been removed. The only durable read the
   snapshot-failure path performs happens inside the failure handler itself,
   after the failure, and is repeated fresh on every taint-retry iteration.
5. **If another caller creates the worktree row while snapshot capture is
   running, will the failing invocation find and taint it?** Yes. Because
   the failure handler's first (and every subsequent) read happens strictly
   after the capture attempt has already failed, any concurrent writer —
   including one that bypasses `WorktreeLock` entirely, which the lock
   cannot itself prevent — has already had the opportunity to land its row
   by the time that read runs.
6. **How is a private Git index initialized when `HEAD` exists?**
   `git read-tree HEAD` against the reserved, not-yet-created
   `GIT_INDEX_FILE` path — Git creates a valid index populated from `HEAD`'s
   tree.
7. **How is it initialized when `HEAD` is unborn?** `git read-tree --empty`
   against that same not-yet-created path — Git creates a valid, genuinely
   empty index. An earlier revision of this plan instead skipped this step
   entirely on an unborn `HEAD` and assumed a bare temp-index file was
   equivalent; verified experimentally that a zero-byte file at
   `GIT_INDEX_FILE` is not a valid index and fails `git add -A -- .`
   deterministically (`fatal: ... index file smaller than expected`).
8. **Can any path through `capture_tree()` pass a zero-byte invalid index to
   `git add`?** No. The RAII guard around the temp-index path only reserves
   and tracks the unique filename; it never creates a file there itself.
   Both branches of index initialization (question 6 and question 7) are
   Git's own `read-tree` calls, which always produce a structurally valid
   index — there is no code path that reaches `git add -A -- .` against a
   path Git has not already validly initialized.
9. **Does the unborn-HEAD integration test run the actual production command
   sequence?** Yes — against a freshly `git init`ed repository with no
   commit, driving `capture_tree` itself (`rev-parse --verify --quiet HEAD`
   → `read-tree --empty` → `add -A` → `write-tree`), not a hand-rolled
   substitute sequence, covering both an unborn repository with a file
   present and one with no files at all.
10. **Does the test avoid hardcoding the SHA-1 empty-tree ID?** Yes. The
    no-files case asserts a zero-entry `git ls-tree` result and lets Git
    compute the tree object ID itself; no test or production code embeds the
    canonical `4b825dc6...` (or SHA-256 equivalent) value.
11. **Can two identical trees in the same Git object database meaningfully
    be "one pinned and one unpinned"?** No — this was the original plan's
    negative-control mistake, now corrected. Git is content-addressed:
    identical content is the same object, so pinning either "copy" pins
    both. The corrected design captures two trees with *distinct* content in
    the same repository, pins only one, and leaves the other genuinely
    unreachable (no ref, branch, tag, or reflog entry).
12. **What object does the GC negative-control test actually prune?** The
    second, distinct-content tree from question 11 — never a second capture
    of identical content, and never the pinned tree itself.
13. **What ref protects the positive-control snapshot?**
    `refs/sce/mutation-cursor/<worktree-id>/<tree-sha>`, created by
    `pin_tree` before the test's `gc`/`prune` pass runs.
14. **Does the GC test prove Git reachability rather than relying on
    incidental object lifetime?** Yes. The control tree is deliberately
    unreachable from every ref-like source Git's GC considers (no branch, no
    tag, no reflog — reflogs only ever record commit-ref history, and these
    loose, never-committed trees have none regardless), so its reclamation
    under `git gc --prune=now`/`git prune --expire=now` demonstrates the test
    environment can actually prune an unreachable object; the pinned tree's
    survival under the identical pass then demonstrates the SCE ref, not
    incidental timing or auto-GC's default grace period, is what protects
    it.
15. **Can a durable `TreeId` ever become unprotected from Git GC before its
    DB row is committed?** No. Pin creation
    (`git update-ref refs/sce/mutation-cursor/<worktree-id>/<tree-sha>`) is a
    blocking precondition to the DB CAS, not a follow-up step after it — the
    coordinator never attempts the CAS without a successful pin already in
    place, so there is no window in which a durable DB row could reference
    an unprotected tree.
16. **What Git refs can leak after crashes or CAS loss?** A
    `refs/sce/mutation-cursor/<worktree-id>/<tree-sha>` ref (and the objects
    it protects) for a tree that never became the worktree's durable cursor
    and never appears in any `mutation_trace_events` row — for example a
    rejected CAS attempt's observation, or a pin created just before a crash
    that never reached the DB CAS at all. This PR never deletes such a ref
    (see "Pin lifecycle: create-only in this PR").
17. **Is ref growth bounded or unbounded before reconciliation exists?**
    Unbounded over the repository's lifetime — this plan's original
    "bounded, linear" description was imprecise and is corrected above.
    Growth is linear *in hook-invocation volume*, which is itself unbounded
    over a repository's life; nothing in this PR's own scope ever removes a
    pin.
18. **What exact PR must add ref reconciliation, and what exact PR must add
    filesystem external-taint durability?** `mutation-cursor-ref-reconciliation`
    and `mutation-cursor-external-taint` respectively — see "Runtime
    completion sequence" under Follow-up PR. They may combine into one PR if
    the result stays coherent, but neither is this PR's own task stack.
19. **Are both required before production harness wiring?** Yes, explicitly.
    The harness-adapter follow-up (step 4 of the runtime completion
    sequence) is the point at which real, sustained invocation volume first
    exists; both deferred gaps are latent-but-harmless in this PR's
    standalone form and become live risks only once that traffic exists.
20. **Can `SnapshotFailure.persisted_taint` ever be `true` after a failed
    CAS?** No, once T04 lands. It is `true` only after `store.commit`
    returns `CasResult::Applied` for a taint transition, or when a fresh
    reload shows the worktree already durably tainted — never merely after
    an attempt.
21. **How is snapshot-failure CAS conflict retried?** By the same bounded,
    no-backoff `MAX_CAS_RETRY_ATTEMPTS` policy the main coordinator loop
    uses: reload the worktree, recompute `protocol::taint` against the fresh
    state, attempt `store.commit` again, up to the bound — the same loop
    whose first iteration also decides the bootstrap-vs-taint branch (see
    question 4).
22. **What exact uniqueness contract must harness adapters satisfy for
    `(ScopeId, EventId)`?** It must uniquely identify one logical hook
    delivery for the lifetime of its scope; `ActorKind` plays no part in
    that identity; a harness whose native IDs are not already scope-unique
    must have its adapter construct a canonicalized ID before ever
    constructing a `RuntimeBoundary`; replaying the same logical delivery
    must always reproduce the identical canonical pair.
23. **Which operation remains the protocol linearization point?** The DB CAS
    inside `MutationTraceStore::commit` (`UPDATE ... WHERE revision = ?`),
    unchanged from the original plan and from `store.rs`'s own design. Ref
    pinning is durability infrastructure around that point, never a
    substitute for it; the runtime lock, the checkout-identity lock, and the
    checkout-identity crash-safe rename are all serialization/durability
    optimizations around that one linearization point, never a second
    decision-making mechanism.
24. **Are all previous PR #244 correctness fixes still preserved?** Yes: the
    normal-object-database-plus-refs design, pin-before-CAS crash ordering,
    the checkout-identity lock and its crash-safe rename, the
    post-failure-only snapshot-failure read, bounded CAS retry on the taint
    path, the runtime completion sequencing, and the `(ScopeId, EventId)`
    replay contract are all unchanged by this revision's two Git-snapshot
    corrections, which are additive precision fixes to the snapshot
    mechanism and its tests, not architectural reversals.
25. **What unsafe state is impossible by construction after these changes?**
    A durable DB row (worktree cursor or `mutation_trace_events` row)
    referencing a `TreeId` Git could prune; two callers durably disagreeing
    about one worktree's checkout identity, or one observing a half-written
    identity; a coordinator reporting a snapshot-failure taint as persisted
    when the DB never actually applied it, or basing that decision on state
    read before the failure occurred; `git add -A -- .` ever running against
    an invalid (zero-byte) index on an unborn `HEAD`; a GC durability test
    whose result is meaningless because its "pinned" and "unpinned" trees
    were the same content-addressed object; and — unchanged from the
    original plan — a CAS-conflict retry re-capturing the worktree instead
    of reusing the invocation's original observation.

## Follow-up PR

### Runtime completion sequence

This PR is standalone and safe to ship on its own — nothing calls
`coordinate()` in production yet, so its two known, intentionally deferred
durability gaps (below) have no live consequence today. They stop being
merely theoretical the moment a harness adapter starts driving real hook
traffic through it, so this plan sequences the work accordingly rather than
jumping straight to harness wiring:

```text
1. mutation-cursor-runtime-coordinator          (this PR)
        ↓
2. mutation-cursor-external-taint               (filesystem externalTaint marker;
        ↓                                        protocol::database_failure wiring)
3. mutation-cursor-ref-reconciliation           (refs/sce/mutation-cursor/** reclaim pass)
        ↓
4. harness boundary adapters + Agent Trace evidence
        (RuntimeBoundary construction, MutationEvent → diff_trees →
         existing diff_traces insertion)
```

Steps 2 and 3 may ship as one combined follow-up PR if the resulting change
stays coherent — they are independent problems (DB-unavailability
durability vs. Git-object storage growth) with no shared implementation, so
combining them is a sequencing convenience, not a technical dependency
between them. What is not acceptable is skipping either of them, or
reordering step 4 ahead of both: a harness adapter is a **production
consumer** of this coordinator, and only once steps 2 and 3 exist does the
coordinator's own durability story hold under the invocation volume a real
harness integration would actually produce.

**Step 2 — external taint (why it cannot wait):** `spec/mutation_cursor.md`
already states that DB unavailability requires a filesystem durability
signal, because a DB write failure cannot be recorded by writing to the same
DB that just failed. Without it, exactly the scenario the model's
`externalTaint` mechanism exists to prevent remains reachable once real
hook traffic flows: a worktree changes → the DB becomes unavailable for that
one invocation → `coordinate()` fails with no durable trace of the lost
observation interval → a later, successful invocation has no signal that it
must conservatively rebaseline rather than continuing to attribute normally
across the gap. This PR's Requirement 10 resolution deliberately keeps this
gap narrow and named (`CoordinateError::Other`, no durable write, see
Design decisions) rather than leaving it unstated; this follow-up is where
it closes.

**Step 3 — ref reconciliation (why it cannot wait):** see "Pin lifecycle:
create-only in this PR — and that growth is unbounded, not bounded, until
reconciliation exists" above for the corrected growth analysis and the
retained-roots contract this follow-up must implement (worktree
`cursor_tree` and every historical `mutation_trace_events`
`before_tree`/`after_tree`), plus the concurrency-safety requirement any
implementation must satisfy (never delete a ref for a tree that could still
become durable — see that section for the two candidate strategies this
plan leaves for the follow-up to choose between).

### Step 4: harness boundary → runtime coordinator → committed `MutationEvent` → `diff_trees(before, after)` → existing Agent Trace `diff_traces` evidence

Once steps 2 and 3 exist, this final integration PR translates one
harness's hook payloads (starting with Claude Code, or whichever is
prioritized) into `RuntimeBoundary` values — and, per the runtime identity
contract above, owns canonicalizing that harness's native session/event IDs
into scope-unique `ScopeId`/`EventId` pairs before ever constructing one —
calls this PR's `coordinate()`, and — when `CoordinateOutcome.mutation_event`
is `Some`, i.e. the triggering boundary produced real evidence — feeds its
`before_tree`/`after_tree` through this PR's `diff_trees` and the existing
`cli/src/services/patch.rs::parse_patch` into an `Agent Trace` `diff_traces`
row, alongside the mutation event's `attribution`/`boundary`/scope
information. It also owns deciding how (or whether) multiple harnesses share
one coordinator invocation path.

## Validation Report

**Status:** validated  
**Date:** 2026-08-30

### Commands run

- `nix flake check` -> exit 0 (all checks passed: `cli-tests` 824 tests,
  `cli-clippy` pedantic/warnings-denied, `cli-fmt`,
  `mutation-trace-quint-connect`, `pkl-generated`, `workflow-actionlint`,
  and the flatpak/cargo-source parity checks)
- `nix run .#pkl-check-generated` -> exit 0 (ephemeral Pkl generation passed:
  141 files, inventory sha256 bf5db9c962cc9ce2776b4fc218dcbd8787fa7567744a5e2faff1fc9f9212a003 — no generated-config drift)
- `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml runtime::` ->
  exit 0 (40/40 passed, covering every `runtime::coordinator`,
  `runtime::git_snapshot`, `runtime::worktree_lock`, and `runtime::tests` case)
- `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml checkout::` ->
  exit 0 (5/5 passed)

### Success-criteria verification

- [x] AC1: A first-observed worktree establishes the observed tree as its
  cursor baseline and emits no evidence for pre-observation changes ->
  `runtime::coordinator::tests::first_observation_establishes_baseline_without_evidence` passed
- [x] AC2: An edit between `Start` and `Advance` commits exactly one
  `AiExclusive` event with matching before/after trees ->
  `runtime::coordinator::tests::exclusive_edit_between_start_and_advance_commits_one_event` passed
- [x] AC3: Re-processing the identical `(scope, event)` boundary duplicates no
  evidence ->
  `runtime::coordinator::tests::replaying_the_same_scope_event_key_does_not_duplicate_evidence` passed
- [x] AC4: A mutation just before `Close` is attributed using the pre-`Close`
  scope set ->
  `runtime::coordinator::tests::close_boundary_attributes_using_pre_close_scope_set` passed
- [x] AC5: Two concurrent scopes yield `AiContended` regardless of shared
  `ActorKind` ->
  `runtime::coordinator::tests::contended_scopes_yield_ai_contended_same_and_different_actor` passed (exercises same- and different-actor pairs)
- [x] AC6: Snapshot capture never mutates the real index/staged/working state,
  reflects staged/unstaged/untracked/deleted state, excludes ignored files, and
  initializes an explicit valid empty index on unborn `HEAD` ->
  `runtime::git_snapshot::tests::{capture_preserves_real_index_and_working_tree_state, capture_excludes_ignored_files, capture_reflects_deletion_of_a_committed_file, capture_on_unborn_head_with_a_file_produces_a_valid_tree, capture_on_unborn_head_with_no_files_produces_an_empty_tree}` passed
- [x] AC7: A pinned `TreeId` stays resolvable after process exit, temp-index
  deletion, and `git gc --prune=now` / `git prune --expire=now` ->
  `runtime::git_snapshot::tests::{snapshot_survives_a_fresh_process_and_temp_index_deletion, pinned_snapshot_survives_git_gc_prune_now, pinned_snapshot_survives_git_prune_expire_now}` passed
- [x] AC8: Racing invocations — exactly one commits, the other reloads and
  recomputes with its own snapshot, no second snapshot taken ->
  `runtime::coordinator::tests::cas_conflict_reloads_and_recomputes_without_a_second_snapshot` passed
- [x] AC9: Same-worktree invocations serialize; different (incl. linked)
  worktrees do not, derive distinct `WorktreeId`s, persist independently into
  the caller-supplied DB ->
  `runtime::worktree_lock::tests::{a_second_acquirer_blocks_until_the_first_releases, acquire_times_out_with_a_distinct_matchable_error_when_still_held, distinct_worktree_paths_do_not_contend}` and `runtime::tests::linked_worktrees_have_independent_locks_and_worktree_ids` passed
- [x] AC10: A tainted / `needs_rebaseline` worktree is recovered exactly once
  with the triggering boundary's snapshot; rebaseline preserves live scopes,
  taint abandons them ->
  `runtime::coordinator::tests::{recovers_from_needs_rebaseline_preserving_live_scopes, recovers_from_snapshot_failure_taint_abandoning_live_scopes}` passed
- [x] AC11: A snapshot failure against an existing worktree durably persists a
  taint under bounded CAS retry (`persisted_taint` true only when `Applied`);
  no prior row makes no durable write; the taint decision reads fresh state
  after the failure ->
  `runtime::coordinator::tests::{snapshot_failure_taints_an_existing_worktree, snapshot_failure_taint_survives_a_losing_cas_and_commits_on_retry, snapshot_failure_taint_reports_not_persisted_after_retries_are_exhausted, snapshot_failure_before_any_baseline_makes_no_durable_write, snapshot_failure_taints_a_worktree_materialized_concurrently_during_capture}` passed
- [x] AC12: All concurrent first-time `get_or_create_checkout_id` callers
  converge on one checkout ID matching the on-disk file ->
  `checkout::tests::concurrent_first_time_callers_converge_on_one_checkout_id` and `runtime::tests::agent_trace_storage_and_coordinator_observe_the_same_checkout_id` passed
- [x] AC13: The canonical `checkout-id` path is always absent or holds one
  complete valid ID, incl. after interruption before rename; an orphaned temp
  file never blocks convergence ->
  `checkout::tests::{interruption_before_rename_leaves_the_canonical_path_absent, completed_rename_leaves_the_canonical_path_with_a_complete_id, an_orphaned_temp_file_does_not_block_convergence_on_the_canonical_id}` passed

### Failed checks and follow-ups

- None. No leftover debug flags, temporary files, intermediate artifacts, or
  local scaffolding found in the production runtime/checkout sources; working
  tree is clean.

### Residual risks

- Ref-pin storage growth is unbounded until the deferred
  `refs/sce/mutation-cursor/**` reconciliation pass ships, and the DB-unavailable
  case leaves no filesystem external-taint marker. Both are documented,
  intentionally deferred, and sequenced (`mutation-cursor-external-taint`,
  `mutation-cursor-ref-reconciliation`) as required runtime-completion work
  before any harness adapter becomes a production consumer of `coordinate()`.
- The coordinator is standalone and not wired into any hook, CLI command, or
  `diff_traces` — no production caller exercises it yet.
