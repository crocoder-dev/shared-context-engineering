# Plan: dwh-scoped-agent-trace-replica-identity

## Change summary

Corrects the storage identity of the Agent Trace DWH Turso Sync replica (`agent-trace-sync.db`). It currently resolves under `<state_root>/sce/repos/<repository_id>/agent-trace-sync.db`, which wrongly ties one local replica to one repository. The architecture is one Agent Trace DWH per workspace, contributed to by many repositories, so the replica must resolve under `<state_root>/sce/dwh/<dwh_id>/agent-trace-sync.db` instead — an opaque, repository-independent identifier for the remote workspace DWH.

This replaces the repository-scoped default-path helpers in `cli/src/services/default_paths.rs` with DWH-scoped equivalents, removes the now-redundant repository-scoped bridge-lock path helpers (dead code today — `AgentTraceDwhReplica` already derives its lock path from the caller-supplied `local_path`), and updates durable context describing the old identity. The source Agent Trace DB path (`agent_trace_db_path_for_repository`), the DWH schema, `AgentTraceDwhReplicaConfig`, and `AgentTraceDwhSync`'s orchestration API are all unaffected — this plan is a persistence-path correction only, not a behavior or schema change. No control-plane, CLI, or credential-discovery work is added; a future caller resolves `dwh_id` and passes it to the helpers this plan adds.

## Acceptance criteria

- [ ] AC1: `agent_trace_dwh_replica_path_for_dwh_at(state_root, dwh_id)` resolves `<state_root>/sce/dwh/<dwh_id>/agent-trace-sync.db` for any `dwh_id`, independent of any repository ID — the function signature does not accept one.
  - Validate: `cargo test --manifest-path cli/Cargo.toml default_paths::tests -- dwh`
- [ ] AC2: Two distinct `dwh_id` values resolve to distinct replica paths and distinct bridge-lock paths, and the same `dwh_id` always resolves to the same path.
  - Validate: `cargo test --manifest-path cli/Cargo.toml default_paths::tests -- dwh`
- [ ] AC3: The repository source path (`agent_trace_db_path_for_repository`) is unchanged and never shares a parent directory with any DWH replica path, for any `repository_id`/`dwh_id` pair.
  - Validate: `cargo test --manifest-path cli/Cargo.toml default_paths::tests -- dwh`
