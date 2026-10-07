# Plan: cli-async-turso-persistence-pr2

## Change summary

PR1 owns the application multi-thread Tokio runtime, but shared Turso persistence still constructs a current-thread runtime and synchronously drives native database futures. PR2 replaces that bridge with directly awaited persistence operations, preserving SQL, schema, migrations, transactions, retry/error policies, encryption, WAL, and repository/auth separation. It also removes obsolete runtime-lifetime blocking scopes and the sync storage cleanup guard where ordinary ownership suffices.

The user explicitly revised the split: "PR2 must contain the mechanical async propagation through all direct DB consumers required to remove the runtime bridge. PR3 becomes cleanup/audit rather than propagation." Because RepositoryAgentTraceDb, LocalDb, and AuthDb are aliases of shared generic adapters, the compile-safe migration includes connected storage, hook, mutation-trace, export, token-storage, lifecycle, setup/doctor, and command callers as necessary. Only signatures, awaits, immediate adapter wiring, and required test adaptations change; unrelated synchronous service logic stays synchronous. PR3 performs cleanup/audit, PR4 retains the requested remaining synchronous compatibility-boundary audit, and PR5 remains future OpenTelemetry work.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the
check that proves it. `/validate` runs these checks; no task in the stack
performs final validation.

- [ ] AC1: Production persistence owns zero Tokio runtimes; build_current_thread_runtime and block_on_isolated are deleted and no replacement synchronous executor drives DB futures.
  - Validate: `nix shell nixpkgs#ripgrep -c rg -n 'build_current_thread_runtime|block_on_isolated|Builder::new_current_thread|runtime\.block_on|Handle::block_on|tokio::runtime::Runtime|block_in_place|try_current|\.block_on\(' cli/src`; classify every match, separating test fixtures from production and confirming only application runtime ownership. Inspect for futures/pollster executors, runtime-containing threads, global runtime singletons, and async DB work inside spawn_blocking.
- [ ] AC2: Both shared Turso adapters, their migration/transaction helpers, and all connected production DB consumers required to compile use awaited native operations on the existing caller runtime.
  - Validate: Inspect the async API/call-chain inventory against source in `cli/src/services/db/`, `agent_trace_db/`, `auth_db/`, `local_db/`, `agent_trace_storage/`, `token_storage.rs`, auth/control-plane callers, exports, mutation-trace, lifecycle, setup/doctor, hooks, sync, and command dispatch. Representative multi-thread Tokio tests exercise create/open, execute/query, reopen, repository insert/query, and encrypted auth behavior without nested runtimes.
