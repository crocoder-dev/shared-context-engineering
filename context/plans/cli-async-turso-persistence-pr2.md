# Plan: cli-async-turso-persistence-pr2

## Change summary

PR1 owns the application multi-thread Tokio runtime, but shared Turso persistence still constructs a current-thread runtime and synchronously drives native database futures. PR2 replaces that bridge with directly awaited persistence operations, preserving SQL, schema, migrations, transactions, retry/error policies, encryption, WAL, and repository/auth separation. It also removes obsolete runtime-lifetime blocking scopes and the sync storage cleanup guard where ordinary ownership suffices.

The user explicitly revised the split: "PR2 must contain the mechanical async propagation through all direct DB consumers required to remove the runtime bridge. PR3 becomes cleanup/audit rather than propagation." Because RepositoryAgentTraceDb, LocalDb, and AuthDb are aliases of shared generic adapters, the compile-safe migration includes connected storage, credential-store, hook, mutation-trace, export, token-storage, lifecycle, setup/doctor, and command callers as necessary. Only signatures, awaits, required static adapter wiring, lifetime correctness, and test adaptations change; unrelated synchronous service logic stays synchronous.

## Architecture decisions

PR2 preserves and extends SCE's compile-time/static dispatch model. Async propagation must use concrete types, generic type parameters, associated types, or static enums. No new trait objects, boxed futures, `async-trait`, dynamic database/service abstractions, or type-erased command/service futures are permitted.

```text
application Tokio runtime
    ↓
static async command dispatch
    ↓
static async service/lifecycle/hook dispatch
    ↓
static async persistence
    ↓
Turso futures
```

These decisions apply to the entire migrated call graph, including tests' injection seams. T01 inventories their concrete paths; it does not defer architecture selection until implementation.

### Commands and persistence

`RuntimeCommand` remains an enum matching concrete command types, with awaits at DB-backed branches. No general dynamic async Command abstraction, `Box<dyn Command>`, `Arc<dyn Command>`, or boxed command/service futures. Keep `TursoDb<M>`, `EncryptedTursoDb<M>`, `RepositoryAgentTraceDb = TursoDb<RepositoryAgentTraceDbSpec>`, `LocalDb = TursoDb<LocalDbSpec>`, and `AuthDb = EncryptedTursoDb<AuthDbSpec>`. No `dyn Database`, `dyn Repository`, `dyn Persistence`, `Box<dyn Db>`, `Arc<dyn Db>`, or dynamic async DB traits. Tests use these concrete/generic types or narrow statically typed seams.

### Credential store and auth token storage

Replace the existing dynamic credential seam with native async methods and generic injection:

```rust
pub trait CredentialStore: Send + Sync {
    async fn load(&self) -> Result<Option<StoredTokens>, ControlPlaneError>;
    async fn save(&self, token: &TokenResponse)
        -> Result<StoredTokens, ControlPlaneError>;
}

pub struct AuthenticatedControlPlaneClient<S = SystemCredentialStore> {
    http: reqwest::Client,
    // Existing concrete configuration fields remain.
    credential_store: S,
    refresh_lock: Arc<tokio::sync::Mutex<()>>,
}
```

Client operations use `S: CredentialStore`; constructors and connected sync/test callers propagate concrete generic types. A borrowed store or statically typed shared ownership is acceptable where ownership requires it. Remove `Arc<dyn CredentialStore>` and `Box<dyn CredentialStore>` constructors, rather than retaining them with `async-trait`, `BoxFuture`, or `Pin<Box<dyn Future>>`. `SystemCredentialStore` directly awaits token storage; delete production credential load/save `spawn_blocking` calls. Preserve valid-token reuse, expired-token refresh, refresh single-flight/exactly-one refresh, unexpected-401 refresh/retry, second-401 terminal behavior, storage error classification, and exact save/no-save assertions.

Replace `token_storage`'s `std::sync::OnceLock<Result<AuthDb, String>>` with static `tokio::sync::OnceCell<Result<AuthDb, String>>`. Use the API supported by locked Tokio (currently 1.52.3 with `sync` enabled), conceptually:

