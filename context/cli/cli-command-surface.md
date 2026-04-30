# SCE CLI Foundation

The repository now includes a Rust CLI crate at `cli/` for SCE automation work.

Operator onboarding currently comes from `sce --help`, command-local `--help` output, and the focused CLI context files under `context/cli/` and `context/sce/`.

## Current implemented slice

- Binary entrypoint: `cli/src/main.rs`
- Runtime shell and startup lifecycle owner: `cli/src/app.rs`
- Top-level command metadata catalog: `cli/src/cli_schema.rs`
- Custom top-level help renderer and known-command classifier: `cli/src/command_surface.rs`
- Turso adapters: `cli/src/services/local_db/mod.rs`, `cli/src/services/agent_trace_db/mod.rs`, and shared infrastructure in `cli/src/services/db/mod.rs`
- Service domains: `cli/src/services/{agent_trace_db,auth,auth_command,completion,config,db,default_paths,hooks,local_db,observability,output_format,resilience,security,setup,style,token_storage,version}` plus the split doctor module at `cli/src/services/doctor/{mod,command,inspect,render,fixes,types}.rs`; service-owned `command.rs` files now own the `RuntimeCommand` impls for help/version/completion/auth/config/setup/doctor/hooks
- Service lifecycle: `cli/src/services/lifecycle.rs` defines the `ServiceLifecycle` trait with `diagnose`, `fix`, and `setup` methods; `config`, `hooks`, `local_db`, and `agent_trace_db` services implement this trait in their respective `lifecycle.rs` files, and `doctor`/`setup` commands aggregate calls across all registered lifecycle providers
- Shared test temp-path helper: `cli/src/test_support.rs` (`TestTempDir`, test-only module)

## Onboarding documentation

- `sce --help` includes a slim top-level command list and quick-start examples for `setup`, `doctor`, and `version`; `auth` and `hooks` remain implemented in code but are hidden from `sce`, `sce help`, and `sce --help` for this phase.
- `cli/src/cli_schema.rs` owns the real top-level command catalog metadata for clap-backed commands (purpose text plus `show_in_top_level_help`), while `command_surface::help_text()` consumes that catalog and adds the synthetic `help` row plus the ASCII banner.
- `cli/src/app.rs` now owns an explicit startup lifecycle (`perform_dependency_check` -> `build_startup_context` -> `initialize_runtime` -> `run_command_lifecycle` -> `render_run_outcome`) so dependency bootstrap, config-backed runtime initialization, command parsing/execution, and final stream rendering are no longer coordinated inside one monolithic startup function.
- `cli/src/app.rs` also routes clap output through an internal `RuntimeCommand` seam (trait defined in `cli/src/services/command_registry.rs`) so parse-time conversion and run-time command execution are separated from one central dispatch match.
- Command-local help is available for implemented commands including bare `sce auth`, `sce auth --help`, `sce auth login --help`, `sce setup --help`, `sce doctor --help`, and `sce completion --help`; when stdout color is enabled those help payloads now reuse the shared heading/command/placeholder styling pass while non-TTY and `NO_COLOR` flows stay plain text. Human-readable stderr diagnostics and interactive setup prompt text now follow the same shared styling policy on their respective terminal streams.
- Current repository verification guidance for this CLI slice prefers the root Nix entrypoints: `nix flake check` for routine validation, `nix build .#default` / `nix run .#sce -- --help` for packaged installability, and targeted `nix develop -c sh -c 'cd cli && <cargo command>'` only when a narrower Rust-only check is explicitly needed.

## Nix release installability surface

- Root `flake.nix` exposes `packages.sce` and `packages.default = packages.sce` for packaged release builds.
- Root `flake.nix` exposes `apps.sce` pointing to `${packages.sce}/bin/sce` for runnable packaged CLI execution.
- Root `flake.nix` is the single repository-level Nix entrypoint for CLI checks and packaging.
- Current installability checks for this surface are:
  - `nix build .#default`
  - `nix run .#sce -- --help`

## Cargo release and future crates.io posture

- `cli/Cargo.toml` includes crates.io-facing package metadata (`description`, `license`, `repository`, `homepage`, `documentation`, `readme`, `keywords`, `categories`) and is aligned to the current crates.io publication posture described by the root release/publish workflows.
- Current local install contract is `cargo install --path cli --locked`.
- Current release build/installability checks run through the root flake (`nix build .#default`, `nix run .#sce -- --help`) so the packaged binary and embedded generated assets stay aligned with the canonical Nix-owned release path.
- Crates.io publication is now a dedicated downstream publish stage (`.github/workflows/publish-crates.yml`) that validates `.version`/tag/Cargo parity before publishing the checked-in crate version.

