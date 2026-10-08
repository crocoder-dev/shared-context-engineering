# Plan: cli-async-turso-persistence-pr2

## Change summary

PR1 owns the application multi-thread Tokio runtime, but shared Turso persistence still constructs a current-thread runtime and synchronously drives native database futures. PR2 replaces that bridge with directly awaited persistence operations, preserving SQL, schema, migrations, transactions, retry/error policies, encryption, WAL, and repository/auth separation. It also removes obsolete runtime-lifetime blocking scopes and the sync storage cleanup guard where ordinary ownership suffices.

The user explicitly revised the split: "PR2 must contain the mechanical async propagation through all direct DB consumers required to remove the runtime bridge. PR3 becomes cleanup/audit rather than propagation." Because RepositoryAgentTraceDb, LocalDb, and AuthDb are aliases of shared generic adapters, the compile-safe migration includes connected storage, credential-store, hook, mutation-trace, export, token-storage, lifecycle, setup/doctor, and command callers as necessary. Only signatures, awaits, required static adapter wiring, lifetime correctness, and test adaptations change; unrelated synchronous service logic stays synchronous.

## Current state after the PR #301 review (2026-10-08)

Re-inspected against branch head `044991d8e7d1117eaa861d4a935844ce05054783` (`remove-tokio-db-runtimes`, clean tree). This section was added by a planning-only update; no production code or tests changed and no new test results are claimed. T01 and T02 history below is unchanged.

**Testing decision.** The large ordinary Rust suites were deliberately deleted during T02 and that decision stands. It is not treated as a mistake and does not require restoring test-count parity, deleted files, or a coverage-equivalence matrix. T02 established the **mechanical async migration**; T03, T06 and T07 establish the **minimum necessary correctness evidence** for the new execution model with a small set of focused, deterministic regression tests (see Minimal safety test set). Acceptance criteria that named deleted suites or filters that now match zero tests are rewritten as invariant-based checks.

**Completed architecture changes (T01, T02).** The Turso runtime bridge is gone; `TursoDb`/`EncryptedTursoDb`, token storage, generic `CredentialStore`, lifecycle, hooks, mutation trace, sync/export and command dispatch use native awaited persistence with static dispatch. The implementation also moved Git snapshot, protected-worktree marker I/O, worktree/adapter lock acquisition, cancellation-shielded ref mutations and guarded-shell supervision onto async/`spawn_blocking` boundaries.

**Confirmed unresolved issues (verified in source at the reviewed head).**
- Transaction exclusivity: `execute_transactional_cas_batch` and `execute_transactional_insert_pair_if_absent` take `&self` and use `Transaction::new_unchecked(&self.core.conn, Immediate)` (`db/mod.rs` ~983–1008, 1117–1128), contradicting T01's exclusive-`&mut` instruction. Owner T03.
- Sync fail-fast: `join_three_to_completion` (`sync/sync.rs` ~323) records the first error but keeps polling every sibling to completion, so a terminal 403 on one stream no longer stops sibling backlogs. Owner T06.
- Guarded-shell descriptors: `LifetimeToken::new` uses plain `pipe` and clears close-on-exec on the writer for its whole lifetime; `spawn_guarded_shell` uses `dup` and clears close-on-exec in the parent before `spawn`, with no `pre_exec` (`external_mutation_guard.rs` ~262–335). Owner T07.
- Credential refresh: `refresh_and_save` awaits the WorkOS renewal and then the save inside the outer `run_with_retry` cancelling timeout (`control_plane.rs` ~342, ~513), with no shield between receiving the replacement token and persisting it. The structural exposure is confirmed; actual token loss depends on WorkOS rotation behavior and is **not** established as production data loss. Owner T06.

**Plausible concurrency risks needing deterministic characterization (not proven defects).**
- Adapter boundary locks: `with_boundary_lock` holds the guard across `operation().await` while state transitions run in separate `spawn_blocking` calls, so a cancelled boundary may release the lock while an older blocking transition is still running. Owner T07.
- Transaction cancellation: Turso 0.8.1 deferred rollback means cancellation after the first write needs one direct test. Owner T03.
- Worktree/marker/Git-ref cancellation ownership and temporary-index cleanup: audited by T04 against existing tests (for example `protected_worktree.rs` cancellation tests); no new per-site tests.

**Deferred maintainability work.** Tracked in the Deferred cleanup ledger below as PR3 handoff; none of it blocks PR2 unless a concrete correctness defect is established.

**Scope reconciliation.** The "Out of scope" line excluding Git/process/filesystem async conversion below is historical: the implemented PR2 includes that work. AC10 and T05 require the final scope to be stated honestly.

## Architecture decisions

PR2 preserves and extends SCE's compile-time/static dispatch model. The entire PR2 propagation closure uses concrete types, generic type parameters, associated types, or static enums for both synchronous and asynchronous dependencies. Every function/module mechanically touched to propagate async DB access converts its existing dynamic dependency-injection and callback seams too. No trait-object DI, boxed futures, `async-trait`, dynamic database/service abstractions, or type-erased command/service futures are permitted inside that closure. This is architectural consistency, not a performance optimization or a repository-wide dyn purge.

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

### Synchronous dependencies within the propagation closure

Pure synchronous dependencies outside the PR2 propagation closure remain out of scope. Any synchronous dependency-injection seam inside a function/module that PR2 must touch is converted to static dispatch as part of the migration. Scope depends on the call graph, not whether a callback returns a future. Pure synchronous operations stay synchronous while their dependency dispatch changes.

Preserve `cli/src/app.rs`'s architecture: `AppRuntime` owns concrete capabilities, `AppContext` borrows concrete capabilities, and `HasLogger`, `HasTelemetry`, `HasFs`, and `HasGit` retain sized associated types. Prefer these existing capability bounds and `GitOps`/`FsOps` in `cli/src/services/capabilities.rs` wherever they already represent the operation; do not duplicate them as callbacks or regress to `Arc<dyn Logger>`, `Arc<dyn FsOps>`, or `Arc<dyn GitOps>`.

Replace all four harness adapters' and ingress conformance's `GitDirResolver = &dyn Fn` aliases with direct generic parameters, borrowed or owned as lifetimes require, conceptually `R: Fn(&str) -> Result<PathBuf>`. Keep the existing absolute-Git-dir resolver behavior and diagnostics: GitOps currently has no matching dedicated operation, so this focused test seam remains a monomorphized closure rather than a new resolver framework. Convert `BashPolicyEvaluator` to `P: Fn(&Path, &str) -> Result<CodexBashPolicyDecision>` with borrowed or owned P; policy fakes remain ordinary closures and policy semantics are unchanged.

Ingress, conformance, model resolution, and doctor repair use direct generic functions and generic future parameters where borrowing permits; use a narrow native async static trait only for a meaningful capability with multiple operations or borrowing requirements that cannot be expressed by a single lifetime-independent future type. Do not invent general callback framework traits. `MutationScopeRepairSeam` becomes fully static async repair with generic repair/future and sized logger types, sharing the ingress operation where natural; no `&dyn Fn`, `&dyn Logger`, `BoxFuture`, or `async-trait` remains. Conformance methods and synchronous rejection/test callbacks also become generic; synchronous test fakes are not exempt.

All migrated logger arguments become `Option<&L>` with sized `L: Logger`, or obtain the logger through `C: HasLogger` and its associated type. Do not add `?Sized` bounds that admit trait objects. Use a concrete no-op logger type or separate logged/unlogged helpers if None inference is awkward, and introduce shared ownership only when required. Preserve logger output and ordering.

Control generic growth: prefer a concrete type for one implementation, then existing capability/associated types, then narrow generic test callbacks, then a small static enum for an existing closed domain. A coherent set of operations may use one narrow static capability, but do not create giant generic plumbing structs. Doctor Git calls/checks use `HasGit`/`GitOps`, filesystem operations use `HasFs`/`FsOps`, and path/config/policy-probe test operations remain focused generic closures with concrete production functions. Preserve doctor Git error-to-None, trimming/empty-output handling and once-per-invocation policy probing exactly.

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
  - Review amendment (2026-10-08): the implemented PR2 also contains Git/process/filesystem async work and its cancellation protections (see Current state), and PR2 now additionally owns transaction isolation and cancellation evidence (T03), credential-rotation and terminal-sync correctness (T06), and adapter-boundary and guard-descriptor safety (T07), proven by a minimal focused test set rather than restored legacy suites. These are merge prerequisites, not PR3 work.
- PR3: Post-migration cleanup and runtime audit: simplify mechanically awkward signatures, remove obsolete compatibility helpers, classify remaining blocking work, and ensure no accidental dynamic dispatch/executor bridge remains. No behavioral redesign or deferred required propagation. Also owns the Deferred cleanup ledger items marked PR3 (cosmetic and maintainability only; never correctness or coverage).
- PR4: OpenTelemetry.

T05 updates existing architecture staging documentation to this roadmap.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the
check that proves it. `/validate` runs these checks; no task in the stack
performs final validation.

- [ ] AC1: Production persistence owns zero Tokio runtimes; build_current_thread_runtime and block_on_isolated are deleted and no replacement synchronous executor drives DB futures.
  - Validate: `nix shell nixpkgs#ripgrep -c rg -n 'build_current_thread_runtime|block_on_isolated|Builder::new_current_thread|runtime\.block_on|Handle::block_on|tokio::runtime::Runtime|block_in_place|try_current|\.block_on\(' cli/src`; classify every match, separating test fixtures from production and confirming only application runtime ownership. Inspect for futures/pollster executors, runtime-containing threads, global runtime singletons, and async DB work inside spawn_blocking.
- [ ] AC2: Both shared Turso adapters, their migration/transaction helpers, and all connected production DB consumers required to compile use awaited native operations on the existing caller runtime.
  - Validate: Inspect the async API/call-chain inventory against source in `cli/src/services/db/`, `agent_trace_db/`, `auth_db/`, `local_db/`, `agent_trace_storage/`, `token_storage.rs`, auth/control-plane callers, exports, mutation-trace, lifecycle, setup/doctor, hooks, sync, and command dispatch. Tests A–C (multi-thread Tokio, real databases) exercise open, execute/query and transactions without nested runtimes; no deleted suite is required.
- [ ] AC3: Transaction integrity — SQL and one-connection scope remain unchanged (BEGIN IMMEDIATE, commit/rollback, idempotent insert pairs, CAS row-count guards, deterministic errors, whole-transaction contention retry) and transaction exclusivity is structurally enforced: exclusive mutable ownership spans an entire transaction (`&mut` receivers and checked `Transaction::new(&mut connection, Immediate)` where the pinned dependency supports it, otherwise a recorded equivalent static proof), so no other operation on the same connection can enter an active transaction. Owner T03.
  - Validate: Inspect transactional signatures and caller chains for exclusive borrows; `nix shell nixpkgs#ripgrep -c rg -n 'new_unchecked' cli/src` finds no unjustified transaction construction; review the SQL/statement-order diff; Test A (runtime test or documented compile-time argument); Quint MBT stays green. The deleted `db/mod.rs`, repository and store suites are not required.
- [ ] AC4: Migration discovery, IDs, numeric ordering, SQL, metadata tracking, per-file transaction behavior, baseline upgrade/rebuild fixtures, and failure behavior match the current implementation.
  - Validate: Migration behavior is verified by source/diff inspection and the retained tests (including the Quint MBT suite); no deleted migration suite is required. `git diff -- cli/build.rs cli/migrations` shows no migration generation/SQL changes; inspect actual migration source paths from the T01 inventory too. Confirm migration execution remains outside open/connect retries, and execute_batch plus separate metadata insertion retain their current boundaries.
- [ ] AC5: Generic retries retain existing attempts/backoff, post-attempt timeout diagnostics, logging and exhaustion behavior; Agent Trace contention retains Busy/BusySnapshot classification, exactly two bounded attempts, jitter 0..=100ms, configured busy timeout/deadline, and pre/post-sleep admission. Owner T03.
  - Validate: Source/diff confirmation that the numeric policies (attempts, backoff, jitter, busy timeout, deadline admission) are unchanged and that DB paths use the non-cancelling `run_with_retry_elapsed` rather than the cancelling `resilience::run_with_retry`; plus Test C, one focused retry/timeout/contention regression (reuse a surviving test if it already covers this). The deleted retry suite is not recreated.
- [ ] AC6: Multiprocess WAL, passive checkpoint policy, lock-overlap behavior and duplicate suppression remain intact. Owner T03.
  - Validate: Test C as the representative real contention verification on independent connections (or processes) at one chosen supported tested load, with no lost or duplicate durable writes and no exhausted retries; passive checkpoint mode and WAL flags confirmed by source inspection. The previous writer/round stress matrices (distinct 2/1000, 3/1000, 4/500; duplicate 2/500, 3/500, 4/500; real hook processes 2/500, 3/500, 4/200) are optional characterization, required only if a specific failure makes them necessary. A filter matching zero tests never counts.
- [ ] AC7: Repository identity/metadata convergence, no-migration hook readiness, no-touch legacy paths, auth encryption/configuration/secret-safe diagnostics, credential behavior, and LocalDb behavior are unchanged. Credential behavior is covered with AC14. Owner T06 for credentials; other items by source/diff inspection.
  - Validate: Inspect AuthCommand's direct awaited load/save/delete paths, unchanged typed `CliError` mappings and secret redaction, and removal of `run_credential_operation`; Test D plus any surviving existing coverage for valid reuse, 401 retry and terminal second-401 (the 28 deleted control-plane tests are not restored).
- [ ] AC8: Transaction cancellation — newly cancellable transactions/writes cannot commit partial state or panic on resource Drop; storage outlives borrowing sync futures; error/cancellation cleanup is safe without a DB-owned runtime. Owner T03.
  - Validate: Test B (cancel after the first write and before commit: no partial committed state, safe connection reuse, a later transaction succeeds, an independent connection sees the correct durable result) plus the compile-time exclusivity argument from AC3. State exact guarantees and upstream Turso 0.8.1 limitations (deferred rollback; a completed commit may already be durable) rather than assuming immediate Drop rollback. No per-helper or per-await-point tests.
