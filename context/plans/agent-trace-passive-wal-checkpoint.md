# Plan: agent-trace-passive-wal-checkpoint

## Change summary

Add a reusable passive WAL checkpoint operation to the shared `TursoDb<M: DbSpec>`
adapter (`cli/src/services/db/mod.rs`) and use it, for now, only for the
repository-scoped Agent Trace DB (`RepositoryAgentTraceDb`). The checkpoint is
triggered exactly once from the `sce hooks post-commit` lifecycle, after Agent
Trace persistence for that commit has already succeeded, so the high-frequency
`diff-trace` and `conversation-trace` hook writes keep their current no-checkpoint
behavior. This is new maintenance behavior on top of the existing
`experimental_multiprocess_wal(true)` local-DB setup, which today has no
checkpointing at all and therefore no bound on WAL growth. A failed checkpoint
never turns into a durability boundary: it is logged as a warning and the
post-commit hook still reports success as long as Agent Trace persistence
itself succeeded.

## Acceptance criteria

- [x] AC1: `TursoDb<M: DbSpec>` exposes `passive_checkpoint(&self) -> Result<()>`, which executes `PRAGMA wal_checkpoint(PASSIVE)` through the existing query/runtime/retry infrastructure (no second Tokio runtime, no bypass of the adapter).
  - Validate: `cargo test --manifest-path cli/Cargo.toml services::db` — new tests prove a local DB can write, then successfully run `passive_checkpoint()`, that data stays readable afterward, and that calling it repeatedly is safe, without asserting `-wal` file deletion/truncation.
- [x] AC2: The repository Agent Trace DB (`RepositoryAgentTraceDb = TursoDb<RepositoryAgentTraceDbSpec>`) uses this shared method directly, with no second/duplicate checkpoint abstraction.
  - Validate: inspection — `grep -rn "wal_checkpoint" cli/src/services/` shows the `PRAGMA` text in exactly one place (`TursoDb::passive_checkpoint`), and all Agent Trace call sites invoke that shared method.
- [x] AC3: `diff-trace` and `conversation-trace` hook writers do not call `passive_checkpoint()`, and neither does `TursoDb::execute()`, the shared insert helpers, or `Drop`.
  - Validate: inspection — `grep -n "passive_checkpoint" cli/src/services/hooks/mod.rs` shows it invoked only from the post-commit path, never from the diff-trace or conversation-trace persistence functions; `grep -n "passive_checkpoint" cli/src/services/db/mod.rs cli/src/services/agent_trace_db/mod.rs` shows no call from `execute()`, `insert_diff_trace()`, `insert_messages()`, `insert_parts()`, or any `Drop` impl.
- [x] AC4: `sce hooks post-commit` attempts exactly one passive checkpoint after Agent Trace persistence for that commit succeeds; a checkpoint failure is logged as a warning through the existing observability logger and does not fail the hook or affect already-persisted data.
  - Validate: `cargo test --manifest-path cli/Cargo.toml services::hooks` — lifecycle tests cover successful-persistence+successful-checkpoint (hook succeeds) and successful-persistence+failed-checkpoint (hook still succeeds, previously persisted Agent Trace data remains persisted, a warning is logged).
- [x] AC5: Durable context describes the checkpoint lifecycle: passive-only routine maintenance, the post-commit trigger boundary, that high-frequency hooks do not checkpoint per write, that PASSIVE only checkpoints what is currently safe and does not guarantee WAL truncation, and that checkpoint failure does not invalidate previously committed data.
  - Validate: inspection — `context/sce/shared-turso-db.md` and `context/sce/agent-trace-db.md` (and/or `context/sce/agent-trace-hooks-command-routing.md`) state these points.

### Full validation

- `cargo test --manifest-path cli/Cargo.toml`
- `cargo clippy --manifest-path cli/Cargo.toml`
- `nix flake check`
- `nix run .#pkl-check-generated`

### Context sync