```rust
static AUTH_DB: tokio::sync::OnceCell<Result<AuthDb, String>> =
    tokio::sync::OnceCell::const_new();

async fn get_auth_db() -> Result<&'static AuthDb, TokenStorageError> {
    let result = AUTH_DB.get_or_init(|| async {
        AuthDb::new().await.map_err(|error| error.to_string())
    }).await;
    // Borrow the cached success or map the cached error through existing diagnostics.
    // Keep the initializer output Result-valued; do not use get_or_try_init.
}
```

This is an API sketch, not a complete function body. Initialization is single-flight: the first completed success is cached and the first completed failure is cached; later calls must not implicitly retry a failed initialization. Cancellation before an initializer completes leaves no completed result and follows Tokio OnceCell cancellation semantics. Verify these semantics and the exact API against locked dependency source, and test cached success/failure and concurrent initialization without resetting the production static.

Make `save_tokens`, `load_tokens`, and `delete_tokens` async. AuthCommand login/whoami directly await `load_tokens()`, refresh/login directly await `save_tokens(&token)`, and logout directly awaits `delete_tokens()`. Remove PR1's `run_credential_operation` DB wrapper and preserve all typed `CliError` mappings and secret redaction. Convert control-plane and connected sync fake stores to concrete native async implementations injected generically. Retain load/save/refresh counts, single-flight, 401, and failure assertions. Delete tests whose sole purpose was proving execution outside Tokio; replace them with proof that async stores execute correctly on the application runtime.

### Lifecycle, hooks, mutation trace, and export

Make `ServiceLifecycle::diagnose<C: HasRepoRoot>`, `fix<C: HasRepoRoot>`, and `setup<C: HasRepoRoot>` native `async fn` with their existing result types. Keep identity/metadata methods synchronous. `LifecycleProvider` remains the static enum dispatcher: its matching concrete providers are awaited through `diagnose().await`, `fix().await`, and `setup().await`. Doctor awaits diagnosis/fixes and Setup awaits setup: `RuntimeCommand → Doctor/Setup → LifecycleProvider → concrete provider → async persistence`. Config/Hooks providers execute their existing pure synchronous work inside the native async methods; no `spawn_blocking` for pure lifecycle work, `Box<dyn ServiceLifecycle>`, `async-trait`, or boxed lifecycle futures.

DB hooks follow `RuntimeCommand::Hooks → await HooksCommand → static enum match → specific handler → await RepositoryAgentTraceDb → await Turso`. Pure hook paths may remain synchronous internally. No boxed futures, trait-object handlers, `async-trait`, DB `spawn_blocking`, or executor bridge. Foreground hooks return only after required persistence completes; the existing detached post-commit auto-sync child remains the only authorized detached hook-adjacent execution.

Propagate awaits from `execute_transactional_cas_batch` and other DB primitives through `MutationTraceStore → mutation-trace runtime/coordinator → provider/closure seams → hook adapter → RuntimeCommand` (read from persistence toward callers). Prefer native async functions; where injection genuinely needs callbacks, use `F: FnOnce(...) -> Fut, Fut: Future<Output = ...>` with concrete generic futures and appropriate borrowed lifetimes. No boxed future callbacks, `block_on`, or `spawn_blocking` to stop propagation.

Make `AgentTraceExportReader` and connected repository reads async as required, and await them directly in the existing async sync engine. Preserve one initial state request, three concurrent remote streams, sequential batches within each stream, cursor reconciliation, retries, `diff_traces` compatibility reporting, and progress ordering. No intermediate executor or behavioral redesign.

### Blocking and storage lifetime

No async DB work inside `spawn_blocking`. No production path from the application Tokio runtime to a Turso future crosses a synchronous executor boundary. Prohibit `block_on`, executor bridges, runtime-containing worker threads, and DB futures scheduled inside blocking workers, including token-storage load/save/delete wrappers. Every remaining production `spawn_blocking` or `block_in_place` must terminate in a genuinely synchronous/blocking external API and be individually justified. A synchronous OS keyring API used for encryption-key acquisition may be isolated in `spawn_blocking`; the worker must contain only the synchronous keyring work, never AuthDb construction or Turso futures.

