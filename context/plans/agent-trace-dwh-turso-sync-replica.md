# Plan: agent-trace-dwh-turso-sync-replica

## Change summary

Add the repository-scoped `agent-trace-sync.db` storage boundary between the existing multiprocess-WAL `agent-trace.db` capture source and the remote Agent Trace DWH. A dedicated `AgentTraceDwhReplica` will acquire a non-blocking process lock, open the local file through Turso Sync with caller-supplied remote credentials, rely on normal remote bootstrap, verify the existing DWH migration contract without provisioning it, expose pull/push, and keep ordinary DWH SQL access within the lock owner's lifetime.

This extends the PR2 `AgentTraceDwhDb` boundary without changing source capture, adding ETL, acquiring credentials, or introducing CLI/background synchronization behavior. It also replaces PR2's temporary documentation assumption that no canonical local DWH replica path exists.

## Acceptance criteria

- [ ] AC1: Repository ID `abc` resolves the replica and bridge-lock paths to `<state-root>/sce/repos/abc/agent-trace-sync.db` and `<state-root>/sce/repos/abc/agent-trace-sync.db.bridge-lock`, distinct from `agent-trace.db` and outside the checkout.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_dwh_replica_path`
- [ ] AC2: An exclusive non-blocking OS file lock permits exactly one owner of a replica path, rejects a concurrent owner with actionable guidance, becomes acquirable after owner drop or process death, leaves the lock file on disk, and does not block normal source `agent-trace.db` operations.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_dwh_replica_lock`
- [ ] AC3: `AgentTraceDwhReplica` accepts only an explicit local path, database URL, and auth token from its caller; acquires the bridge lock before opening Turso Sync; does not enable multiprocess WAL; exposes lock-lifetime-bound `AgentTraceDwhDb` access; and reports lock/open/schema/pull/push failures without including the token.
  - Validate: targeted replica API/error tests plus inspection of `cli/src/services/agent_trace_dwh_replica/` and `cli/Cargo.toml`
- [ ] AC4: Opening a missing local replica uses Turso Sync's supported remote bootstrap and succeeds only when the bootstrapped database passes `AgentTraceDwhDb::ensure_dwh_schema_ready()`; missing or incompatible DWH schema is reported without locally provisioning a competing schema.
  - Validate: the Turso Sync integration harness opens a fresh path against a prepared DWH remote, asserts the local file and schema readiness, and asserts the incompatible-schema failure class
- [ ] AC5: Pull makes independently published remote DWH data visible locally, push publishes a local DWH write that an independent remote/replica connection can observe, and deleting the local replica plus Turso sidecars permits reconstruction of remote data through a fresh open/pull.
  - Validate: the Turso Sync integration harness runs independent pull, push, and reconstruction cases and records whether it used the pinned local `tursodb` harness or caller-provided test-remote environment
- [ ] AC6: Existing local Turso adapters continue opening with multiprocess WAL where they do today, while no ETL, credential discovery/persistence, control-plane, lifecycle, doctor, setup, hook, `sce trace sync`, or background-sync behavior is introduced.
  - Validate: `nix flake check` and inspection of command/lifecycle registrations
- [ ] AC7: Durable context distinguishes the multiprocess local capture source, single-owner reconstructible Turso Sync replica, and remote DWH; records caller-owned credentials and the exact ownership rule; and captures observed bootstrap behavior and SDK constraints relevant to the next ETL PR.
  - Validate: inspect the updated focused Agent Trace DWH/DB/Turso/path context and root architecture/context-map/glossary entries against the implemented API and tests

### Full validation

- `nix flake check`
- `nix run .#pkl-check-generated`

### Context sync

- Add focused durable context for the `AgentTraceDwhReplica` boundary and register it in `context/context-map.md`.
- Update `context/sce/agent-trace-dwh-db.md`, `context/sce/agent-trace-db.md`, `context/sce/shared-turso-db.md`, and `context/cli/default-path-catalog.md` for the three-database architecture, canonical paths, ownership, bootstrap/readiness behavior, and SDK constraints.
- Update `context/architecture.md` and `context/glossary.md` with the single-owner replica boundary and repair the temporary PR2 statements that the DWH has no canonical local sync path or sync owner.
- Update `context/overview.md` only where its high-level database-boundary description needs the new current state.

## Constraints and non-goals

