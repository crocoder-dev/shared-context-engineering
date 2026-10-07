# Plan: CLI single Tokio runtime — PR1

## 1. Current-state findings

This PR changes the execution model: one application Tokio runtime drives directly awaited command execution. It removes auth/sync command runtime ownership while retaining synchronous service lifecycles and the existing Turso adapter. The repository does not already provide this boundary. The smallest useful migration is the app/dispatch/auth/sync call chain; converting persistence or every command would increase scope without satisfying an additional PR1 requirement.

Inspection baseline: `cli/src/main.rs`, `app.rs`, `services/app_support.rs`, `services/command_registry.rs`, `services/observability/traits.rs`, `services/auth_command/{command.rs,mod.rs}`, `services/auth.rs`, `services/sync/{command.rs,sync.rs,progress.rs,render_sync.rs,auto_sync.rs}`, `services/agent_trace_sync/{mod.rs,control_plane.rs,test_http_server.rs}`, `services/db/mod.rs`, `cli/Cargo.toml`, `cli/Cargo.lock`, `nix/flatpak/cargo-sources.nix`, `flake.nix`, and `.sce/config.json`, informed by the loaded durable-context brief.

- `main` calls synchronous `app::run`. The app separates dependency checking, startup resolution, concrete dependency initialization, command parsing/execution, and final output rendering. `try_run_with_dependency_check` currently embeds command execution in a synchronous `Result::map` closure.
- `AppRuntime` owns logger, `NoopTelemetry`, filesystem, Git, catalog and startup diagnostic. `AppContext` borrows the concrete dependencies. Capability traits use associated types; `ContextWithRepoRoot` preserves them. `RuntimeCommand` is an enum; `CommandRegistry` supplies deterministic names. `LifecycleProvider` is also static enum dispatch. None requires type erasure.
- `run_command_lifecycle` wraps dispatch with `Telemetry::with_default_subscriber`, currently a synchronous `&mut dyn FnMut() -> Result<String, CliError>`. `NoopTelemetry` is the production implementation. Arguments are taken from an `Option` so repeated invocation returns `REPEATED_COMMAND_DISPATCH_ERROR`. The wrapper must await execution rather than return an unpolled future after its scope ends.
- Auth has `AUTH_RUNTIME`, a `OnceLock<tokio::runtime::Runtime>`, and `shared_runtime()` constructing a current-thread runtime. `run_login`, renewal/device helpers and `run_whoami` call `block_on` over already-async WorkOS/Control Plane operations. Logout and render/config/storage helpers are synchronous.
- Sync has `SYNC_RUNTIME`, `OnceLock<Runtime>`, `shared_runtime()`, and `runtime.block_on(run_sync_async(...))`. Eight orchestration wrappers, including test-only wrappers, propagate into this bridge. The underlying stream engine is already async.
- Sync makes one initial `/state` call, then polls messages, parts and agent_traces concurrently through `try_join_three`. It retains `Rc<RefCell<...>>` for borrowed progress; the future is intentionally not required to be `Send`. Each stream processes batches sequentially. `diff_traces` is a local compatibility report entry that echoes the server cursor, not a fourth uploaded stream.
- `AuthenticatedControlPlaneClient::load_credentials` and `save_credentials` are the two production `spawn_blocking` sites. They isolate synchronous storage/keyring work; refresh locking and HTTP remain async. Auth's existing direct `token_storage` calls are synchronous and its auth DB singleton is separate persistence infrastructure.
- `services/db/mod.rs` owns `build_current_thread_runtime`, `block_on_isolated`, and the synchronous `TursoDb`/`EncryptedTursoDb` APIs. The bridge detects `Handle::try_current()` and invokes its runtime on a fresh scoped thread when called inside Tokio. PR1 makes that path more common. `TursoCore` also owns the adapter runtime with no custom blocking Drop. Inspection of the pinned Tokio 1.52.3 source (`runtime/blocking/pool.rs:263`, `runtime/blocking/shutdown.rs:37–52`) shows runtime shutdown calls `wait(None)`, which requires a blocking region and panics inside async execution even without spawned blocking workers. Therefore directly dropping command-local DB storage in the new async lifecycle is a concrete incompatibility, not just a speculative risk. Caller-side construction/destruction boundaries are required while DB production remains unchanged.
- Direct Tokio features are currently `rt`, `io-util`, `sync`, `time`, with `default-features = false`. The lockfile pins Tokio 1.52.3 and does not currently include `tokio-macros`. Adding the entrypoint macro also affects checked-in Flatpak Cargo sources and their fixed-output hash.
- Existing sync orchestration tests live inline in `services/sync/sync.rs`; control-plane tests have a test-only `block_on` helper and `RuntimeCheckingCredentialStore`. Stream engine tests also have test-only runtimes. Those test fixtures are not production runtime ownership. `app.rs` currently has no inline tests; `app_support.rs` tests centralized error rendering but not the complete command boundary.
- Existing post-commit auto-sync deliberately launches a detached child process. Preserve this behavior; no other hook becomes a background Tokio task.
- `.sce/config.json` prefers `nix flake check` over direct Cargo verification and requires ad-hoc utilities through Nix. Targeted Rust tests, when necessary, use `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml ...`. Generated assistant output must remain ephemeral.