Remove PR1 DB-runtime-only blocking regions for Setup, Doctor, DB-backed Hooks, sync storage construction, and `SyncStorageGuard` Drop. Ask: **Does SyncStorageGuard still express any invariant ordinary Rust ownership cannot express?** If no, remove it and use scoped ownership. Storage must outlive all futures borrowing it and drop safely on cancellation/errors, without detached cleanup tasks or leaks. A remaining guard requires a documented independent invariant and cannot retain blocking Drop for a removed DB runtime.

### PR roadmap and handoff

- PR1: Application-owned Tokio runtime — complete.
- PR2: Async Turso persistence plus mechanically required async propagation: DB core, token/credential storage, hooks, mutation trace, lifecycle, setup/doctor, sync/export DB reads, removal of the DB runtime bridge and obsolete DB-lifetime blocking islands.
- PR3: Post-migration cleanup and runtime audit: simplify mechanically awkward signatures, remove obsolete compatibility helpers, classify remaining blocking work, and ensure no accidental dynamic dispatch/executor bridge remains. No behavioral redesign or deferred required propagation.
- PR4: OpenTelemetry.

T05 updates existing architecture staging documentation to this roadmap.

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
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::agent_trace_storage`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::auth_db`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::local_db`; run affected token-storage/auth/control-plane, storage initialization and lifecycle tests identified by T01. Prove valid reuse, expired refresh, exactly-one single-flight refresh, unexpected-401 retry, terminal second-401, storage errors, exact load/save/refresh counts and save/no-save behavior with concrete async fake stores. Inspect AuthCommand's direct awaited load/save/delete paths, unchanged typed CliError mappings and secret redaction; require removal of run_credential_operation and its worker-only tests.
- [ ] AC8: Newly cancellable transactions/writes cannot commit partial state or panic on resource Drop; storage outlives borrowing sync futures and error/cancellation cleanup is safe without a DB-owned runtime.
  - Validate: Focused multi-thread Tokio regressions cover materially new risks around transaction acquisition/body cancellation, subsequent same-connection reuse and commit boundaries where controllable. Inspect pinned Turso 0.8.1 deferred rollback behavior, including cancellation before the transaction guard exists; state exact tested guarantees and any upstream limitations rather than assuming immediate Drop rollback. Existing transaction failure tests retain zero-partial-row assertions.
- [ ] AC9: Blocking audit — zero DB-runtime-related block_in_place, zero blocking SyncStorageGuard Drop, and zero token-storage DB spawn_blocking. Every remaining production spawn_blocking/block_in_place has an individually documented genuinely synchronous/blocking external API reason independent of DB runtime ownership.
  - Validate: Audit `cli/src/services/command_registry.rs` Setup/Doctor/Hooks sites and `cli/src/services/sync/sync.rs` storage construction/SyncStorageGuard Drop against `nix shell nixpkgs#ripgrep -c rg -n 'block_in_place|spawn_blocking|SyncStorageGuard' cli/src` and Source audit. Multi-thread command/storage lifetime regressions exercise the affected success, error and cancellation paths without runtime-drop protection. Ordinary synchronous filesystem/process work is assessed independently.
- [ ] AC10: The expanded PR2 scope consists only of required mechanical DB propagation and lifetime/cancellation correctness; it introduces no unrelated behavior, schema/protocol change, dynamic DB framework, artificial locking layer, or OpenTelemetry work.
  - Validate: Inspect changed files and `nix shell nixpkgs#ripgrep -c rg -n 'BoxFuture|async_trait|dyn .*Db|Arc<Mutex|spawn_blocking|tokio::spawn|opentelemetry|tracing-opentelemetry|OTLP' cli/src cli/Cargo.toml`; classify new versus pre-existing occurrences. Check no new async filesystem/process conversion, connection pool/cache, exporter, span, telemetry auth or SQL cleanup is included.
- [ ] AC11: Durable context accurately describes application runtime → static async commands → static async services/lifecycle/hooks → static async persistence → Turso, the revised PR2/PR3 split, removed bridge symbols/workarounds, remaining synchronous work, and justified blocking boundaries.
  - Validate: Inspect the existing context owners listed below and architectural decision update/supersession; reconcile recorded drift with source without creating duplicate architecture documents. The final review report lists before/after architecture, deleted symbols, async APIs, removed/retained blocking scopes and reasons, focused/full results, remaining risks, PR3 cleanup/runtime audit and PR4 OpenTelemetry handoff, and no telemetry implementation in PR2.