- **In scope:** Turso's current `sync` crate feature, a minimal direct OS-lock dependency, canonical replica/lock path helpers, a dedicated replica module and safe DWH connection seam, focused ownership/path tests, practical Turso Sync integration coverage, and durable architecture documentation.
- **Out of scope:** Source extraction or transformation, hashing, watermark reads/advancement, source busy retry, control-plane/provisioning calls, OAuth, credential discovery or persistence, token rotation, CLI commands, lifecycle/setup/doctor/hook wiring, automatic/background sync, archive/retention behavior, and partial sync.
- **Constraints:** Keep Turso at `0.7.0` unless compilation proves a version change necessary; enable its supported `sync` feature without changing existing local-only behavior; never call `experimental_multiprocess_wal(true)` on the replica; acquire `<local_path>.bridge-lock` non-blockingly before any sync open; retain the lock file and hold its OS lock for the complete replica lifetime; never expose the auth token in diagnostics; use the existing DWH readiness check without running local DWH migrations during replica open.
- **Non-goal:** Generalize every `TursoDb` into a sync-capable adapter or let application/hook processes open the replica. The new abstraction is the only Turso Sync builder owner and is designed for the future ETL bridge process.

## Assumptions

- `fs4` `0.13.1`, already present transitively in `cli/Cargo.lock`, is the minimal project-compatible direct dependency for non-blocking advisory whole-file locks on Linux/macOS; the implementation will hold the opened `File` rather than deleting the lock path.
- Turso `0.7.0` exposes sync through the `sync` feature and `turso::sync::Builder`; its builder defaults `bootstrap_if_empty` to true, so reconstruction uses normal SDK bootstrap rather than physical copying.
- The central/test DWH is provisioned before a replica opens and carries the same `__sce_migrations` contract as `AgentTraceDwhDb`; this PR detects missing/incompatible schema but does not create it remotely.
- The replica may use synchronous public wrappers around its owned current-thread Tokio runtime if that is required to preserve the CLI's existing blocking `AgentTraceDwhDb` SQL API; exact async spelling is subordinate to lock-safe ownership and thin Turso Sync pull/push semantics.
- Remote integration tests may use the pinned local `tursodb` package when it supports the required sync protocol, otherwise a caller-provided disposable test URL/token environment; credentials and remote state are never checked in or printed.

## Task stack

- [x] T01: `Add replica paths and the single-owner bridge lock` (status:done)
  - Task ID: T01
  - Goal: Establish the canonical repository-scoped replica/lock locations and a reusable non-blocking lock guard that proves exactly-one-process ownership without touching source storage.
  - Boundaries (in/out of scope): In — `default_paths` replica and lock helpers with repository-ID validation parity, direct minimal file-lock dependency, bridge-lock guard/acquisition errors, lock lifetime/drop behavior, same-process contention/reacquisition, subprocess-death, source-isolation, and path tests in the repository's filesystem-test-appropriate harness. Out — Turso Sync open, credentials, schema readiness, pull, and push.
  - Dependencies: none
  - Done when: Paths resolve under the repository state directory without colliding with `agent-trace.db`; lock acquisition is non-blocking and actionable; the lock remains held by the guard and is released by drop/process exit; the on-disk lock file is retained; source DB work succeeds while the bridge lock is held; targeted ownership/path tests pass on supported Unix CI platforms.
  - Verification notes (commands or checks): `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_dwh_replica_path`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_dwh_replica_lock`; inspect that only the `.bridge-lock` file is passed to the locking API.
  - Implementation evidence: Added `agent_trace_dwh_replica_path_for_repository[_at]` and `agent_trace_dwh_bridge_lock_path_for_repository[_at]` to `cli/src/services/default_paths.rs`, mirroring the existing `agent_trace_db_path_for_repository_at` repository-ID validation and resolving `<state_root>/sce/repos/<repository_id>/agent-trace-sync.db[.bridge-lock]`. Added `cli/src/services/agent_trace_dwh_replica/{mod.rs,lock.rs}` with a `BridgeLock` guard built on `fs4`'s non-blocking `try_lock_exclusive`/`unlock` (`fs4 = "0.13.1"` promoted to a direct `cli/Cargo.toml` dependency; only the `.bridge-lock` `File` is ever passed to the locking API). The guard creates and retains the lock file, never deletes it, and releases only the OS lock on `Drop`. Registered the new module in `cli/src/services/mod.rs`.
  - Verification evidence: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_dwh_replica_path` (5 passed); `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_dwh_replica_lock` (6 passed, 1 ignored subprocess-helper test invoked by the real-subprocess-death test); `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml` (213 passed, 1 ignored, 0 failed — full workspace regression); `nix develop -c ./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings` (clean); `nix develop -c ./scripts/run-cli-cargo.sh fmt --manifest-path cli/Cargo.toml -- --check` (clean). Lock tests cover: file creation/retention, non-blocking contention rejection with actionable guidance, reacquisition after drop, reacquisition after a real killed subprocess (proving OS-level release on process death, not just `Drop`), same-process double-acquisition rejection, and that source `agent-trace.db` writes (via `RepositoryAgentTraceDb`) succeed while the bridge lock is held. Inspected call sites: `try_lock_exclusive`/`unlock` in `lock.rs` are only ever invoked on the `File` opened at the caller-supplied bridge-lock path.

