# Agent Trace DWH Turso Sync Replica

`AgentTraceDwhReplica` is the sole owner of a Turso Sync connection to a repository's `agent-trace-sync.db` — a disposable, single-owner local database distinct from both the multiprocess-WAL source `agent-trace.db` (see [agent-trace-db.md](agent-trace-db.md)) and the `Agent Trace DWH`'s own explicit-path adapter (see [agent-trace-dwh-db.md](agent-trace-dwh-db.md)). It is the boundary a future ETL bridge process will use to pull from and push to the remote Agent Trace DWH; this repository does not yet run ETL, credential discovery/persistence, or any background sync against it.

## Ownership and lock-before-open

`cli/src/services/agent_trace_dwh_replica/replica.rs` defines `AgentTraceDwhReplica::open(config: AgentTraceDwhReplicaConfig)`, where `AgentTraceDwhReplicaConfig { local_path, database_url, auth_token }` are all caller-supplied explicit values — the replica never discovers, stores, or persists credentials itself. `open()`:

1. Derives the sibling `.bridge-lock` path from `local_path` (the same suffix convention as `agent_trace_dwh_bridge_lock_path_for_repository`, see [../cli/default-path-catalog.md](../cli/default-path-catalog.md)) and acquires a `BridgeLock` **before** any Turso access. A concurrently held lock fails the whole call with `AgentTraceDwhReplicaError::Lock` before a Turso Sync builder, the local file, or the network is ever touched.
2. Opens the local file through `turso::sync::Builder::new_remote(local_path).with_remote_url(..).with_auth_token(..)`, never calling `.experimental_multiprocess_wal(true)` — that flag is reserved for the source capture database this replica never opens.
3. Wraps the resulting connection into `AgentTraceDwhDb` via a narrow `TursoDb::from_connection(conn, runtime)` seam (see [shared-turso-db.md](shared-turso-db.md)) and calls `AgentTraceDwhDb::ensure_dwh_schema_ready()`. This check is non-mutating: a missing or incompatible remote schema fails as `AgentTraceDwhReplicaError::SchemaNotReady` without this code ever running local DWH migrations or provisioning a competing schema.

The returned `AgentTraceDwhReplica` owns both the `BridgeLock` and the `AgentTraceDwhDb` connection for its lifetime; dropping it releases the lock. `AgentTraceDwhReplica::db()` exposes lock-lifetime-bound SQL access through the same `AgentTraceDwhDb` the replica opened — no second connection is created.

## Pull and push

`pull()` and `push()` wrap `turso::sync::Database::pull`/`push`, driven through `TursoDb::block_on(future)` — a seam that lets a companion async handle (the Turso Sync `Database`) run on the same current-thread runtime that owns the SQL connection, without the replica owning a second runtime. `pull()` returns whether any remote changes were applied; it is a no-op for an already up-to-date replica. Deleting the local replica file and its Turso sidecars, then reopening at the same path, reconstructs all previously published remote data via `open()`'s normal bootstrap path — the replica carries no state that isn't recoverable from the remote.

## Credential-safe errors

`AgentTraceDwhReplicaError` (`Lock`, `Runtime`, `Open`, `SchemaNotReady`, `Pull`, `Push`) never includes the caller-supplied auth token. Every message that could echo SDK/network error text is passed through a `redact_token` helper that replaces every occurrence of the token with `<redacted>` before the error is constructed.

## Observed Turso Sync SDK behavior

- `turso::sync::Builder::new_remote(path)`'s `path` argument is the *local* file path, not a remote identifier — the name is unrelated to `with_remote_url`.
- The local file's parent directory must exist before `build()`; the sync builder does not create it. `AgentTraceDwhReplica::open` relies on `BridgeLock::acquire`'s existing directory-creation behavior for this.
- `bootstrap_if_empty` defaults to `true`, so a missing/empty local file is bootstrapped from the remote automatically inside `build()`— there is no separate explicit bootstrap call.
- `turso::sync::Database` exposes only async `pull()`/`push()`/`connect()`/`stats()`/`checkpoint()`; there is no synchronous wrapper in the SDK itself.
- The `sync` Cargo feature (`turso = { version = "0.7.0", features = ["sync"] }`) is additive and required no Turso version change from the pinned `0.7.0`.
- The local `tursodb` binary available via `nix develop .#database` supports `--sync-server <addr>` and speaks the same `/v2/pipeline` HTTP protocol as Turso Cloud, so it serves as a disposable local remote for integration tests without any external network dependency.

## Not yet implemented

ETL extraction/transformation/hashing, watermark reads or advancement, source-busy retry, control-plane/provisioning calls, OAuth or credential discovery/persistence, token rotation, any CLI command, lifecycle/setup/doctor/hook wiring, automatic/background sync, archive/retention behavior, and partial sync all remain out of scope for this boundary.

See also: [agent-trace-dwh-db.md](agent-trace-dwh-db.md), [agent-trace-db.md](agent-trace-db.md), [shared-turso-db.md](shared-turso-db.md), [../cli/default-path-catalog.md](../cli/default-path-catalog.md), [../glossary.md](../glossary.md), [../context-map.md](../context-map.md), and the accepted decision at [../decisions/2026-08-08-agent-trace-dwh-turso-sync-replica-ownership.md](../decisions/2026-08-08-agent-trace-dwh-turso-sync-replica-ownership.md)