- [ ] AC9: Blocking audit — zero DB-runtime-related block_in_place, zero blocking SyncStorageGuard Drop, and zero token-storage DB spawn_blocking. Every remaining production spawn_blocking/block_in_place has an individually documented genuinely synchronous/blocking external API reason independent of DB runtime ownership. **Review amendment (owner T04, after T06/T07):** the audit also covers encryption-key lookup, adapter state transactions, adapter boundary-lock and worktree-lock acquisition, external-taint marker persistence/clearing, cancellation-shielded Git ref mutations, guarded-shell supervision and temporary-index destructor cleanup, recording per site the six ownership/cancellation/shutdown answers listed in T04; `spawn_blocking` that provides necessary cancellation safety is retained, not mechanically removed.
  - Validate: Audit `cli/src/services/command_registry.rs` Setup/Doctor/Hooks sites and `cli/src/services/sync/sync.rs` storage construction/SyncStorageGuard Drop against `nix shell nixpkgs#ripgrep -c rg -n 'block_in_place|spawn_blocking|SyncStorageGuard' cli/src` and Source audit. Source inspection plus the focused tests owned by T03/T06/T07 and existing tests (for example `protected_worktree.rs` cancellation tests) cover the affected paths; no separate lifetime test matrix is required. Ordinary synchronous filesystem/process work is assessed independently.
- [ ] AC10: The expanded PR2 scope consists only of required mechanical DB propagation and lifetime/cancellation correctness; it introduces no unrelated behavior, schema/protocol change, dynamic DB framework, artificial locking layer, or OpenTelemetry work. **Review amendment — scope honesty (owner T05):** the original wording "Check no new async filesystem/process conversion" is not satisfied by the implementation, which includes Git/process/filesystem async work. The final PR scope is explicitly reconciled with that implemented work (what was migrated, why it was required, which cancellation protections it carries) in the plan, the PR description and context, rather than pretending it did not occur; still no unrelated behavior, schema/protocol change, dynamic DB framework or OpenTelemetry work.
  - Validate: Inspect changed files and `nix shell nixpkgs#ripgrep -c rg -n 'BoxFuture|async_trait|dyn .*Db|Arc<Mutex|spawn_blocking|tokio::spawn|opentelemetry|tracing-opentelemetry|OTLP' cli/src cli/Cargo.toml`; classify new versus pre-existing occurrences. Check no new async filesystem/process conversion, connection pool/cache, exporter, span, telemetry auth or SQL cleanup is included.
- [ ] AC11: **Documentation (review amendment, owner T05):** architecture, patterns and ADRs describe the verified implementation and claim no guarantee stronger than the code and tests establish (including the stale synchronous process/filesystem guidance in `context/patterns.md` and the blocking-worker enumeration in `context/architecture.md`). Durable context accurately describes application runtime → static async commands → static async services/lifecycle/hooks → static async persistence → Turso, the revised PR2/PR3 split, removed bridge symbols/workarounds, remaining synchronous work, and justified blocking boundaries.
  - Validate: Inspect the existing context owners listed below and architectural decision update/supersession; reconcile recorded drift with source without creating duplicate architecture documents. The final review report lists before/after architecture, deleted symbols, async APIs, removed/retained blocking scopes and reasons, focused/full results, remaining risks, PR3 cleanup/runtime audit and PR4 OpenTelemetry handoff, and no telemetry implementation in PR2.
- [ ] AC12: Validation — all newly required focused tests (A–G), existing relevant tests, the Quint checks and final CI validation pass at the final PR head (not the reviewed head `044991d8`).
  - Validate: All commands under Full validation succeed, with actual outcomes recorded in the validation report; compilation alone is insufficient. Historical tests that are no longer part of the codebase are not demanded.
- [ ] AC13: The entire PR2 propagation closure uses static dispatch for both synchronous and asynchronous seams. Any function/module mechanically changed by PR2 contains no dynamic dependency-injection or future-dispatch seam. Dependencies are concrete types, generic parameters, associated types, or static enums.
  - Validate: Run all Source audit searches and inspect changed/migrated production files, signatures, constructors, callback aliases and bounds (including qualified Logger names and `?Sized`). Require zero relevant `dyn CredentialStore`, `dyn ServiceLifecycle`, `dyn .*Db`, `&dyn Fn`, `Box<dyn Fn`, `Arc<dyn Fn`, `&dyn Logger`, `Arc<dyn Logger`, `BoxFuture`, `Pin<Box<dyn Future`, or `async_trait` occurrences inside those files. Also exclude dyn Database/Repository/Persistence/Resolver/Evaluator/Seam and boxed command/service futures. Classify every occurrence, including pre-existing ones, against the exact paths/reasons in T01; synchronous DI and migrated test/conformance fakes receive no exception. Untouched unrelated modules may retain individually classified legacy dispatch.
- [ ] AC14: Credential storage — AuthenticatedControlPlaneClient<S> uses S: CredentialStore with native async load/save and concrete async fake injection; SystemCredentialStore directly awaits production token storage. No credential DB operation runs in spawn_blocking; refresh/single-flight/401 behavior remains unchanged. A received replacement refresh token cannot be lost to caller cancellation or the outer `/state` timeout before durable persistence is attempted; the single-flight guard spans refresh and persistence; the remote-operation ambiguity and crash limitations are documented. Owner T06.
  - Validate: Inspect generic client/store implementations and connected sync callers; Test D (cancelled refresh cannot lose persisted credentials, with single-flight where feasible) plus existing coverage where available. The deleted control-plane suite is not restored.
- [ ] AC15: Lifecycle — native async diagnose/fix/setup propagate through Doctor/Setup and LifecycleProvider's static enum matches to concrete providers and async DB persistence. Pure Config/Hooks work stays synchronous internally; no boxed providers/futures or pure-work spawn_blocking.
  - Validate: Inspect every provider and both command chains; run any surviving lifecycle/setup/doctor tests; otherwise compilation and inspection suffice.
- [ ] AC16: Auth token storage — async Result-valued OnceCell initialization caches both completed success and completed failure, never implicitly retrying a returned initialization error. load_tokens/save_tokens/delete_tokens are directly async; run_credential_operation is removed.
  - Validate: Check locked Tokio API/source (recorded in T01) and the Result-valued `get_or_init` usage so a cached failure never re-invokes the initializer; no new test unless that cannot be established by inspection. Inspect cancellation-before-completion separately and preserve diagnostics/secret safety.
- [ ] AC17: End-to-end executor invariant — No production path from the application Tokio runtime to a Turso future crosses a synchronous executor boundary.
  - Validate: Trace every DB caller class (credentials/auth, lifecycle/setup/doctor, hooks/mutation trace, sync/export, repository/local persistence) from static RuntimeCommand dispatch to Turso. Source audit plus inspection must exclude block_on substitutes, runtime-containing workers, boxed callback executors, and DB futures inside spawn_blocking/block_in_place. Foreground hooks await required persistence; only the existing detached post-commit auto-sync child remains authorized.

The criteria below were added by the 2026-10-08 review update. AC1–AC17 are retained and amended only where they named deleted suites or zero-match filters; safety requirements are not weakened.

- [ ] AC18: **Minimal safety regression coverage (owners T03, T06, T07).** The required evidence is the smallest set of tests that proves distinct, material correctness invariants — Tests A–G in the Minimal safety test set (guidance: roughly 6–10 new tests, fewer where existing tests, types or Quint suffice). Tests execute and carry meaningful assertions. No test-count parity, percentage-based coverage threshold, or automatic restoration of deleted tests. A command that matches zero tests never counts as passing.
  - Validate: For each of Tests A–G, `<filter> -- --list` shows the intended test (or the test is explicitly replaced by a documented compile-time argument) and the run passes; each test's completion record states the regression it catches that existing tests, types and Quint do not.
- [ ] AC19: **Boundary cancellation (owner T07).** Cancellation cannot permit an older state operation to invalidate or overtake a newer protected boundary.
  - Validate: Test F plus inspection of the chosen lock-ownership design against source.
- [ ] AC20: **FD inheritance (owner T07).** Unrelated spawned subprocesses cannot inherit the guarded shell's protected descriptors; the intended guarded child does.
  - Validate: Test G plus inspection of descriptor creation/duplication and the post-fork child setup.
- [ ] AC21: **Terminal sync (owner T06).** Terminal stream errors prevent unnecessary additional batches without cancelling required credential persistence.
  - Validate: Test E.
- [ ] AC22: **Deferred findings are tracked (owner T05).** Every lower-priority finding appears in the Deferred cleanup ledger as a PR3 handoff or documented limitation, and no cosmetic cleanup is mixed into T03–T07.
  - Validate: Inspect the ledger against the final diff and T05's context records.

### Acceptance criteria ownership map

| AC | Owning task(s) | Primary verification |
| --- | --- | --- |
| AC1, AC17 | T04 | Source audit; caller trace |
| AC2, AC13, AC15, AC16 | T02 (done); re-checked by T04 | Source audit; inspection |
| AC3, AC8 | T03 | Exclusive-borrow inspection; Tests A, B |
| AC4 | T02 (done); T03 inspection | `git diff -- cli/build.rs cli/migrations`; Quint/flake check |
| AC5, AC6 | T03 | Test C; numeric-policy source diff |
| AC7, AC14 | T06 | Test D; inspection |
| AC9 | T04 | Source audit + retained-site table |
| AC10, AC11, AC22 | T05 | Scope reconciliation; context/ADR inspection; ledger |
| AC12 | `/validate` | Full validation |
| AC18 | T03, T06, T07 | Tests A–G |
| AC19, AC20 | T07 | Tests F, G |
| AC21 | T06 | Test E |

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
  '&dyn Fn|Box<dyn Fn|Arc<dyn Fn|&dyn Logger|Arc<dyn Logger|dyn .*Resolver|dyn .*Evaluator|dyn .*Seam' \
  cli/src
nix shell nixpkgs#ripgrep -c rg -n \
  'CredentialStore|ServiceLifecycle|LifecycleProvider|AUTH_DB|run_credential_operation|SyncStorageGuard' \
  cli/src
```

Require zero dynamic dependency/future dispatch in the migrated production closure, including old synchronous DI and credential trait objects. The final source report assigns every dynamic seam match an exact path/site and one of: `removed by PR2`, `unrelated untouched legacy seam`, `test-only unrelated seam`; T01 records the current classification below. Supplement with `nix shell nixpkgs#ripgrep -c rg -n '\bdyn\b|\?Sized|SyncFuture' cli/src` to catch qualified Logger spellings, lifetime-bearing aliases and FnMut. Distinguish library-mandated diagnostic protocol signatures from dependency injection. Classify **every** remaining production spawn_blocking/block_in_place, including synchronous OS keyring work. Also inspect futures/pollster executors, global runtime singletons, worker-thread runtime ownership, dyn Database/Repository/Persistence, type-erased callbacks and boxed command/service futures.

### Full validation

Repository-wide checks `/validate` runs after the last task, regardless of
which criterion they map to.

- `nix flake check`
- `nix build .#ci-checks`
- `git diff --check`
- Zero-match rule: every focused test named by AC18 is confirmed to exist with `-- --list` before its run counts; a command matching zero tests never counts as passing.

### Context sync

- `context/architecture.md`, `context/overview.md`, and `context/patterns.md`: runtime ownership and accurate remaining synchronous work.
- `context/sce/shared-turso-db.md`: async APIs, retry distinctions, transaction/cancellation and connection contract, unchanged WAL/checkpoint/migrations, hook readiness drift repair.
- `context/sce/agent-trace-db.md`, `context/sce/auth-db.md`, `context/sce/local-db.md`: inherited async APIs, unchanged separation/encryption, current migration 006, configured retry defaults.
- `context/cli/agent-trace-storage.md`, `context/cli/service-lifecycle.md`, existing auth/export/mutation-trace/hook/sync owners discovered in T01: required await propagation and corrected repository/no-migration lifecycle descriptions.
- `context/sce/agent-trace-db-write-contention-evidence.md`: measured regression evidence with the same supported-load acceptance contract.
- Review-update additions (T05): `context/architecture.md` and `context/patterns.md` (actual PR2 scope, blocking-worker enumeration, cancellation-protection model, corrected synchronous process/filesystem guidance); a successor ADR to `context/decisions/2026-10-07-directly-awaited-turso-persistence.md` if its accepted decision text changes (its history stays immutable); owners for exclusive transaction ownership, credential refresh, adapter lifecycle protection, Git ref-mutation shielding, worktree lock leases, marker ownership, temporary-index limitations, guard FD inheritance and sync early-termination policy.
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
- **Out of scope:** OpenTelemetry, exporters, new spans; SQL/schema cleanup; repository/sync/hook redesign; batching, pools, caches, unrelated generic interfaces beyond the required static seam conversions; unrelated bugs, pure transformations, parsing/rendering, Git/process/filesystem async conversions, CLI schema and Quint logic. *(Historical wording; superseded 2026-10-08: the implementation already contains Git/process/filesystem async work, which AC10/T05 reconcile. T03 may make the smallest production changes necessary to establish transaction exclusivity and cancellation correctness; unrelated architecture redesigns remain out of scope.)*
- **Constraints:** pinned Turso 0.8.1 APIs and static dispatch throughout commands, services, lifecycle, hooks, callbacks, credentials, and persistence as specified in Architecture decisions. No DB-owned/per-operation/global executor, Handle::block_on, futures/pollster block_on, async DB work in spawn_blocking/block_in_place, or runtime-containing spawned thread. Every retained production blocking boundary requires an individually documented synchronous external API reason. Tests alone may construct runtimes for genuinely synchronous fixtures; async tests use the existing test runtime and prefer multi-thread Tokio.
- **Constraints:** retain exact SQL, retry classification/count/duration/jitter, configured defaults, elapsed-time diagnostics, transaction scope, error taxonomy/messages, secret-safe behavior, encryption and OS keyring fallback, repository/auth separation and migration ordering. Mechanical async sleeps are documented individually. Preserve existing connection serialization; do not add Arc<Mutex>, boxed async APIs, dynamic traits or artificial Send/Sync bounds.
- **Constraints:** use Nix and repository-supported Cargo wrapper. Generated target trees remain absent. No canonical generation inputs are expected to change; if a mechanically required change touches them, add `nix run .#pkl-check-generated` to relevant checks and full validation without generating committed trees.
- **Non-goal:** making unrelated synchronous service logic async or designing future telemetry integration. The required native async lifecycle seam may wrap existing pure synchronous provider work without changing that work.