## Command surface contract

`sce`, `sce help`, and `sce --help` now render a top-level help surface that starts with an ASCII art "SCE" banner followed by a slim command list:

- the banner uses a per-column right-to-left color gradient (cyan on the right, magenta on the left) when stdout color is enabled, and renders as plain ASCII when color is disabled (non-TTY or `NO_COLOR`)
- the banner is rendered by `command_surface::help_text()` calling `style::banner_with_gradient(SCE_BANNER_LINES)` before the heading
- the visible real-command rows are sourced from `cli_schema::TOP_LEVEL_COMMANDS`, so top-level purpose text and help visibility are defined once for both help rendering and known-command classification
- the visible command list is `help`, `config`, `setup`, `doctor`, `version`, and `completion`
- top-level help omits implemented/placeholder labels
- top-level examples cover setup plus doctor/version machine-readable or repair-intent flows (`doctor --format json`, `doctor --fix`, `version --format json`) and use the shared example-command styling when stdout color is enabled
- `auth` and `hooks` stay parser-valid and directly invocable, but are hidden from those top-level help surfaces

Deferred or gated command surfaces currently avoid claiming unimplemented behavior.
`hooks` routes through implemented subcommand parsing/dispatch for `pre-commit`, `commit-msg`, `post-commit`, `post-rewrite`, and `diff-trace`; current behavior remains attribution-only and disabled by default for commit attribution, while `diff-trace` is active STDIN intake with required non-empty `sessionID`/`diff` plus required `u64` `time` (Unix epoch milliseconds) validation, non-lossy AgentTraceDb `time_ms` conversion, collision-safe per-invocation `context/tmp/<timestamp>-000000-diff-trace.json` writes, and AgentTraceDb insertion.
`config` exposes deterministic inspect/validate entrypoints (`sce config show`, `sce config validate`) with explicit precedence (`flags > env > config file > defaults`), a shared auth-runtime resolver for supported keys that declare env/config/optional baked-default inputs starting with `workos_client_id`, first-class `policies.bash` reporting for preset/custom blocked-command rules, and deterministic text/JSON output modes where `show` reports resolved values with provenance while `validate` reports pass/fail plus validation issues and warnings only.
`version` exposes deterministic runtime identification output in text mode by default and JSON mode via `--format json`.
`completion` exposes deterministic shell completion generation via `sce completion --shell <bash|zsh|fish>`.
`setup` defaults to an `inquire` interactive target selection (OpenCode, Claude, Both) and accepts mutually-exclusive non-interactive target flags (`--opencode`, `--claude`, `--both`); the interactive prompt title and target labels now reuse shared prompt styling helpers when stdout color is enabled.
`auth` now emits auth-local guidance for bare `sce auth` and `sce auth --help`, listing `login`, `logout`, and `status` plus copy-ready next steps.
`setup`, `doctor`, `hooks`, `version`, and `completion` all support command-local `--help`/`-h` usage output via top-level parser routing in `cli/src/app.rs`.
`setup` now also exposes compile-time embedded config assets for OpenCode/Claude targets, sourced from the generated `config/.opencode/**` and `config/.claude/**` trees via `cli/build.rs` with normalized forward-slash relative paths and target-scoped iteration APIs; the embedded asset set includes the OpenCode bash-policy plugin/runtime files generated from the canonical preset catalog (Claude bash-policy enforcement has been removed from generated outputs).
`setup` additionally includes a repository-root install engine (`install_embedded_setup_assets`) that stages embedded files, intentionally leaves generated `skills/*/tile.json` manifests in `config/` only, skips those tile files during repo-root installs, and uses a unified remove-and-replace policy for `.opencode/`/`.claude/` (removing existing targets before swapping staged content, with deterministic recovery guidance on swap failure) while treating bash-policy enforcement files as first-class SCE-managed assets.
`setup` now executes end-to-end and prints deterministic completion details including selected target(s) and per-target install count.
`doctor` now executes end-to-end with explicit diagnosis and repair-intent surfaces: `sce doctor` stays read-only, `sce doctor --fix` selects repair-intent mode, and text/JSON output expose stable mode/problem/fix-result/database-record scaffolding. The current runtime aggregates `ServiceLifecycle::diagnose` and `ServiceLifecycle::fix` calls across all registered service providers (`config`, `local_db`, `agent_trace_db`, `hooks`) plus integration checks, covering state-root resolution, global and repo-local `sce/config.json` readability/schema validation, local DB and Agent Trace DB path/health, DB-parent readiness barriers, an intentionally empty repo-scoped SCE database section for the active repository, the repo hook rollout slice when a repository target is detected, and repo-root installed OpenCode integration presence for `plugins`, `agents`, `commands`, and `skills`; those integration checks are presence-only and fail a group when any required installed file is missing. Fix mode delegates to each provider's `fix` implementation, which reuses the canonical setup hook install flow to repair missing/stale/non-executable required hooks and missing hooks directories, and it can bootstrap missing canonical database parent directories when the resolved paths match canonical owned locations.
A user-invocable `sync` command is not wired in the current CLI surface; local DB and Agent Trace DB bootstrap currently happen through `setup`, and DB health/repair currently happens through `doctor`. Command wiring for `sce sync` is deferred to `0.4.0`.