The loaded context reports relevant drift: `context/architecture.md` describes four concurrently uploaded streams and omits the already-present Tokio `sync` feature. `context/cli/capability-traits.md` prohibits borrowing across await points without distinguishing scoped borrowing. Correct these touched descriptions in the documentation task; avoid unrelated metadata cleanup.

## 2. Architectural decisions

1. Use `#[tokio::main(flavor = "multi_thread")] async fn main() -> ExitCode { app::run(std::env::args()).await }`. Add only `macros` and `rt-multi-thread` to the existing explicit Tokio feature list. Keep defaults disabled; do not add `full`, tune worker counts, or add a scheduler library.
2. Await the complete command before rendering/returning. Keep startup computations, parsing, formatting, output writers, domain operations and synchronous command implementations synchronous wherever they do not propagate async work.
3. Keep direct enum dispatch. Only auth/sync arms await service execution; existing synchronous command APIs run inside a scoped `tokio::task::block_in_place` closure at the enum boundary. This allows their DB/keyring resources to be constructed and destroyed in a blocking region without requiring `Send`, `'static`, or detachment. Keep pure help/version/completion paths synchronous as well; grouping synchronous enum arms under this boundary is acceptable and avoids missing indirect lifecycle-provider DB owners. Do not introduce `async-trait`, new `BoxFuture`, command trait objects or a `Send` requirement. Existing stream-level `SyncFuture` is outside command dispatch and remains unchanged.
4. Adapt the telemetry wrapper to a statically dispatched async method taking generic `FnMut() -> Fut`, with `Fut: Future<Output = Result<String, CliError>>`. `NoopTelemetry` awaits the action once. Keep the wrapper around future polling, the repeat guard, error classification and lifecycle event ordering. Do not install a new subscriber or change logging behavior.
5. Preserve scoped borrowed DI. `&AppContext` through a directly awaited command is sound: `AppRuntime` outlives the context and the future; dispatch does not detach or require `'static`. `#[tokio::main]` does not require its main future to be `Send`. Generic `&C` and `&mut W` arguments may be borrowed until completion without new `Send + Sync` bounds. Resolve a repository root before sync I/O and pass the resulting path and needed dependencies, rather than storing the context in orchestration state. Update the blanket await prohibition to this precise scoped rule. Introduce explicit lifetimes only if a compiler error demonstrates a need.
6. The telemetry closure needs a single concrete returned future without escaping reborrows of captured mutable writers. Put both collected arguments and the mutable stderr reference in `Option`s, take them at closure invocation, then construct an `async move` block. Copy shared runtime/context references into the block. A second call returns the same runtime-classified repeat error; preserve its existing log behavior and avoid panics. Do not replace `FnMut` with `FnOnce` and lose the guard.
7. Auth callback helpers should accept generic futures. For `run_login_with_stored_credentials`, pass owned `StoredTokens` to the renewal callback, then borrow within its `async move` future. This avoids a future-returning closure tied to a short-lived callback argument without HRTB boxing or token cloning. Logout becomes an async orchestration wrapper solely to await its synchronous credential operation.
8. Retain both control-plane `spawn_blocking` calls and their awaited join/error mapping. Wrap auth's direct `token_storage::load_tokens`, `save_tokens`, and `delete_tokens` call sites in awaited owned `spawn_blocking` closures too, preserving the synchronous token-storage API and existing storage error classification; map a failed blocking join through existing unexpected-failure handling. Move tokens into save closures rather than retaining borrowed callback input. This isolates actual keyring/runtime resources and is a narrow caller adaptation, not a storage redesign.
9. Leave DB production code and policies unchanged. The retained adapter runtimes are an explicit exception to one **application-level** runtime. PR1 does not claim one runtime object in the entire process. Sync must construct storage inside `block_in_place`, retain borrowed reader access across async HTTP, and destroy storage inside `block_in_place` after the awaited result, including error returns. Keep ownership in a small private concrete sync-side cleanup guard so future cancellation/unwinding cannot drop the runtime directly in async context; the guard takes and drops storage in its blocking scope. No DB API, runtime helper or retry changes. Async tests must similarly construct/drop their DB fixtures within blocking regions.

Assumptions: use default multi-thread worker configuration, existing Rust generic async support and current repository test fixtures. No new library or public CLI option is needed. Open questions: None; the request supplies the migration boundary and behavioral constraints.

## 3. Scope

In scope: entrypoint, minimal Tokio/lock/Flatpak metadata, async app and execution plumbing, narrow telemetry callback signature adaptation, auth/sync orchestration signatures and awaits, required caller-side DB/keyring blocking scopes, related regression tests, and current architecture context.