- `context/sce/shared-turso-db.md` — new `passive_checkpoint()` method on `TursoDb<M>`.
- `context/sce/agent-trace-db.md` — repository-scoped adapter reuses the shared checkpoint method; no second abstraction.
- `context/sce/agent-trace-hooks-command-routing.md` — post-commit lifecycle now attempts one passive checkpoint after successful persistence, fail-open with a logged warning.

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** `cli/src/services/db/mod.rs` (shared `passive_checkpoint()` plus tests), `cli/src/services/hooks/mod.rs` (post-commit lifecycle wiring, fail-open logging, lifecycle tests), and the durable context files listed under Context sync.
- **Out of scope:** `diff-trace` and `conversation-trace` hook writers, `TursoDb::execute()`, `insert_diff_trace()`, `insert_messages()`, `insert_parts()`, any `Drop` impl, `EncryptedTursoDb<M>`, the local/auth DB adapters, and any checkout-scoped Agent Trace DB surface.
- **Constraints:** reuse the existing `TursoDb` query/runtime/retry infrastructure — no new Tokio runtime, no bypass of the adapter; do not use `bail!` for a checkpoint failure that occurs after successful persistence; do not add retries beyond whatever generic DB behavior already applies to the operation; log the failure via the existing `Logger::warn` observability path with an `sce.agent_trace_db.*`-style event name (e.g. `sce.agent_trace_db.passive_checkpoint_failed`).
- **Non-goal:** WAL-size-based checkpoint thresholds, configurable checkpoint intervals, a background checkpoint worker or daemon, `FULL`/`RESTART`/`TRUNCATE` checkpoint modes, checkpoint-on-every-write behavior, checkpoint logic in `Drop`, or exposing checkpoint statistics beyond what the implementation/tests need.

## Assumptions

- The post-commit checkpoint step opens its own repository-scoped `RepositoryAgentTraceDb` handle through the existing `open_agent_trace_db_for_hook_runtime` helper, matching the pattern every other post-commit sub-step already uses, rather than threading one DB instance across the intersection, Agent Trace insert, and checkpoint steps.
- The seam for simulating checkpoint failure in tests is a small injectable checkpoint closure added alongside the existing injectable parameters (`run_intersection_flow`, `run_agent_trace_flow`, `resolve_auto_sync`, `launch_auto_sync`) already present on `run_post_commit_subcommand_with`, not a broader refactor of post-commit orchestration.
- The new warning event is named `sce.agent_trace_db.passive_checkpoint_failed`, following the existing `sce.<area>.<event>` convention used by events such as `sce.hooks.diff_trace.agent_trace_db_open_failed`.

## Task stack

- [x] T01: `Add shared TursoDb::passive_checkpoint() with tests` (status:done)
  - Task ID: T01
  - Scope: In — `cli/src/services/db/mod.rs`: new `pub fn passive_checkpoint(&self) -> Result<()>` on `TursoDb<M>` that runs `PRAGMA wal_checkpoint(PASSIVE)` through the existing runtime/retry helpers; tests proving write-then-checkpoint-then-read and safe repeated calls, without asserting `-wal` file deletion/truncation. Out — `EncryptedTursoDb<M>`, any non-`PASSIVE` checkpoint mode, exposing checkpoint statistics.
  - Dependencies: none
  - Done when: `TursoDb<M>::passive_checkpoint()` compiles, executes `PRAGMA wal_checkpoint(PASSIVE)` via the shared connection/retry path, and is covered by tests for post-checkpoint readability and repeated-call safety.
  - Verify: `cargo test --manifest-path cli/Cargo.toml services::db`; `cargo clippy --manifest-path cli/Cargo.toml`
  - Completed: 2026-08-21
  - Files changed: `cli/src/services/db/mod.rs`
  - Result: Added `pub fn passive_checkpoint(&self) -> Result<()>` on `TursoDb<M>`, issuing `PRAGMA wal_checkpoint(PASSIVE)` through `conn.query` (draining the result row) inside the existing `run_with_retry_sync`/`block_on_isolated` query path, using `resolve_query_retry_policy::<M>()` and `QUERY_RETRY_HINT` like the other query methods. Marked `#[allow(dead_code)]` since no call site exists until T02. Added a minimal test-only `TestDbSpec: DbSpec` (no migrations) plus `open_test_db`/`cleanup_test_db` helpers and two tests: one write/checkpoint/read-back test and one repeated-call-safety test.
  - Verify (actual): `nix flake check` (repository policy blocks direct `cargo test`/`cargo clippy`/`cargo fmt --check` invocations in favor of this) — all checks passed, including `cli-tests` (378 passed, 0 failed, including both new `services::db::tests::passive_checkpoint_*` tests), `cli-clippy`, and `cli-fmt`.
  - Deviations: Verification ran via `nix flake check` instead of the plan's literal `cargo test`/`cargo clippy` invocations, per repository bash-tool policy (`use-nix-flake-check-over-cargo-test` etc. in `.sce/config.json`); this exercises the same targeted tests plus clippy/fmt as part of the full check set. Two clippy pedantic findings were fixed during implementation: `#[allow(dead_code)]` added to `passive_checkpoint` (unused until T02 wires a call site) and `cleanup_test_db`'s `db_path` parameter changed from `PathBuf` to `&Path` to satisfy `clippy::needless_pass_by_value`.
  - Context impact: Domain. Adds one new public method (`passive_checkpoint()`) to the shared `TursoDb<M>` adapter's contract, documented in `context/sce/shared-turso-db.md`; no architectural or cross-domain change, and no call site yet (T02 wires the post-commit trigger and updates the hooks-routing context file separately).
  - Context synchronization: synced