## Command loop and error model

- Argument parsing is handled by `clap` derive macros in `cli/src/cli_schema.rs` and dispatched from `cli/src/app.rs`.
- `cli/src/app.rs` now runs commands through explicit phases with `StartupContext`, `AppRuntime`, and `RunOutcome` carrying startup-derived observability config, logger/telemetry state, registry state, and final render data across the lifecycle.
- `parse_command_phase` delegates clap output conversion to `cli/src/services/parse/command_runtime.rs`, which returns boxed `RuntimeCommand` implementations (trait defined in `cli/src/services/command_registry.rs`); `services::app_support::execute_command_phase` emits lifecycle logs around `command.execute(...)` instead of branching through one central command-dispatch match.
- Top-level failures are classified into stable exit-code classes owned by `cli/src/app.rs`: `2` parse, `3` validation, `4` runtime, and `5` dependency.
- User-facing diagnostics are rendered on `stderr` as `Error [SCE-ERR-<CLASS>]: ...` with class-default `Try:` remediation appended only when missing; when stderr color is enabled the heading, error code, and diagnostic body all render through shared stderr styling helpers.
- Unknown commands/options and extra positional arguments return deterministic, actionable guidance to run `sce --help`.
- `sce setup --help` returns setup-specific usage output with target-flag contract details and deterministic examples, including one-run non-interactive setup+hooks and composable follow-up validation/repair-intent flows (`sce doctor --format json`, `sce doctor --fix`).
- `sce auth` and `sce auth --help` return auth-specific usage output with available subcommands and deterministic examples, while `sce auth <login|renew|logout|status> --help` stays scoped to the selected auth subcommand.
- `sce doctor --help` and `sce hooks --help` return command-local usage output and deterministic copy-ready examples.
- Interactive `sce setup` prompt cancellation/interrupt exits cleanly with: `Setup cancelled. No files were changed.`
- Command handlers return deterministic status messaging:
- `setup`: `Setup completed successfully.` plus selected targets and per-target install destinations/counts.
- `doctor`: current runtime emits `SCE doctor diagnose` / `SCE doctor fix` human text headers plus ordered `Environment`, `Configuration`, `Repository`, `Git Hooks`, and `Integrations` sections with bracketed `[PASS]`/`[FAIL]`/`[MISS]` row tokens, shared-style green pass plus red fail/miss colorization when enabled, simplified `label (path)` rows, top-level-only hook rows, and a deterministic summary footer; JSON output carries stable problem/fixability records plus deterministic fix-result records in fix mode and reports the neutral DB record under `local_db`. The runtime validates global and repo-local `sce/config.json` inputs plus local DB and Agent Trace DB health, keeps the repo-scoped database section empty unless a future repo-owned SCE database family is introduced, diagnoses repo hook rollout integrity plus repo-root installed OpenCode integration presence for `OpenCode plugins`, `OpenCode agents`, `OpenCode commands`, and `OpenCode skills`, and in fix mode reuses canonical setup hook installation for supported hook repairs plus bounded bootstrap of canonical missing SCE-owned DB parent directories while preserving manual-only reporting for unsupported issues.
  - `hooks`: deterministic hook subcommand status messaging for runtime entrypoint invocation and argument/STDIN contract validation.