- [ ] AC12: Normal and CI/release verification are green for the implemented change.
  - Validate: All commands under Full validation succeed, with actual outcomes recorded in the validation report; compilation alone is insufficient.
- [ ] AC13: Static dispatch — all new async boundaries in the entire migrated call graph use concrete types, generic type parameters, associated types, borrowed references, or static enums. No new dynamic async DB/service/lifecycle/hook/command abstraction, boxed futures, or async-trait; the existing dynamic CredentialStore seam is removed.
  - Validate: Run the Source audit below and inspect signatures/constructors/callback bounds. Require zero migrated `dyn CredentialStore`, `dyn ServiceLifecycle`, dynamic DB traits, BoxFuture, Pin<Box<dyn Future>>, async_trait, boxed command/service futures, Box<dyn Command>, or Arc<dyn CredentialStore>. Classify unrelated/pre-existing occurrences individually rather than failing on unrelated dynamic dispatch.
- [ ] AC14: Credential storage — AuthenticatedControlPlaneClient<S> uses S: CredentialStore with native async load/save and concrete async fake injection; SystemCredentialStore directly awaits production token storage. No credential DB operation runs in spawn_blocking; refresh/single-flight/401 behavior remains unchanged.
  - Validate: Inspect generic client/store implementations and connected sync callers; run the AC7 auth/control-plane tests, including an async fake-store application-runtime regression replacing the outside-runtime assertion.
- [ ] AC15: Lifecycle — native async diagnose/fix/setup propagate through Doctor/Setup and LifecycleProvider's static enum matches to concrete providers and async DB persistence. Pure Config/Hooks work stays synchronous internally; no boxed providers/futures or pure-work spawn_blocking.
  - Validate: Inspect every provider and both command chains; run affected lifecycle/setup/doctor tests with existing health/fix/setup assertions.
- [ ] AC16: Auth token storage — async Result-valued OnceCell initialization caches both completed success and completed failure, never implicitly retrying a returned initialization error. load_tokens/save_tokens/delete_tokens are directly async; run_credential_operation is removed.
  - Validate: Check locked Tokio API/source and focused isolated-cell success/failure/concurrent-initialization tests; assert subsequent reads of cached failure do not invoke the initializer again. Inspect cancellation-before-completion separately and preserve diagnostics/secret safety.
- [ ] AC17: End-to-end executor invariant — No production path from the application Tokio runtime to a Turso future crosses a synchronous executor boundary.
  - Validate: Trace every DB caller class (credentials/auth, lifecycle/setup/doctor, hooks/mutation trace, sync/export, repository/local persistence) from static RuntimeCommand dispatch to Turso. Source audit plus inspection must exclude block_on substitutes, runtime-containing workers, boxed callback executors, and DB futures inside spawn_blocking/block_in_place. Foreground hooks await required persistence; only the existing detached post-commit auto-sync child remains authorized.

### Source audit

Run these through Nix, classify every match by production/test, new/pre-existing, migrated/unrelated, and record exact sites/reasons. Searches are evidence, not a substitute for tracing calls and inspecting aliases or differently spelled executor/type-erasure APIs.

```bash
nix shell nixpkgs#ripgrep -c rg -n \
  'build_current_thread_runtime|block_on_isolated|Builder::new_current_thread|runtime\.block_on|Handle::block_on|\.block_on\(|block_in_place|spawn_blocking|tokio::runtime::Runtime' \
  cli/src
nix shell nixpkgs#ripgrep -c rg -n \
  'dyn CredentialStore|dyn ServiceLifecycle|dyn .*Db|BoxFuture|Pin<Box<dyn Future|async_trait|Box<dyn|Arc<dyn' \
  cli/src cli/Cargo.toml
nix shell nixpkgs#ripgrep -c rg -n \
  'CredentialStore|ServiceLifecycle|LifecycleProvider|AUTH_DB|run_credential_operation|SyncStorageGuard' \
  cli/src
```