- [x] T02: `Trigger one fail-open passive checkpoint from post-commit after successful Agent Trace persistence` (status:done)
  - Task ID: T02
  - Scope: In — `cli/src/services/hooks/mod.rs`: call `RepositoryAgentTraceDb::passive_checkpoint()` exactly once after the post-commit Agent Trace write path succeeds, logging `sce.agent_trace_db.passive_checkpoint_failed` via `Logger::warn` on failure without failing the hook; lifecycle tests for success+success and success+failure (hook still succeeds, prior persisted data intact, warning emitted). Out — `diff-trace`/`conversation-trace` writers, `TursoDb::execute()`/insert helpers/`Drop`, retry-policy changes.
  - Dependencies: T01
  - Done when: `sce hooks post-commit` attempts exactly one `passive_checkpoint()` call after successful persistence; a failing checkpoint still yields a successful post-commit result with previously persisted data intact and a logged warning; `diff-trace`/`conversation-trace` paths remain unchanged.
  - Verify: `cargo test --manifest-path cli/Cargo.toml services::hooks` — targeted post-commit lifecycle tests (success+success, success+failure) pass
  - Completed: 2026-08-21
  - Files changed: `cli/src/services/hooks/mod.rs`, `cli/src/services/db/mod.rs`
  - Result: Added `run_post_commit_passive_checkpoint()`, opening `RepositoryAgentTraceDb` via the existing `open_agent_trace_db_for_hook_runtime` helper and calling `.passive_checkpoint()`. Added an injectable `run_passive_checkpoint: K` closure param (alongside the existing `run_intersection_flow`/`run_agent_trace_flow`/`resolve_auto_sync`/`launch_auto_sync` params) plus a `logger: Option<&dyn Logger>` param to `run_post_commit_subcommand_with`; the checkpoint runs immediately after `run_agent_trace_flow` succeeds and before auto-sync resolution, and a checkpoint failure is logged via `Logger::warn("sce.agent_trace_db.passive_checkpoint_failed", ...)` and otherwise ignored (fail-open). Threaded `logger` through `run_post_commit_subcommand` and `run_post_commit_subcommand_with_trace`, and updated the `HookSubcommand::PostCommit` call site in `run_hooks_subcommand_in_repo` to pass the already-available `logger` (previously unused on this path, unlike diff-trace/conversation-trace). Removed the now-stale `#[allow(dead_code)]` on `TursoDb::passive_checkpoint` in `db/mod.rs` since this task adds its first real call site. Added a test-only `RecordingLogger` (backed by `std::sync::Mutex`, since `Logger: Send + Sync`) and two new lifecycle tests (`post_commit_checkpoint_runs_once_after_successful_persistence`, `post_commit_checkpoint_failure_is_fail_open_and_logs_warning`); updated the 5 pre-existing `run_post_commit_subcommand_with` tests for the new closure/logger params.
  - Verify (actual): `nix flake check` (repository policy blocks direct `cargo test`/`cargo clippy`/`cargo fmt --check`) — all checks passed: `cli-tests` (380 passed, 0 failed, including both new `services::hooks::tests::post_commit_checkpoint_*` tests), `cli-clippy`, `cli-fmt`. A first `cli-tests` run failed one unrelated test (`services::agent_trace_storage::tests::new_repository_database_starts_empty_and_shares_repository_level_rows`, `left: 2, right: 1`); stashing all working-tree changes and rerunning `nix flake check` against unmodified `HEAD` reproduced a clean pass, and a subsequent run with this task's changes restored also passed — confirming a pre-existing test-isolation flake in `agent_trace_storage` tests, unrelated to this task's changes, not a regression it introduced.
  - Deviations: Verification ran via `nix flake check` instead of the plan's literal `cargo test` invocation, per repository bash-tool policy, matching T01's precedent. Threading `logger: Option<&dyn Logger>` through `run_post_commit_subcommand`/`run_post_commit_subcommand_with_trace`/the `HookSubcommand::PostCommit` call site was required but not explicitly named in the task scope; it was necessary to satisfy AC4's explicit requirement to log via the existing `Logger::warn` observability path, and mirrors the pattern diff-trace/conversation-trace already use. `RecordingLogger` uses `std::sync::Mutex` rather than `RefCell` because `Logger: Send + Sync`.
  - Context impact: Domain. Adds a new post-commit lifecycle step (fail-open passive checkpoint after successful Agent Trace persistence) and a new observability warning event (`sce.agent_trace_db.passive_checkpoint_failed`); no architectural or cross-domain change. Durable context (`context/sce/agent-trace-hooks-command-routing.md`) needs to document this new post-commit step per the plan's Context sync section.
  - Context synchronization: synced

