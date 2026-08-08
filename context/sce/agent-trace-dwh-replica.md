# Agent Trace DWH Turso Sync Replica

`AgentTraceDwhReplica` is the sole owner of a Turso Sync connection to a repository's `agent-trace-sync.db` — a disposable, single-owner local database distinct from both the multiprocess-WAL source `agent-trace.db` (see [agent-trace-db.md](agent-trace-db.md)) and the `Agent Trace DWH`'s own explicit-path adapter (see [agent-trace-dwh-db.md](agent-trace-dwh-db.md)). It is the boundary used by the CLI-independent `AgentTraceEtl`, `ConversationEtl`, and `CodeChangesEtl` bridges for local fact/watermark loading; ETL never performs pull/push, credential discovery/persistence, or background sync.

## Ownership and lock-before-open

`cli/src/services/agent_trace_dwh_replica/replica.rs` defines `AgentTraceDwhReplica::open(config: AgentTraceDwhReplicaConfig)`, where `AgentTraceDwhReplicaConfig { local_path, database_url, auth_token }` are all caller-supplied explicit values — the replica never discovers, stores, or persists credentials itself. `open()`:

1. Derives the sibling `.bridge-lock` path from `local_path` (the same suffix convention as `agent_trace_dwh_bridge_lock_path_for_repository`, see [../cli/default-path-catalog.md](../cli/default-path-catalog.md)) and acquires a `BridgeLock` **before** any Turso access. A concurrently held lock fails the whole call with `AgentTraceDwhReplicaError::Lock` before a Turso Sync builder, the local file, or the network is ever touched.
2. Opens the local file through `turso::sync::Builder::new_remote(local_path).with_remote_url(..).with_auth_token(..)`, never calling `.experimental_multiprocess_wal(true)` — that flag is reserved for the source capture database this replica never opens.
3. Wraps the resulting connection into `AgentTraceDwhDb` via a narrow `TursoDb::from_connection(conn, runtime)` seam (see [shared-turso-db.md](shared-turso-db.md)) and classifies its schema state via `AgentTraceDwhDb::classify_schema_state()` (see [agent-trace-dwh-db.md](agent-trace-dwh-db.md)). A `Ready` schema is left untouched. A genuinely `Empty` schema is initialized locally with `AgentTraceDwhDb::run_migrations()` and published with a single `push()`, narrowly recovering from a push conflict with one best-effort `pull()` plus a readiness re-verification (the *original* push failure is returned unless that re-verification now reports ready, in which case another initializer is treated as having won the race). An `Incompatible` schema — an unrelated schema, a partial DWH schema, or a migration ledger with unexpected entries — fails the whole call as `AgentTraceDwhReplicaError::IncompatibleSchema`, without repairing or partially completing it.

The returned `AgentTraceDwhReplica` owns both the `BridgeLock` and the `AgentTraceDwhDb` connection for its lifetime; dropping it releases the lock. `AgentTraceDwhReplica::db()` exposes lock-lifetime-bound SQL access through the same `AgentTraceDwhDb` the replica opened — no second connection is created.

## Pull and push

`pull()` and `push()` wrap `turso::sync::Database::pull`/`push`, driven through `TursoDb::block_on(future)` — a seam that lets a companion async handle (the Turso Sync `Database`) run on the same current-thread runtime that owns the SQL connection, without the replica owning a second runtime. `pull()` returns whether any remote changes were applied; it is a no-op for an already up-to-date replica. Deleting the local replica file and its Turso sidecars, then reopening at the same path, reconstructs all previously published remote data via `open()`'s normal bootstrap path — the replica carries no state that isn't recoverable from the remote.

## Crash and recovery semantics

The `Empty` branch of `open()` has three distinct interruption points, because the local schema and the remote publication are not committed atomically:

- **Interrupted before `run_migrations()` completes.** Nothing has been written locally or remotely. Losing the process also releases the `BridgeLock` at the OS level, so a fresh `open()` against the same `local_path` reclassifies the (still-untouched) local file as `Empty` again and restarts initialization from scratch.
- **Interrupted after `run_migrations()` but before `push()` starts or completes.** The local file now carries a fully-applied local schema that was never published. A fresh `open()` against the *same* `local_path` reopens that existing file directly rather than bootstrapping, so `classify_schema_state()` observes the already-applied local schema and classifies it `Ready` — `open()` takes the unchanged-on-open branch and does not retry the publish, leaving the remote `Empty` until some other opener publishes to it. Deleting the local file (the replica's normal disposability guarantee) forces the next `open()` at that path to bootstrap fresh from the still-`Empty` remote and re-run initialization.
- **Interrupted during or immediately after `push()`, with an ambiguous outcome.** This is the case `initialize_empty_schema`'s narrow recovery path exists for: a `push()` error (crash-adjacent or genuine SDK/network failure) triggers exactly one best-effort `pull()` plus `ensure_dwh_schema_ready()`. If that reports `Ready`, the push must have landed (by this opener or a racing one) and initialization is treated as successful; otherwise the *original* `push()` error is returned unchanged — never a swallowed or generic error.

## Concurrent-initializer behavior

Multiple local replicas may independently observe the same remote as `Empty` and race to initialize it — there is no external coordination beyond the recovery path above. `assert_concurrent_first_initializers_converge` in `replica.rs`'s integration harness races six distinct local replica paths against one freshly spawned, untouched remote, released simultaneously via a `Barrier`. Every racer is required to either win the race outright or recover through the one-`pull()`-and-re-verify path; the remote converges on exactly one valid schema and migration ledger (no duplicated `__sce_migrations` rows), and a third, entirely fresh replica opened afterward observes that same converged schema as `Ready`.

Observed real-SDK behavior (recorded across repeated runs, including with temporary instrumentation on the push-failure branch): the pinned local `tursodb --sync-server` never surfaced an actual push conflict/error even at six-way simultaneous concurrency — every racer's `run_migrations()` + `push()` succeeded on its own first attempt. The one-`pull()`-and-re-verify recovery path therefore remains defensively in place but has not been exercised by an observed real conflict against this harness; it is not proven dead, only unexercised. This is a `tursodb --sync-server`-specific observation and should not be assumed to hold against Turso Cloud or another remote implementation.

## Credential-safe errors

`AgentTraceDwhReplicaError` (`Lock`, `Runtime`, `Open`, `SchemaInspection`, `IncompatibleSchema`, `SchemaInitialization`, `SchemaPublication`, `ReadinessVerification`, `Pull`, `Push`) never includes the caller-supplied auth token. Every message that could echo SDK/network error text is passed through a `redact_token` helper that replaces every occurrence of the token with `<redacted>` before the error is constructed.

## Observed Turso Sync SDK behavior

- `turso::sync::Builder::new_remote(path)`'s `path` argument is the *local* file path, not a remote identifier — the name is unrelated to `with_remote_url`.
- The local file's parent directory must exist before `build()`; the sync builder does not create it. `AgentTraceDwhReplica::open` relies on `BridgeLock::acquire`'s existing directory-creation behavior for this.
- `bootstrap_if_empty` defaults to `true`, so a missing/empty local file is bootstrapped from the remote automatically inside `build()`— there is no separate explicit bootstrap call.
- `turso::sync::Database` exposes only async `pull()`/`push()`/`connect()`/`stats()`/`checkpoint()`; there is no synchronous wrapper in the SDK itself.
- The `sync` Cargo feature (`turso = { version = "0.7.0", features = ["sync"] }`) is additive and required no Turso version change from the pinned `0.7.0`.
- The local `tursodb` binary available via `nix develop .#database` supports `--sync-server <addr>` and speaks the same `/v2/pipeline` HTTP protocol as Turso Cloud, so it serves as a disposable local remote for integration tests without any external network dependency.

## ETL separation

`AgentTraceDwhReplica::run_agent_trace_etl()` accepts an already-open repository source and an `AgentTraceEtl` configuration, then delegates while retaining the replica's bridge-lock ownership. `run_code_changes_etl()` provides the same boundary for `CodeChangesEtl` and the source `diff_traces` to `code_changes` bridge. These ETLs verify source metadata, extract short bounded read snapshots, and commit facts plus per-lineage watermarks locally. Pull/push remain explicit operations owned by the caller and are never invoked by ETL. Code-change transformation preserves `session_id` for conversation queries but does not infer `message_id` attribution; see [code-changes-etl.md](code-changes-etl.md).

Control-plane/provisioning calls, OAuth or credential discovery/persistence, token rotation, CLI/lifecycle/setup/doctor/hook wiring, automatic/background sync, archive/retention behavior, and partial sync remain out of scope for this boundary.

See also: [agent-trace-dwh-db.md](agent-trace-dwh-db.md), [agent-trace-db.md](agent-trace-db.md), [shared-turso-db.md](shared-turso-db.md), [../cli/default-path-catalog.md](../cli/default-path-catalog.md), [../glossary.md](../glossary.md), [../context-map.md](../context-map.md), and the accepted decision at [../decisions/2026-08-08-agent-trace-dwh-empty-remote-auto-initialization.md](../decisions/2026-08-08-agent-trace-dwh-empty-remote-auto-initialization.md) (superseding [../decisions/2026-08-08-agent-trace-dwh-turso-sync-replica-ownership.md](../decisions/2026-08-08-agent-trace-dwh-turso-sync-replica-ownership.md))