## Assumptions

- Core async API changes and their connected caller adaptations form one coherent compile-safe migration commit because aliases/direct calls provide no independent synchronous adapter seam; task-local editing may be sequential, but no broken or bridge-bearing intermediate commit is planned.
- PR1 application runtime is complete. PR2 completes async persistence and its mechanically required caller closure, static credential/lifecycle seams, and obsolete DB-runtime blocking removal. PR3 is post-migration cleanup/runtime audit; PR4 is OpenTelemetry. This plan implements neither follow-up and leaves no required propagation or executor bridge for PR3.
- Existing dependencies, schema and test assertions are authoritative. Any discovered unrelated bug is reported separately.

## T01 source inventory (2026-10-07)

Audited baseline: `9ceff8414613a4a619cf505ae9ac0e3eb46db69f`, clean staged/unstaged/untracked state. This is the pre-migration inventory; target signatures below are instructions for T02, not claims that async persistence already exists. All paths and line numbers refer to that baseline.

### Persistence APIs and unchanged policies

- `cli/src/main.rs:12` owns the application multi-thread runtime through `#[tokio::main]`; `cli/src/app.rs` directly awaits static command dispatch. `cli/src/services/command_registry.rs:26` owns `RuntimeCommand`. Persistence currently owns additional runtimes in `cli/src/services/db/mod.rs:457` (`TursoConnectionCore<M>` contains one `turso::Connection`, one `tokio::runtime::Runtime`, and `PhantomData<fn() -> M>`). `build_current_thread_runtime` at 178 constructs them; `block_on_isolated` at 198 either blocks directly or starts a scoped OS thread to drive the DB runtime when `Handle::try_current()` succeeds. Delete the field, constructor and bridge; retain concrete connection ownership and static `DbSpec` metadata.
- `cli/src/services/db/mod.rs:893–1454`: await `TursoDb::{new,new_at,open_without_migrations,open_without_migrations_at,execute,execute_idempotent_write,query,query_values,query_map,execute_transactional_insert_pair_if_absent,execute_transactional_cas_batch,run_migrations,passive_checkpoint,migration_metadata_problems,ensure_schema_ready}`. `query` returns native `Rows`, whose subsequent `next()` is async; `query_values` fetches owned column/value collections; `query_map` fetches all native rows inside the retry, then runs synchronous mapping outside it. Keep parameter conversion before retries and clone the same owned params per attempt. `TransactionStatement::{new,expect_rows_affected}`, row mappers, SQL builders, config/path resolution and filesystem health helpers remain synchronous.
- `cli/src/services/db/mod.rs:1475–1651`: await `EncryptedTursoDb::{new,execute,query,query_map,run_migrations}` and shared core migrations. Preserve key acquisition before encrypted open, `experimental_encryption(true)`, `aegis256`, path-qualified secret-safe failures, and the absence of a busy handler on encrypted connections. Unlike the unencrypted builder, the encrypted builder currently does **not** set `experimental_multiprocess_wal(true)`; do not silently change its flags.
- Aliases stay static: `cli/src/services/agent_trace_db/repository.rs:114` has `RepositoryAgentTraceDb = TursoDb<RepositoryAgentTraceDbSpec>`; `cli/src/services/local_db/mod.rs:36` has `LocalDb = TursoDb<LocalDbSpec>`; `cli/src/services/auth_db/mod.rs:37` has `AuthDb = EncryptedTursoDb<AuthDbSpec>`. LocalDb has no service tables/migrations, but shared initialization still creates `__sce_migrations`. No dynamic DB trait exists.
- `cli/src/services/agent_trace_db/repository.rs:120–329`: await hook opens/readiness, narrow metadata repair and its private object queries, metadata select/initialize/claim, diff/intersection/Agent Trace/model-state/message/part/conversation insert/read methods. `cli/src/services/agent_trace_db/mod.rs:292–539` contains the generic typed SQL wrappers `insert_diff_trace_with`, `insert_post_commit_patch_intersection_with`, `insert_agent_trace_with`, `upsert_claude_model_state_with`, `claude_model_state_by_session_and_agent_with`, `insert_message[s]_with`, `insert_part[s]_with`, `insert_conversation_text_event_with`, `recent_diff_trace_patches_with`; all DB work becomes awaited, while parsing/mapping stays synchronous. Preserve multi-row statement construction and idempotent conversation pairing.
- `cli/src/services/db/mod.rs:290,412,1146,1281`: the two transaction primitives use `Transaction::new_unchecked(&conn, Immediate)` on the **same logical connection**. Insert pair: existence check → first insert → optional injected failure → second insert → commit (including no-op duplicate). CAS: guard rows `0` means false/no body, `1` permits ordered body, other counts are deterministic failure; optional body row expectations are enforced. Body failures explicitly await rollback, ignoring rollback errors as today; commit failures remain classified. Retry unit is the whole acquisition/body/commit, never a single constituent statement. Preserve non-Agent-Trace CAS deterministic outcomes carried outside generic retry.
- `cli/src/services/resilience.rs:103` owns generic `run_with_retry_sync`: success wins even after the configured elapsed threshold; only a returned error is labelled `attempt N exceeded ... and failed`. Its async DB counterpart must await a generic `FnMut(u32) -> Fut`, measure elapsed after completion and await backoff without `tokio::time::timeout`. Existing `run_with_retry` at 36 cancels attempts and remains for HTTP; it is not the DB substitute. Open fallback is 3 attempts / 1000ms diagnostic threshold / 25..200ms capped exponential backoff; query fallback is 5 / 200ms / 25..100ms. Actual open sleeps are 25,50ms and query sleeps 25,50,100,100ms. Config overrides are resolved for **all three** aliases by `resolve_connection_open_retry_policy` / `resolve_query_retry_policy` at 477/494 via `config::{get_database_retry_config,DATABASE_RETRY_CONFIG}`; do not change that synchronous config cache.
- `cli/src/services/db/mod.rs:525–746`: Agent Trace busy timeout defaults to 1000ms and contention admission deadline to 2250ms (configurable); other specs resolve zero. Only explicit idempotent writes and both transaction primitives opt into typed Busy/BusySnapshot retries, at most 2 attempts, full jitter `0..=100ms`. Before sleeping require positive remaining deadline and `remaining >= backoff + busy_timeout`; afterward require `remaining >= busy_timeout`. No cancellation deadline, generic retry wrapping, or deterministic-error retry is added. Exhaustion preserves exact error fields and one `sce.agent_trace_db.contention_exhausted` warning. Generic reads/execute/checkpoint remain on generic retry.
- Sleep conversions needed for DB propagation: `resilience.rs:159` generic retry backoff, `db/mod.rs:634` contention backoff, `agent_trace_storage/mod.rs:181` initialization backoff (20 attempts / 100ms between failed initializations). Preserve duration, placement and replay unit; adapt the injected sleep/clock/backoff/attempt callbacks statically. Do not retain `rand::ThreadRng` across an await when a caller requires a Send future: draw before awaiting and reacquire for later draws; the production full-jitter distribution and seeded-test reproducibility must remain. OS-lock polling sleeps in `hooks/mutation_scope_lock.rs`, `hooks/claude_mutation_scope/state.rs`, and `mutation_trace/runtime/worktree_lock.rs` are independent synchronous work, not DB retry conversions.
- `TursoDb::open_without_migrations_at` at 935 sets multiprocess WAL, builds/connects inside open retry, then installs the connection busy timeout. `new`/`new_at` run migrations **after** open retry. `passive_checkpoint` at 1355 queries/fully consumes `PRAGMA wal_checkpoint(PASSIVE)` under generic retry; retain passive mode and required foreground completion.

### Migration sources and boundaries

- Canonical SQL is in `cli/migrations/`, not `config/lib` or generated target trees. `cli/build.rs:545,623–694` discovers immediate database directories and immediate `.sql` files, validates numeric prefix before `_`, uses filename stem IDs, sorts by numeric prefix then full ID, and emits `OUT_DIR/generated_migrations.rs` including staged `OUT_DIR/static/migrations/...`. `cli/src/main.rs:5` includes that manifest. Producer staging is owned by `scripts/produce-cli-generated-input.sh` and build.rs; leave generation unchanged.
- Repository list: `cli/migrations/agent-trace-repository/001_repository_schema.sql`, `002_repository_source_instance_id.sql`, `003_claude_model_state.sql`, `004_mutation_trace_protocol.sql`, `005_mutation_scope_provenance.sql`, `006_mutation_trace_health_invariant.sql`. Auth list: `cli/migrations/auth/001_create_auth_tokens.sql`, `002_create_auth_credentials_updated_at_trigger.sql`. LocalDb returns `&[]` rather than an embedded list.
- `cli/src/services/db/mod.rs:211–286` ensures `__sce_migrations`, checks each ID, awaits `execute_batch(sql)`, then separately inserts metadata. There is no shared per-file transaction wrapper: preserve SQL-owned boundaries (notably the transaction/table rebuild in 006), multi-statement behavior, separate metadata insertion and failure diagnostics. Migrations are outside generic open/query retries; do not “fix” this by transactional_batch or wrapping metadata in a new transaction.
- `cli/src/services/agent_trace_storage/mod.rs:150–212`: lifecycle/setup open first tries no-migration open, narrow metadata repair/readiness and repository metadata; falls back to migrated construction with bounded initialization recovery. Hook open at 199 never falls back to migrations. `repository.rs:134` repair verifies all required tables before recording missing baseline metadata, then checks readiness. Initial metadata insertion and guarded source-instance claim converge across independent first openers. Preserve no-touch legacy DB paths and repository/auth separation.

### Required callers and dispatch conversions