Atomic migration requirement: the first commit that enables the top-level runtime must also remove auth/sync `block_on` ownership and propagate all necessary signatures. Landing `main` first with the old command bridges would cause nested-runtime panics. Dependency preparation can precede this atomic boundary.

## 4. Explicit non-goals

- No async Turso API, persistence migration, changes to `block_on_isolated` or DB runtime construction, retry/deadline/busy-timeout adjustments, contention behavior changes, transaction replay changes, WAL changes or migration changes.
- No async `LifecycleProvider`/`ServiceLifecycle` APIs, generic conversion of every service, async filesystem/Git rewrite, new background hook tasks, sync scheduling redesign or cancellation policy.
- No changed WorkOS refresh rules, credential formats, authentication error behavior, HTTP retries, cursor authority, reconciliation policies, telemetry implementation or progress design.
- No dynamic DI/command dispatch, replacement of `AppRuntime`/`AppContext`, global Tokio handle singleton, command-owned runtime convenience helper, or additional top-level runtime.
- No new flags, output/help/error strings, JSON fields, exit codes or user-visible behaviors. No unrelated generated config edits or context cleanup. No implementation, tests execution or commits during plan authoring.

## 5. Ordered implementation tasks

### Task context synchronization lifecycle

Every task carries `pending | synced | blocked`. A completed task must be `synced` before another task starts or the plan finishes. Record transitions in this file. For `blocked`, persist **Blocker**, **Required action**, and **Retry condition** next to the status; never infer synchronization from chat history. Task-local verification is part of each change; final full validation is specified under section 7 rather than a separate task.

- [ ] T01: `Prepare minimal Tokio features and packaging parity` (status:todo)
  - Task ID: T01
  - Goal: Make the existing build support one macro-based multi-thread runtime.
  - Files likely affected: `cli/Cargo.toml`, `cli/Cargo.lock`, `packaging/flatpak/cargo-sources.json`, `nix/flatpak/cargo-sources.nix`.
  - Scope: In — required features and resulting lock/package metadata. Out — entrypoint activation, version upgrades or unrelated lock churn.
  - Exact change: Retain `default-features = false` and `rt`, `io-util`, `sync`, `time`; add `macros`, `rt-multi-thread`. Resolve only the necessary lockfile additions using Cargo through the Nix dev shell. Regenerate Flatpak Cargo sources with `nix run .#regenerate-cargo-sources`; refresh the generator fixed-output hash in `nix/flatpak/cargo-sources.nix` from the regenerated derivation's reported expected hash when required. Keep the existing generator pipeline; do not hand-edit crate entries or replace the pinning scheme.
  - Behavioral invariants: No execution behavior changes; no broad features, crate upgrades, config generation output or CLI contracts change.
  - Done when: The macro/multi-thread features and minimal lock changes are available, and Flatpak metadata matches the lockfile.
  - Verification: `nix develop -c ./scripts/run-cli-cargo.sh build --manifest-path cli/Cargo.toml`; inspect dependency diff for only required additions; build `.#checks.x86_64-linux.cargo-sources-parity` on the repository's Linux validation platform.
  - Dependencies: none.
  - Context synchronization: pending

