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
- **Known limits:** large futures keep `#![recursion_limit = "256"]` pressure
  (measured in PR3); the temporary-index cleanup limitation above stands.
  The ordinary Rust suites deleted during the async migration are not restored;
  correctness evidence is the focused regression set, the Quint MBT suite and
  surviving targeted tests.

## Staging

- **PR1** (complete): application-owned multi-thread Tokio runtime and static async command dispatch.
- **PR2**: async Turso core and the callers it required (credentials, token storage, lifecycle, hooks, mutation trace, sync/export, setup/doctor); the Git/process/filesystem async work and cancellation protections above; transaction isolation, credential-rotation, terminal-sync, adapter-boundary and FD-inheritance fixes; removal of the DB runtime bridge and its blocking scopes.
- **PR3**: post-migration cleanup and runtime audit — redundant `async` wrappers, unused `run_with_retry_sync`, redundant `drop(storage)`, stale `#[allow(dead_code)]`, orphaned test support and unused seams, `join_three_to_completion` re-evaluation, large-future/recursion-limit measurement and an optional temporary-index sweep. Cosmetic and maintainability only.
- **PR4**: OpenTelemetry (not implemented in PR2).