| Boundary and exact source (relative to `cli/src/services/`) | Current dispatch | Target dispatch | Fixed T02 target mechanism |
| --- | --- | --- | --- |
| `db/mod.rs` core and aliases above | Concrete/generic, synchronous public APIs driving native futures | Static | Concrete/generic native async APIs; no executor boundary |
| `agent_trace_storage/mod.rs:51–212`, including `open_storage_with` at 134 | Concrete storage plus synchronous generic opener callback | Static | Async resolve/open helpers; concrete awaited opener selected statically; where callback retained use generic future bounds and fixed borrowed lifetimes or owned path/ID inputs |
| `token_storage.rs:16–18,73,109,148` | Static `OnceLock<Result<AuthDb,String>>`, sync load/save/delete | Static | Static Result-valued Tokio OnceCell; async initializer and direct async token operations |
| `agent_trace_sync/control_plane.rs:227–255,293–325,523–550` | Dynamic sync `CredentialStore`, `Box<dyn CredentialStore>` constructor, `Arc<dyn CredentialStore>` field, DB blocking workers | Static | Native async `CredentialStore`, `AuthenticatedControlPlaneClient<S = SystemCredentialStore>` with `S: CredentialStore`; concrete default constructor and generic injected stores; direct awaited load/save |
| `auth_command/mod.rs:64–208,438–449`, `auth_command/command.rs` | Static async command calling blocking `run_credential_operation` | Static | Direct load/login/whoami, save/refresh/device-login, delete/logout awaits; delete worker helper and worker-only tests; retain `auth_storage_error` typed mappings and redaction |
| `lifecycle.rs:165–256`, `config/lifecycle.rs`, `{local_db,auth_db,agent_trace_db,hooks}/lifecycle.rs` | Static generic trait and static enum | Static | Native async diagnose/fix/setup with unchanged result types; enum awaits concrete providers; id/order remain synchronous, pure provider work stays synchronous internally |
| `doctor/{command.rs,mod.rs,inspect.rs,fixes.rs}`, `setup/command.rs` | Concrete sync services; provider calls and Doctor mutation repair | Static | Async orchestration from static commands through lifecycle and DB-capable repair; replace iterator closures that require await with ordered loops |
| `hooks/{command.rs,mod.rs,runtime.rs,commit_hooks.rs,diff_trace.rs,conversation_trace.rs,claude_model_state.rs}`, `hooks/codex/{mod.rs,apply_patch/mod.rs,stop.rs,user_prompt_submit.rs}` | Static subcommand/event matches; concrete DBs plus synchronous generic persistence/query callbacks | Static | Native async DB-capable handlers and static awaits through command; generic future callbacks for retained query/persist seams; pure payload parsing/clock/Git/snapshot/render logic stays synchronous |
| `hooks/{claude,codex,opencode,pi}_mutation_scope/{mod.rs,lifecycle.rs,health.rs}` (where present), `hooks/mutation_scope_ingress_conformance.rs:12,29–34` | Dynamic borrowed `IngressSeam = &dyn Fn(...)->Result<String>`; synchronous adapter state machines | Static | Generic ingress/future parameters and sized generic logger; direct generic functions preferred, narrow native async static capability only for meaningful operations/borrowed lifetimes; await all DB ingress/recovery/repair and convert synchronous conformance fakes too |
| `doctor/inspect.rs:233–293` `MutationScopeRepairSeam` | Dynamic sync callback with `Option<&dyn Logger>` to mutation-scope ingress | Static | Generic async repair/future and sized `L: Logger`, sharing static ingress operation; borrowed or owned repair as lifetimes require; preserve target/order/final-health rendering |
| `hooks/claude_mutation_scope/lifecycle.rs:34–35` `ClaudeModelStateResolver` | Dynamic sync callback doing model-state DB read | Static | Generic async resolver/future with concrete production/fake implementations; native async static method when borrowed lifetimes require it; no boxed resolver |
| `hooks/{claude,codex,opencode,pi}_mutation_scope/lifecycle.rs:18,23,65,62` respectively; `hooks/mutation_scope_ingress_conformance.rs:10` `GitDirResolver` and adapter conformance implementations in each harness `tests.rs` | Dynamic synchronous `&dyn Fn(&str) -> Result<PathBuf>` | Static | Direct generic `R: Fn(&str) -> Result<PathBuf>`, borrowed `&R` or owned R; ordinary monomorphized closure fakes; retain existing resolver/diagnostics rather than duplicating GitOps with a new trait |
| `hooks/codex_mutation_scope/lifecycle.rs:27–28` `BashPolicyEvaluator`, including preflight at 349 | Dynamic synchronous `&dyn Fn(&Path, &str) -> Result<CodexBashPolicyDecision>` | Static | Direct borrowed/owned generic P with the same Fn signature; concrete production function and generic policy fakes, unchanged decisions |
| `doctor/mod.rs:55–56` `run_git_command`, `check_git_available` | Dynamic synchronous Git callbacks | Static | Existing `HasGit` associated type / `GitOps::run_command` and `is_available`; retain error-to-None, trimming and empty-output mapping; filesystem work naturally uses `HasFs`/`FsOps`, not duplicate callbacks |
| `doctor/mod.rs:57–59,66` `resolve_state_root`, `resolve_global_config_path`, `validate_config_file`, `probe_codex_hook_policy`; dependent inspection/filesystem helpers | Dynamic synchronous path/config/probe callbacks; concrete filesystem calls | Static | Focused generic test closures with concrete production functions for operations not represented by existing capabilities; existing `HasFs`/`FsOps` for matching filesystem operations; preserve probe-once/order semantics without a giant callback plumbing struct |
| `hooks/{mod.rs,commit_hooks.rs,diff_trace.rs,conversation_trace.rs,claude_model_state.rs,mutation_scope.rs}`, `hooks/codex/{mod.rs,apply_patch/mod.rs}`, all four mutation-scope `lifecycle.rs` and connected health/test fakes, `doctor/inspect.rs:234,260,4784` | Dynamic `Option<&dyn Logger>` (including qualified Logger spellings and closure arguments) | Static | Existing `HasLogger` associated type at context entrypoints; sized generic `L: Logger` and `Option<&L>` at leaf/ingress/repair helpers; concrete logger/unlogged helper for None inference, no trait-object-permitting `?Sized` or unnecessary Arc |
| `mutation_trace/runtime/coordinator.rs:722–748` `HookedFailingCapture` | Dynamic test-only `Box<dyn FnMut>` snapshot fault hook in a migrated module | Static | Generic `H: FnMut()` field/constructor/implementation for pure fault injection; DB-bearing fake operations move to static async test seams; test-only placement does not exempt migrated DI |
| `hooks/mutation_scope.rs:285–315,528–590` | Generic sync open/coordinate/abandon callbacks and guard protocol | Static | Await static generic providers (`FnOnce() -> Fut`), coordinator/abandon and guard DB finalization; retained callbacks with borrowed structured payloads use native async generic seam methods, avoiding impossible lifetime-independent future bounds |
| `mutation_trace/store.rs:530–950` | Concrete borrowed store over RepositoryAgentTraceDb | Static | Store constructor/encoding/transition computation stay sync; initialize/register/load/page/provenance/tree-root/commit queries become async; transactional store holds an exclusive mutable DB borrow and commit takes a mutable store borrow, awaiting the existing CAS primitive |
| `mutation_trace/runtime/{coordinator.rs,scope_runtime.rs,ref_reconciliation.rs}` | Generic synchronous DB providers and injected after-load/recovery hooks; static `SnapshotCapture` | Static | Async entrypoints and DB error/recovery/commit helpers; `P: FnOnce() -> Fut`, concrete futures; DB-using injected hooks get async generic bounds, pure snapshot methods remain sync |
| `mutation_trace/runtime/mutation_attribution.rs:28–58,118–434` | Generic `MutationEventPageSource` and synchronous Git-only `TreeReadSource`, with trait-object-admitting `?Sized` bounds | Static | Native async page/provenance trait methods and awaited window/project/post-commit attribution; sized generic P/R, remove DI `?Sized` bounds, keep R tree operations synchronous |
| `mutation_trace/runtime/external_mutation_guard.rs:530–807` | Concrete `ArmedExternalMutationGuard<P>` with generic deferred DB opener | Static | Keep static P plus future bounds; `exec` and run helpers await final `coordinate_on_held_worktree` after existing synchronous process supervision. No DB future in control-reader thread or blocking worker; preserve lock/taint ownership and descendant lifetime protocol |
| `agent_trace_export/mod.rs:146–267` | Concrete borrowed reader with four sync DB reads | Static | Async `read_{messages,parts,diff_traces,agent_traces}_after`; constructor, validation and row mappers remain sync |
| `agent_trace_sync/mod.rs:162–179` `SyncFuture`, `sync/sync.rs:346–538` stream callbacks | Dynamic future dispatch: generic closures returning **type-erased** `Pin<Box<dyn Future>>`; sync reads wrapped in ready futures | Static | Remove SyncFuture type erasure from the migrated engine. Use a native async statically generic stream-operations seam with associated row type and concrete per-stream adapters; read/ingest/refresh methods can borrow rows through native async signatures. Preserve the existing cursor algorithm and test fake operations. No boxed callbacks or executor substitute |
| `sync/sync.rs` client parameters/test factories, `sync/command.rs`, `command_registry.rs:78–109` | Concrete async sync; static commands with DB compatibility scopes | Static | Propagate generic credential-store type through injected sync clients; await async storage/export/commands directly. Pin concurrent concrete futures without type erasure (stack pinning suffices); no Send/'static/spawn requirement for borrowed progress/writers |

No dynamic DI seam in the PR2 propagation closure remains a deferred design choice: ingress/repair/model-resolution become static generic async seams, Git-dir/policy/conformance callbacks become generic, doctor reuses existing capabilities and focused generic callbacks, logger arguments use sized generics/HasLogger, SyncFuture becomes static generic stream operations, credentials become generic S, and lifecycle/commands remain enums. Pure synchronous dependencies outside the PR2 propagation closure remain out of scope. Any synchronous dependency-injection seam inside a function/module that PR2 must touch is converted to static dispatch as part of the migration. The already generic HRTB insert callbacks in `hooks/commit_hooks.rs:247,458` need borrowed native async seam methods rather than boxed futures; local snapshot/trace payloads must outlive the awaited insert.

### Dynamic seam audit classification

The two dynamic-seam Source audit commands plus the supplemental dyn search cover every current match as follows. Line references describe the T01 baseline; re-run searches after migration and record exact resulting sites. No matching DI/future seam is preserved inside a migrated production module.

| Current match sites (relative to `cli/src/services/`) | Classification | Reason / T02 disposition |
| --- | --- | --- |
| All matches in `hooks/{mod.rs,commit_hooks.rs,diff_trace.rs,conversation_trace.rs,claude_model_state.rs,mutation_scope.rs}`, `hooks/codex/{mod.rs,apply_patch/mod.rs}`, all four harness `lifecycle.rs`, `doctor/{mod.rs,inspect.rs}` | removed by PR2 | DB-backed caller modules must be mechanically changed; convert every resolver, ingress, policy, repair, doctor dependency and logger DI signature/alias, including qualified names and inline fakes, by the table above |
| All matches in `hooks/mutation_scope_ingress_conformance.rs`, all four harness `tests.rs`, Claude/OpenCode/Pi `health.rs`, Pi `lifecycle_tests.rs` | removed by PR2 | Shared conformance helpers and adapter/health test callbacks exercise migrated ingress; convert aliases, fake closure logger parameters and trait-object-admitting callback bounds even when a fake is synchronous |
| `agent_trace_sync/control_plane.rs:255,313` | removed by PR2 | Replace Arc/Box credential trait objects with generic S and native async concrete stores |
| `agent_trace_sync/mod.rs:162` | removed by PR2 | Delete SyncFuture type erasure in favor of static stream operations |
| `mutation_trace/runtime/coordinator.rs:724` | removed by PR2 | Generic fault hook; this test DI sits in a mechanically migrated module |
| `parse/command_runtime.rs:10` `Option<&dyn LoggerTrait>` | unrelated untouched legacy seam | Argument parsing only constructs a RuntimeCommand before execution (`cli/src/app.rs:408`); it never runs persistence or calls async services, and its signature/module needs no DB propagation change. Leave this exact seam for later cleanup; if PR2 mechanically changes this module, convert it too |

No `test-only unrelated seam` is matched by the two requested DI/future audits at this baseline. The supplemental whole-word dyn audit also finds `setup/mod.rs:61` and `setup/install.rs:397` std::error::Error::source signatures (classify as `unrelated untouched legacy seam` at the signature level: mandated error-reporting protocol, not injected dependencies or future dispatch), and `db/mod.rs:2613` tracing::field::Visit::record_debug's `&dyn std::fmt::Debug` (classify as `test-only unrelated seam`: mandated diagnostic visitor signature, not test DI). These protocol occurrences remain even if their surrounding modules migrate; they do not exempt any DI seam. Non-code substring hits in `mutation_trace/types.rs:168` and `agent_trace_sync/test_http_server.rs:103` are prose, not dispatch. No dyn ServiceLifecycle/DB/Resolver/Evaluator/Seam trait object, BoxFuture or async_trait implementation was found. Final validation must classify any newly discovered match individually using the same rule.

Supplemental `?Sized` classification: `mutation_trace/runtime/mutation_attribution.rs:128–129,177,215,257,328,343,370` and `hooks/codex_mutation_scope/tests.rs:749–750` are trait-object-admitting DI bounds to remove by PR2 in favor of sized generics. `bash_policy.rs:617` is an unrelated untouched serialization helper's `T: ?Sized + Serialize`, not DI or future dispatch; leave it unchanged.

The conversion changes dispatch only: preserve Git resolution behavior, policy decisions, logger output, mutation-scope transitions, doctor repair ordering, hook output, retry semantics, database behavior and test expectations. Do not expand into unrelated subsystems or defer any migrated seam to PR3.

### Runtime/blocking/lifetime classification

- Production runtime owner to retain: `cli/src/main.rs:12`. DB runtime creation/isolation and every production bridge call are confined to `cli/src/services/db/mod.rs` (migration calls at 235/250/268; open at 951/1491; operations at 1001/1029/1064/1095/1173/1246/1294/1363/1549/1580/1611). Remove them all; test-only direct core runtime use at 1737 is adapted, not retained as compatibility plumbing. No alternative futures/pollster executor, global DB runtime singleton, dynamic database/repository/persistence object, or dynamic command object was found in cli/src.
- Production `block_in_place`: `command_registry.rs:84` non-context-only Setup, 87 Doctor, 103 DB-capable Hooks; `sync/sync.rs:220` storage construction and 164 guard Drop. All five are PR1 DB runtime lifetime workarounds; remove during native propagation where necessary to keep T02 compile-safe. Test-only instances: `hooks/tests.rs:4111,4137`, `sync/sync.rs:625,652,676,687,705`. Setup's test name `replaces_block_in_place_preserving_surrounding_foreign_content` refers to a managed text block, not Tokio.
- Production `spawn_blocking`: `auth_command/mod.rs:443` `run_credential_operation`, `agent_trace_sync/control_plane.rs:525,540` load/save. All three include DB work and must disappear. There are no other existing production `spawn_blocking` sites to retain. `db/encryption_key.rs:136` is currently synchronous; an async encrypted constructor must isolate only its synchronous OS keyring registration/read/write in an awaited worker, independently justified by keyring-core/platform APIs. Env-key validation can stay synchronous; no AuthDb creation, Turso build or SQL may enter that worker.
- `sync/sync.rs:152–166` `SyncStorageGuard` adds only blocking destruction of an Option-owned storage, with no independent invariant. Ordinary scoped `ResolvedAgentTraceStorage` ownership expresses its lifetime once the runtime field is gone; remove the guard. Borrowed readers and all three stream futures complete or drop before storage. Existing error/cancellation cleanup regressions become ownership regressions, without blocking fixture Drop or detached cleanup.
- Test runtime helpers: `agent_trace_sync/mod.rs:240`, `agent_trace_sync/control_plane.rs:876` currently create current-thread runtimes for sync tests; credential “outside runtime” fixtures at control_plane 1053/1061 and auth_command 569 specifically prove PR1 isolation. Convert affected tests to application-runtime async tests, deleting worker-only proof and join-failure behavior rather than asserting it after worker removal. RuntimeFlavor observations in app/hooks/auth tests are assertions, not extra owners.
- Existing production OS threads outside the bridge: `hooks/mutation_scope.rs:570` reads control-channel stdin and sends cancel signals only; `codex_hook_policy.rs:142` reads child app-server stdout only. Neither owns/drives a runtime or DB future. Existing detached post-commit auto-sync subprocess remains in `cli/src/services/sync/auto_sync.rs`; required DB checkpoint/persistence still completes in the foreground. Synchronous Git/filesystem/OS-lock/process supervision stays synchronous; removing a DB workaround does not establish a new blocking-scope justification for those services.

### Locked dependency ownership and cancellation

Inspected actual local registry source selected by `cli/Cargo.lock`: `turso`, `turso_core`, `turso_sdk_kit` 0.8.1 and Tokio 1.52.3. Dependency paths below are relative to their crate source roots (locally `/home/davidabram/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/`); no floating web documentation was used.