- [ ] T02: `Migrate the complete app, dispatch, auth and sync execution boundary atomically` (status:todo)
  - Task ID: T02
  - Goal: Auth/sync async I/O reaches the one application runtime through static dispatch.
  - Files likely affected: `cli/src/main.rs`, `app.rs`, `services/app_support.rs`, `services/command_registry.rs`, `services/observability/traits.rs`, `services/auth_command/{command.rs,mod.rs}`, `services/sync/{command.rs,sync.rs}`; narrowly affected test callers discovered by signature search.
  - Scope: In — one connected signature/await migration and existing caller/test adaptation. Out — new domain behavior, persistence changes or stream refactors.
  - Exact change, app: Make `main`, `app::run`, `run_with_dependency_check`, `run_with_dependency_check_and_streams`, `try_run_with_dependency_check`, `run_command_lifecycle`, `app_support::execute_command_phase`, `RuntimeCommand::execute`, and `RuntimeCommand::execute_with_stderr` async. Await their delegates. In enum dispatch, execute the existing synchronous service arms inside scoped `block_in_place` so synchronous lifecycle/provider DB construction and destruction happen in a permitted blocking region; await auth/sync arms directly. No runtime/Handle block_on. Restructure the synchronous startup `Result` chain so startup errors retain their current `RunOutcome` and command execution can be awaited before moving `runtime.logger` into the outcome. Keep `perform_dependency_check`, `build_startup_context`, `initialize_runtime`, `parse_command_phase`, `render_run_outcome`, registry construction and all rendering helpers sync.
  - Exact change, telemetry/context: Apply section 2's generic async telemetry signature to `Telemetry::with_default_subscriber` and `NoopTelemetry`. Preserve single-use argument/stderr extraction and event order; await parsing/execution inside its action future. Retain context/runtime ownership through completion and update the `AppContext` doc comment to allow scoped awaited borrowing. Do not spawn the command or add erased futures.
  - Exact change, auth: Make `AuthCommand::execute`, `run_auth_subcommand`, `run_auth_subcommand_with`, `run_login`, `run_logout`, `run_whoami`, `maybe_renew_stored_credentials`, `run_login_with_stored_credentials`, `run_text_login_with_runtime` and `run_login_json` async. Rename the now runtime-free text helper to `run_text_login`; remove its runtime argument and every other auth runtime argument. Replace existing `block_on` calls with awaits on the same operations, with the same error mapping and storage/prompt ordering. Adapt injected dispatch callbacks to `FnOnce(AuthFormat) -> Fut` (separate generic future types), renewal/device callbacks similarly, with owned renewal input as described above. Await owned `spawn_blocking` closures around existing direct load/save/delete operations, preserving storage errors and mapping join failures through existing unexpected-auth-error classification. Keep token storage APIs, config resolution, prompt writes and renderers sync. Delete `AUTH_RUNTIME`, its `OnceLock` import and `shared_runtime()` only.
  - Exact change, sync: Make `SyncCommand::execute`, `execute_with_stderr`, `execute_with_stderr_and_clock` async. Await sync before `finish_successfully` and final rendering. Make `run_current_sync`, `run_current_sync_with_progress`, `run_current_sync_with_progress_and_clock`, `run_current_sync_without_progress`, `run_sync_against`, `run_sync_against_with_progress`, `run_sync_against_with_progress_and_clock`, `run_sync_against_without_progress` async; await delegates. Replace `runtime.block_on(run_sync_async(...))` with `run_sync_async(...).await`. Remove `SYNC_RUNTIME`, `shared_runtime()` and unused runtime/OnceLock imports. Leave `run_sync_async`, `sync_one_stream`, `try_join_three`, `Rc<RefCell<_>>`, state acquisition, cursor classification, retries and progress ordering otherwise unchanged.
  - Exact change, sync storage lifetime: Run `resolve_agent_trace_storage` within `block_in_place`, including its failure cleanup. Own the returned storage in a private concrete caller-side cleanup guard before any subsequent fallible setup/await, exposing borrowed metadata/DB to unchanged orchestration. Evaluate the complete async operation result, then release storage in `block_in_place` before returning it. Guard Drop uses the same blocking scope for early return/unwinding/cancellation. Do not change storage or DB API types, retries, bridge or stream futures.
  - Exact change, tests: Convert inline sync tests that call migrated wrappers to `#[tokio::test(flavor = "multi_thread")] async fn` and await calls. Construct and destroy their owned DB fixtures via `block_in_place`/the test cleanup scope so runtime shutdown is safe; do not disable failing fixture cleanup. Retain every assertion and all fixtures. Adapt any existing injected auth callbacks with ready async closures. Pure renderer/classification/parser tests remain synchronous. Do not wrap async command calls in a newly constructed runtime while already in an async test.
  - Behavioral invariants: All CLI contracts; static DI/enums; one initial state call; three concurrent remote streams with sequential batches; compatibility diff report; awaited hooks; subscriber scope, repeated-dispatch error and log order; unchanged auth refresh/persistence and control-plane blocking boundaries.
  - Done when: App execution is directly awaited; auth/sync contain no production runtime construction/block_on helper; every existing caller compiles and retains assertions; DB production diff is empty.
  - Verification: Build through `scripts/run-cli-cargo.sh` under Nix. Run focused `services::sync::sync::tests` and `services::auth_command::tests` via the same wrapper. Inspect the runtime cleanup searches in section 7 and unchanged DB/control-plane production diffs.
  - Dependencies: T01.
  - Context synchronization: pending