- [ ] T02: `Implement and prove the Turso Sync replica boundary` (status:todo)
  - Task ID: T02
  - Goal: Add the lock-owning `AgentTraceDwhReplica` API with caller-provided credentials, remote bootstrap, DWH readiness, local SQL access, pull, and push, backed by integration evidence.
  - Boundaries (in/out of scope): In — Turso `sync` feature enablement, typed config, dedicated sync builder ownership, any narrow shared-DB connection/runtime seam needed to reuse `AgentTraceDwhDb`, lock-before-open ordering, schema-readiness classification, credential-safe errors, pull/push wrappers, concurrent replica rejection, fresh bootstrap, independent pull/push verification, and reconstruction tests. Out — local schema migration/provisioning, ETL/domain writes, retries beyond SDK behavior, token acquisition/storage, lifecycle/CLI/background wiring, partial sync, and changes to source DB open semantics.
  - Dependencies: T01
  - Done when: A prepared DWH remote can bootstrap a missing `agent-trace-sync.db`; open fails before Turso access when the lock is owned; the resulting object owns both lock and DWH access; schema readiness is non-mutating; pull/push work through independent peers; local deletion reconstructs remote data; token-bearing failure cases are redacted; no sync builder enables multiprocess WAL; targeted local and remote-harness tests pass and record observed SDK/bootstrap constraints for context sync.
  - Verification notes (commands or checks): `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_dwh_replica`; run the repository-owned Turso Sync integration harness against a disposable prepared DWH remote (using `nix develop .#database` when a local server is supported), then independently inspect pull, push, reconstruction, and incompatible-schema outcomes; inspect error assertions with a sentinel auth token.

- [ ] T03: `Document the source-replica-DWH operating model` (status:todo)
  - Task ID: T03
  - Goal: Make the three-database architecture, single-owner rule, reconstructibility, credential boundary, and observed Turso SDK behavior durable and discoverable for the next ETL PR.
  - Boundaries (in/out of scope): In — focused replica context, context-map registration, source/DWH/shared-Turso/default-path updates, root architecture/glossary updates where important, and concrete bootstrap/pull/push test observations plus SDK constraints from T02. Out — ETL/control-plane/CLI design beyond explicitly naming those deferred boundaries, speculative retry or synchronization policy, and implementation changes.
  - Dependencies: T02
  - Done when: Documentation states exactly one OS process may own a repository's `agent-trace-sync.db`; application/hooks never open it; its lock cannot block `agent-trace.db`; the replica is disposable/reconstructible; callers provide URL/token; remote provisioning, ETL, and CLI sync remain deferred; current code no longer conflicts with PR2's temporary no-sync-path wording; and T02's bootstrap/integration results and next-PR SDK constraints are recorded.
  - Verification notes (commands or checks): inspect all context-sync paths against `cli/src/services/agent_trace_dwh_replica/`, `default_paths`, dependency features, and the integration evidence; verify every new context file is linked from `context/context-map.md` and root terminology is consistent.

## Open questions

None. The request fixes the storage boundary, ownership model, credential responsibility, schema-readiness policy, non-goals, and required behavior; the remaining runtime-bridging and disposable-remote test-harness choices are reversible implementation details constrained by the pinned Turso SDK and repository test conventions.