- `turso-0.8.1/src/connection.rs:52–98` asserts Connection Send + Sync. It contains an Arc-owned SDK connection, an atomic dangling-transaction flag and an operation gate. Clone shares the underlying connection/gate but copies the dangling flag into a different atomic; **cloning is not a separate DB connection and cannot be used for independent writers or deferred rollback ownership**. Keep one logical connection per adapter, use borrowed `&Connection` and directly awaited sequential transaction body operations, and use separate opens for contention tests. No extra Arc/Mutex/pool is needed; exclusive mutable transaction borrowing is the static serialization mechanism.
- `connection.rs:100–150,513–566` handles dangling rollback before query/execute/batch. The shared/exclusive operation gate guards statement/batch activity, not a full transaction's application-level lifetime. `transaction.rs:123–132` `new_unchecked` constructs the guard only after awaited `BEGIN IMMEDIATE` completes; unlike `Transaction::new(&mut Connection)`, it does not statically prevent nesting. For the mechanical migration, take exclusive mutable adapter borrows for transaction primitives and propagate mutable store/DB ownership through transactional callers, keeping shared borrows for read-only export and encrypted token operations; prove serialization in focused tests; Send/Sync does not authorize overlapping transaction units.
- `transaction.rs:166–185,232–244`: commit/rollback clear `in_progress` only after their awaited SQL succeeds. Default Drop stores `Rollback` into `conn.dangling_tx`; it runs no SQL and owns no runtime. Next query/execute/checked transaction performs deferred rollback and resets the flag after success. Cancelling a body while a guard exists therefore schedules rollback, **not immediate release**. Cancelling acquisition before a guard exists has no transaction Drop hook; statement cleanup and actual connection state require direct characterization. A completed commit may already be durable when its caller is cancelled; no stronger exactly-once cancellation claim is inferred.
- `turso-0.8.1/src/lib.rs:487–507` executes by polling statements and resets on ordinary completion; `turso_sdk_kit-0.8.1/src/rsapi.rs:1541–1544` statement Drop removes the registered statement. Neither establishes that dropping every pending acquisition/commit future rolls back immediately. T03 must coordinate acquisition/body/commit boundaries where controllable, check zero partial rows plus subsequent same-connection reuse and independent observer state, and state any upstream limitations. Existing explicit-error rollback assertions remain authoritative; no new cancellation timeout is introduced in production.
- `tokio-1.52.3/src/sync/once_cell.rs:168,350–389,400,464–469`: `OnceCell::const_new()` and `get_or_init<F: FnOnce() -> Fut, Fut: Future<Output=T>>` are supported by locked Tokio with `sync` enabled. A semaphore provides single-flight initialization; completed T is stored and the semaphore closed. With `T = Result<AuthDb,String>`, both Ok and Err are completed cached values. `get_or_try_init` instead leaves Err uncached and is prohibited here. Cancel/panic before completion drops the permit without storing a value, allowing a waiter/later call to initialize again; recursive initialization deadlocks. Static cell requires its T to be Send + Sync, consistent with Connection and static adapter ownership. Test fresh local Result-valued cells for success, failure, concurrent callers and pre-completion cancellation; never reset the production static or require a live user keyring.

### Test and verification map

- `cli/src/services/db/mod.rs` inline tests: busy-timeout config/defaults and held-lock overlap; CAS no-op/ordered-success/deterministic rollback/guard and body row counts; Busy/BusySnapshot classification; checkpoint repeated/readability; numeric retry budget; seeded jitter, pre/post-sleep admission/equality/oversleep; exact exhaustion error/event fields and attempt caps. `cli/src/services/resilience.rs` tests include slow-success/slow-error sync semantics; retain those and add awaited non-cancelling DB retry characterization rather than mirroring HTTP timeout tests.
- Thread-local test instrumentation in `db/mod.rs:752–862`: read-statement/write-statement counts, contention counts and attempt/backoff timeline. Callers are `agent_trace_db/{repository.rs,lock_contention_tests.rs}` and `mutation_trace/store.rs`. Synchronous `FnOnce` scopes and tracing `with_default` at 2655 cannot be held blindly across migrated task polling: use task-scoped concrete instrumentation and polling-scoped subscriber context, with instrumentation enclosing the awaited operation. Preserve exact counters, timings and independent connections; keep injected clock/backoff determinism.
- `agent_trace_db/repository.rs` inline tests own baseline/schema/metadata/no-write readiness/no-migration paths, source-instance concurrent convergence, conversation pair duplication/injected rollback, migration 003 upgrades, and 006 fresh-vs-upgraded SQL, normalization, rerun and constraint fixtures. `auth_db/mod.rs` tests use a generic TestAuthDbSpec/path OnceLock for **unencrypted** baseline schema assertions; encrypted/key-policy behavior is covered by `db/encryption_key.rs` tests and must also get representative encrypted async execution coverage. `local_db/lifecycle.rs` tests cover path/bootstrap behavior; the LocalDb module itself has no dedicated inline DB suite. Do not claim a Cargo filter running zero tests proves async LocalDb behavior.
- `agent_trace_storage/mod.rs` tests cover repository/clone/worktree identity, explicit overrides, stable source-instance metadata, legacy-path isolation, no-migration missing/baseline DB rejection, and setup-open/hook-open agreement. `mutation_trace/store.rs`, `mutation_trace/runtime/{tests.rs,coordinator.rs,scope_runtime.rs,ref_reconciliation.rs,external_mutation_guard.rs,mutation_attribution/tests.rs}` cover durable CAS/idempotency and runtime recovery/ownership; convert their threaded DB callbacks/fakes to static async calls without losing independent-connection races. Pure protocol/MBT/lineage tests do not need blanket async conversion.
- `agent_trace_sync/control_plane.rs`: FakeCredentialStore, ConcurrentCredentialStore and RuntimeCheckingCredentialStore at 955–1088, `client_with_store` factory and sync's `AlwaysValidCredentialStore`/factory at `sync/sync.rs:732–771` must be concrete async injections. Retain valid-token no-resave, expired refresh/save, exactly-one concurrent refresh/save, unexpected-401 retry and second-401 terminal, missing credentials and storage/status classification assertions. The concurrent fake's 10ms synchronous sleep at 1026 is test coordination to adapt to async, not production retry policy. `auth_command/mod.rs` tests retain renewal/fallback/redaction and typed storage errors; worker join failure and outside-runtime fixtures become obsolete.
- `agent_trace_export/mod.rs` inline tests own cursor/limit/safe-integer/raw-row/read-only/source-instance contracts. `agent_trace_sync/mod.rs` tests own accepted cursor validation, conflict/ambiguous reconciliation, terminal/no-refresh and bounded convergence. `sync/sync.rs` owns three-stream overlap, per-stream sequential batches, one initial state request, incremental second run, diff_traces compatibility, progress order/end on error, and storage error/cancellation fixtures at 636/659. Preserve borrowed/non-Send progress compatibility; don't spawn streams merely to make them concurrent.
- Command/provider verification paths: `cli/src/app.rs` inline async dispatch/output/telemetry tests; `services/{auth_command,setup,doctor,hooks,sync}/command.rs`; `setup/tests.rs`; `doctor/{mod.rs,inspect.rs}` inline tests; `hooks/tests.rs` (`async_dispatch_doctor_and_hook_persist_before_completion` and `isolated_async_dispatch_boundary`); all four harness `tests.rs`, OpenCode/Claude health inline tests, `hooks/pi_mutation_scope/{runtime_seam_tests.rs,guard_reconciliation_tests.rs}`, and ingress conformance. Adapt DB fixtures and ingress fakes in these suites, keeping fail-open/fail-closed and completion ordering assertions.
- `agent_trace_db/lock_contention_tests.rs` contains **three actual ignored tests** (842/858/1154): `concurrent_duplicate_delivery_persists_each_event_once_under_write_contention`, `concurrent_distinct_events_persist_every_event_under_write_contention`, `concurrent_real_codex_hook_processes_persist_every_distinct_event` (Unix only). Strict distinct/duplicate checks require zero losses/errors/exhaustions plus expected pair counts; process checks require exact message/part counts and zero nonzero exits. Missing SCE_BIN returns early and is not evidence. Preserve held-lock overlap, first-attempt 100ms hold, initialized hook-open zero writes and budget tests. *(Superseded 2026-10-08: this file was deleted in T02 and is not restored; AC6 now relies on Test C, with the former stress matrices optional characterization.)*
- Toolchain/check ownership: `cli/Cargo.toml` enables Tokio rt-multi-thread/sync/time/macros, edition 2021 and deny warnings/Clippy; `flake.nix:93` pins Rust 1.95.0, Crane owns cli-tests/cli-clippy/cli-fmt and ci-checks/release packaging. `scripts/run-cli-cargo.sh` generates a fresh temporary payload through the producer, sets `SCE_CLI_GENERATED_INPUT_DIR`, and cleans it after Cargo. Focused migrated suites use `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml <filter>`; formatting autofix uses repo Nix. No .cargo/config.toml or CARGO_TARGET_DIR/BUILD_TARGET override was found in this audited session; wrapper build's default binary is `cli/target/debug/sce`, to be resolved/verified from actual Cargo metadata/build output before AC6 process runs. Full-plan checks remain `/validate`'s `nix flake check`, `nix build .#ci-checks`, `git diff --check`.

### Context owners, drift and audit evidence

- Existing owners: `context/{overview,architecture,patterns,glossary,context-map}.md`; `context/sce/{shared-turso-db,agent-trace-db,auth-db,local-db,agent-trace-db-write-contention-evidence,agent-trace-export-readers}.md`; `context/cli/{cli-command-surface,agent-trace-storage,service-lifecycle,agent-trace-sync-command,mutation-trace-store,mutation-trace-runtime-coordinator,mutation-trace-agent-attribution,mutation-trace-ref-reconciliation,mutation-trace-scope-abandonment,mutation-trace-external-mutation-guard,mutation-scope-runtime}.md`; harness integration/health owners linked from context-map; immutable `context/decisions/2026-10-07-application-owned-async-command-runtime.md`.
- Recorded drift for T05/source truth: shared DB contract bullet incorrectly includes migrations in generic operation retry, while its later paragraph and source correctly exclude them; LocalDb owner calls policies hardcoded although shared config overrides exist; service-lifecycle description claims hooks lazily initialize through the setup resolver while the storage owner and runtime opener correctly describe a separate no-migration path; reconcile repository migration descriptions against the six-file list including 006. PR1 ADR documents its historical sync-persistence boundary; preserve it as immutable history and use current-state owners/convention-compliant successor for PR2. T01 establishes no implemented runtime/interface change and does not prematurely rewrite current architecture to the target.
- Verification performed in original T01: the original three Source audit searches succeeded (60 runtime/blocking lines, 4 explicit trait-object/boxed-future lines, 133 named seam lines). Supplemental Nix ripgrep covered dyn Fn/SyncFuture, DB aliases and callers, SQL/transaction/retry/WAL/migration symbols, sleeps/threads, counters, ignored tests and alternative executor spellings. Source reads included Cargo/flake/wrapper/producer and locked Turso/Tokio files.
- Plan-correction verification: the added exact synchronous-DI audit succeeded with 175 matching lines (174 targeted for removal by PR2; parser logger is the one unrelated untouched DI match). The retained credential/boxed-future audit succeeded with 4 matching lines, all targeted for removal, including the coordinator test FnMut. Supplemental whole-word dyn/`?Sized`/SyncFuture search and source inspection classified qualified logger names, conformance implementations, diagnostic protocol signatures and trait-object-admitting bounds above. Inspected AppRuntime/AppContext and capability APIs to select existing HasLogger/HasGit/HasFs dispatch and focused closures without changing semantics. Caller-table shape, T01 done/T02 todo status, absence of the old synchronous exception, plan-only diff and `git diff --check` passed. Counts describe current source, not zero-match migration acceptance. No application tests or full-plan checks are needed or run for this documentation-only correction.

## Review findings map, minimal safety test set and deferred ledger (added 2026-10-08)

### Review finding → owner

| Finding | Status | Owner | Evidence | Criterion |
| --- | --- | --- | --- | --- |
| `&self` + `new_unchecked` transactions | confirmed | T03 | `&mut` compile-time argument + Test A | AC3 |
| Transaction cancellation after first write | plausible | T03 | Test B | AC8 |
| Retry/contention semantics | needs one representative check | T03 | Test C (or surviving test) + source inspection of numeric policies | AC5, AC6 |
| Refresh-token persistence can be cancelled | confirmed structural exposure | T06 | Test D | AC7, AC14 |
| Sync no longer fails fast | confirmed | T06 | Test E (extends existing `sibling_failure_does_not_cancel_in_flight_credential_save`) | AC21 |
| Adapter boundary lock vs. blocking state writes | plausible | T07 | Test F | AC19 |
| Guarded-shell FD inheritance | confirmed | T07 | Test G | AC20 |
| Retained blocking boundaries unaudited | audit gap | T04 | source inspection + T03/T06/T07 tests + existing protected-worktree/guard tests | AC9 |
| Scope drift, stale patterns/architecture/ADR | confirmed | T05 | context inspection | AC10, AC11 |
| Lower-priority maintainability findings | deferred | PR3 | ledger below | AC22 |

### Minimal safety test set

Guidance, not a numeric acceptance criterion: roughly 6–10 new tests in total, fewer where existing tests, the type system or Quint already prove the invariant. Test count parity with the deleted suites, restoring deleted files, and a coverage-equivalence matrix are explicitly **not** required. Add a test only if it answers: *what specific regression would this catch that the existing tests, the type system, and the Quint specification would not?* A test filter that matches zero tests never counts as passing; confirm with `-- --list` that the intended test exists before relying on a run.