- [ ] T03: `Prove auth and credential blocking behavior under the application runtime` (status:todo)
  - Task ID: T03
  - Goal: Auth command migration preserves refresh/fallback/output and existing credential isolation.
  - Files likely affected: inline tests in `services/auth_command/mod.rs`, `services/agent_trace_sync/control_plane.rs`; reuse `services/agent_trace_sync/test_http_server.rs` only if fixture extension is necessary; narrow test injection seam in auth orchestration if required.
  - Scope: In — behavioral regression tests and minimal test seams. Out — live WorkOS/keyring access, auth policy changes, replacing storage architecture.
  - Exact change: Add multi-thread Tokio tests invoking the migrated injected auth dispatcher/login helper from an already-running runtime: dispatch selects login/logout/whoami once; stored-token renewal returns the same report without device login; renewal failure falls back to device flow once; persistence failures retain the same typed source/classification; awaits complete before output. Exercise an actual runtime-free auth HTTP helper against the existing local HTTP fixture with bounded responses, preserving text prompt timing and JSON shape. Use a minimal client/base-url test seam rather than changing production endpoint resolution or credentials. Test public command unauthenticated paths using isolated state in subprocesses if needed.
  - Exact change, control plane: Retain `credential_store_operations_run_outside_the_async_runtime` and its `RuntimeCheckingCredentialStore` check. Add a multi-thread runtime refresh test whose credential store validates both `load` and `save` occur on a blocking boundary (including a nested runtime check in that blocking callback, as the existing load test does); assert awaited completion and error propagation. Exercise expired-token and unexpected-401 paths with local HTTP/memory storage. Reuse existing refresh single-flight tests under the multi-thread flavor without reducing simultaneous requests or counts.
  - Behavioral invariants: WorkOS loading/refresh, refresh-once/retry-once, persistence and typed failures, secret redaction, prompt/report behavior. Both existing control-plane `spawn_blocking` calls remain intact; auth's synchronous storage API is isolated by its awaited caller closures.
  - Done when: Migrated auth command/helper tests run inside an existing multi-thread runtime; both credential load/save are proven isolated; refresh/401 assertions remain exact.
  - Verification: Targeted wrapper tests for `services::auth_command::tests` and `services::agent_trace_sync::control_plane::tests`, especially `expired_token_is_refreshed_and_saved`, `concurrent_expired_tokens_share_one_refresh_and_save`, `unexpected_401_refreshes_once_and_retries_once_on_success`, `unexpected_401_twice_fails_without_a_third_attempt`, and the blocking-isolation tests.
  - Dependencies: T02.
  - Context synchronization: pending

- [ ] T04: `Cover async app output, temporary Turso integration and hook completion` (status:todo)
  - Task ID: T04
  - Goal: Detect lifecycle/output/nested-runtime regressions at the real boundary.
  - Files likely affected: new inline tests in `cli/src/app.rs`; `services/app_support.rs` tests; `services/observability/traits.rs` tests; test-only additions to `services/db/mod.rs` and selected existing hook tests; `services/sync/sync.rs` tests.
  - Scope: In — behavioral tests at migrated boundaries. Out — DB production or hook semantic changes, generalized test framework.
  - Exact change, app: Exercise `run_with_dependency_check_and_streams` from multi-thread Tokio tests with captured byte writers: help/no args, unknown command and unknown subcommand closest-parent help, version text/JSON, completion and config output, actual parse failure, dependency-check failure, startup invalid-config diagnostics, successful payload newline behavior and stdout write failure. Assert current exit classes (0, 2, 3, 4, 5 as applicable), stdout/stderr bytes and ordering; derive expected strings from the existing contract, not new snapshots that bless changed behavior. Isolate filesystem/environment/cwd tests using existing repo guards or subprocesses. Retain all existing `app_support` rendering assertions.
  - Exact change, telemetry: Add a recording test implementation supporting the generic async wrapper; assert dispatch logs before/after the awaited work, error paths omit success events, and a repeated action invokes no second parse/command, returning the same `REPEATED_COMMAND_DISPATCH_ERROR`. Ensure instrumentation surrounds future polling, not only future creation.
  - Exact change, Turso/hooks: Add test-only compatibility coverage invoked from the multi-thread runtime that constructs and destroys a temporary `TursoDb` in caller `block_in_place` scopes, while reads/writes through its unchanged bridge are also exercised directly inside async execution. Test migrations, success, early setup/HTTP failure and cleanup so DB runtime Drop cannot panic. Exercise the sync storage guard's cleanup when the async operation is dropped, without adding a product cancellation policy. Exercise `EncryptedTursoDb`/auth storage through established isolated credential fixtures or a subprocess without contacting a user's keyring. Invoke representative synchronous doctor and persistence-backed hook command paths through async enum/app dispatch with isolated repository/state; assert DB effects are committed and hook payloads returned before command completion. Cover Claude, Codex, OpenCode, Pi and Git hook adapter output/completion using their existing fixtures; run existing adapter suites unchanged. Preserve the explicitly detached post-commit auto-sync process test. Test the real application-context caller scopes rather than hiding all DB ownership outside the async test.
  - Exact change, sync: Keep `concurrent_sync_overlaps_all_three_stream_batches_after_one_state_request` and `concurrent_sync_keeps_batches_sequential_within_one_stream` meaningful under the top-level runtime flavor, together with progress/cursor/reconciliation tests. Add coverage if needed for JSON without human stderr and text progress finishing only after awaited success.
  - Behavioral invariants: DB code/policies stay unchanged; no nested runtime or runtime-drop panic; deterministic output/classification; synchronous hook completion; original concurrency and retry policies.
  - Done when: Real async app/enum execution covers sync commands and persistence-backed hooks, DB lifecycle is tested inside Tokio, and existing concurrency/output assertions pass without weakening.
  - Verification: Focused wrapper tests for app, app_support, telemetry, DB compatibility, affected hooks and `services::sync::sync::tests`. Review production DB diff as empty. Prove the required blocking construction/destruction scopes prevent the pinned Tokio shutdown panic on success and failure. Any additional incompatibility requiring DB production changes is a PR1 scope blocker, not permission to change policies or weaken tests.
  - Dependencies: T02, T03.
  - Context synchronization: pending