- [ ] AC4: Invalid `dwh_id` path segments (`""`, `"."`, `".."`, values containing `/` or `\`) are rejected by the DWH replica path resolver, matching the existing repository-ID validation behavior.
  - Validate: `cargo test --manifest-path cli/Cargo.toml default_paths::tests -- dwh`
- [ ] AC5: No repository-scoped DWH replica or bridge-lock path helper remains in the codebase.
  - Validate: `grep -rn "agent_trace_dwh_replica_path_for_repository\|agent_trace_dwh_bridge_lock_path_for_repository" cli/src` returns nothing
- [ ] AC6: `AgentTraceDwhReplicaConfig`, `AgentTraceDwhReplica::open`, and `AgentTraceDwhSync::run` keep their current signatures (`local_path`/`database_url`/`auth_token`; `repository_id`/`source`/`replica_config`) — no `dwh_id`/`workspace_id` field or parameter is added to any of them.
  - Validate: `git diff --stat cli/src/services/agent_trace_dwh_replica cli/src/services/agent_trace_dwh_sync.rs` shows no signature changes outside doc comments
- [ ] AC7: Durable context describes the replica as workspace/DWH-scoped, with many repositories able to ETL into the same replica, and no longer describes it as repository-scoped.
  - Validate: `grep -rn "repository-scoped.*agent-trace-sync\|repos/{repository_id}/agent-trace-sync\|repos/<repository-id>/agent-trace-sync" context/` returns nothing

### Full validation

- `cargo test --manifest-path cli/Cargo.toml`
- `nix flake check`

### Context sync

- `context/cli/default-path-catalog.md`
- `context/glossary.md`
- `context/architecture.md`
- `context/sce/agent-trace-dwh-replica.md`
- `context/sce/agent-trace-dwh-sync.md`

## Constraints and non-goals

- **In scope:** `cli/src/services/default_paths.rs` (path helpers and their tests); doc-comment references to the renamed helpers in `cli/src/services/agent_trace_dwh_replica/replica.rs`; the five context files listed under Context sync.
- **Out of scope:** `AgentTraceDwhReplica`, `AgentTraceDwhReplicaConfig`, `AgentTraceDwhSync`, any ETL module, the Agent Trace source DB schema, the DWH schema, watermark identities, credential handling, control-plane APIs, CLI commands (`sce trace sync` or otherwise), and migration/import of any existing repository-scoped replica file on disk.
- **Constraints:** `dwh_id` is validated only as an opaque path segment (same rejection rules already used for `repository_id`: empty, `.`, `..`, `/`, `\`) — its contents are never interpreted, and it is not required to be a UUID.
- **Non-goal:** Do not add a repository-scoped-to-DWH-scoped migration path for on-disk replica files. The old `sce/repos/<repository_id>/agent-trace-sync.db` file (pre-production, never shipped) is left untouched — not copied, renamed, or deleted. A new DWH-scoped replica bootstraps normally from the remote.
- **Non-goal:** Do not add a `workspace_id` concept to the path or to any type. `dwh_id` is the sole storage identity for this plan.

## Assumptions

- New DWH-scoped helpers keep the same `#[allow(dead_code)]` posture the repository-scoped helpers currently carry: no CLI/lifecycle caller is wired yet (that remains explicitly out of scope, per non-goals), so the lint attribute stays until a future caller resolves `dwh_id` and calls them.
- The removed bridge-lock helpers (`agent_trace_dwh_bridge_lock_path_for_repository[_at]`) are deleted outright rather than renamed to a DWH-scoped equivalent: they are `#[allow(dead_code)]` today with no real caller, and `AgentTraceDwhReplica::open` already derives its lock path from the caller-supplied `local_path` (see `bridge_lock_path_for_replica` in `replica.rs`), so a parallel default-path lock helper would be a second, unused source of the same identity.
- The internal-only helper `agent_trace_dwh_replica_dir_for_repository_at` is renamed to `agent_trace_dwh_replica_dir_for_dwh_at` alongside the public rename, since it exists solely to build the public path.

## Task stack

- [x] T01: `Replace repository-scoped DWH replica path helpers with DWH-scoped equivalents` (status:done)
  - Task ID: T01
  - Goal: In `cli/src/services/default_paths.rs`, replace `agent_trace_dwh_replica_path_for_repository`/`_at` with `agent_trace_dwh_replica_path_for_dwh`/`_at` resolving `<state_root>/sce/dwh/<dwh_id>/agent-trace-sync.db`, remove `agent_trace_dwh_bridge_lock_path_for_repository`/`_at` and their doc-comment references, rename the internal directory helper, and add/replace tests proving path identity, cross-DWH isolation, source/destination separation, and path-traversal rejection for `dwh_id`.
  - Boundaries (in/out of scope): In — `default_paths.rs` public/internal helpers and their unit tests; the stale doc-comment reference to the old helper name in `cli/src/services/agent_trace_dwh_replica/replica.rs`. Out — any change to `AgentTraceDwhReplica`, `AgentTraceDwhReplicaConfig`, `AgentTraceDwhSync`, or `agent_trace_db_path_for_repository`.
  - Dependencies: none
  - Done when: `agent_trace_dwh_replica_path_for_dwh_at(state_root, "dwh-A")` returns `<state_root>/sce/dwh/dwh-A/agent-trace-sync.db`; `dwh-A` and `dwh-B` resolve to distinct replica paths; `repository_id = "repo-A"` and `dwh_id = "dwh-X"` resolve to `sce/repos/repo-A/agent-trace.db` and `sce/dwh/dwh-X/agent-trace-sync.db` respectively with no shared parent; `""`, `"."`, `".."`, `"a/b"`, `"a\b"` are rejected as `dwh_id`; no repository-scoped DWH path/lock helper remains in `cli/src`.
  - Verification notes (commands or checks): `cargo test --manifest-path cli/Cargo.toml default_paths::tests`; `grep -rn "agent_trace_dwh_replica_path_for_repository\|agent_trace_dwh_bridge_lock_path_for_repository" cli/src`
  - Evidence: Renamed `agent_trace_dwh_replica_path_for_repository[_at]` → `agent_trace_dwh_replica_path_for_dwh[_at]` and the internal `agent_trace_dwh_replica_dir_for_repository_at` → `agent_trace_dwh_replica_dir_for_dwh_at` in `cli/src/services/default_paths.rs`, switching the resolved segment from `repos/<repository_id>` to `dwh/<dwh_id>` with the same empty/`.`/`..`/`/`/`\` validation. Deleted `agent_trace_dwh_bridge_lock_path_for_repository`/`_at` outright (per plan assumption — `bridge_lock_path_for_replica` in `replica.rs` already derives the lock path from the caller-supplied `local_path`). Updated the two stale doc-comment references in `cli/src/services/agent_trace_dwh_replica/replica.rs` (lines 31 and the `bridge_lock_path_for_replica` doc comment) to stop naming the removed/renamed helpers. Rewrote the `default_paths.rs` test module: renamed tests to `dwh_id` terminology, added a same-ID-stability/cross-DWH-distinctness test and a source/replica no-shared-parent test, kept empty/escaping-ID rejection tests, and dropped the two bridge-lock tests (helper removed). `agent_trace_db_path_for_repository[_at]`, `AgentTraceDwhReplica`, `AgentTraceDwhReplicaConfig`, and `AgentTraceDwhSync` were not touched.
  - Verification: `nix flake check` (the repo's bash policy blocks direct `cargo test`/`cargo fmt --check` invocations and requires this instead) — passed on a clean rerun (an initial run hit 3 pre-existing, unrelated flaky SQLite-lock failures in `agent_trace_db`/`agent_trace_dwh_db` tests that reproduce with or without this change; a second `nix flake check` run passed with all tests green, confirming this change caused no regression). All 5 new/renamed `default_paths::tests::agent_trace_dwh_replica_path_for_dwh_*` tests passed. `grep -rn "agent_trace_dwh_replica_path_for_repository\|agent_trace_dwh_bridge_lock_path_for_repository" cli/src` returns no matches (AC5 satisfied).

- [ ] T02: `Update durable context for DWH-scoped replica identity` (status:todo)
  - Task ID: T02
  - Goal: Update `context/cli/default-path-catalog.md`, `context/glossary.md`, `context/architecture.md`, `context/sce/agent-trace-dwh-replica.md`, and `context/sce/agent-trace-dwh-sync.md` so the replica is described as one local replica per remote Agent Trace DWH (workspace-scoped), with many repositories able to ETL into the same replica, referencing the new `agent_trace_dwh_replica_path_for_dwh`/`_at` helpers and the `sce/dwh/<dwh_id>/` path instead of the old repository-scoped description.
  - Boundaries (in/out of scope): In — the five listed context files. Out — historical/completed plan files under `context/plans/` (`agent-trace-dwh-turso-sync-replica.md`, `agent-trace-dwh-sync.md`), which stay as historical records of what was built at the time; out — any decision-record file (superseding language belongs in a new decision only if a reader would otherwise be misled about current identity, which this task's target files already resolve).
  - Dependencies: T01
  - Done when: none of the five files describe the replica or its bridge lock as repository-scoped; each instead states one replica per remote DWH, keyed by `dwh_id`, with many repositories contributing to it.
  - Verification notes (commands or checks): `grep -rn "repository-scoped.*agent-trace-sync\|repos/{repository_id}/agent-trace-sync\|repos/<repository-id>/agent-trace-sync" context/`

## Open questions

None. The change request fully specifies the target path shape, validation rules, API boundaries, non-goals, and documentation scope; no scope, criterion, or ordering decision was left open.