| Test | Owner | Invariant | Distinct regression caught | Static / existing alternative |
| --- | --- | --- | --- | --- |
| A | T03 | Another operation on the same adapter cannot run inside an active transaction | someone reintroducing `&self`/`new_unchecked` or a clone-based escape hatch | If `&mut self` makes the overlap unrepresentable in safe Rust, document that compile-time proof (and, if cheap, a `compile_fail` doctest) instead of a runtime test |
| B | T03 | Cancel after the first write, before commit: no partial committed state, connection reusable, a later transaction succeeds, an independent connection sees only the durable result | Turso deferred-rollback behavior leaking an uncommitted write or wedging the connection | Quint MBT covers protocol transitions, not DB cancellation |
| C | T03 | One bounded contention scenario on independent connections: Busy/BusySnapshot classified, bounded attempts/deadline, whole-transaction retry, no duplicate or lost durable writes | retry unit shrinking to a statement, unbounded retry, duplicate/lost rows | Reuse a surviving test if one exists; numeric policies confirmed by source diff inspection |
| D | T06 | Refresh succeeds, save held pending, caller cancelled/timed out, save completes, next request reads replacement credentials (with a concurrent caller proving single-flight) | losing a rotated refresh token to the outer `/state` timeout | none |
| E | T06 | One stream returns 403 while another has several batches: no further batch starts after the terminal error is observed; explicitly protected operation completes | reverting to drain-all-siblings behavior | Extend the existing `sibling_failure_does_not_cancel_in_flight_credential_save` in `sync/sync.rs` |
| F | T07 | Pause a state update inside a blocking worker, cancel the outer boundary caller, attempt a second boundary: it cannot observe an invalid intermediate state or overtake the first transition | a started blocking transition outliving its boundary lock | Test the shared lock primitive in `hooks/mutation_scope_lock.rs`; add a per-adapter test only for a genuinely different transition protocol |
| G | T07 | An unrelated subprocess spawned concurrently with guarded-shell setup does not retain the lock/lifetime descriptors; the guarded child does; the unrelated child does not delay guard completion | parent-side descriptor inheritance window | `FD_CLOEXEC` atomic creation is verified by source inspection; the existing guard tests (`guarded_shell_*`) are reused for lock/lifetime protocol |

Reused rather than re-created: the Quint model-based suite (`mutation_trace::mbt`, flake check `mutation-trace-quint-connect`) for mutation-cursor/coordinator protocol invariants; surviving pure tests in `mutation_trace/tests.rs`, `lineage`, `agent_trace` golden tests, `protected_worktree.rs` cancellation tests, `external_mutation_guard.rs` and `git_snapshot.rs` tests, and `sync.rs` `sibling_failure_does_not_cancel_in_flight_credential_save`.

Optional characterization (never a merge gate): the previous writer/round stress matrices, larger contention loads, and per-await-point cancellation variants.

### Deferred cleanup ledger (PR3 handoff; not PR2 merge blockers unless a concrete correctness defect is established)

None of these may be mixed into T03–T07. Dead test modules are not restored merely because they exist on disk; unused test infrastructure is deleted in PR3 if no focused test needs it.

| Finding | Disposition |
| --- | --- |
| Redundant `async { ... }.await` wrappers (for example `sync/sync.rs` storage scope) | PR3 |
| Unused `run_with_retry_sync` and duplicate retry-helper logic | PR3, without blurring elapsed-time vs cancelling semantics |
| Redundant `drop(storage)` (`sync/sync.rs`) | PR3 (T04 may remove it incidentally if it removes `SyncStorageGuard` remnants) |
| Stale `#[allow(dead_code)]` annotations | PR3 |
| Orphaned test-support files and fixtures; unused test seams (for example `fail_before_second`) | PR3: delete if no focused test uses them |
| Custom `join_three_to_completion` complexity | PR3 re-evaluation after T06 settles terminal behavior |
| Large futures and `#![recursion_limit = "256"]` pressure | PR3 measurement; recorded as a known limitation by T05 |
| Possible orphaned temporary Git `index-<uuid>.lock` files after killed snapshot commands | accepted limitation documented by T04/T05; PR3 may add a sweep |

## Task stack

- [x] T01: `Map persistence ownership and the required async caller closure` (status:done)
  - Task ID: T01
  - Scope: In — source-grounded runtime/API/transaction/retry/migration/test inventory recorded with exact paths in this plan, connected callers through application dispatch, all PR1 blocking sites, context owners and pinned Turso ownership/cancellation semantics. Explicitly map CredentialStore, AuthenticatedControlPlaneClient, SystemCredentialStore, token_storage AUTH_DB OnceLock, run_credential_operation, ServiceLifecycle, LifecycleProvider, Doctor, Setup, HooksCommand, mutation_trace callbacks/providers, AgentTraceExportReader, SyncStorageGuard, all block_in_place, and all spawn_blocking. Out — application behavior changes or building a replacement bridge.
  - Dependencies: none
  - Done when: Inventory covers both shared adapters and aliases, direct typed SQL wrappers, synchronous callback/trait/OnceLock boundaries, contention instrumentation and actual ignored suites; classifies every production runtime/blocking site and establishes the mechanically required propagation closure plus unchanged policy baseline. For every dynamic/trait seam record current dispatch (static/dynamic), target dispatch (static), and the concrete/generic/enum conversion. Explicitly confirm no additional dyn seam in the DB propagation closure leaves a design choice unresolved: any discovered seam follows Architecture decisions, never type erasure. Audit Turso Connection Send/Sync and intended same-connection use to select mechanical borrowing/lifetime adaptations within the fixed static design. Verify locked Tokio OnceCell API and Result-valued success/failure caching semantics.
  - Verify: Run Source audit and Nix ripgrep searches for runtime/SQL/retry/WAL/migration symbols across persistence and callers; inspect actual tests, Cargo/flake/wrapper configuration and pinned Tokio/Turso transaction/connection source. No tests or final checks are necessary for the inventory-only change.
    - Passed: Original three Source audit searches and supplemental Nix ripgrep searches; source inspection of both adapters, aliases, typed wrappers, complete caller/seam closure, policy/migration boundaries, instrumentation and ignored suites. Findings and exact sites are recorded in the T01 source inventory above.
    - Passed: Plan-correction synchronous-DI and retained credential/future audits, supplemental dyn/`?Sized`/SyncFuture audit, existing capability/source reads and complete match classification; amended caller table and AC13/T02 static scope. T01 remains done and T02 remains todo; only this plan changes.
    - Passed: Locked Turso 0.8.1 Connection/Transaction/statement source and Tokio 1.52.3 OnceCell source inspection, plus Cargo/flake/wrapper/producer review. Deferred rollback, acquisition-before-guard risks, static mutable transaction borrowing and Result-valued OnceCell caching/cancellation semantics are recorded above.
    - Passed: Nix-backed Python inventory path check (22 explicit paths, no missing paths), six repository/two auth migration counts and grouped consumer-owner existence checks; `git diff --check`.
    - Not required: Application tests and full-plan checks for this inventory-only task; none run.
  - Completed: 2026-10-07
  - Files changed: `context/plans/cli-async-turso-persistence-pr2.md` (baseline-relative; clean baseline at `9ceff8414613a4a619cf505ae9ac0e3eb46db69f`).
  - Result: Recorded and amended the source inventory above, including all DB-backed callers and both synchronous/asynchronous static conversions: credentials, ingress/conformance, GitDirResolver, BashPolicyEvaluator, model/repair, doctor Git/path/config/probe dependencies, logger arguments, coordinator fault hooks and SyncFuture. Existing AppContext associated-type capabilities remain static. Every dynamic seam audit match has an explicit migration or unrelated-path classification; no synchronous DI exception remains in the migrated closure and no architectural question is deferred. Ordinary storage ownership, retry/WAL/SQL/migration policies, cancellation limitations, relevant tests and context owners/drift remain recorded. No application or test source changed.
  - Done-check evidence: Persistence/alias/wrapper/policy/migration sections establish the baseline; caller-conversion table establishes the complete static propagation closure; runtime classification covers every production bridge/blocking site; dependency section selects borrowing/lifetime adaptations and verifies OnceCell semantics; verification map inventories contention instrumentation and all three actual ignored suites. Tests characterize cancellation in T03, not T01.
  - Context impact: none — source-grounded inventory and future migration instructions live in this plan; current implementation, interfaces, runtime ownership and terminology are unchanged. Existing owner drift is recorded for T05 without prematurely describing the target as implemented.
  - Context synchronization: synced
  - Context synchronization evidence: `no_context_change`; verified `context/overview.md`, `context/architecture.md`, `context/glossary.md`, `context/patterns.md`, and `context/context-map.md` against this documentation-only change, plus existing DB/storage/lifecycle owners and the PR1 runtime ADR. Current runtime architecture remains unchanged; no new feature, terminology or implemented qualifying decision requires owner edits or an ADR. Pre-existing migration/retry/hook-readiness drift remains explicitly recorded above for T05. Inventory paths/migration counts and final whitespace check passed; only this plan changed relative to the captured baseline.

