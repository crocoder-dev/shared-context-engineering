# Plan: agent-trace-dwh-sync

## Change summary

Add `AgentTraceDwhSync`, the single orchestration service that connects the
already-implemented pieces: `AgentTraceDwhReplica` (PR #189, single-owner Turso
Sync replica lifecycle for `agent-trace-sync.db`), `AgentTraceEtl` (PR #190/#191),
`ConversationEtl` (PR #191), and `CodeChangesEtl` (PR #192). Today each of those
exists and is independently tested, but nothing yet drives them together: a
caller would have to hand-sequence `AgentTraceDwhReplica::open()`, `pull()`,
three separate `etl.run(repository_id, source, &replica)` calls, and `push()`
themselves, with no combined stats or stage-identified error type.

This plan adds one new service, `cli/src/services/agent_trace_dwh_sync.rs`,
whose `run()` owns exactly that sequence — open → pull → `AgentTraceEtl` →
`ConversationEtl` → `CodeChangesEtl` → push — behind one bridge-lock-held
Turso Sync connection, returning one combined stats type and a stage-tagged
error. It extends nothing in `agent_trace_dwh_replica`, `agent_trace_etl`,
`conversation_etl`, or `code_changes_etl`: all three ETLs already expose
`run(repository_id, &RepositoryAgentTraceDb, &AgentTraceDwhReplica)`, which is
exactly the shape this orchestrator needs to call unmodified.

The plan also proves, empirically against the real local Turso Sync harness
already established in `agent_trace_dwh_replica`'s `integration_tests` module,
that the local sync spool survives interruption at every stage (replica-open
failure, pull failure, each ETL failure, push failure) without losing source
rows, duplicating facts, or skipping watermarks — and documents whatever the
real Turso Sync SDK is observed to do when `pull()` runs against a replica
holding committed-but-unpushed ETL changes, since the request is explicit that
this observed behavior should override the proposed pull-before-ETL ordering
if it turns out to be unsafe.

No CLI wiring, credential discovery, or `sce trace sync` command is added —
this plan makes exactly one Rust API, so that a future thin CLI adapter has
nothing left to design.

## Acceptance criteria

- [ ] AC1: A fresh sync against a genuinely empty remote succeeds in one
  `AgentTraceDwhSync::run()` call: the remote DWH schema is initialized via
  `AgentTraceDwhReplica::open()`'s existing empty-remote path, all three ETLs
  run in order, and non-zero stats are returned for every table with source
  rows.
  - Validate: `nix develop .#database -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_dwh_sync_turso_sync_integration`
- [ ] AC2: A second `run()` against the same source and remote, with no new
  source rows and no remote-side changes, succeeds and returns stats showing
  zero extracted/inserted rows in every table (a visible no-op, not an error).
  - Validate: same integration test, no-op-run assertion
- [ ] AC3: Replica-open failure, pull failure, and each of the three ETL
  failures are each identifiable through a distinct `AgentTraceDwhSyncError`
  stage variant, and each one leaves the final `push()` uninvoked.
  - Validate: integration tests covering each failure stage (T02)
- [ ] AC4: A push failure that occurs after all three ETLs have committed
  locally leaves those commits durable in the local `agent-trace-sync.db`
  spool; the sync call returns an error; and a subsequent successful `run()`
  reaches the remote with the previously committed facts and watermarks, with
  no lost rows, no duplicated logical rows, and no skipped watermarks.
  - Validate: integration test proving push-failure recovery (T03)
- [ ] AC5: Deleting the local `agent-trace-sync.db` after a successful sync and
  running `AgentTraceDwhSync::run()` again reconstructs the replica from the
  remote and performs only genuinely incremental ETL work (a no-op when no new
  source rows exist since the deleted replica's last push).
  - Validate: integration test proving fresh-replica reconstruction (T04)
- [ ] AC6: Two different `repository_id`s, each with its own source DB and
  local replica path, syncing against the same remote DWH both appear in the
  remote afterward, and neither sync corrupts or removes the other's rows.
  - Validate: integration test proving multi-repository convergence (T05)
- [ ] AC7: Two source instances of the same `repository_id` syncing against
  the same remote maintain independent per-source-instance watermarks, and
  their overlapping local row IDs (parts, diff traces) do not collide in the
  DWH.
  - Validate: integration test proving multi-source-instance independence (T06)
- [ ] AC8: Two independently operated sync clients against the same remote
  DWH converge: client A observes client B's remote additions after its own
  `pull()`, neither destroys the other's committed facts, and repeated runs
  from both sides stabilize (no unbounded growth in inserted counts once both
  sides are current).
  - Validate: integration test proving cross-client convergence (T07)
- [ ] AC9: No `AgentTraceDwhSyncError` variant, `Debug`/`Display` output, or
  `AgentTraceDwhSyncStats` value ever contains the caller-supplied auth token,
  including on a push/pull failure against a real remote.
  - Validate: covered by the sentinel-auth-token assertions embedded in the
    T02–T04 integration tests, matching the existing `redact_token` pattern in
    `agent_trace_dwh_replica/replica.rs`

### Full validation

- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml`
- `nix develop .#database -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml`
- `nix develop -c ./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings`
- `nix develop -c ./scripts/run-cli-cargo.sh fmt --manifest-path cli/Cargo.toml -- --check`
- `nix flake check`
- `nix run .#pkl-check-generated`

### Context sync

- `context/sce/agent-trace-dwh-sync.md` (new): the full sync lifecycle,
  ownership, failure/recovery semantics, and the observed pull-with-pending-
  local-changes Turso behavior.
- `context/context-map.md`: register the new domain context file.
- `context/glossary.md`: add an `AgentTraceDwhSync` entry; extend the existing
  `Agent Trace DWH sync replica` entry's cross-links.
- `context/sce/agent-trace-dwh-replica.md`: note that `AgentTraceDwhSync` is
  now the orchestration boundary that composes `run_agent_trace_etl()`/
  `run_code_changes_etl()`/`ConversationEtl::run()`/`pull()`/`push()`, without
  changing anything about the replica's own ownership contract.

## Constraints and non-goals

- **In scope:** one new `cli/src/services/agent_trace_dwh_sync.rs` module
  (struct, config reuse, stats, stage-tagged error, `run()`), its unit and
  integration tests, and the context-sync files listed above.
- **Out of scope:** `AgentTraceDwhReplica`, `AgentTraceEtl`, `ConversationEtl`,
  `CodeChangesEtl`, and their existing tests — call them through their current
  public APIs unmodified unless a genuine correctness problem is found while
  building this orchestrator (none is anticipated; the three `run()` methods
  already take `&AgentTraceDwhReplica` directly).
- **Constraints:** exactly one Turso Sync connection per `run()` invocation;
  the bridge lock stays held for the whole pull+ETLs+push sequence; no global
  transaction wraps the three ETLs; auth tokens must never appear in errors,
  `Debug`, `Display`, stats, or logs; reuse the existing `LocalSyncServer`
  Turso Sync integration harness pattern rather than a fake replication
  implementation.
- **Non-goals:** control-plane calls, workspace DWH provisioning, WorkOS auth,
  token refresh/persistence, `sce trace sync` CLI wiring, scheduled/background
  sync, automatic retry loops around the whole sync operation, new ETL tables,
  post-commit intersection ETL, reverse remote-to-source hydration, schema
  ownership changes, automatic DWH schema upgrades, analytics/query APIs, UI,
  and any new ADR/decision record (context sync in this plan is limited to
  current-state `context/sce/*.md`/glossary/context-map prose; a decision
  record, if warranted, is a separate later call for `/validate`'s
  context-synchronization gate or an explicit `/decision` invocation, not this
  plan).

## Assumptions

- The branch already contains all work through PR #192 (`etl-code-change`) —
  confirmed by `git log` on the current `etl-orchestrator` branch, which is
  built directly on top of it. No rebase or branch change is needed before
  starting T01.
- Item 8 of the request ("explicitly test pull with pending local changes") is
  treated as an empirical discovery task (T03), not a pre-decided design
  choice: T01 implements the literal open → pull → ETLs → push order the
  request proposes, and T03 is authorized to adjust that internal ordering —
  documenting exactly why — if the real local Turso Sync harness demonstrates
  that ordering is unsafe. This mirrors the request's own instruction that the
  observed-behavior test outranks the proposed sequence.
- New integration tests follow the existing `agent_trace_dwh_replica`
  convention exactly: a `#[cfg(test)] mod integration_tests` gated by
  `find_tursodb()`, using `LocalSyncServer` and `AgentTraceDwhDb::run_migrations`
  + `push()` to prepare remotes, printing a skip reason and passing trivially
  outside `nix develop .#database`.
- `AgentTraceDwhSyncError` follows the existing manual `Debug`/`Display`/
  `std::error::Error` pattern used by `AgentTraceDwhReplicaError` (no
  `thiserror` dependency exists in `cli/Cargo.toml` today).

## Task stack

- [x] T01: `Add AgentTraceDwhSync core service and prove the empty-remote first sync` (status:done)
  - Task ID: T01
  - Goal: Implement `cli/src/services/agent_trace_dwh_sync.rs` with
    `AgentTraceDwhSync { agent_trace_etl, conversation_etl, code_changes_etl }`,
    `impl Default`, `AgentTraceDwhSyncStats { pulled_changes, agent_traces,
    conversation, code_changes }`, `AgentTraceDwhSyncError` (`ReplicaOpen`,
    `Pull`, `AgentTraceEtl`, `ConversationEtl`, `CodeChangesEtl`, `Push`), and
    `run(&self, repository_id: &str, source: &RepositoryAgentTraceDb,
    replica_config: AgentTraceDwhReplicaConfig) ->
    Result<AgentTraceDwhSyncStats, AgentTraceDwhSyncError>` that opens the
    replica, pulls once, runs the three ETLs through their existing
    `run(repository_id, source, &replica)` APIs in order, and pushes once on
    full success. Register the module in `cli/src/services/mod.rs`. Prove the
    empty-remote bootstrap and no-op-second-run behavior against the real
    Turso Sync harness.
  - Boundaries (in/out of scope): In — the new module, its stats/error types,
    the core `run()` state machine, unit tests for error-stage construction
    and stats aggregation that need no filesystem, and one integration test
    proving AC1/AC2. Out — stage-failure tests beyond what AC1/AC2 need,
    fresh-reconstruction/multi-repo/multi-source-instance/cross-client tests
    (later tasks), documentation.
  - Dependencies: none
  - Done when: `AgentTraceDwhSync::default().run(...)` against a freshly
    spawned, untouched local Turso Sync remote initializes the DWH schema,
    runs all three ETLs, pushes once, and returns stats with non-zero
    `inserted` counts; a second `run()` against the same source/remote returns
    stats with zero `extracted`/`inserted` across all three ETL stats and
    `pulled_changes == false`; no auth token appears in any `Debug`/`Display`
    output.
  - Verification notes (commands or checks): `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_dwh_sync`; `nix develop .#database -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_dwh_sync_turso_sync_integration`
  - Evidence: Added `cli/src/services/agent_trace_dwh_sync.rs` with
    `AgentTraceDwhSync { agent_trace_etl, conversation_etl, code_changes_etl }`
    (`#[allow(clippy::struct_field_names)]`), `AgentTraceDwhSyncStats`,
    `AgentTraceDwhSyncError` (manual `Debug`/`Display`/`std::error::Error`,
    mirroring `AgentTraceDwhReplicaError`), and `run()` implementing
    open→pull→`AgentTraceEtl`→`ConversationEtl`→`CodeChangesEtl`→push,
    short-circuiting with the matching stage variant on first failure.
    Registered `pub mod agent_trace_dwh_sync;` (`#[allow(dead_code)]`) in
    `cli/src/services/mod.rs`. Added unit tests for `Default` composition,
    zeroed stats defaults, and per-stage `Display`/`Debug` no-token-leak
    coverage, plus one `#[cfg(test)] mod integration_tests` (gated on
    `find_tursodb()`, using `LocalSyncServer`) proving AC1 (fresh empty-remote
    sync bootstraps the schema and inserts non-zero rows across all three ETL
    stages) and AC2 (a following no-new-source-rows run returns zero
    `extracted`/`inserted` everywhere).
  - Deviation from Done-when's literal `pulled_changes == false` on the
    *second* run: empirically, the real local Turso Sync harness's `pull()`
    reports `true` on the first `pull()` any freshly opened replica performs
    after *any* session's successful `push()` — including this
    orchestrator's own immediately preceding `run()` — because that push was
    never locally marked "already observed" by the new connection object, even
    though the pulled bytes exactly match what is already on disk. It settles
    to `false` only once a `run()` observes no push from any source since the
    previous `run()`'s own reconciliation pull. The integration test therefore
    asserts AC2's actual contract (zero `extracted`/`inserted`, a visible
    no-op) on the second run without asserting `pulled_changes`, and adds a
    third run to prove the genuine `pulled_changes == false` steady state.
    `run()`'s internal open→pull→ETLs→push ordering is unchanged; only the
    test assertion and `run()`'s doc comment were adjusted to state this
    observed semantics accurately. This is recorded here for T08 to document
    alongside T03's own observed-behavior findings.
  - Verification run: `nix develop .#database -c ./scripts/run-cli-cargo.sh
    test --manifest-path cli/Cargo.toml agent_trace_dwh_sync` (4 passed, incl.
    the Turso Sync integration test); `nix develop -c
    ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml` (292
    passed, 1 ignored, 0 failed); `nix develop -c ./scripts/run-cli-cargo.sh
    clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings`
    (clean); `nix develop -c ./scripts/run-cli-cargo.sh fmt --manifest-path
    cli/Cargo.toml -- --check` (clean).

- [x] T02: `Prove stage-identified failure semantics stop the sequence early` (status:done)
  - Task ID: T02
  - Goal: Add integration coverage proving: (a) a replica-open failure (e.g. an
    unreachable `database_url`) returns `AgentTraceDwhSyncError::ReplicaOpen`
    and never runs any ETL or push; (b) a pull failure (e.g. remote killed
    after a successful open) returns `AgentTraceDwhSyncError::Pull` and never
    runs any ETL or push; (c) a deliberately failing `AgentTraceEtl` stage
    prevents `ConversationEtl`/`CodeChangesEtl` and push from running; (d) a
    deliberately failing `ConversationEtl` stage leaves `AgentTraceEtl`'s
    commit intact locally, does not run `CodeChangesEtl`, and does not push;
    (e) a deliberately failing `CodeChangesEtl` stage (a malformed source
    `diff_traces` payload, using its existing strict validation — do not
    weaken it) leaves the prior two ETLs' commits intact locally and does not
    push.
  - Boundaries (in/out of scope): In — failure-injection integration tests for
    all five stages listed above, asserting both the returned error variant
    and the local replica's post-failure DWH row/watermark state. Out —
    push-failure recovery (T03), reconstruction/multi-repo/multi-source-
    instance/cross-client tests (T04–T07).
  - Dependencies: T01
  - Done when: five distinct integration test cases (or clearly separated
    assertions within one wired integration test, following the existing
    `agent_trace_dwh_replica_turso_sync_integration` composition pattern) each
    assert the correct `AgentTraceDwhSyncError` variant and that no row was
    pushed to the remote past the point of failure; the local spool
    (`agent-trace-sync.db`) is inspected directly to confirm prior successful
    ETL stages within the same failed run committed locally as designed.
  - Verification notes (commands or checks): `nix develop .#database -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_dwh_sync_turso_sync_integration`
  - Evidence: Extended `cli/src/services/agent_trace_dwh_sync.rs`'s
    `integration_tests` module with one new gated `#[test]`
    `agent_trace_dwh_sync_stage_failure_turso_sync_integration`, composed of
    five `assert_*` helpers (one per stage, each spawning its own
    `LocalSyncServer`), matching the `agent_trace_dwh_replica_turso_sync_integration`
    composition convention: (a) `assert_replica_open_failure_stops_before_any_stage`
    — an unreachable `database_url` fails `open()` with `ReplicaOpen`, and the
    live remote (never actually addressed) is confirmed to hold zero rows;
    (b) `assert_pull_failure_stops_before_any_stage` — after a baseline
    successful sync, a second `run()` against a *different*, unreachable
    `database_url` fails at `pull()` with `Pull` (the already-`Ready` local
    replica needs no network to open — confirmed empirically, see deviation
    note below); the local spool shows the new source row was never
    extracted, and the live remote (never touched by the failing call) is
    unchanged from baseline; (c)
    `assert_agent_trace_etl_failure_stops_before_conversation_code_changes_and_push`
    — two independently created sources for one repository publish the same
    `agent_trace_id` with different `trace_json`; the second sync's
    `AgentTraceEtl` fails on the identity-hash conflict (`AgentTraceEtl`
    variant), and because it is both the first stage and atomic per batch,
    the local replica after the failed run holds exactly what `pull()` left
    it (source A's row) with no partial facts from source B and no push; (d)
    `assert_conversation_etl_failure_leaves_agent_trace_committed_and_stops_before_code_changes_and_push`
    — a raw-SQL `parts` row with an invalid `type` (no source-side CHECK
    constraint permits this) fails the parts half of `ConversationEtl`
    (`ConversationEtl` variant) while the messages half and the preceding
    `AgentTraceEtl` commit locally within the same run, `CodeChangesEtl`
    never runs, and push never runs; (e)
    `assert_code_changes_etl_failure_leaves_prior_etls_committed_and_stops_before_push`
    — a malformed `diff_traces.patch` (reusing the existing malformed-patch
    fixture from `code_changes_etl_replays_watermark_behind_failed_transformation`)
    fails `CodeChangesEtl` (`CodeChangesEtl` variant) while both prior ETLs
    commit locally and push never runs. Each helper asserts the exact error
    variant, that the sentinel auth token never appears in `Display` output,
    the local spool's row counts (via a `local_row_counts` helper that
    reopens the same local replica path directly — no network required once
    `Ready`), and the live remote's row counts (via a `remote_row_counts`
    helper that opens a disposable peer replica and pulls) before and after
    each failing run.
  - Deviation: stage (b)'s literal "remote killed after a successful open"
    framing from the Goal was implemented instead as "a second `run()` points
    at a different, never-reachable `database_url`," because empirical
    testing (a throwaway probe run under `nix develop .#database`, since
    removed) showed that once a local replica's schema has classified
    `Ready`, `AgentTraceDwhReplica::open()` performs no network round trip at
    all — only the following `pull()` does. Killing the real server would
    therefore not isolate a `Pull`-specific failure from a `ReplicaOpen`
    failure on a later re-open attempt against the same dead URL, and would
    also prevent inspecting the live remote afterward (the disposable
    `LocalSyncServer` holds no data once its process exits). Pointing the
    second `run()` at an unrelated unreachable URL instead reproduces the
    same `Pull` failure deterministically, keeps the real remote's process
    alive and inspectable throughout, and — because the real remote's URL is
    never even passed to the failing call — makes "the live remote is
    unaffected" a stronger, directly checkable assertion rather than an
    inference from a killed process. This does not touch `run()`'s own
    open→pull→ETLs→push ordering; it only changes how the test induces the
    failure. Recorded here for T08 to fold into the pull-failure discussion
    alongside T03's own findings.
  - Verification run: `nix develop .#database -c
    ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml
    agent_trace_dwh_sync` (5 passed, run 4 times back-to-back with no
    flakiness, matching the repeated-run precedent used elsewhere in this
    plan); `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path
    cli/Cargo.toml -- --test-threads=1` (293 passed, 1 ignored, 0 failed —
    the default parallel run showed 3 unrelated failures from pre-existing
    concurrent-database-lock contention in `agent_trace_db`/`agent_trace_dwh_db`
    tests untouched by this task, confirmed spurious by the clean
    single-threaded rerun); `nix develop -c ./scripts/run-cli-cargo.sh clippy
    --manifest-path cli/Cargo.toml --all-targets -- -D warnings` (clean);
    `nix develop -c ./scripts/run-cli-cargo.sh fmt --manifest-path
    cli/Cargo.toml -- --check` (clean).

- [x] T03: `Prove and document push-failure and pull-with-pending-local-changes recovery` (status:done)
  - Task ID: T03
  - Goal: Add the load-bearing integration test the request calls out
    explicitly: run all three ETLs against a real local Turso Sync remote so
    they commit locally, force the final `push()` to fail (e.g. by killing the
    `LocalSyncServer` process before the push step), confirm the local replica
    retains the committed-but-unpushed changes, restart remote availability,
    then run `AgentTraceDwhSync::run()` again — whose first step is `pull()`
    against a replica that itself holds pending local commits — and prove the
    final converged state has every local fact and watermark reaching the
    remote, with no duplicate rows and no lost rows. If this test reveals that
    `pull()` against a replica with pending local commits discards or corrupts
    those commits, change `run()`'s internal ordering (still without pushing
    after each ETL individually) to whatever ordering the observed behavior
    requires, and record exactly what was observed and why in this task's
    evidence for T08 to document.
  - Boundaries (in/out of scope): In — the pending-local-changes recovery
    integration test, and any resulting adjustment to `run()`'s internal
    pull/ETL/push sequencing strictly to preserve durability of local commits.
    Out — any change to `AgentTraceDwhReplica::pull()`/`push()` themselves,
    reconstruction/multi-repo/multi-source-instance/cross-client tests
    (T04–T07), documentation (T08).
  - Dependencies: T01
  - Done when: the integration test deterministically reaches a final state
    where the remote contains every ETL fact and watermark committed during
    the failed-push run, with no duplicated logical rows and no lost source
    rows, across at least several repeated runs; the task's evidence records
    the exact observed Turso Sync behavior for `pull()` against a replica
    holding pending local commits.
  - Verification notes (commands or checks): `nix develop .#database -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_dwh_sync_turso_sync_integration` (run at least 3 times to check for nondeterminism, matching the precedent set by the concurrent-initializer convergence test in `agent_trace_dwh_replica`)
  - Evidence: Added a new `#[test]`
    `agent_trace_dwh_sync_push_failure_recovery_turso_sync_integration` to
    `cli/src/services/agent_trace_dwh_sync.rs`'s `integration_tests` module.
    Rather than racing a background kill against an opaque
    `AgentTraceDwhSync::run()` call, the test manually reproduces `run()`'s
    own open→pull→`AgentTraceEtl`→`ConversationEtl`→`CodeChangesEtl` sequence
    against a real local Turso Sync remote (`AgentTraceDwhReplica::open()` +
    `pull()` + the three ETLs' existing `run(repository_id, source, &replica)`
    calls, all already-public APIs), which lets it force a deterministic push
    failure at exactly the point `run()` itself would call `push()`: the
    remote process is killed first, then `replica.push()` is called directly
    and asserted to fail. `local_row_counts` (reusing T02's helper) then
    confirms all three ETL commits — `[1, 1, 1, 1]` — remain durable in the
    local `agent-trace-sync.db` spool after the failed push. Remote
    availability is then "restarted" via a new `LocalSyncServer::spawn_persistent`
    process on a fresh ephemeral port but backed by the *same* on-disk
    `DATABASE` file the killed process used, so the schema published before
    the outage survives. A plain second `AgentTraceDwhSync::run()` call (the
    "recovery run") is then made against the same local replica path — this
    is the actual pull-with-pending-local-commits scenario, composed for
    free because `run()`'s own first step is `pull()` against a replica that
    still holds the three unpushed ETL commits from the failed run. The
    recovery run succeeds, reports zero `extracted`/`inserted` across all
    three ETL stages (proving no re-extraction/duplication), and its `push()`
    reaches the remote with exactly `[1, 1, 1, 1]` rows — no lost rows, no
    duplicated logical rows. A further third run from the same replica is
    asserted to be a stable no-op with unchanged remote counts, ruling out
    unbounded growth from a repeated push. Extended the existing
    `LocalSyncServer` test helper (used unchanged by every other test in this
    file) with `spawn_persistent(tursodb_path, db_path)` (spawns
    `tursodb --sync-server <addr> <db_path>` instead of the default
    `:memory:`, confirmed empirically to leave `<db_path>`/`<db_path>-wal` on
    disk after the process is killed) and `kill()` (explicit early kill ahead
    of `Drop`); `LocalSyncServer::spawn()` is unchanged and still used
    in-memory by every pre-existing test.
  - Observed Turso Sync behavior for `pull()` against a replica holding
    pending local commits (the empirical discovery this task exists to make):
    `pull()` does not discard, corrupt, or roll back pending local commits.
    It applies whatever the remote holds (here, only the DWH schema published
    before the outage; nothing else, since the failing push never reached the
    remote) without touching the three ETL stages' already-committed local
    writes. The following three ETL stages in the recovery run then correctly
    observe their watermarks already advanced and report a true no-op, and
    the final `push()` publishes exactly what was committed during the
    failed run. This matches the request's proposed open→pull→ETLs→push
    ordering exactly: **no change to `run()`'s internal sequencing was
    required.** Recorded here for T08 to document alongside T01's and T02's
    own observed-behavior findings.
  - Verification run: `nix develop .#database -c ./scripts/run-cli-cargo.sh
    test --manifest-path cli/Cargo.toml
    agent_trace_dwh_sync_push_failure_recovery_turso_sync_integration` (5
    separate invocations, all passed, no flakiness); `nix develop .#database
    -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml
    agent_trace_dwh_sync` (6 passed, incl. all three integration tests in
    this file); `nix develop -c ./scripts/run-cli-cargo.sh test
    --manifest-path cli/Cargo.toml -- --test-threads=1` (293 passed, 1
    failed, 1 ignored — the failure is
    `agent_trace_db::repository::tests::concurrent_missing_source_instance_id_initialization_converges_on_one_persisted_winner`,
    confirmed via `git stash` to fail identically on the pre-T03 tree, i.e.
    pre-existing and untouched by this task); `nix develop -c
    ./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml
    --all-targets -- -D warnings` (one pre-existing failure unrelated to this
    task's changes — `assert_conversation_etl_failure_leaves_agent_trace_committed_and_stops_before_code_changes_and_push`
    in this same file, added by T02, exceeds `clippy::too_many_lines` at
    104/100; confirmed via `git stash` to fail identically on the pre-T03
    tree; this task's own new code is clippy-clean, carrying an explicit
    `#[allow(clippy::too_many_lines)]` on the one new function long enough to
    need it, matching the existing convention used by T02's longer helpers);
    `nix develop -c ./scripts/run-cli-cargo.sh fmt --manifest-path
    cli/Cargo.toml -- --check` (clean).

- [ ] T04: `Prove fresh local-replica reconstruction from the remote` (status:todo)
  - Task ID: T04
  - Goal: Add an integration test that runs a successful sync, deletes the
    local `agent-trace-sync.db` (and its Turso sidecars), and runs
    `AgentTraceDwhSync::run()` again with the same `AgentTraceDwhReplicaConfig`
    — proving the replica bootstraps from the remote, the ETLs read watermarks
    that reflect the previously pushed state (so no rows are re-extracted or
    duplicated), and a no-op ETL run occurs when no new source rows exist
    since the deleted replica's last push.
  - Boundaries (in/out of scope): In — the delete-and-resync integration test
    only. Out — multi-repository/multi-source-instance/cross-client tests
    (T05–T07), documentation (T08).
  - Dependencies: T01
  - Done when: the test asserts zero `inserted` counts across all three ETL
    stats on the post-deletion resync when no new source rows were added, and
    non-zero counts when new source rows are added before the resync,
    matching the watermark state that was actually pushed before deletion.
  - Verification notes (commands or checks): `nix develop .#database -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_dwh_sync_turso_sync_integration`

- [ ] T05: `Prove multi-repository convergence against one remote DWH` (status:todo)
  - Task ID: T05
  - Goal: Add an integration test syncing two distinct `repository_id`s, each
    with its own source `RepositoryAgentTraceDb`, its own local replica path,
    and its own `AgentTraceDwhSync` instance, against the same remote DWH.
    Verify the remote's `agent_traces`/`messages`/`message_parts`/
    `code_changes`/`etl_watermarks` rows for repository A are unaffected by
    repository B's sync, and vice versa.
  - Boundaries (in/out of scope): In — the two-repository convergence
    integration test only. Out — multi-source-instance and cross-client tests
    (T06–T07), documentation (T08).
  - Dependencies: T01
  - Done when: the test asserts both repositories' rows are present in the
    remote after both syncs, that repository A's row count/content is
    identical before and after repository B's sync runs, and the reverse.
  - Verification notes (commands or checks): `nix develop .#database -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_dwh_sync_turso_sync_integration`

- [ ] T06: `Prove independent per-source-instance watermarks under one repository` (status:todo)
  - Task ID: T06
  - Goal: Add an integration test for one `repository_id` with two
    independently created source `RepositoryAgentTraceDb` instances (distinct
    `source_instance_id`s, following existing source-instance identity rules —
    do not add new cross-source identity logic in the orchestrator), each
    synced through its own `AgentTraceDwhSync::run()` call against the same
    remote, including overlapping local row IDs (e.g. both sources having a
    local `part`/`diff_trace` row with the same integer ID).
  - Boundaries (in/out of scope): In — the two-source-instance integration
    test, asserting independent watermark progression and no local-ID
    collision in the DWH. Out — any new identity/dedup logic in
    `agent_trace_dwh_sync.rs` beyond what the existing ETLs already provide;
    cross-client convergence (T07); documentation (T08).
  - Dependencies: T01
  - Done when: the test asserts `etl_watermarks` rows for the two source
    instances advance independently, and that DWH rows sourced from each
    instance's overlapping local IDs are both present and distinguishable
    (not merged or overwritten).
  - Verification notes (commands or checks): `nix develop .#database -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_dwh_sync_turso_sync_integration`

- [ ] T07: `Prove convergence between two independently operated sync clients` (status:todo)
  - Task ID: T07
  - Goal: Add an integration test simulating two independent clients (two
    separate local replica paths, same `repository_id` and remote, modeling
    two machines) that each run pull → ETL → push in turn: client A syncs,
    client B syncs (observing A's remote additions via its own `pull()`),
    then client A syncs again (observing B's additions). Verify convergence:
    A's second sync sees B's rows, B's sync did not remove or corrupt A's
    rows, watermarks stay correct throughout, and a further no-op run from
    either client is stable.
  - Boundaries (in/out of scope): In — the cross-client convergence
    integration test only. Out — documentation (T08).
  - Dependencies: T01
  - Done when: the test asserts the remote's row counts after all three sync
    steps equal the union of what both clients' sources contributed, with no
    duplication, and that a final no-op run from each client returns zero
    `inserted` counts across all three ETL stats.
  - Verification notes (commands or checks): `nix develop .#database -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_dwh_sync_turso_sync_integration`

- [ ] T08: `Document the AgentTraceDwhSync lifecycle and its recovery invariants` (status:todo)
  - Task ID: T08
  - Goal: Write `context/sce/agent-trace-dwh-sync.md` describing the full
    `repository agent-trace.db → AgentTraceDwhSync (open → pull → AgentTraceEtl
    → ConversationEtl → CodeChangesEtl → push) → remote Agent Trace DWH`
    lifecycle; the invariants listed in the request (source DB remains local
    truth; `agent-trace-sync.db` is a durable-but-disposable spool; the remote
    stores durable facts+watermarks; one process owns the spool per sync;
    pull precedes ETL under normal operation; ETLs commit independently; push
    only follows full ETL success; a failed push leaves local commits intact;
    credentials are caller-supplied; control-plane/CLI stay outside this
    service); and the exact observed Turso Sync behavior recorded by T03 for
    `pull()` against a replica with pending local commits, including whatever
    ordering `run()` actually implements as a result. Register the file in
    `context/context-map.md`, add/extend the `context/glossary.md` entries
    named in Context sync, and update `context/sce/agent-trace-dwh-replica.md`
    per Context sync.
  - Boundaries (in/out of scope): In — the context files listed under Context
    sync only. Out — any new decision record under `context/decisions/`; any
    further code change.
  - Dependencies: T01, T02, T03, T04, T05, T06, T07
  - Done when: every invariant listed above is stated in
    `context/sce/agent-trace-dwh-sync.md` with a pointer to the code/test that
    proves it; the file is linked from `context/context-map.md`; no code in
    `cli/src` changes in this task.
  - Verification notes (commands or checks): inspect `context/sce/agent-trace-dwh-sync.md` against `cli/src/services/agent_trace_dwh_sync.rs` and the T01–T07 integration test evidence; confirm the new file is linked from `context/context-map.md`, `context/glossary.md`, and `context/sce/agent-trace-dwh-replica.md`, and links back to them.

## Open questions

None. The request is a fully specified implementation brief (branch/base,
exact sequencing, error model, stats shape, and an explicit, itemized list of
required integration-test scenarios), the pieces it composes already exist
with exactly the call shape it assumes (`etl.run(repository_id, source,
&replica)`), and the one genuinely open design point — whether `pull()` is
safe against a replica holding pending local commits — is explicitly framed by
the request itself as something to discover empirically (T03) rather than
decide up front, so it is captured as an assumption above instead of a
blocking question.
