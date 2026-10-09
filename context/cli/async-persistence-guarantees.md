# PR2 async persistence guarantees and staging

Verified guarantees, known limits and PR staging for the directly awaited Turso persistence migration. The runtime architecture and retained blocking workers live in [architecture](../architecture.md#application-execution-runtime).


Each statement points to a test, a type-level argument or a named limitation.

- **Exclusive transactions:** `execute_transactional_cas_batch` and
  `execute_transactional_insert_pair_if_absent` take `&mut self` and use the
  checked `Transaction::new`, so no other operation on the same adapter can enter
  an active transaction (compile-time argument). Turso 0.8.1 rolls back lazily:
  an abandoned uncommitted transaction leaves no committed partial state and the
  connection stays reusable (`db::transaction_cancellation_tests`); a commit that
  already completed may be durable when its caller is cancelled. Bounded
  contention retry is covered by `agent_trace_db::transaction_tests`. See
  [shared Turso adapter](../sce/shared-turso-db.md).
- **Credential rotation:** the replacement token from a refresh is persisted by an
  owned unit that outlives caller cancellation and the outer `/state` timeout,
  and the single-flight guard spans refresh and persistence. Limit: a crash after
  the remote rotation and before the save, or an ambiguous remote result, can
  still lose the rotated token. See [sync command](agent-trace-sync-command.md).
- **Sync termination:** a terminal stream error (for example `403`) stops sibling
  streams from starting further batches while in-flight credential persistence
  still completes (`terminal_stream` tests).
- **Adapter lifecycle and guard descriptors:** a started state transition keeps the
  boundary lease until it ends, and guarded-shell descriptors are inherited only
  by the guarded child; see [adapter lifecycle](opencode-mutation-scope-adapter-lifecycle.md)
  and [external mutation guard](mutation-trace-external-mutation-guard.md).
- **Known limits:**
  - Future size and `recursion_limit` (PR3 audit, Linux debug build,
    `RUSTC_BOOTSTRAP=1 RUSTFLAGS=-Zprint-type-sizes`, disposable worktree):
    the top-level `app::run` future is 14000 bytes and `main`'s `block_on`
    future 14016; the chain `run_with_dependency_check` 13952,
    `run_command_lifecycle` 13568, `execute_command_phase` 13168,
    `SyncCommand::execute_with_stderr` 13072, `run_current_sync_without_progress`
    12784, `run_sync_async` 12040 and `join_three_to_completion` 11832 carries
    almost all of it. The dependency's own `DatabaseSyncEngine::open_db` future
    is 12704 bytes, so the size is dominated by a third-party future captured
    inline and each wrapper adds roughly 50-200 bytes. No safe reduction exists
    without boxing (out of scope). Building without the override fails with
    "queries overflow the depth limit" (depth increased by 130 computing the
    layout of `app::run`; the default limit is 128), so `#![recursion_limit =
    "256"]` in `cli/src/main.rs` is kept.
  - Temporary Git index: `TempIndexGuard` removes its file on completion or drop,
    but a killed process can leave an orphaned `index-<uuid>` file under the
    runtime tmp directory. No automatic sweeper is added: there is no ownership
    or lock record for the file and name or age alone is not evidence of
    abandonment. A sweep needs a separate change that first defines that
    protocol.
  - Guarded-shell descriptor atomicity: the lifetime pipe is created
    close-on-exec atomically with `pipe2` on Linux only; elsewhere `pipe` then
    `fcntl` leaves a small fork/exec window. This race is an unresolved
    limitation and is not fixed here.
  - Blocking sites: the 14 production `spawn_blocking` sites match the
    retained-worker list in [architecture](../architecture.md#application-execution-runtime);
    none runs a database or Turso future.
  The ordinary Rust suites deleted during the async migration are not restored;
  correctness evidence is the focused regression set, the Quint MBT suite and
  surviving targeted tests.

## Staging

- **PR1** (complete): application-owned multi-thread Tokio runtime and static async command dispatch.
- **PR2**: async Turso core and the callers it required (credentials, token storage, lifecycle, hooks, mutation trace, sync/export, setup/doctor); the Git/process/filesystem async work and cancellation protections above; transaction isolation, credential-rotation, terminal-sync, adapter-boundary and FD-inheritance fixes; removal of the DB runtime bridge and its blocking scopes.
- **PR3** (implementation complete; final `/validate` pending): post-migration cleanup and runtime audit. Verified-dead code and blanket dead-code allowances were deleted, redundant `async` wrappers, `run_with_retry_sync` and the redundant `drop(storage)` were removed, `join_three_to_completion` was retained, the `recursion_limit = "256"` override was kept, and no temporary-index sweeper was added. The final unsuppressed audit on Linux reports 32 binary-target and 27 test-target diagnostics, all covered by declaration-level `#[allow(dead_code, reason = ...)]` exceptions: the `diff_traces` compatibility surface (cites the 2026-10-01 ADR), the single-event mutation loaders with no production reader (the former ref-reconciliation exemptions were removed once `sce doctor --fix` wired the pass), cfg-gated `PlatformFamily` variants and `GuardError::UnsupportedPlatform`, and never-read `CoordinateOutcome`/`committed`/`completed` fields kept as suspected defects. Linux-only diagnostics do not prove cross-platform safety. Cosmetic and maintainability only.
- **PR4**: OpenTelemetry (not implemented in PR2).