- [ ] T05: `Document the staged runtime architecture and scoped DI borrowing` (status:todo)
  - Task ID: T05
  - Goal: Durable context describes the completed PR1 architecture and PR2 boundary accurately.
  - Files likely affected: `context/architecture.md`, `context/overview.md`, `context/patterns.md`, `context/cli/cli-command-surface.md`, `context/cli/capability-traits.md`, `context/cli/sync-command.md`, `context/cli/agent-trace-sync-command.md`, `context/sce/cli-observability-contract.md`, `context/sce/shared-turso-db.md`; narrowly update `context/sce/cli-exit-code-contract.md`, `context/sce/cli-stdout-stderr-contract.md`, `context/context-map.md` only where boundary ownership/annotations need adjustment.
  - Scope: In — new execution boundary and touched context drift. Out — unrelated docs, new ADR workflow, documenting PR2 as implemented.
  - Exact change: Record one application Tokio multi-thread runtime, direct async enum dispatch, awaited auth/sync/control-plane HTTP, retained credential `spawn_blocking`, synchronous lifecycle providers and temporary synchronous Turso adapter. Replace blanket AppContext await prohibition with scoped borrowing/owner lifetime guidance. Describe the async telemetry polling scope and preserved repeat guard. Correct architecture's stream count to three uploaded streams plus diff compatibility report and explicit Tokio features. Preserve exit/output contracts while refreshing affected helper ownership references. Link the existing owners rather than creating duplicate runtime documentation.
  - Behavioral invariants: Documentation cannot claim Turso async, sole runtime object process-wide, new detached hooks, changed output or changed refresh/sync policies.
  - Done when: Durable owners match final code and clearly separate PR1 application runtime ownership from retained PR2 DB infrastructure.
  - Verification: Inspect every touched paragraph against source and section 9; search touched docs for stale auth/sync command-runtime ownership and blanket borrow-across-await prohibition.
  - Dependencies: T02, T03, T04.
  - Context synchronization: pending

## 6. File-by-file impact

| File/surface | Planned impact |
| --- | --- |
| `cli/Cargo.toml` | Two Tokio feature additions, defaults still disabled. |
| `cli/Cargo.lock` | Minimal macro dependency resolution; retain existing versions where possible. |
| `packaging/flatpak/cargo-sources.json`, `nix/flatpak/cargo-sources.nix` | Generator parity and resulting fixed-output hash. |
| `cli/src/main.rs` | Sole application runtime macro and awaited `app::run`. |
| `cli/src/app.rs` | Five async run/lifecycle functions, ownership-safe startup flow, scoped borrowed context, new app tests. |
| `cli/src/services/app_support.rs` | Async execution phase; render/error/output helpers unchanged; regression coverage. |
| `cli/src/services/command_registry.rs` | Async execute methods, direct awaits in auth/sync arms and scoped block_in_place for synchronous command arms; catalog/enum/other command APIs retained. |
| `cli/src/services/observability/traits.rs` | Generic async telemetry action/polling, Noop adaptation, regression tests. |
| `cli/src/services/auth_command/command.rs` | Async command execute wrapper. |
| `cli/src/services/auth_command/mod.rs` | Async orchestration/helper callbacks including logout, owned credential blocking callers, runtime removal, tests; sync rendering/storage APIs retained. |
| `cli/src/services/sync/command.rs` | Async wrappers; awaited progress lifetime and completion. |
| `cli/src/services/sync/sync.rs` | Eight async wrappers, runtime removal, caller blocking construction/destruction with concrete cleanup guard, inline test fixture adaptation; engine/join unchanged. |
| `cli/src/services/agent_trace_sync/control_plane.rs` | Test additions/adaptations only; production credential isolation, refresh and HTTP policies retained. |
| `cli/src/services/agent_trace_sync/test_http_server.rs` | Reuse fixture; extend only if required by focused tests. |
| `cli/src/services/db/mod.rs` | Test-only compatibility additions; no production bridge/API/policy changes. |
| Existing hook test modules | Runtime-context dispatch/completion regression additions where needed; production adapters unchanged. |
| Durable context owners listed in T05 | Accurate current async application boundary and temporary DB exception. |

Inspected unchanged production surfaces: `services/auth.rs` (already async HTTP), `services/token_storage.rs`, `services/agent_trace_sync/mod.rs`, `services/sync/{progress.rs,render_sync.rs,auto_sync.rs}`, `services/lifecycle`, synchronous service-owned command implementations, persistence/mutation trace/hook DB services, parser, error catalog and output format. Any newly discovered signature caller may receive a narrow await adaptation only.

## 7. Tests / validation

Use `.sce/config.json` and `AGENTS.md` policy. Planning runs no tests; commands below are implementation/final verification instructions. Full checks are listed once here. Keep exact existing assertions; runtime migration is not permission to reduce coverage or replace concurrency overlap tests with mere successful completion.