- [ ] AC3: Transaction SQL and one-connection scope remain unchanged, including BEGIN IMMEDIATE, commit/rollback, idempotent insert pairs, CAS row-count guards, deterministic errors, and whole-transaction contention retry.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::db`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::agent_trace_db::repository`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::mutation_trace::store`; inspect SQL/statement ordering diff, rollback/no-op assertions, and single-connection transaction lifetimes.
- [ ] AC4: Migration discovery, IDs, numeric ordering, SQL, metadata tracking, per-file transaction behavior, baseline upgrade/rebuild fixtures, and failure behavior match the current implementation.
  - Validate: Repository/shared/auth migration tests in AC3 and AC7 pass with their original assertions. `git diff -- cli/build.rs cli/migrations` shows no migration generation/SQL changes; inspect actual migration source paths from the T01 inventory too. Confirm migration execution remains outside open/connect retries, and execute_batch plus separate metadata insertion retain their current boundaries.
- [ ] AC5: Generic retries retain existing attempts/backoff, post-attempt timeout diagnostics, logging and exhaustion behavior; Agent Trace contention retains Busy/BusySnapshot classification, exactly two bounded attempts, jitter 0..=100ms, configured busy timeout/deadline, and pre/post-sleep admission.
  - Validate: Shared DB retry tests retain numeric, exhaustion/event, deterministic-error, elapsed-overlap, and admission assertions; focused regressions distinguish generic elapsed-time diagnostics from cancelling timeout. Review every mechanical synchronous-sleep conversion and verify identical duration, retry unit, ordering, and policy. Do not substitute the existing cancelling `resilience::run_with_retry` for `run_with_retry_sync` semantics.
- [ ] AC6: Multiprocess WAL, passive checkpoint policy, lock-overlap/held-lock behavior, duplicate suppression, and supported-load loss/error/exhaustion assertions remain intact.
  - Validate: Shared DB checkpoint and lock tests and `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::agent_trace_db::lock_contention_tests` pass. Also execute strict ignored suites with the exact matrix below; require zero losses, errors and exhausted retries at supported load, expected pair counts, and existing lock-budget assertions rather than success alone. Verify thread-local instrumentation has been safely adapted to async task polling.
    - Distinct events: for writer/round pairs 2/1000, 3/1000, 4/500, run `SCE_LOCK_CONTENTION_STRICT=1 SCE_LOCK_CONTENTION_WRITERS=<writers> SCE_LOCK_CONTENTION_ROUNDS=<rounds> nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml concurrent_distinct_events_persist_every_event_under_write_contention -- --ignored --nocapture`.
    - Duplicate events: for 2/500, 3/500, 4/500, run the same environment/wrapper with filter `concurrent_duplicate_delivery_persists_each_event_once_under_write_contention -- --ignored --nocapture`.
    - Real hook processes: build with `nix develop -c ./scripts/run-cli-cargo.sh build --manifest-path cli/Cargo.toml`; resolve the resulting absolute binary path from Cargo configuration, then for 2/500, 3/500, 4/200 run `SCE_BIN=<absolute-built-sce-path> SCE_LOCK_CONTENTION_STRICT=1 SCE_LOCK_CONTENTION_WRITERS=<writers> SCE_LOCK_CONTENTION_ROUNDS=<rounds> nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml concurrent_real_codex_hook_processes_persist_every_distinct_event -- --ignored --nocapture`. Missing SCE_BIN/skipped tests do not count as proof. N=8 remains characterization rather than acceptance.
- [ ] AC7: Repository identity/metadata convergence, no-migration hook readiness, no-touch legacy paths, auth encryption/configuration/secret-safe diagnostics, credential behavior, and LocalDb behavior are unchanged.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::agent_trace_storage`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::auth_db`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::local_db`; run affected token-storage/auth/control-plane, storage initialization and lifecycle tests identified by T01. Inspect typed/source error propagation and persistent auth initialization-failure caching.
- [ ] AC8: Newly cancellable transactions/writes cannot commit partial state or panic on resource Drop; storage outlives borrowing sync futures and error/cancellation cleanup is safe without a DB-owned runtime.
  - Validate: Focused multi-thread Tokio regressions cover materially new risks around transaction acquisition/body cancellation, subsequent same-connection reuse and commit boundaries where controllable. Inspect pinned Turso 0.8.1 deferred rollback behavior, including cancellation before the transaction guard exists; state exact tested guarantees and any upstream limitations rather than assuming immediate Drop rollback. Existing transaction failure tests retain zero-partial-row assertions.
- [ ] AC9: Obsolete PR1 DB-runtime-lifetime block_in_place scopes and blocking storage Drop are absent; every remaining production block_in_place has an individually documented blocking reason independent of DB runtime ownership.
  - Validate: Audit `cli/src/services/command_registry.rs` Setup/Doctor/Hooks sites and `cli/src/services/sync/sync.rs` storage construction/SyncStorageGuard Drop against `nix shell nixpkgs#ripgrep -c rg -n 'block_in_place|SyncStorageGuard' cli/src`. Multi-thread command/storage lifetime regressions exercise the affected success, error and cancellation paths without runtime-drop protection. Ordinary synchronous filesystem/process work is assessed independently.
- [ ] AC10: The expanded PR2 scope consists only of required mechanical DB propagation and lifetime/cancellation correctness; it introduces no unrelated behavior, schema/protocol change, dynamic DB framework, artificial locking layer, or OpenTelemetry work.
  - Validate: Inspect changed files and `nix shell nixpkgs#ripgrep -c rg -n 'BoxFuture|async_trait|dyn .*Db|Arc<Mutex|spawn_blocking|tokio::spawn|opentelemetry|tracing-opentelemetry|OTLP' cli/src cli/Cargo.toml`; classify new versus pre-existing occurrences. Check no new async filesystem/process conversion, connection pool/cache, exporter, span, telemetry auth or SQL cleanup is included.
- [ ] AC11: Durable context accurately describes application runtime → async persistence, the revised PR2/PR3 split, removed bridge symbols/workarounds, remaining synchronous work, and any justified blocking boundaries.
  - Validate: Inspect the existing context owners listed below and architectural decision update/supersession; reconcile recorded drift with source without creating duplicate architecture documents. The final review report lists before/after architecture, deleted symbols, async APIs, removed/retained blocking scopes and reasons, focused/full results, remaining risks, revised PR3/PR4 handoff, and no OpenTelemetry inclusion.
- [ ] AC12: Normal and CI/release verification are green for the implemented change.
  - Validate: All commands under Full validation succeed, with actual outcomes recorded in the validation report; compilation alone is insufficient.

### Full validation

Repository-wide checks `/validate` runs after the last task, regardless of
which criterion they map to.

- `nix flake check`
- `nix build .#ci-checks`
- `git diff --check`

### Context sync