Require zero new dynamic dispatch in the PR2 migrated call graph and removal of its old credential trait object. Classify **every** remaining production spawn_blocking/block_in_place, including synchronous OS keyring work. Also inspect futures/pollster executors, global runtime singletons, worker-thread runtime ownership, dyn Database/Repository/Persistence, type-erased callbacks and boxed command/service futures.

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
- **Out of scope:** OpenTelemetry, exporters, new spans; SQL/schema cleanup; repository/sync/hook redesign; batching, pools, caches, unrelated generic interfaces beyond the required static seam conversions; unrelated bugs, pure transformations, parsing/rendering, Git/process/filesystem async conversions, CLI schema and Quint logic.
- **Constraints:** pinned Turso 0.8.1 APIs and static dispatch throughout commands, services, lifecycle, hooks, callbacks, credentials, and persistence as specified in Architecture decisions. No DB-owned/per-operation/global executor, Handle::block_on, futures/pollster block_on, async DB work in spawn_blocking/block_in_place, or runtime-containing spawned thread. Every retained production blocking boundary requires an individually documented synchronous external API reason. Tests alone may construct runtimes for genuinely synchronous fixtures; async tests use the existing test runtime and prefer multi-thread Tokio.
- **Constraints:** retain exact SQL, retry classification/count/duration/jitter, configured defaults, elapsed-time diagnostics, transaction scope, error taxonomy/messages, secret-safe behavior, encryption and OS keyring fallback, repository/auth separation and migration ordering. Mechanical async sleeps are documented individually. Preserve existing connection serialization; do not add Arc<Mutex>, boxed async APIs, dynamic traits or artificial Send/Sync bounds.
- **Constraints:** use Nix and repository-supported Cargo wrapper. Generated target trees remain absent. No canonical generation inputs are expected to change; if a mechanically required change touches them, add `nix run .#pkl-check-generated` to relevant checks and full validation without generating committed trees.
- **Non-goal:** making unrelated synchronous service logic async or designing future telemetry integration. The required native async lifecycle seam may wrap existing pure synchronous provider work without changing that work.

## Assumptions

- Core async API changes and their connected caller adaptations form one coherent compile-safe migration commit because aliases/direct calls provide no independent synchronous adapter seam; task-local editing may be sequential, but no broken or bridge-bearing intermediate commit is planned.
- PR1 application runtime is complete. PR2 completes async persistence and its mechanically required caller closure, static credential/lifecycle seams, and obsolete DB-runtime blocking removal. PR3 is post-migration cleanup/runtime audit; PR4 is OpenTelemetry. This plan implements neither follow-up and leaves no required propagation or executor bridge for PR3.
- Existing dependencies, schema and test assertions are authoritative. Any discovered unrelated bug is reported separately.

## Task stack

- [ ] T01: `Map persistence ownership and the required async caller closure` (status:todo)
  - Task ID: T01
  - Scope: In — source-grounded runtime/API/transaction/retry/migration/test inventory recorded with exact paths in this plan, connected callers through application dispatch, all PR1 blocking sites, context owners and pinned Turso ownership/cancellation semantics. Explicitly map CredentialStore, AuthenticatedControlPlaneClient, SystemCredentialStore, token_storage AUTH_DB OnceLock, run_credential_operation, ServiceLifecycle, LifecycleProvider, Doctor, Setup, HooksCommand, mutation_trace callbacks/providers, AgentTraceExportReader, SyncStorageGuard, all block_in_place, and all spawn_blocking. Out — application behavior changes or building a replacement bridge.
  - Dependencies: none
  - Done when: Inventory covers both shared adapters and aliases, direct typed SQL wrappers, synchronous callback/trait/OnceLock boundaries, contention instrumentation and actual ignored suites; classifies every production runtime/blocking site and establishes the mechanically required propagation closure plus unchanged policy baseline. For every dynamic/trait seam record current dispatch (static/dynamic), target dispatch (static), and the concrete/generic/enum conversion. Explicitly confirm no additional dyn seam in the DB propagation closure leaves a design choice unresolved: any discovered seam follows Architecture decisions, never type erasure. Audit Turso Connection Send/Sync and intended same-connection use to select mechanical borrowing/lifetime adaptations within the fixed static design. Verify locked Tokio OnceCell API and Result-valued success/failure caching semantics.
  - Verify: Run Source audit and Nix ripgrep searches for runtime/SQL/retry/WAL/migration symbols across persistence and callers; inspect actual tests, Cargo/flake/wrapper configuration and pinned Tokio/Turso transaction/connection source. No tests or final checks are necessary for the inventory-only change.
  - Context synchronization: pending