## Service contracts

- `cli/src/services/setup/mod.rs` defines setup parsing/selection contracts plus runtime install orchestration (`run_setup_for_mode`) over the embedded asset install engine; `cli/src/services/setup/command.rs` owns the setup runtime command handler. Setup now aggregates `ServiceLifecycle::setup` calls across registered providers (`config`, `local_db`, `agent_trace_db`, `hooks`) in order, using `AppContext` with resolved repository root.
- `cli/src/services/setup/mod.rs` now keeps its larger internal responsibilities behind focused inline support modules: `install` owns repository canonicalization, staging/swap install flows, required-hook installation, and repo/writeability guards, while `prompt` owns interactive target selection and styled prompt labels.
- `cli/src/services/config/mod.rs` defines config parser/runtime contracts (`show`, `validate`, `--help`), strict config-file key/type validation, deterministic text/JSON rendering, repo-configured bash-policy preset/custom validation and reporting under `policies.bash`, and shared auth-key metadata that declares env key, config-file key, and optional baked-default eligibility for supported auth runtime values starting with `workos_client_id` (`WORKOS_CLIENT_ID` vs `workos_client_id`); auth-key provenance/preference metadata stays on `show`, while `validate` stays trimmed to validation status plus issues/warnings. `cli/src/services/config/lifecycle.rs` implements `ServiceLifecycle` for config health checks and setup (global/local config validation and repo-local config bootstrap).
- `cli/src/services/doctor/mod.rs` defines the implemented doctor request/report contract (`DoctorRequest`, `DoctorMode`, `run_doctor`) while focused submodules under `cli/src/services/doctor/` handle runtime command dispatch (`command.rs`), diagnosis (`inspect.rs`), rendering (`render.rs`), fix execution (`fixes.rs`), and doctor-owned domain types (`types.rs`). Together they preserve explicit fix-mode parsing, stable text/JSON problem and database-record rendering, deterministic fix-result reporting, and aggregation of `ServiceLifecycle::diagnose`/`ServiceLifecycle::fix` across registered providers (`config`, `local_db`, `agent_trace_db`, `hooks`). The doctor module coordinates state-root/config/database reporting and validation, an empty default repo-scoped database inventory, path-source detection plus required-hook presence/executable/content checks when a repository target is detected, repo-root installed OpenCode integration presence inventory for `plugins`, `agents`, `commands`, and `skills` derived from the embedded OpenCode setup asset catalog, shared-style bracketed human status token rendering (`[PASS]`, `[FAIL]`, `[MISS]`) with simplified `label (path)` text rows, and repair-mode delegation to service-owned fix implementations.
- `cli/src/services/version/mod.rs` defines the version parser/output contract (`parse_version_request`, `render_version`) with deterministic text/JSON output modes; `cli/src/services/version/command.rs` owns the version runtime command handler.
- `cli/src/services/completion/mod.rs` defines the completion output contract (`render_completion`) using clap_complete to generate deterministic shell scripts for Bash, Zsh, and Fish; `cli/src/services/completion/command.rs` owns the completion runtime command handler.
- `cli/src/services/hooks/mod.rs` defines production local hook runtime parsing/dispatch (`HookSubcommand`, `run_hooks_subcommand`) for `pre-commit`, `commit-msg`, `post-commit`, `post-rewrite`, and `diff-trace`; `cli/src/services/hooks/command.rs` owns the hook runtime command handler. Current runtime behavior is commit-msg-only attribution behind the disabled-default attribution gate, while `pre-commit`/`post-commit`/`post-rewrite` are deterministic no-ops and `diff-trace` performs STDIN JSON intake, required non-empty `sessionID`/`diff` plus required `u64` `time` (Unix epoch milliseconds) validation, non-lossy AgentTraceDb `time_ms` conversion, collision-safe `context/tmp/<timestamp>-000000-diff-trace.json` persistence, and command-failing AgentTraceDb insertion. `cli/src/services/hooks/lifecycle.rs` implements `ServiceLifecycle` for hook health checks, fix, and setup (hook rollout integrity and required-hook installation).
- `cli/src/services/resilience.rs` defines shared bounded retry/timeout/backoff execution policy (`RetryPolicy`, `run_with_retry`) with deterministic failure messaging and retry observability hooks.
- No `cli/src/services/sync.rs` module exists in the current codebase; `sce sync` command wiring is deferred, while local DB initialization and health ownership are split between setup and doctor.
- `cli/src/services/default_paths.rs` defines the canonical per-user persisted-location seam for config/state/cache roots plus named default file paths for current persisted artifacts (`global config`, `auth tokens`, `local DB`, `agent trace DB`) used by config discovery, token storage, database adapters, and doctor diagnostics; its internal `roots` seam now owns the platform-aware root-directory resolution so non-test production modules consume shared path accessors instead of resolving owned roots directly.
- `cli/src/services/token_storage.rs` defines WorkOS token persistence (`save_tokens`, `load_tokens`, `delete_tokens`) with shared default-path-seam resolution for the default token file, JSON payload storage including `stored_at_unix_seconds`, graceful missing-file deletion behavior, missing/corrupted-file handling, and restrictive on-disk permissions (`0600` on Unix; Windows best-effort ACL hardening via `icacls`).
- `cli/src/services/auth_command/mod.rs` defines the auth command orchestration surface (`AuthRequest`, `AuthSubcommand`, `run_auth_subcommand`) for `login`, `renew`, `logout`, and `status`, including shared text/JSON rendering, token refresh/forced renewal handling for `sce auth renew`, token-storage-backed logout deletion with path-aware remediation guidance, expiry-aware status reporting, canonical credentials-file path reporting sourced from the shared default-path seam, precedence-aware client-ID guidance sourced from the shared auth-runtime resolver instead of env-only assumptions, and a lazily initialized current-thread Tokio runtime with both I/O and time enabled so the auth flows can drive the WorkOS device/refresh paths without the prior I/O-disabled panic; `cli/src/services/auth_command/command.rs` owns the `AuthCommand` struct and its `RuntimeCommand` impl.
- `cli/src/app.rs` parses `auth`, `config`, `setup`, `doctor`, `hooks`, `version`, and `completion` into service-owned runtime command handlers so runtime messages are sourced from domain modules instead of inline strings.