- `context/architecture.md`, `context/overview.md`, and `context/patterns.md`: runtime ownership and accurate remaining synchronous work.
- `context/sce/shared-turso-db.md`: async APIs, retry distinctions, transaction/cancellation and connection contract, unchanged WAL/checkpoint/migrations, hook readiness drift repair.
- `context/sce/agent-trace-db.md`, `context/sce/auth-db.md`, `context/sce/local-db.md`: inherited async APIs, unchanged separation/encryption, current migration 006, configured retry defaults.
- `context/cli/agent-trace-storage.md`, `context/cli/service-lifecycle.md`, existing auth/export/mutation-trace/hook/sync owners discovered in T01: required await propagation and corrected repository/no-migration lifecycle descriptions.
- `context/sce/agent-trace-db-write-contention-evidence.md`: measured regression evidence with the same supported-load acceptance contract.
- `context/decisions/2026-10-07-application-owned-async-command-runtime.md` and repository-convention successor if required: immutable decision history, application-only runtime ownership and revised staging. Update `context/context-map.md` only for changed owner descriptions/decision links.

## Task context synchronization lifecycle

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** shared Turso and encrypted core, retry/migration/transaction helpers, immediate typed SQL/repository wrappers and every connected direct DB consumer requiring mechanical awaits. Include token-storage initialization and auth/control-plane DB scheduling, export readers, mutation-trace store/coordinator/provider closures, hook ingress, lifecycle/setup/doctor and command dispatch only to the extent needed for a compile-safe bridge removal.
- **Out of scope:** OpenTelemetry, exporters, new spans; SQL/schema cleanup; repository/sync/hook redesign; batching, pools, caches, new generic interfaces; unrelated bugs, pure transformations, parsing/rendering, Git/process/filesystem async conversions, CLI schema and Quint logic.
- **Constraints:** pinned Turso 0.8.1 APIs and concrete types/static dispatch; no DB-owned/per-operation/global executor, Handle::block_on, futures/pollster block_on, async DB work in spawn_blocking, or runtime-containing spawned thread. Tests alone may construct runtimes for genuinely synchronous fixtures; async tests use the existing test runtime and prefer multi-thread Tokio.
- **Constraints:** retain exact SQL, retry classification/count/duration/jitter, configured defaults, elapsed-time diagnostics, transaction scope, error taxonomy/messages, secret-safe behavior, encryption and OS keyring fallback, repository/auth separation and migration ordering. Mechanical async sleeps are documented individually. Preserve existing connection serialization; do not add Arc<Mutex>, boxed async APIs, dynamic traits or artificial Send/Sync bounds.
- **Constraints:** use Nix and repository-supported Cargo wrapper. Generated target trees remain absent. No canonical generation inputs are expected to change; if a mechanically required change touches them, add `nix run .#pkl-check-generated` to relevant checks and full validation without generating committed trees.
- **Non-goal:** making unrelated synchronous service logic async or designing future telemetry integration.

## Assumptions

- Core async API changes and their connected caller adaptations form one coherent compile-safe migration commit because aliases/direct calls provide no independent synchronous adapter seam; task-local editing may be sequential, but no broken or bridge-bearing intermediate commit is planned.
- PR4 retains its originally requested remaining synchronous compatibility-boundary cleanup/audit role and PR5 remains future OpenTelemetry. The revised PR3 audits the mechanically propagated DB call graph; this plan does not implement either follow-up.
- Existing dependencies, schema and test assertions are authoritative. Any discovered unrelated bug is reported separately.

## Task stack

- [ ] T01: `Map persistence ownership and the required async caller closure` (status:todo)
  - Task ID: T01
  - Scope: In — source-grounded runtime/API/transaction/retry/migration/test inventory recorded with exact paths in this plan, connected callers through application dispatch, all five PR1 blocking sites, context owners and pinned Turso ownership/cancellation semantics. Out — application behavior changes or building a replacement bridge.
  - Dependencies: none
  - Done when: Inventory covers both shared adapters and aliases, direct typed SQL wrappers, synchronous callback/trait/OnceLock boundaries, contention instrumentation and actual ignored suites; classifies every production runtime/blocking site and establishes the mechanically required propagation closure plus unchanged policy baseline. Audit Turso Connection Send/Sync and intended same-connection use before choosing borrowing/lifetime adaptations.
  - Verify: Nix ripgrep searches for the request's runtime/SQL/retry/WAL/migration symbols across the five requested persistence areas and their callers; inspect actual tests, Cargo/flake/wrapper configuration and pinned Turso transaction/connection source. No tests or final checks are necessary for the inventory-only change.
  - Context synchronization: pending