- [ ] T02: `Replace the Turso runtime bridge with awaited persistence and callers` (status:todo)
  - Task ID: T02
  - Scope: In — remove core runtime field/constructor and bridge; convert open/execute/query/materialization/checkpoint/readiness/migration/transaction/encrypted operations and retry helpers; mechanically adapt the complete caller closure and existing tests in one compile-safe migration commit. This includes the resolved credential, token-storage, lifecycle, hook, mutation-trace, and sync/export architectures below. No intermediate commit may need a temporary block_on/executor bridge. Out — unrelated service logic and behavior changes.
  - Dependencies: T01
  - Done when: TursoDb/EncryptedTursoDb are async with no DB runtime field, build_current_thread_runtime, or block_on_isolated; all native futures are awaited on the application caller runtime with no synchronous executor. The entire closure compiles using static dispatch, including:
    - Async token_storage with Result-valued OnceCell caching completed success/failure; direct AuthCommand login/whoami load, refresh/login save, and logout delete awaits; removed run_credential_operation with typed CliError/secret-safe behavior preserved.
    - Native async generic CredentialStore and AuthenticatedControlPlaneClient<S: CredentialStore>, concrete SystemCredentialStore and async fake-store injection through connected sync/test callers; removed credential trait-object fields/constructors and DB spawn_blocking. Preserve token reuse, refresh/single-flight/exactly-one refresh, 401 retry/terminal behavior, storage failures and exact counts/save assertions; replace outside-runtime tests with application-runtime async-store proof.
    - Native async ServiceLifecycle diagnose/fix/setup; LifecycleProvider static enum matches await concrete providers; RuntimeCommand::Doctor/Setup await Doctor/Setup services and lifecycle operations. Pure Config/Hooks work runs synchronously inside those async methods without a blocking worker.
    - RuntimeCommand::Hooks awaits HooksCommand static matches and DB-backed handlers; required foreground persistence completes before returning, preserving the sole existing detached post-commit auto-sync child.
    - Awaited MutationTraceStore transaction primitives through coordinator/runtime, generic providers/callbacks, hook adapter and command; native async functions or genuinely needed F: FnOnce(...) -> Fut / Fut: Future bounds, never boxed callbacks.
    - Async AgentTraceExportReader/connected repository reads awaited in the existing sync engine; unchanged initial state request, three concurrent streams, per-stream sequential batches, cursor reconciliation, retries, diff_traces compatibility and progress ordering.
    - Generic retries preserve post-attempt diagnostic timeout semantics rather than adopting cancelling timeout; contention retries preserve whole-operation units and deterministic errors. Each converted sleep is recorded with unchanged policy. Transactions retain the same logical connection/SQL; migrations, encryption, retry and WAL policy are unchanged. Async-aware test instrumentation preserves overlap, rollback and attempt assertions. No dynamic async abstraction or artificial locking layer is introduced.
  - Verify: Run focused wrapper suites for `services::db`, `services::agent_trace_db`, `services::agent_trace_storage`, `services::auth_db`, `services::local_db`, `services::mutation_trace`, and affected auth/control-plane/export/hook/lifecycle/command tests from T01 before repository-wide validation. Use `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml <filter>`. Run Source audit, trace all caller classes against AC13–AC17, and review SQL/policy diff. Auto-format only through Nix when needed.
  - Context synchronization: pending