## Local and Agent Trace Turso adapter behavior

- `cli/src/services/local_db/mod.rs` provides `LocalDb = TursoDb<LocalDbSpec>` with `new()`, `execute()`, and `query()` inherited from the shared Turso adapter.
- `LocalDb::new()` resolves the canonical per-user DB path through `default_paths::local_db_path()`, creates parent directories, opens the local Turso database, and currently runs zero local migrations.
- `cli/src/services/agent_trace_db/mod.rs` provides `AgentTraceDb = TursoDb<AgentTraceDbSpec>` plus `DiffTraceInsert<'_>` and `insert_diff_trace()` for parameterized writes to `diff_traces`.
- `AgentTraceDb::new()` resolves `<state_root>/sce/agent-trace.db` through `default_paths::agent_trace_db_path()`, creates parent directories through `TursoDb`, opens the Turso database, and runs the embedded `cli/migrations/agent-trace/001_create_diff_traces.sql` migration.
- `cli/src/services/local_db/lifecycle.rs` implements `ServiceLifecycle` for local DB health checks and setup (DB path/health validation and DB bootstrap).
- `cli/src/services/agent_trace_db/lifecycle.rs` implements `ServiceLifecycle` for Agent Trace DB health checks and setup (DB path/health validation and DB bootstrap).
- `sce setup` aggregates `ServiceLifecycle::setup` calls, which includes `LocalDbLifecycle::setup()` and `AgentTraceDbLifecycle::setup()` for DB initialization as part of local prerequisite bootstrap.
- `sce doctor` aggregates `ServiceLifecycle::diagnose` and `ServiceLifecycle::fix` calls, which includes both DB lifecycle providers for DB path/health validation and can bootstrap missing canonical parent directories when repair mode is appropriate.

## ServiceLifecycle trait

- `cli/src/services/lifecycle.rs` defines the `ServiceLifecycle` trait with default no-op `diagnose`, `fix`, and `setup` methods that accept `&AppContext`.
- `AppContext` (from `cli-observability-di`) provides shared context including optional `repo_root` for service lifecycle operations.
- Services implementing `ServiceLifecycle`:
  - `ConfigLifecycle` in `cli/src/services/config/lifecycle.rs`
  - `HooksLifecycle` in `cli/src/services/hooks/lifecycle.rs`
  - `LocalDbLifecycle` in `cli/src/services/local_db/lifecycle.rs`
  - `AgentTraceDbLifecycle` in `cli/src/services/agent_trace_db/lifecycle.rs`