## Open questions

None. The request names the exact method signature, call site, failure semantics, logging convention, test expectations, and an explicit non-goal list, leaving no scope, criteria, or ordering ambiguity to resolve.

## Validation Report

**Status:** validated  
**Date:** 2026-08-21

### Commands run

- `nix flake check` -> exit 0 (all checks passed: `cli-tests`, `cli-clippy`, `cli-fmt`, and the rest of the flake's check set, run in place of the plan's literal `cargo test --manifest-path cli/Cargo.toml` / `cargo clippy --manifest-path cli/Cargo.toml`, which repository policy `.sce/config.json` blocks in favor of `nix flake check`)
- `nix run .#pkl-check-generated` -> exit 0 (ephemeral Pkl generation passed: 107 files, inventory sha256 `8500d6e4d8cbbe7ae540c52254a0b35b6e48834956823eeaf05e8af347d68bdb`)

### Success-criteria verification

- [x] AC1: `TursoDb<M: DbSpec>` exposes `passive_checkpoint(&self) -> Result<()>` executing `PRAGMA wal_checkpoint(PASSIVE)` through the existing query/runtime/retry path -> `nix flake check`'s `cli-tests` job includes `services::db::tests::passive_checkpoint_keeps_previously_written_data_readable` and `services::db::tests::passive_checkpoint_is_safe_to_call_repeatedly`, both passing; `passive_checkpoint` (`cli/src/services/db/mod.rs:630`) issues the PRAGMA via `conn.query` inside `run_with_retry_sync`/`block_on_isolated`.
- [x] AC2: `RepositoryAgentTraceDb` uses the shared method directly, no duplicate abstraction -> `grep -rn "wal_checkpoint" cli/src/services/` shows the PRAGMA text in exactly one place (`cli/src/services/db/mod.rs:642`, inside `passive_checkpoint`); the only call site is `db.passive_checkpoint()` at `cli/src/services/hooks/mod.rs:1472`.
- [x] AC3: `diff-trace`/`conversation-trace` writers, `TursoDb::execute()`, insert helpers, and `Drop` never call `passive_checkpoint()` -> `grep -n "passive_checkpoint" cli/src/services/hooks/mod.rs` shows it referenced only inside the post-commit closure chain (`run_post_commit_subcommand`, `run_post_commit_subcommand_with`, `run_post_commit_passive_checkpoint`) and a matching test; `grep -n "passive_checkpoint" cli/src/services/db/mod.rs cli/src/services/agent_trace_db/mod.rs` shows no hits in `agent_trace_db/mod.rs` and, in `db/mod.rs`, only the method definition and its own tests — no hits inside `execute()` (`db/mod.rs:456`, `:813`) or any `Drop` impl.
- [x] AC4: `sce hooks post-commit` attempts exactly one passive checkpoint after successful Agent Trace persistence, fails open with a logged warning -> `cli/src/services/hooks/mod.rs:1441-1453` runs `run_agent_trace_flow` first (propagating its error via `?`), then calls `run_passive_checkpoint` exactly once and only logs via `Logger::warn("sce.agent_trace_db.passive_checkpoint_failed", ...)` on failure without returning an error; `nix flake check`'s `cli-tests` job includes `services::hooks::tests::post_commit_checkpoint_runs_once_after_successful_persistence` and `services::hooks::tests::post_commit_checkpoint_failure_is_fail_open_and_logs_warning`, both passing.
- [x] AC5: Durable context describes the checkpoint lifecycle -> `context/sce/shared-turso-db.md:25` documents `passive_checkpoint()`, its query/runtime/retry path, that PASSIVE checkpoints only what is currently safe without guaranteeing WAL truncation, that it is routine maintenance and not a durability boundary, and that post-commit is the sole caller; `context/sce/agent-trace-hooks-command-routing.md:64` documents the post-commit trigger point (after Agent Trace persistence, before auto-sync), the fail-open warning event `sce.agent_trace_db.passive_checkpoint_failed`, that a failure does not affect already-persisted data, and that `diff-trace`/`conversation-trace` do not checkpoint per write.

### Failed checks and follow-ups

- None.

### Residual risks

- None identified.