- [x] T02: `Replace the Turso runtime bridge with awaited persistence and callers` (status:done)
  - Task ID: T02
  - Scope: In — remove core runtime field/constructor and bridge; convert open/execute/query/materialization/checkpoint/readiness/migration/transaction/encrypted operations and retry helpers; mechanically adapt the complete caller closure and existing tests in one compile-safe migration commit. This includes the resolved credential, token-storage, lifecycle, hook, mutation-trace, and sync/export architectures below and all synchronous DI in mechanically touched functions/modules. No intermediate commit may need a temporary block_on/executor bridge. Out — unrelated untouched legacy seams, unrelated service logic and behavior changes. This plan correction must be committed before T02 implementation begins; T02 remains unstarted during the correction.
  - Dependencies: T01
  - Done when: TursoDb/EncryptedTursoDb are async with no DB runtime field, build_current_thread_runtime, or block_on_isolated; all native futures are awaited on the application caller runtime with no synchronous executor. The entire closure compiles using static dispatch, including:
    - Remove dynamic dispatch from every migrated seam, including synchronous testability/policy/logging callbacks encountered in the required async caller closure: GitDirResolver, BashPolicyEvaluator, mutation-scope ingress/conformance, doctor repair, model resolver, doctor filesystem/Git/config/probe callbacks, logger argument seams, coordinator fault injection, SyncFuture and CredentialStore. No dynamic seam may remain merely because it does not itself return a future. Preserve AppRuntime/AppContext concrete ownership/borrowing and HasLogger/HasTelemetry/HasFs/HasGit associated types; use existing capabilities where natural, sized logger/generic callback parameters elsewhere, and concrete generic futures/native async static capabilities for borrowed async seams. Zero dynamic DI/future dispatch in changed/migrated production files, with every unrelated occurrence classified by exact site/reason.
    - Async token_storage with Result-valued OnceCell caching completed success/failure; direct AuthCommand login/whoami load, refresh/login save, and logout delete awaits; removed run_credential_operation with typed CliError/secret-safe behavior preserved.
    - Native async generic CredentialStore and AuthenticatedControlPlaneClient<S: CredentialStore>, concrete SystemCredentialStore and async fake-store injection through connected sync/test callers; removed credential trait-object fields/constructors and DB spawn_blocking. Preserve token reuse, refresh/single-flight/exactly-one refresh, 401 retry/terminal behavior, storage failures and exact counts/save assertions; replace outside-runtime tests with application-runtime async-store proof.
    - Native async ServiceLifecycle diagnose/fix/setup; LifecycleProvider static enum matches await concrete providers; RuntimeCommand::Doctor/Setup await Doctor/Setup services and lifecycle operations. Pure Config/Hooks work runs synchronously inside those async methods without a blocking worker.
    - RuntimeCommand::Hooks awaits HooksCommand static matches and DB-backed handlers; required foreground persistence completes before returning, preserving the sole existing detached post-commit auto-sync child.
    - Awaited MutationTraceStore transaction primitives through coordinator/runtime, generic providers/callbacks, hook adapter and command; native async functions or genuinely needed F: FnOnce(...) -> Fut / Fut: Future bounds, never boxed callbacks.
    - Async AgentTraceExportReader/connected repository reads awaited in the existing sync engine; unchanged initial state request, three concurrent streams, per-stream sequential batches, cursor reconciliation, retries, diff_traces compatibility and progress ordering.
    - Generic retries preserve post-attempt diagnostic timeout semantics rather than adopting cancelling timeout; contention retries preserve whole-operation units and deterministic errors. Each converted sleep is recorded with unchanged policy. Transactions retain the same logical connection/SQL; migrations, encryption, retry and WAL policy are unchanged. Async-aware test instrumentation preserves overlap, rollback and attempt assertions. No dynamic async abstraction or artificial locking layer is introduced.
  - Verify: Run focused wrapper suites for `services::db`, `services::agent_trace_db`, `services::agent_trace_storage`, `services::auth_db`, `services::local_db`, `services::mutation_trace`, and affected auth/control-plane/export/hook/lifecycle/command tests from T01 before repository-wide validation. Use `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml <filter>`. Run Source audit, trace all caller classes against AC13–AC17, and review SQL/policy diff. Auto-format only through Nix when needed.
  - Completed: 2026-10-07
  - Files changed (baseline `d8d1aefb`, T01 complete):
    - `cli/src/main.rs`
    - `cli/src/services/agent_trace.rs`
    - `cli/src/services/agent_trace_db/lifecycle.rs`
    - `cli/src/services/agent_trace_db/lock_contention_tests.rs`
    - `cli/src/services/agent_trace_db/mod.rs`
    - `cli/src/services/agent_trace_db/repository.rs`
    - `cli/src/services/agent_trace_export/mod.rs`
    - `cli/src/services/agent_trace_storage/mod.rs`
    - `cli/src/services/agent_trace_sync/control_plane.rs`
    - `cli/src/services/agent_trace_sync/mod.rs`
    - `cli/src/services/app_support.rs`
    - `cli/src/services/auth_command/mod.rs`
    - `cli/src/services/auth_db/lifecycle.rs`
    - `cli/src/services/auth_db/mod.rs`
    - `cli/src/services/codex_hook_config.rs`
    - `cli/src/services/command_registry.rs`
    - `cli/src/services/config/lifecycle.rs`
    - `cli/src/services/config/render.rs`
    - `cli/src/services/db/encryption_key.rs`
    - `cli/src/services/db/mod.rs`
    - `cli/src/services/doctor/command.rs`
    - `cli/src/services/doctor/inspect.rs`
    - `cli/src/services/doctor/mod.rs`
    - `cli/src/services/doctor/render.rs`
    - `cli/src/services/hooks/claude_bridge_session.rs`
    - `cli/src/services/hooks/claude_model_state.rs`
    - `cli/src/services/hooks/claude_mutation_scope/health.rs`
    - `cli/src/services/hooks/claude_mutation_scope/lifecycle.rs`
    - `cli/src/services/hooks/claude_mutation_scope/mod.rs`
    - `cli/src/services/hooks/claude_mutation_scope/state.rs`
    - `cli/src/services/hooks/claude_mutation_scope/tests.rs`
    - `cli/src/services/hooks/claude_transcript.rs`
    - `cli/src/services/hooks/codex/apply_patch/mod.rs`
    - `cli/src/services/hooks/codex/apply_patch/normalize.rs`
    - `cli/src/services/hooks/codex/apply_patch/parser.rs`
    - `cli/src/services/hooks/codex/apply_patch/path.rs`
    - `cli/src/services/hooks/codex/bash_policy.rs`
    - `cli/src/services/hooks/codex/mod.rs`
    - `cli/src/services/hooks/codex/stop.rs`
    - `cli/src/services/hooks/codex/user_prompt_submit.rs`
    - `cli/src/services/hooks/codex_mutation_scope/health.rs`
    - `cli/src/services/hooks/codex_mutation_scope/lifecycle.rs`
    - `cli/src/services/hooks/codex_mutation_scope/mod.rs`
    - `cli/src/services/hooks/codex_mutation_scope/state.rs`
    - `cli/src/services/hooks/codex_mutation_scope/tests.rs`
    - `cli/src/services/hooks/command.rs`
    - `cli/src/services/hooks/commit_hooks.rs`
    - `cli/src/services/hooks/conversation_trace.rs`
    - `cli/src/services/hooks/diff_trace.rs`
    - `cli/src/services/hooks/lifecycle.rs`
    - `cli/src/services/hooks/mod.rs`
    - `cli/src/services/hooks/mutation_scope.rs`
    - `cli/src/services/hooks/mutation_scope_ingress_conformance.rs`
    - `cli/src/services/hooks/mutation_scope_lock.rs`
    - `cli/src/services/hooks/mutation_scope_owner.rs`
    - `cli/src/services/hooks/mutation_scope_state_conformance.rs`
    - `cli/src/services/hooks/opencode_mutation_scope/health.rs`
    - `cli/src/services/hooks/opencode_mutation_scope/lifecycle.rs`
    - `cli/src/services/hooks/opencode_mutation_scope/mod.rs`
    - `cli/src/services/hooks/opencode_mutation_scope/state.rs`
    - `cli/src/services/hooks/opencode_mutation_scope/tests.rs`
    - `cli/src/services/hooks/pi_mutation_scope/guard_reconciliation_tests.rs`
    - `cli/src/services/hooks/pi_mutation_scope/health.rs`
    - `cli/src/services/hooks/pi_mutation_scope/lifecycle.rs`
    - `cli/src/services/hooks/pi_mutation_scope/lifecycle_tests.rs`
    - `cli/src/services/hooks/pi_mutation_scope/mod.rs`
    - `cli/src/services/hooks/pi_mutation_scope/runtime_seam_tests.rs`
    - `cli/src/services/hooks/pi_mutation_scope/state.rs`
    - `cli/src/services/hooks/pi_mutation_scope/tests.rs`
    - `cli/src/services/hooks/runtime.rs`
    - `cli/src/services/hooks/tests.rs`
    - `cli/src/services/lifecycle.rs`
    - `cli/src/services/local_db/lifecycle.rs`
    - `cli/src/services/mutation_trace/runtime/coordinator.rs`
    - `cli/src/services/mutation_trace/runtime/external_mutation_guard.rs`
    - `cli/src/services/mutation_trace/runtime/external_taint.rs`
    - `cli/src/services/mutation_trace/runtime/git_snapshot.rs`
    - `cli/src/services/mutation_trace/runtime/mod.rs`
    - `cli/src/services/mutation_trace/runtime/mutation_attribution.rs`
    - `cli/src/services/mutation_trace/runtime/mutation_attribution/tests.rs`
    - `cli/src/services/mutation_trace/runtime/protected_worktree.rs`
    - `cli/src/services/mutation_trace/runtime/ref_reconciliation.rs`
    - `cli/src/services/mutation_trace/runtime/scope_runtime.rs`
    - `cli/src/services/mutation_trace/runtime/tests.rs`
    - `cli/src/services/mutation_trace/runtime/worktree_lock.rs`
    - `cli/src/services/mutation_trace/store.rs`
    - `cli/src/services/resilience.rs`
    - `cli/src/services/setup/command.rs`
    - `cli/src/services/setup/mod.rs`
    - `cli/src/services/setup/tests.rs`
    - `cli/src/services/structured_patch.rs`
    - `cli/src/services/sync/auto_sync.rs`
    - `cli/src/services/sync/command.rs`
    - `cli/src/services/sync/progress.rs`
    - `cli/src/services/sync/render_sync.rs`
    - `cli/src/services/sync/sync.rs`
    - `cli/src/services/token_storage.rs`
  - Result: Removed the Turso runtime bridge and propagated native async persistence through the connected production callers, credentials/token storage, lifecycle, hooks, mutation trace, sync/export, setup/doctor, and command dispatch. Kept static dispatch and existing SQL, migration, encryption, retry, and WAL policy. Restored synchronous formatting/counting helpers that had been needlessly made async. `TransactionStatement::new` stays synchronous.
    - Test history: the affected ordinary Rust unit-test and conformance code was first disabled with `cfg(any())`. At the user's direction, commit `fc37be7b` then deleted it rather than restoring it, along with production seams used only by those tests. The 14 orphaned external test files (for example `mutation_trace/runtime/tests.rs` and the hook adapter `tests.rs` files) were deleted afterwards. No `cfg(any())` suppression remains. The Quint mutation-trace MBT suite (`mutation_trace::mbt`) stays enabled. Fixture directories are kept because `flake.nix`, docs, and the Pi TypeScript tests reference them.
    - Blocking boundaries: no DB or Turso future runs inside `spawn_blocking`. The synchronous OS credential-store lookup in `EncryptedTursoDb::new` is intentionally isolated in `spawn_blocking`. Synchronous advisory file-lock acquisition is isolated the same way: adapter boundary/state locks go through `AdapterLockSpec::acquire_async`, and the mutation-trace worktree lock goes through `worktree_lock::acquire_inner_async`. Each worker returns its owned guard to the async caller. Timeouts, poll intervals, lock paths, contention-callback semantics, external-taint handling, and the external mutation guard's inherited-lock ownership are unchanged. Claude's duplicate private state lock now uses the shared `AdapterLockSpec` with the same path, timeout, and poll interval.
  - Verify: Focused wrapper suites listed above; Source audit; review SQL/policy diff; Nix formatting when needed.
    - Passed: `nix develop -c sh -c 'cd cli && cargo fmt --all -- --check'`.
    - Passed: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml --no-run`.
    - Passed: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml`: 318 passed, 0 failed (this includes the 16 `mutation_trace::mbt` Quint Connect tests).
    - Passed: `nix develop -c ./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets --all-features -- -D warnings`, after fixing five lints that predated the lock change.
    - Passed: `quint typecheck spec/mutation_cursor.qnt`, `quint test spec/mutation_cursor.qnt --match '^test.*'` (41 passing), and the randomized `verifyStep` safety run (5000 samples, 20 steps).
    - Passed: `nix flake check`, including `cli-tests`, `cli-clippy`, `cli-fmt`, and `mutation-trace-quint-connect`.
    - Passed: Source audit found no production runtime bridge, `block_on`, or `block_in_place`; three production `spawn_blocking` sites, all at synchronous external boundaries; no `cfg(any())`; and no new dynamic dispatch.
  - Done-check evidence: The production closure compiles without the owned runtime bridge and passes the remaining Rust suite and Quint MBT. Behavioral assertions from the deleted ordinary Rust suites are not covered; this verification reduction was made at the user's direction and is recorded explicitly.
  - Context impact: root — persistence execution and async interfaces now follow the application-owned Tokio runtime through the shared command/service/storage closure; this changes cross-domain architecture and belongs in root and domain context.
  - Context synchronization: synced
  - Post-review clarification (2026-10-08, plan update; the T02 evidence above is unchanged and no new test run is claimed): the deletion of the ordinary Rust suites was an intentional decision and is not being reversed. T02 established the mechanical async migration; T03, T06 and T07 establish the minimum necessary correctness evidence (Tests A–G) for the new execution model, and T01's exclusive-`&mut` transaction instruction is not yet reflected at head `044991d8` (T03).

- [x] T03: `Enforce transaction isolation and characterize async cancellation` (status:done)
  - Task ID: T03
  - Scope: In — make transaction exclusivity structural and add Tests A–C from the Minimal safety test set. Inspect `TursoDb::execute_transactional_insert_pair_if_absent`, `TursoDb::execute_transactional_cas_batch` and the `MutationTraceStore` transactional callers (coordinator, scope runtime, ref reconciliation, hook adapters). Enforce exclusive mutable ownership across an entire transaction: prefer `&mut self` receivers and checked `Transaction::new(&mut connection, Immediate)` where the pinned Turso 0.8.1 supports it; otherwise record the exact upstream reason and the equivalent static proof. No other operation on the same connection may enter an active transaction. Preserve SQL, retries, WAL, error classification and rollback semantics. T03 may make the smallest production changes necessary to establish transaction exclusivity and cancellation correctness. Unrelated architecture redesigns remain out of scope. Out — a connection pool, `Arc<Mutex<_>>` or connection cloning to solve what static borrowing prevents, a custom transaction framework, restoring deleted suites or test-count parity, per-await-point test variants, the exhaustive strict-load matrix as a merge gate.
  - Dependencies: T02
  - Done when: Transaction exclusivity is structurally enforced (signatures take exclusive borrows through the whole transaction; no `new_unchecked` transaction construction remains unless justified in writing). Test A: either a deterministic test (pause transaction A after `BEGIN`, attempt operation B on the same adapter, prove B cannot execute within A) or, where exclusive borrowing makes the overlap unrepresentable in safe Rust, a documented compile-time proof (optionally a `compile_fail` doctest). Test B: cancel transaction A after its first write and before commit, at the most dangerous controllable boundary only; assert no partial committed state, safe subsequent reuse of the connection, a later legitimate transaction succeeds, and an independent connection sees the correct durable result; record exact guarantees and upstream limitations (deferred rollback is not immediate release; a completed commit may already be durable). Test C: one representative bounded contention scenario on independent connections asserting Busy/BusySnapshot classification, bounded attempts/deadline, whole-transaction retry and no duplicate or lost durable writes — reuse a surviving test if one already covers it. No deleted test-suite restoration is required.
  - Verify: Inspect signatures and call chains for exclusive borrows; `nix shell nixpkgs#ripgrep -c rg -n 'new_unchecked' cli/src`; review the SQL/statement-order diff and confirm unchanged numeric retry/deadline constants by source diff. Run Tests A–C through the Cargo wrapper (`nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml <filter>`), each preceded by the same filter with `-- --list` confirming the test exists (zero matches never pass). Run `nix flake check` (includes the Quint MBT suite).
  - Completed: 2026-10-08
  - Files changed (baseline `31489f17`, clean tree):
    - `cli/src/app.rs`
    - `cli/src/services/agent_trace_db/mod.rs`
    - `cli/src/services/agent_trace_db/repository.rs`
    - `cli/src/services/agent_trace_db/transaction_tests.rs` (new)
    - `cli/src/services/db/mod.rs`
    - `cli/src/services/db/transaction_cancellation_tests.rs` (new)
    - `cli/src/services/hooks/codex/stop.rs`
    - `cli/src/services/hooks/codex/user_prompt_submit.rs`
    - `cli/src/services/hooks/commit_hooks.rs`
    - `cli/src/services/mutation_trace/runtime/coordinator.rs`
    - `cli/src/services/mutation_trace/runtime/mutation_attribution.rs`
    - `cli/src/services/mutation_trace/runtime/ref_reconciliation.rs`
    - `cli/src/services/mutation_trace/runtime/scope_runtime.rs`
    - `cli/src/services/mutation_trace/store.rs`
    - `cli/src/services/resilience.rs`
  - Result: `execute_transactional_insert_pair_if_absent` and `execute_transactional_cas_batch` now take `&mut self` and begin the transaction with the checked `Transaction::new(&mut Connection, Immediate)` (supported by Turso 0.8.1); no `new_unchecked` remains in `cli/src`. The exclusive borrow propagates through `MutationTraceStore` (`db: &'a mut`, `commit(&mut self)`), the coordinator, scope runtime, ref reconciliation, post-commit attribution, the conversation-text-event insert and the Codex Stop/UserPromptSubmit and post-commit hooks. Because an `FnMut` retry closure cannot return a future borrowing its own captured `&mut`, the retry helpers take a small native-async `RetryOperation` (resilience) / `WriteAttempt` (db) trait with blanket impls for the existing closures; the two transactions are `InsertPairAttempt` / `CasBatchAttempt` structs. Retry numbers, jitter, busy timeout, deadline admission, SQL and statement order are unchanged. `app.rs` test call boxed to satisfy `clippy::large_futures` (the future grew past 16384 bytes).
  - Test A (compile-time proof, no runtime test): with `&mut self` and `Transaction::new(&mut conn, ..)` the transaction holds the only borrow of the adapter's connection for its whole lifetime, so another adapter operation inside it does not compile; documented on both primitives. `compile_fail` doctest not added because the crate is a binary (doctests do not run).
  - Test B `db::transaction_cancellation_tests::abandoned_transaction_commits_nothing_and_rolls_back_on_next_use_of_its_connection`: a `Transaction` on the adapter's connection with one write is dropped uncommitted (what future cancellation does). Verified: an independent connection sees no row; that connection's writer stays blocked (contention-exhausted error) until the owning connection is next used; the owner's next transaction succeeds and performs the deferred rollback; the observer can then write; abandoned row never appears, later rows appear exactly once. Finding: local Turso futures never return `Pending` (a poll-sweep cancellation test completed on the first poll), so a mid-transaction drop cannot occur in the current primitives; the test therefore drops the guard directly. Limitations (upstream Turso 0.8.1): rollback is deferred to the next use of the same connection and the writer lock is held until then; a completed commit may already be durable when its caller is cancelled.
  - Test C `agent_trace_db::transaction_tests::{write_contention_retries_whole_transaction_once_and_persists_one_pair, write_contention_exhausts_bounded_attempts_without_persisting_rows}`: independent connections; lock held 1.3s -> attempt 1 Busy, attempt 2 succeeds, exactly one pair; lock held throughout -> `attempts=2` contention-exhausted error, zero rows, later write succeeds.
  - Verify: Passed: `rg new_unchecked cli/src` (no matches). Passed: each filter confirmed with `-- --list` (3 transaction tests listed) then run: 4 passed. Passed: SQL/numeric-policy diff review (no constant or SQL changes). Passed: `nix develop -c sh -c 'cd cli && cargo fmt'`; `git diff --check`. Passed: `nix flake check` (cli-tests, cli-clippy, cli-fmt, mutation-trace-quint-connect).
  - Done-check evidence: exclusivity is structural (`&mut` signatures plus checked `Transaction::new`); Tests A (compile-time), B and C as above.
  - Context impact: domain — transaction primitives require exclusive borrows and the store/hook callers changed accordingly; belongs in shared-turso-db and mutation-trace-store owners (T05 records the guarantees and limitations).
  - Context synchronization: synced
  - Context synchronization evidence: `synced`; edited `context/sce/shared-turso-db.md` (exclusive `&mut` transaction primitives, checked `Transaction::new`, retry-operation traits, Turso 0.8.1 cancellation limits, tests) and `context/cli/mutation-trace-store.md` (`commit(&mut self)`). Root files (`overview`, `architecture`, `glossary`, `patterns`, `context-map`) read and unaffected; no qualifying ADR (local ownership change within the accepted directly-awaited persistence decision); both edited files within 250 lines.