Targeted Rust template when needed: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml <module-or-test-filter>`. For a single exact test use its fully qualified name plus `-- --exact`. Do not use bare Cargo or bypass generated-payload preparation.

Required existing sync regressions, all retained in `services/sync/sync.rs`: `progress_events_cover_batches_empty_streams_and_fixed_order`, `full_sync_uploads_three_remote_streams_and_second_run_is_naturally_incremental`, `sync_succeeds_when_local_diff_trace_row_is_rejected_by_export_reader`, `diff_traces_report_echoes_server_cursor_without_uploading_newer_local_rows`, `concurrent_sync_overlaps_all_three_stream_batches_after_one_state_request`, `concurrent_sync_keeps_batches_sequential_within_one_stream`, `invalid_state_cursor_fails_before_any_batch_request`, `terminal_batch_status_fails_without_state_reconciliation`, `progress_events_end_after_terminal_failure`, `malformed_2xx_batch_response_still_reconciles_via_state`, `forbidden_state_response_fails_without_mutating_local_metadata`.

Keep control-plane tests for storage isolation, token reuse/refresh/single-flight, exact state request count/body, one 401 refresh and retry, failure after second 401, status classification and safe diagnostics. Retain engine-level `agent_trace_sync` retry/reconciliation tests and DB contention/WAL/migration tests under the normal full suite. Test-only `block_on` helpers may remain where invoked outside another runtime; no test runtime cleanup requirement is implied by production cleanup.

Production cleanup check (expect no matches in command files; distinguish separately reviewed test-only helpers):

```sh
nix shell nixpkgs#ripgrep -c rg -n 'AUTH_RUNTIME|SYNC_RUNTIME|shared_runtime|OnceLock|tokio::runtime|Builder::new_current_thread|block_on' cli/src/services/auth_command/command.rs cli/src/services/auth_command/mod.rs cli/src/services/sync/command.rs cli/src/services/sync/sync.rs
```

Because inline tests may deliberately use runtime fixtures, inspect any matching test-only lines against their `#[cfg(test)]` boundary; do not declare success from a raw match count. Prefer `#[tokio::test]` for new command tests so these command files need no replacement bridge helpers. A no-match `rg` exits 1 and is expected here.

Audit the entire source for runtime ownership, blocking and spawn sites:

```sh
nix shell nixpkgs#ripgrep -c rg -n 'tokio::runtime|\bRuntime\b|Builder::new_current_thread|block_on|shared_runtime|AUTH_RUNTIME|SYNC_RUNTIME|OnceLock|tokio::spawn|spawn_blocking|block_in_place' cli/src
nix shell nixpkgs#ripgrep -c rg -n '#\[tokio::main|rt-multi-thread|macros|default-features' cli/src/main.rs cli/Cargo.toml
nix shell nixpkgs#ripgrep -c rg -n 'build_current_thread_runtime|block_on_isolated|spawn_blocking' cli/src/services/db/mod.rs cli/src/services/agent_trace_sync/control_plane.rs
```

Classify matches as application entrypoint, retained DB bridge, unchanged persistence singleton, existing test fixture or existing blocking credential work. Expected production construction owners after PR1: `main` for application execution and shared DB for temporary adapter runtimes. Review the PR production diff to prove DB bridge/policies and the two credential blocking calls unchanged. Inspect command futures for no `tokio::spawn`, dynamic dispatch or artificial `Send` bounds.

### Full validation

- `nix flake check` — authoritative whole repository Rust tests/clippy/fmt, config/plugin and packaging checks, including Cargo-source parity; use the repository's supported platform matrix through CI.
- `git diff --check` — patch whitespace integrity.
- `nix run .#pkl-check-generated` only if implementation unexpectedly touches canonical generation inputs; this plan does not require such edits or committed generated trees.

### Context sync

T05's durable owners must describe the completed execution model. Preserve current static DI/lifecycle, hook, auth/sync and persistence contracts. Existing map annotations change only if descriptions require it. No new context bootstrap or unrelated cleanup is needed.

Expected documented architecture:

```text
SCE process
  └── one application Tokio multi-thread runtime
        ├── async app lifecycle and static command dispatch
        ├── async auth
        ├── async sync / control-plane HTTP
        │     └── awaited spawn_blocking for credential load/save
        ├── scoped blocking execution for synchronous commands
        └── temporary synchronous Turso adapter
              └── existing current-thread runtime / block_on_isolated bridge
                  (caller construction/destruction in blocking scopes)
```

## 8. Risks and migration hazards