- [ ] T03: `Characterize new async cancellation and retry boundaries` (status:todo)
  - Task ID: T03
  - Scope: In — focused missing behavioral regressions for newly cancellable transaction acquisition/body/connection reuse, same-connection serialization, non-cancelling generic retry timing semantics, and async-aware write-contention measurement where existing coverage is insufficient. Out — architectural migration fixes left over from T02, a new abstraction layer/cancellation framework, mirrored implementation tests, broader concurrency abstraction or changed acceptance assertions. T02 must already satisfy the async/static architecture before T03 starts.
  - Dependencies: T02
  - Done when: Tests establish no partial transaction effects and safe resource Drop at materially new cancellation boundaries, correctly account for Turso deferred rollback and acquisition-before-guard risks, and distinguish retained generic retry semantics from deadline cancellation. Existing contention/transaction assertions still measure overlap, exact row pairs, retry bounds and rollback; independent DB connections/processes retain meaningful contention. Document precise guarantees and upstream limitations found by the audit.
  - Verify: Run each new focused regression through the Cargo wrapper, then affected DB/repository suites and strict ignored supported-load matrices under AC6. Use deterministic coordination for cancellation where feasible; keep production retries, SQL and timings unchanged.
  - Context synchronization: pending

- [ ] T04: `Remove obsolete DB runtime lifetime blocking scopes` (status:todo)
  - Task ID: T04
  - Scope: In — remove obsolete command_registry Setup/Doctor/DB-backed Hooks compatibility scopes, sync storage construction, SyncStorageGuard Drop and runtime-drop test fixtures; confirm T02 removed all credential DB blocking wrappers and audit every production spawn_blocking/block_in_place. Out — indiscriminate deletion of independently necessary synchronous external API protection or detached cleanup tasks/leaks. If a blocking scope prevents T02's native async/static closure from compiling, remove it in T02 and record it here; T04 never legitimizes an intermediate executor bridge.
  - Dependencies: T02, T03
  - Done when: Zero DB-runtime-related block_in_place, zero blocking SyncStorageGuard Drop, zero token-storage DB spawn_blocking; every scope solely needed for DB runtime construction/destruction is removed. Explicitly answer whether SyncStorageGuard expresses an invariant ordinary ownership cannot express; remove it if not. Scoped storage outlives borrowing futures and drops safely on errors/cancellation without detached cleanup or leaks. Each retained production spawn_blocking/block_in_place has an exact site and independently valid genuinely synchronous/blocking external dependency documented (for example OS keyring work alone). Tests reflect actual lifetime behavior after removal.
  - Verify: Focused command dispatch/setup/doctor/hook and sync success/error/cancellation tests from T01; run Source audit and inspect all production blocking sites/SyncStorageGuard occurrences and direct storage Drop on multi-thread Tokio. Confirm no replacement DB bridge or runtime-containing fixture inside async tests.
  - Context synchronization: pending

- [ ] T05: `Record application-owned persistence and the revised PR staging` (status:todo)
  - Task ID: T05
  - Scope: In — existing architecture/persistence/consumer context owners, relevant drift repairs, ADR update or convention-compliant immutable successor, context-map references and precise PR3 cleanup/runtime-audit and PR4 OpenTelemetry handoff. Out — duplicate architecture documents, claiming all service logic is async, final validation execution or future telemetry implementation.
  - Dependencies: T02, T03, T04
  - Done when: Durable owners describe application Tokio runtime → static async command dispatch → static async service/lifecycle/hook dispatch → static async persistence → Turso and all mechanically propagated consumers, including generic credentials and cached-result async auth initialization. Name deleted bridge/helpers and removed/retained blocking sites with reasons; preserve database policy facts and measured evidence. Stage PR1 application runtime (complete), PR2 async core plus required consumers and obsolete DB-lifetime blocking removal, PR3 post-migration cleanup/runtime audit, PR4 OpenTelemetry. Correct only relevant recorded drift, including no-migration hooks, repository lifecycle, migration 006 and configured LocalDb retries. Record API inventory and remaining risks/handoff in existing owners rather than deferring propagation completed by PR2.
  - Verify: Cross-check context/decision descriptions against source inventories and task evidence, check owner links and document diff whitespace. Repository-wide final checks remain exclusively in Full validation.
  - Context synchronization: pending

## Open questions

None. Static generic credentials, native async/static enum lifecycle and command/hook dispatch, Result-valued async-once success/failure caching, direct auth/mutation/export awaits, blocking policy, lifetime cleanup, and PR staging are resolved above. T01 inventories source paths and T03 proves cancellation/retry behavior within this fixed architecture; neither leaves an architectural choice deferred.