- [ ] T06: `Protect credential rotation and stop sync on terminal failure` (status:todo)
  - Task ID: T06
  - Scope: In — (A) a cancellation-protected native-async refresh-and-persist operation and Test D; (B) cooperative early termination of sibling streams after a terminal failure and Test E. Out — moving any Turso operation into `spawn_blocking`, changing the WorkOS protocol, redesigning the cursor algorithm, restoring the deleted control-plane/sync suites, a generic cancellation framework.
    - **A. Refresh-token persistence.** Today the `/state` path (`ingestion_state → run_with_retry → tokio::time::timeout → execute_authenticated → force_refresh_access_token → refresh_and_save → credential save`) lets the outer timeout cancel persistence after WorkOS has rotated the refresh token. Choose a native-async design with explicit ownership and cancellation guarantees (candidates evaluated against source: an owned, shielded refresh-and-save unit awaited before command termination; or restructuring so the timeout cannot wrap post-response persistence — moving the timeout outside one function is not assumed sufficient). Preserve single-flight refresh, exactly one intended refresh per rejected credential generation, durable storage before publishing/reusing the new token, typed errors, secret redaction, bounded HTTP/network operations, and no Turso work in `spawn_blocking`. Explicitly characterize the unavoidable remote-operation ambiguity window (remote refresh succeeds but the process crashes or the response never arrives) and process-crash limitations.
    - **B. Terminal sync errors.** `join_three_to_completion` currently waits for every sibling to exhaust its backlog after one stream fails terminally. Require cooperative early termination: stop scheduling new sibling batches after a terminal failure; preserve in-flight credential persistence and do not abandon an irreversible operation (so simply dropping siblings via `try_join!` is not acceptable); preserve cursor/reconciliation semantics and per-stream sequential batching; return the appropriate terminal error (first observed, deterministic) without waiting for unnecessary uploads.
  - Dependencies: T02, T03
  - Done when: Credential rotation and persistence have explicit, documented cancellation guarantees and Test D passes: a deterministic fake WorkOS response and controlled async credential store; refresh returns replacement credentials; the save starts and is held pending; the outer request times out or is cancelled; the save completes; a subsequent request reads the replacement credentials; where feasible a concurrent caller proves single-flight (one rotation) in the same test. Terminal sync failure does not unnecessarily drain remaining backlogs and Test E passes: one stream returns 403, another has several batches, no further batch starts after the terminal error is observed, and any explicitly protected operation completes (extend the existing `sibling_failure_does_not_cancel_in_flight_credential_save` rather than duplicating the sync integration suite). Split Test D only if clarity requires.
  - Verify: Run Tests D and E through the Cargo wrapper, each preceded by a `-- --list` check that the test exists; `nix shell nixpkgs#ripgrep -c rg -n 'spawn_blocking' cli/src/services/agent_trace_sync cli/src/services/token_storage.rs` shows no credential DB use; inspect logs/errors for secret redaction; `nix flake check`.
  - Context synchronization: pending

- [ ] T07: `Close adapter boundary and guarded-shell descriptor races` (status:todo)
  - Task ID: T07
  - Scope: In — (A) adapter boundary-lock cancellation safety and Test F; (B) guarded-shell descriptor inheritance and Test G. Inspect `hooks/mutation_scope_lock.rs`, the Codex, OpenCode and Pi lifecycles, and the relevant Claude state boundaries; and `mutation_trace/runtime/external_mutation_guard.rs`. Out — a generic cancellation abstraction unless unavoidable, removing guarded-shell FD inheritance (it is part of the guard protocol), per-adapter test duplication, restoring deleted adapter suites.
    - **A. Boundary locks.** The boundary lock is dropped when its async owner is cancelled, but an already-started `run_locked_blocking` state transition may continue. Choose the smallest correct design (for example a lock-ownership lease retained by the started blocking operation, or cancellation-safe ownership transfer) guaranteeing: a state transition cannot outlive the boundary protection it requires; new boundaries cannot observe delayed writes from an older cancelled boundary in an invalid order; lock acquisition and worker-shutdown behavior are understood and documented; existing recovery transitions remain fail-closed.
    - **B. Guard descriptors.** Parent lock/lifetime descriptors must stay `FD_CLOEXEC` (created atomically close-on-exec: `pipe2(O_CLOEXEC)` / `F_DUPFD_CLOEXEC` on Linux, with a documented treatment where macOS lacks an atomic variant); only the intended guarded child receives them through post-fork-safe Unix child setup (reviewed `pre_exec` or equivalent); duplication is race-safe; lock and lifetime semantics are unchanged on Linux and macOS.
  - Dependencies: T02, T03
  - Done when: Adapter boundary cancellation and guarded-shell descriptor inheritance are safe under the specifically characterized races. Test F: pause a state update inside a blocking worker, cancel the outer boundary caller, attempt a second boundary, and verify it cannot observe or act on an invalid intermediate state or overtake the first transition; use the shared lock primitive if that proves the invariant for all adapters and add a per-adapter test only for a genuinely different transition protocol. Test G: launch an unrelated subprocess concurrently with guarded-shell setup; the intended guarded child retains its lock/lifetime descriptors, the unrelated child does not, and the unrelated child's lifetime does not delay guarded-shell completion (no large subprocess matrix; reuse the existing `guarded_shell_*` tests for the lock/lifetime protocol).
  - Verify: Run Tests F and G through the Cargo wrapper, each preceded by a `-- --list` check; inspect descriptor creation/duplication sites for atomic close-on-exec and the `pre_exec` body for post-fork-safe operations; `nix flake check`.
  - Context synchronization: pending

- [ ] T04: `Remove obsolete DB runtime lifetime blocking scopes` (status:todo)
  - Task ID: T04
  - Scope: In — remove obsolete command_registry Setup/Doctor/DB-backed Hooks compatibility scopes, sync storage construction, SyncStorageGuard Drop and runtime-drop test fixtures; confirm T02 removed all credential DB blocking wrappers; audit every retained production `spawn_blocking`/`block_in_place` after the T03/T06/T07 fixes using source inspection and those tasks' focused tests plus existing tests (for example the `protected_worktree.rs` cancellation tests). Out — a separate test for every `spawn_blocking` site; removing a blocking worker that intentionally provides cancellation protection; indiscriminate deletion of independently necessary synchronous external API protection or detached cleanup tasks/leaks. If a blocking scope prevents T02's native async/static closure from compiling, remove it in T02 and record it here; T04 never legitimizes an intermediate executor bridge.
  - Dependencies: T02, T03, T06, T07
  - Done when: Zero DB-runtime-related block_in_place, zero blocking SyncStorageGuard Drop, zero token-storage DB spawn_blocking; every scope solely needed for DB runtime construction/destruction is removed; the question whether SyncStorageGuard expresses an invariant ordinary ownership cannot express is answered (remove it if not); scoped storage outlives borrowing futures and drops safely on errors/cancellation without detached cleanup or leaks. An audit table lists each retained site — advisory lock acquisition; adapter state transactions; worktree lock leases; external-taint marker fsync/persistence and clearing; Git ref mutations; guarded-shell supervision; temporary-index cleanup; credential encryption-key lookup; any site added by T06/T07 — with: why blocking execution is required; who owns the operation after caller cancellation; which lock or resource stays held; and its termination/shutdown behavior (including whether runtime shutdown can wait). The original runtime-bridge-removal criteria (AC1, AC9, AC17) stay intact.
  - Verify: Run Source audit and inspect all production blocking sites/SyncStorageGuard occurrences; run the focused tests from T03/T06/T07 and any surviving relevant existing tests; confirm no replacement DB bridge or runtime-containing fixture inside async tests.
  - Context synchronization: pending

- [ ] T05: `Record application-owned persistence, the actual PR2 scope and the revised PR staging` (status:todo)
  - Task ID: T05
  - Scope: In — existing architecture/persistence/consumer context owners, relevant drift repairs, ADR update or convention-compliant immutable successor (ADRs are immutable; a changed decision gets a new dated record), context-map references and precise PR3/PR4 handoff. Correct the mismatch between the original PR2 scope and the implementation, which expanded beyond Turso conversion to include async Git subprocesses, filesystem operations off Tokio workers, worktree lock leases, cancellation-shielded Git ref mutation, external-taint marker ownership, adapter state transaction workers and guarded-shell supervision. Fix outdated statements in `context/patterns.md`, `context/architecture.md` and the relevant shared persistence and mutation-trace owners; document the verified cancellation-protection model, exclusive transaction ownership, credential-refresh guarantees and limitations, adapter lifecycle protection, guard FD inheritance, the sync early-termination policy and temporary-index cleanup limitations, claiming nothing stronger than code and tests establish. Record the Deferred cleanup ledger as the PR3 handoff. Out — duplicate architecture documents, claiming all service logic is async, new behavioral tests, final validation execution, telemetry implementation.
  - Dependencies: T02, T03, T04, T06, T07
  - Done when: Durable owners describe application Tokio runtime → static async command dispatch → static async service/lifecycle/hook dispatch → static async persistence → Turso and all mechanically propagated consumers, including generic credentials and cached-result async auth initialization; name deleted bridge/helpers and removed/retained blocking sites with reasons (from T04's audit table); preserve database policy facts; stage PR1 (complete), PR2 (async core, required consumers, Git/process/filesystem async work as actually implemented, cancellation protections, obsolete blocking removal), PR3 (post-migration cleanup/runtime audit per the ledger), PR4 OpenTelemetry. Correct only relevant recorded drift, including no-migration hooks, repository lifecycle, migration 006 and configured LocalDb retries.
  - Verify: Cross-check context/decision descriptions against source and task evidence (every stated guarantee points to a passing test, a type-level argument or a named limitation), check owner links and document diff whitespace. Repository-wide final checks remain exclusively in Full validation.
  - Context synchronization: pending

## Open questions

Non-blocking questions added by the 2026-10-08 review update (none changes scope or ordering):

- Whether pinned Turso 0.8.1 permits checked `Transaction::new(&mut Connection, Immediate)` with the adapter's ownership is unverified; T03 decides from source and records the equivalent static proof if not.
- T06's cancellation-safe refresh design (shielded owned unit vs. restructuring the timeout) and T07's lock-ownership design are selected by those tasks against source evidence; both must satisfy the stated requirements and neither may add a generic cancellation framework.
- Physical stack order is T03, T06, T07, T04, T05 so every task's dependencies precede it while IDs stay stable; selection by plan order with satisfied dependencies yields T03 → T06 → T07 → T04 → T05.

Original resolution, retained: None. Static dispatch for all synchronous/asynchronous seams in the migrated closure, existing AppContext capabilities, generic credentials/resolvers/policy/conformance/repair/logging, static stream operations, native async/static enum lifecycle and command/hook dispatch, Result-valued async-once success/failure caching, direct auth/mutation/export awaits, blocking policy, lifetime cleanup, and PR staging are resolved above. T01 inventories source paths and T03 proves cancellation/retry behavior within this fixed architecture; neither leaves an architectural choice deferred.