- [ ] T02: `Replace the Turso runtime bridge with awaited persistence and callers` (status:todo)
  - Task ID: T02
  - Scope: In — remove core runtime field/constructor and bridge; convert open/execute/query/materialization/checkpoint/readiness/migration/transaction/encrypted operations and retry helpers; mechanically adapt the complete caller closure and existing tests in one compile-safe commit. Out — unrelated service logic and behavior changes.
  - Dependencies: T01
  - Done when: All native futures are awaited on caller runtime; no synchronous DB executor remains; shared/consumer signatures and callback bounds compile. Generic retries preserve post-attempt diagnostic timeout semantics rather than adopting cancelling timeout; contention retries preserve whole-operation units and deterministic errors. Each converted sleep is recorded with unchanged policy. Async auth initialization preserves one-time result/failure caching and secret safety without moving DB futures into blocking tasks. Transactions keep the same logical connection and SQL. Adapt existing test instrumentation so async task movement does not corrupt counters/timelines; retain overlap, rollback and attempt assertions.
  - Verify: Run focused wrapper suites for `services::db`, `services::agent_trace_db`, `services::agent_trace_storage`, `services::auth_db`, `services::local_db`, `services::mutation_trace`, and affected auth/control-plane/export/hook/lifecycle/command tests from T01 before repository-wide validation. Use `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml <filter>`. Audit source for removed runtime symbols/replacement executors and review SQL/policy diff. Auto-format only through Nix when needed.
  - Context synchronization: pending

- [ ] T03: `Characterize new async cancellation and retry boundaries` (status:todo)
  - Task ID: T03
  - Scope: In — focused missing behavioral regressions for newly cancellable transaction acquisition/body/connection reuse, same-connection serialization, non-cancelling generic retry semantics, and async-aware contention measurement where existing coverage is insufficient. Out — a cancellation framework, mirrored implementation tests, broader concurrency abstraction or changed acceptance assertions.
  - Dependencies: T02
  - Done when: Tests establish no partial transaction effects and safe resource Drop at materially new cancellation boundaries, correctly account for Turso deferred rollback and acquisition-before-guard risks, and distinguish retained generic retry semantics from deadline cancellation. Existing contention/transaction assertions still measure overlap, exact row pairs, retry bounds and rollback; independent DB connections/processes retain meaningful contention. Document precise guarantees and upstream limitations found by the audit.
  - Verify: Run each new focused regression through the Cargo wrapper, then affected DB/repository suites and strict ignored supported-load matrices under AC6. Use deterministic coordination for cancellation where feasible; keep production retries, SQL and timings unchanged.
  - Context synchronization: pending

- [ ] T04: `Remove obsolete DB runtime lifetime blocking scopes` (status:todo)
  - Task ID: T04
  - Scope: In — command_registry Setup/Doctor/DB-backed Hooks compatibility scopes, sync storage construction, SyncStorageGuard Drop and runtime-drop test fixtures. Out — indiscriminate deletion of independently necessary blocking protection or detached cleanup tasks/leaks.
  - Dependencies: T02, T03
  - Done when: Every scope whose sole reason was DB runtime construction/destruction is removed. Ordinary local storage ownership preserves borrowing-future lifetime and error/cancellation safety; unnecessary guard/blocking Drop is gone. Each retained production block_in_place has an exact site and independently valid blocking dependency documented. Tests reflect real lifetime behavior after removing nested runtime workarounds.
  - Verify: Focused command dispatch/setup/doctor/hook and sync success/error/cancellation tests from T01; inspect all production block_in_place/SyncStorageGuard occurrences and direct storage Drop on multi-thread Tokio. Confirm no replacement DB bridge or runtime-containing fixture inside async tests.
  - Context synchronization: pending

- [ ] T05: `Record application-owned persistence and the revised PR staging` (status:todo)
  - Task ID: T05
  - Scope: In — existing architecture/persistence/consumer context owners, relevant drift repairs, ADR update or convention-compliant immutable successor, context-map references and precise PR3/PR4 handoff. Out — duplicate architecture documents, claiming all service logic is async, final validation execution or future telemetry implementation.
  - Dependencies: T02, T03, T04
  - Done when: Durable owners describe application runtime → async persistence and the actual mechanically propagated consumers; name deleted bridge symbols and removed/retained blocking sites with reasons; preserve database policy facts and measured evidence. Stage PR1 application runtime, PR2 async core plus required consumers, PR3 cleanup/audit, PR4 remaining synchronous compatibility boundaries, PR5 future OpenTelemetry. Correct only relevant recorded drift, including no-migration hooks, repository lifecycle, migration 006 and configured LocalDb retries. Record API inventory and remaining risks/handoff in existing owners rather than promising deferred propagation already completed by PR2.
  - Verify: Cross-check context/decision descriptions against source inventories and task evidence, check owner links and document diff whitespace. Repository-wide final checks remain exclusively in Full validation.
  - Context synchronization: pending

## Open questions

None. The user resolved the compile-safe staging conflict explicitly; runtime removal, necessary mechanical caller propagation and preservation checks are now scoped. Connection/cancellation mechanics are source-audit and regression work within these constraints, not an unresolved authorization question.
