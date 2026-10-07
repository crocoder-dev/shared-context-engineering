# Decision: Directly awaited Turso persistence

Date: 2026-10-07
Status: Accepted
Plan: `context/plans/cli-async-turso-persistence-pr2.md`
Task: `T02`

## Context

PR1 established directly awaited static command dispatch under the application
multi-thread Tokio runtime while retaining synchronous Turso adapters that
owned separate runtimes. PR2 migrated the shared adapters and every connected
production caller so database futures can run on that application runtime.
The production build passes, the test target compiles, and the preserved
mutation-trace Quint MBT suite passes. Ordinary Rust test-only code in the
affected closure was disabled under the user's direction after its async
migration build failed; the task plan records that verification reduction.

## Decision

`TursoDb<M>` and `EncryptedTursoDb<M>` expose native async persistence APIs and
own no Tokio runtime. The application runtime directly awaits Turso operations
through statically dispatched commands, services, lifecycle providers, hooks,
sync/export, credentials, and token storage.

## Rationale

This completes PR1's application-owned runtime boundary without changing SQL,
migrations, transaction scope, encryption, WAL, retry classification, or
credential error handling. Native futures remove the nested executor bridge and
the caller-side runtime lifetime scopes while preserving concrete database
types and static command/service dispatch.

## Alternatives considered

- **Keep synchronous adapters and caller-side blocking scopes** — preserves a
  second runtime owner and executor bridge in every connected database path.
- **Run database operations in `spawn_blocking`** — keeps Turso futures behind
  a worker boundary and complicates borrowing and cancellation behavior.
- **Use boxed async database or service traits** — erases the repository's
  established static dispatch and introduces unnecessary type erasure.

## Compatibility and risks

- SQL, migrations, one-connection transactions, encryption, WAL, retries,
  errors, and foreground hook persistence ordering remain unchanged.
- Native async transactions can be cancelled at await points. T03 owns focused
  characterization of transaction acquisition/body cancellation, connection
  reuse, retry timing, and contention instrumentation.
- The ordinary Rust test suites for the affected closure were disabled after
  async migration compile failures at the user's direction. The preserved
  Quint MBT suite passes; ordinary behavioral coverage remains reduced and is
  recorded in the plan.

## Guardrails

- Keep database and caller dependencies concrete, generic, associated-type
  based, or represented by static enums; do not introduce dynamic database or
  service traits, boxed futures, or a replacement executor bridge.
- Preserve retry and transaction policies when moving operations to async
  control flow.
- Keep pure parsing, formatting, filesystem, and process operations synchronous
  unless they directly await database work.

## Consequences

- The application Tokio runtime owns the execution of local Turso futures.
- Auth token persistence and control-plane credential operations are directly
  awaited; they do not use database `spawn_blocking` wrappers.
- Setup, Doctor, Hooks, sync, and export propagate async calls to persistence.

## Follow-up

- T03 characterizes cancellation and retry boundaries introduced by native
  async persistence.
- T04 audits and removes any remaining DB-runtime lifetime blocking scopes.
- PR3 remains cleanup and audit; it does not own further caller propagation.

## References

- Plan: [CLI async Turso persistence PR2](../plans/cli-async-turso-persistence-pr2.md)
- Task: `T02`
- Current-state context: [Architecture](../architecture.md),
  [Shared Turso adapter](../sce/shared-turso-db.md),
  [Service lifecycle](../cli/service-lifecycle.md),
  [Agent Trace sync](../cli/agent-trace-sync-command.md)
- Evidence: [Turso adapters](../../cli/src/services/db/mod.rs),
  [Application entrypoint](../../cli/src/main.rs),
  [Static command dispatch](../../cli/src/services/command_registry.rs)
- Related decision: [Application-owned async command runtime](2026-10-07-application-owned-async-command-runtime.md)