- `doctor` command aggregates `diagnose`/`fix` across all registered lifecycle providers.
- `setup` command aggregates `setup` across all registered lifecycle providers in order (config → local_db → agent_trace_db → hooks).

## Parser-focused tests

- `cli/src/app.rs` unit tests cover default-help behavior, auth/config/setup/hooks routing, auth bare/help/nested-help routing, command-local `--help` routing for `doctor`/`hooks`, and failure paths for unknown commands/options and extra arguments.
- `cli/src/app.rs` additionally validates setup contract routing for interactive default, explicit target flags, and mutually-exclusive setup flag failures.
- `cli/src/services/local_db/mod.rs` tests cover in-memory and file-backed local Turso initialization plus execute/query smoke checks.
- `cli/src/services/resilience.rs` tests lock deterministic retry behavior for transient failures, timeout exhaustion, and actionable terminal error messaging.
- `cli/src/services/setup/mod.rs` and `cli/src/services/hooks/mod.rs` include contract-focused tests for setup flag parsing/validation, interactive selection/cancellation dispatch, setup run messaging, and hook runtime argument/IO/finalization behavior.
- `cli/src/services/token_storage.rs` tests cover token save/load round-trips, missing-file handling, token deletion outcomes, invalid JSON corruption handling, and Unix `0600` file-permission enforcement.
- `cli/src/services/auth.rs` tests cover WorkOS device/token payload shape parsing, RFC 8628 device and refresh grant constant wiring, terminal OAuth error mapping with `Try:` guidance, polling decision handling for `authorization_pending`/`slow_down`/terminal outcomes, token-expiry evaluation, and refresh-token re-login guidance for terminal refresh errors.
- `cli/src/services/auth_command/mod.rs` tests cover auth subcommand dispatch, login/logout/status text-or-JSON report shapes (including canonical credentials-file path reporting), `Try:` guidance preservation, and runtime-I/O readiness for the login flow.
- `cli/src/services/setup/mod.rs` tests also verify embedded-manifest completeness against runtime `config/` trees, deterministic sorted path normalization, target-scoped iterator behavior (`OpenCode`, `Claude`, `Both`), and iterator-level omission of `skills/*/tile.json` while keeping `SKILL.md`; sandbox-sensitive filesystem install coverage has been removed from the unit-test slice for later integration-test coverage.
- `cli/src/services/setup/mod.rs` and `cli/src/services/local_db/mod.rs` now share temporary path setup through `crate::test_support::TestTempDir` to keep filesystem test fixtures consistent and cleanup deterministic.
- `cli/src/services/doctor/` unit coverage is intentionally limited to flake-safe output-shape assertions; filesystem, git, and real repair-flow coverage is deferred to future integration tests so `nix flake check` stays sandbox-safe.

## Dependency baseline

- `cli/Cargo.toml` currently declares: `anyhow`, `clap`, `clap_complete`, `comfy-table`, `dirs`, `hmac`, `inquire`, `opentelemetry`, `opentelemetry-otlp`, `opentelemetry_sdk`, `owo-colors`, `reqwest`, `serde`, `serde_json`, `sha2`, `tokio`, `tracing`, `tracing-opentelemetry`, `tracing-subscriber`, and `turso`.
- `tokio` is pinned with `default-features = false` and keeps a constrained runtime footprint for current-thread `Runtime::block_on` usage, plus timer-backed bounded retry/timeout behavior in resilience-wrapped operations.
- `cli/src/services/auth.rs` now includes both the T03 Device Authorization Flow runtime (`start_device_auth_flow`) and T04 token-refresh runtime (`ensure_valid_token`) for WorkOS: it requests device codes, polls `/oauth/device/token` at fixed API interval (adding 5 seconds on `slow_down`), maps RFC 8628 terminal errors to actionable `Try:` guidance, checks token expiry from persisted `stored_at_unix_seconds + expires_in` with a bounded skew guard, refreshes expired access tokens through `/oauth/token` using `grant_type=refresh_token`, retries transient refresh failures via the shared resilience wrapper, and persists rotated tokens via `cli/src/services/token_storage.rs`.

## Scope boundary for this phase

- This slice establishes compile-safe crate/module boundaries with implemented setup orchestration and deterministic messaging.
- Local Turso DB bootstrap and health coverage are implemented through `setup` and `doctor`, while `sce sync` command wiring and broader cloud behavior remain intentionally deferred.