- A partially migrated entrypoint nests auth/sync runtimes. Avoid with T02's atomic signature/ownership change, not a temporary bridge helper.
- Borrowing itself is safe while the owner lives; spawning a borrowed command or requiring `Send` would unnecessarily break the current `Rc<RefCell>` progress model. Await directly and keep runtime/context alive until completion.
- Future-returning `FnMut` closures cannot return arbitrary mutable reborrows of their captured environment. Move taken command inputs/writer references into the returned future; use owned renewal inputs. Do not respond with boxed command futures or pervasive lifetime rewrites.
- A subscriber guard surrounding future creation does not necessarily surround polling. Await inside the telemetry abstraction and verify log ordering/repeat behavior. No new telemetry semantics are authorized.
- The database bridge protects `block_on` reentry but does not protect runtime Drop: pinned Tokio shutdown panics when directly dropped in async context. Required caller-side `block_in_place` construction/destruction, sync cleanup guard, and awaited owned auth credential closures address this without DB production changes. Multi-thread Tokio is required for `block_in_place`; new command integration tests must use that flavor, while pure lower-level test runtimes may remain current-thread.
- More synchronous DB/filesystem work now occurs in a Tokio-entered context, and the existing bridge can create scoped threads. This is intentional PR1 staging; no claim of fully nonblocking persistence or performance improvement. Avoid opportunistic batching/backoff changes.
- Sync progress must remain alive across the await and finish only on success. Keep Started/Finished behavior even on failure, JSON suppression, stream ordering, and final report content.
- Global auth storage and environment/cwd mutation can contaminate tests. Use existing isolated state fixtures/subprocesses and guards; do not use a real user's database or credential store.
- Tokio macro dependencies alter reproducible packaging. Minimal lock edits without matching fixed-output metadata can fail `nix flake check`; keep T01 parity in the same commit.
- Top-level runtime initialization failures now belong to the entrypoint, whereas removed command-runtime creation failures disappear. Do not invent CLI diagnostic rewrites for ordinary execution; preserve existing classified operation failures and prove normal/error output behavior in regression tests.

## 9. Acceptance criteria

- [ ] AC1: Exactly one application-level multi-thread Tokio runtime drives `app::run`; direct Tokio defaults remain disabled and only the required features are added.
  - Validate: Inspect `cli/src/main.rs` for the multi-thread `#[tokio::main]` and awaited `app::run`; inspect Cargo feature/lock diff and classify all production runtime-construction matches using section 7's source audit.
- [ ] AC2: The app lifecycle and `RuntimeCommand` execution await auth/sync while retaining concrete enum dispatch, concrete capabilities and synchronous parsing/rendering/lifecycle providers.
  - Validate: Inspect the exact T02 call chain, auth/sync match arms and capability bounds; run app/static registry/parser tests through the full suite; confirm no new erased command future/trait objects or command spawn.
- [ ] AC3: Auth and sync contain no production `AUTH_RUNTIME`, `SYNC_RUNTIME`, runtime-owning OnceLocks, `shared_runtime()` or command `block_on` calls.
  - Validate: Run section 7's targeted cleanup search, inspect test boundaries for any matches, and verify no replacement command runtime infrastructure in the full source audit.
- [ ] AC4: Existing Turso production APIs/runtime bridge and all database policies remain unchanged while persistence-backed commands safely execute from the application runtime.
  - Validate: Inspect DB/persistence production diff as empty; run T04 construction/use/destruction/error-path compatibility tests and existing migrations/WAL/contention tests in the full suite. A runtime panic fails acceptance.
- [ ] AC5: Auth token loading/renewal/persistence, refresh single-flight and 401 retry contracts work from an existing application runtime; credential load/save remain awaited blocking operations.
  - Validate: Run T03 async auth/credential tests and existing named control-plane refresh/401 tests; inspect both production `spawn_blocking` bodies and join/error mappings as unchanged.
- [ ] AC6: Sync preserves one initial state call, three concurrent remote streams, sequential per-stream batches, cursors/reconciliation/retries, diff compatibility and progress/output.
  - Validate: Run section 7's eleven named sync tests under multi-thread async execution plus engine/control-plane regressions; assert actual HTTP overlap/order/counts and progress/JSON invariants.
- [ ] AC7: Synchronous commands retain stdout/stderr routing, exit codes, help/unknown-command behavior, JSON/text output, diagnostics, redaction and typed errors.
  - Validate: Run T04 app byte/output/exit tests and unchanged app_support/parser/rendering tests; inspect output/error/catalog diffs for no semantic changes.
- [ ] AC8: All hook adapters preserve completion, persistence, payloads and foreground semantics; only the existing explicit post-commit auto-sync child remains detached.
  - Validate: Run existing Claude/Codex/OpenCode/Pi/Git hook fixtures plus T04 async-boundary completion tests; inspect hook/auto-sync production diff and audit no new hook `tokio::spawn`.
- [ ] AC9: Telemetry surrounds awaited command polling with unchanged event order and repeated-dispatch error; borrowed AppContext remains command-scoped and owned dependencies outlive awaits.
  - Validate: Run recording/repeated-action tests from T04; inspect context lifetimes/no spawned context and T05's revised borrowing guidance.
- [ ] AC10: Reproducible packaging and durable context represent PR1 accurately, including retained synchronous Turso bridging for PR2.
  - Validate: Cargo-source parity and full validation under section 7 pass; inspect every T05 owner against final code and the staged architecture diagram. No document claims async persistence.
