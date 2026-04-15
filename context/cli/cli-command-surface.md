# SCE CLI Foundation

The repository now includes a Rust CLI crate at `cli/` for SCE automation work.

Operator onboarding currently comes from `sce --help`, command-local `--help` output, and the focused CLI context files under `context/cli/` and `context/sce/`.

## Current implemented slice

- Binary entrypoint: `cli/src/main.rs`
- Runtime shell: `cli/src/app.rs`
- Command contract catalog: `cli/src/command_surface.rs`
- Local Turso adapter: `cli/src/services/local_db.rs`
- Service domains: `cli/src/services/{agent_trace,auth,auth_command,completion,config,default_paths,local_db,setup,doctor,hooks,resilience,sync,token_storage,version}.rs`
- Shared test temp-path helper: `cli/src/test_support.rs` (`TestTempDir`, test-only module)

## Onboarding documentation

- `sce --help` includes a slim top-level command list and quick-start examples for `setup`, `doctor`, and `version`; `auth`, `hooks`, and `sync` remain implemented in code but are hidden from `sce`, `sce help`, and `sce --help` for this phase.
- Command-local help is available for implemented commands including bare `sce auth`, `sce auth --help`, `sce auth login --help`, `sce setup --help`, `sce doctor --help`, and `sce completion --help`; when stdout color is enabled those help payloads now reuse the shared heading/command/placeholder styling pass while non-TTY and `NO_COLOR` flows stay plain text. Human-readable stderr diagnostics and interactive setup prompt text now follow the same shared styling policy on their respective terminal streams.
- Current verification guidance for the CLI slice uses crate-local `cargo test --manifest-path cli/Cargo.toml`, plus release/install commands for installability (`cargo build --manifest-path cli/Cargo.toml --release`, `cargo install --path cli --locked`).

## Nix release installability surface

- Root `flake.nix` exposes `packages.sce` and `packages.default = packages.sce` for packaged release builds.
- Root `flake.nix` exposes `apps.sce` pointing to `${packages.sce}/bin/sce` for runnable packaged CLI execution.
- Root `flake.nix` is the single repository-level Nix entrypoint for CLI checks and packaging.
- Current installability checks for this surface are:
  - `nix build .#default`
  - `nix run .#sce -- --help`

## Cargo release and future crates.io posture

- `cli/Cargo.toml` includes crates.io-facing package metadata (`description`, `license`, `repository`, `homepage`, `documentation`, `readme`, `keywords`, `categories`) while keeping `publish = false`.
- Current local install contract is `cargo install --path cli --locked`.
- Current release build/installability check command is `cargo build --manifest-path cli/Cargo.toml --release`.
- Future crates.io publication is readiness-only in this phase: before first publish, flip publish posture intentionally and run `cargo publish --manifest-path cli/Cargo.toml --dry-run` as a gate.

## Command surface contract

`sce`, `sce help`, and `sce --help` now render a top-level help surface that starts with an ASCII art "SCE" banner followed by a slim command list:

- the banner uses a per-column right-to-left color gradient (cyan on the right, magenta on the left) when stdout color is enabled, and renders as plain ASCII when color is disabled (non-TTY or `NO_COLOR`)
- the banner is rendered by `command_surface::help_text()` calling `style::banner_with_gradient(SCE_BANNER_LINES)` before the heading
- the visible command list is `help`, `config`, `setup`, `doctor`, `version`, and `completion`
- top-level help omits implemented/placeholder labels
- top-level examples cover setup plus doctor/version machine-readable or repair-intent flows (`doctor --format json`, `doctor --fix`, `version --format json`) and use the shared example-command styling when stdout color is enabled
- `auth`, `hooks`, and `sync` stay parser-valid and directly invocable, but are hidden from those top-level help surfaces

Placeholder commands currently acknowledge planned behavior and do not claim production implementation.
`sync` routes through an explicit service-contract placeholder.
`hooks` routes through implemented subcommand parsing/dispatch for `pre-commit`, `commit-msg`, `post-commit`, and `post-rewrite`; current behavior is attribution-only and disabled by default.
`config` exposes deterministic inspect/validate entrypoints (`sce config show`, `sce config validate`) with explicit precedence (`flags > env > config file > defaults`), a shared auth-runtime resolver for supported keys that declare env/config/optional baked-default inputs starting with `workos_client_id`, first-class `policies.bash` reporting for preset/custom blocked-command rules, and deterministic text/JSON output modes where `show` reports resolved values with provenance while `validate` reports pass/fail plus validation issues and warnings only.
`version` exposes deterministic runtime identification output in text mode by default and JSON mode via `--format json`.
`completion` exposes deterministic shell completion generation via `sce completion --shell <bash|zsh|fish>`.
`setup` defaults to an `inquire` interactive target selection (OpenCode, Claude, Both) and accepts mutually-exclusive non-interactive target flags (`--opencode`, `--claude`, `--both`); the interactive prompt title and target labels now reuse shared prompt styling helpers when stdout color is enabled.
`auth` now emits auth-local guidance for bare `sce auth` and `sce auth --help`, listing `login`, `logout`, and `status` plus copy-ready next steps.
`setup`, `doctor`, `hooks`, `sync`, `version`, and `completion` all support command-local `--help`/`-h` usage output via top-level parser routing in `cli/src/app.rs`.
`setup` now also exposes compile-time embedded config assets for OpenCode/Claude targets, sourced from the generated `config/.opencode/**` and `config/.claude/**` trees via `cli/build.rs` with normalized forward-slash relative paths and target-scoped iteration APIs; the embedded asset set includes the OpenCode bash-policy plugin/runtime files generated from the canonical preset catalog (Claude bash-policy enforcement has been removed from generated outputs).
`setup` additionally includes a repository-root install engine (`install_embedded_setup_assets`) that stages embedded files, intentionally leaves generated `skills/*/tile.json` manifests in `config/` only, skips those tile files during repo-root installs, and uses a unified remove-and-replace policy for `.opencode/`/`.claude/` (removing existing targets before swapping staged content, with deterministic recovery guidance on swap failure) while treating bash-policy enforcement files as first-class SCE-managed assets.
`setup` now executes end-to-end and prints deterministic completion details including selected target(s) and per-target install count.
`doctor` now executes end-to-end with explicit diagnosis and repair-intent surfaces: `sce doctor` stays read-only, `sce doctor --fix` selects repair-intent mode, and text/JSON output expose stable mode/problem/fix-result/database-record scaffolding. The current runtime now covers state-root resolution, global and repo-local `sce/config.json` readability/schema validation, local DB path/health, DB-parent readiness barriers, an intentionally empty repo-scoped SCE database section for the active repository, the repo hook rollout slice when a repository target is detected, and repo-root installed OpenCode integration presence for `plugins`, `agents`, `commands`, and `skills`; those integration checks are presence-only and fail a group when any required installed file is missing. Fix mode now reuses the canonical setup hook install flow to repair missing/stale/non-executable required hooks and missing hooks directories, and it can bootstrap the missing canonical local DB parent directory when the resolved path matches the canonical owned location.
`sync` includes a local Turso smoke gate backed by a lazily initialized shared tokio current-thread runtime, bounded retry/timeout/backoff policy for the smoke operation, and a placeholder cloud-sync gateway plan; it now supports deterministic `text` output (default) and `--format json` output with stable placeholder fields.

## Command loop and error model

- Argument parsing is handled by `clap` derive macros in `cli/src/cli_schema.rs` and dispatched from `cli/src/app.rs`.
- Top-level failures are classified into stable exit-code classes owned by `cli/src/app.rs`: `2` parse, `3` validation, `4` runtime, and `5` dependency.
- User-facing diagnostics are rendered on `stderr` as `Error [SCE-ERR-<CLASS>]: ...` with class-default `Try:` remediation appended only when missing; when stderr color is enabled the heading, error code, and diagnostic body all render through shared stderr styling helpers.
- Unknown commands/options and extra positional arguments return deterministic, actionable guidance to run `sce --help`.
- `sce setup --help` returns setup-specific usage output with target-flag contract details and deterministic examples, including one-run non-interactive setup+hooks and composable follow-up validation/repair-intent flows (`sce doctor --format json`, `sce doctor --fix`).
- `sce auth` and `sce auth --help` return auth-specific usage output with available subcommands and deterministic examples, while `sce auth <login|renew|logout|status> --help` stays scoped to the selected auth subcommand.
- `sce doctor --help`, `sce hooks --help`, and `sce sync --help` return command-local usage output and deterministic copy-ready examples.
- Interactive `sce setup` prompt cancellation/interrupt exits cleanly with: `Setup cancelled. No files were changed.`
- Command handlers return deterministic status messaging:
- `setup`: `Setup completed successfully.` plus selected targets and per-target install destinations/counts.
- `doctor`: current runtime emits `SCE doctor diagnose` / `SCE doctor fix` human text headers plus ordered `Environment`, `Configuration`, `Repository`, `Git Hooks`, and `Integrations` sections with bracketed `[PASS]`/`[FAIL]`/`[MISS]` row tokens, shared-style green pass plus red fail/miss colorization when enabled, simplified `label (path)` rows, top-level-only hook rows, and a deterministic summary footer; JSON output carries stable problem/fixability records plus deterministic fix-result records in fix mode and reports the neutral DB record under `local_db`. The runtime validates global and repo-local `sce/config.json` inputs plus local DB health, keeps the repo-scoped database section empty unless a future repo-owned SCE database family is introduced, diagnoses repo hook rollout integrity plus repo-root installed OpenCode integration presence for `OpenCode plugins`, `OpenCode agents`, `OpenCode commands`, and `OpenCode skills`, and in fix mode reuses canonical setup hook installation for supported hook repairs plus bounded bootstrap of the canonical missing SCE-owned local DB parent directory while preserving manual-only reporting for unsupported issues.
  - `hooks`: deterministic hook subcommand status messaging for runtime entrypoint invocation and argument/STDIN contract validation.
  - `TODO: 'sync' cloud workflows are planned and not implemented yet. Local Turso smoke check succeeded (1) row inserted; cloud sync placeholder enumerates 3 phase(s) and plan holds 3 checkpoint(s). Next step: rerun with '--format json' for machine-readable placeholder checkpoints.`

## Service contracts

- `cli/src/services/setup.rs` defines setup parsing/selection contracts plus runtime install orchestration (`run_setup_for_mode`) over the embedded asset install engine.
- `cli/src/services/config.rs` defines config parser/runtime contracts (`show`, `validate`, `--help`), strict config-file key/type validation, deterministic text/JSON rendering, repo-configured bash-policy preset/custom validation and reporting under `policies.bash`, and shared auth-key metadata that declares env key, config-file key, and optional baked-default eligibility for supported auth runtime values starting with `workos_client_id` (`WORKOS_CLIENT_ID` vs `workos_client_id`); auth-key provenance/preference metadata stays on `show`, while `validate` stays trimmed to validation status plus issues/warnings.
- `cli/src/services/doctor.rs` now defines the implemented doctor request/report contract (`DoctorRequest`, `DoctorMode`, `run_doctor`) with explicit fix-mode parsing, stable text/JSON problem and database-record rendering, deterministic fix-result reporting, state-root/config/local-DB reporting and validation, an empty default repo-scoped database inventory, path-source detection plus required-hook presence/executable/content checks when a repository target is detected, repo-root installed OpenCode integration presence inventory for `plugins`, `agents`, `commands`, and `skills` derived from the embedded OpenCode setup asset catalog, shared-style bracketed human status token rendering (`[PASS]`, `[FAIL]`, `[MISS]`) with simplified `label (path)` text rows, repair-mode reuse of canonical setup hook installation for supported hook repairs, and a bounded doctor-owned local-DB directory bootstrap routine for the canonical missing DB parent path.
- `cli/src/services/version.rs` defines the version parser/output contract (`parse_version_request`, `render_version`) with deterministic text/JSON output modes.
- `cli/src/services/completion.rs` defines the completion output contract (`render_completion`) using clap_complete to generate deterministic shell scripts for Bash, Zsh, and Fish.
- `cli/src/services/hooks.rs` defines production local hook runtime parsing/dispatch (`HookSubcommand`, `parse_hooks_subcommand`, `run_hooks_subcommand`) for `pre-commit`, `commit-msg`, `post-commit`, and `post-rewrite`; current runtime behavior is commit-msg-only attribution behind the disabled-default attribution gate, while the other entrypoints are deterministic no-ops.
- `cli/src/services/resilience.rs` defines shared bounded retry/timeout/backoff execution policy (`RetryPolicy`, `run_with_retry`) with deterministic failure messaging and retry observability hooks.
- `cli/src/services/sync.rs` defines cloud-sync abstraction points (`CloudSyncGateway`, `CloudSyncRequest`, `CloudSyncPlan`) layered after the local Turso smoke gate, plus `SyncRequest` parsing/rendering for deterministic text or `--format json` placeholder output and command-local usage text (`sync_usage_text`).
- `cli/src/services/default_paths.rs` defines the canonical per-user persisted-location seam for config/state/cache roots plus named default file paths and an explicit inventory of current default persisted artifacts (`global config`, `auth tokens`, `local DB`) used by config discovery, token storage, local DB bootstrap, and doctor diagnostics; no default cache-backed persisted artifact exists yet.
- `cli/src/services/token_storage.rs` defines WorkOS token persistence (`save_tokens`, `load_tokens`, `delete_tokens`) with shared default-path-seam resolution for the default token file, JSON payload storage including `stored_at_unix_seconds`, graceful missing-file deletion behavior, missing/corrupted-file handling, and restrictive on-disk permissions (`0600` on Unix; Windows best-effort ACL hardening via `icacls`).
- `cli/src/services/auth_command.rs` defines the auth command orchestration surface (`AuthRequest`, `AuthSubcommand`, `run_auth_subcommand`) for `login`, `renew`, `logout`, and `status`, including shared text/JSON rendering, token refresh/forced renewal handling for `sce auth renew`, token-storage-backed logout deletion with path-aware remediation guidance, expiry-aware status reporting, canonical credentials-file path reporting sourced from the shared default-path seam, precedence-aware client-ID guidance sourced from the shared auth-runtime resolver instead of env-only assumptions, and a lazily initialized current-thread Tokio runtime with both I/O and time enabled so the auth flows can drive the WorkOS device/refresh paths without the prior I/O-disabled panic.
- `cli/src/app.rs` dispatches `auth`, `config`, `setup`, `doctor`, `hooks`, `sync`, `version`, and `completion` through service-level modules so runtime messages are sourced from domain modules instead of inline strings.

## Local Turso adapter behavior

- `cli/src/services/local_db.rs` provides `run_smoke_check(...)` with local target options:
  - in-memory (`:memory:`)
  - file-backed path (`Builder::new_local(<path>)`)
- The smoke path creates `sce_smoke`, inserts one row, and runs a query round-trip to confirm readable results.
- `cli/src/services/sync.rs` wraps this in a lazily initialized shared tokio current-thread runtime and applies bounded retries (3 attempts), operation timeout (2000ms), and capped backoff (100-400ms) before returning placeholder-safe messaging.
- The same sync path now derives deferred cloud checkpoint messaging from `PlaceholderCloudSyncGateway`.
- `cli/src/services/local_db.rs` retains a neutral empty-file local DB bootstrap seam for future runtime use without schema migrations or trace-table installation.

## Parser-focused tests

- `cli/src/app.rs` unit tests cover default-help behavior, auth/config/setup/hooks routing, auth bare/help/nested-help routing, command-local `--help` routing for `doctor`/`hooks`/`sync`, and failure paths for unknown commands/options and extra arguments.
- `cli/src/app.rs` additionally validates setup contract routing for interactive default, explicit target flags, and mutually-exclusive setup flag failures.
- `cli/src/services/local_db.rs` tests cover in-memory and file-backed local Turso initialization plus execute/query smoke checks.
- `cli/src/services/resilience.rs` tests lock deterministic retry behavior for transient failures, timeout exhaustion, and actionable terminal error messaging.
- `cli/src/services/sync.rs` tests confirm `sync` runs the local smoke gate, preserves deterministic text placeholder messaging, and emits stable JSON placeholder fields.
- `cli/src/services/{setup,hooks,sync}.rs` include contract-focused tests for setup flag parsing/validation, interactive selection/cancellation dispatch, setup run messaging, and hook runtime argument/IO/finalization behavior.
- `cli/src/services/token_storage.rs` tests cover token save/load round-trips, missing-file handling, token deletion outcomes, invalid JSON corruption handling, and Unix `0600` file-permission enforcement.
- `cli/src/services/auth.rs` tests cover WorkOS device/token payload shape parsing, RFC 8628 device and refresh grant constant wiring, terminal OAuth error mapping with `Try:` guidance, polling decision handling for `authorization_pending`/`slow_down`/terminal outcomes, token-expiry evaluation, and refresh-token re-login guidance for terminal refresh errors.
- `cli/src/services/auth_command.rs` tests cover auth subcommand dispatch, login/logout/status text-or-JSON report shapes (including canonical credentials-file path reporting), `Try:` guidance preservation, and runtime-I/O readiness for the login flow.
- `cli/src/services/setup.rs` tests also verify embedded-manifest completeness against runtime `config/` trees, deterministic sorted path normalization, target-scoped iterator behavior (`OpenCode`, `Claude`, `Both`), and iterator-level omission of `skills/*/tile.json` while keeping `SKILL.md`; sandbox-sensitive filesystem install coverage has been removed from the unit-test slice for later integration-test coverage.
- `cli/src/services/setup.rs` and `cli/src/services/local_db.rs` now share temporary path setup through `crate::test_support::TestTempDir` to keep filesystem test fixtures consistent and cleanup deterministic.
- `cli/src/services/doctor.rs` unit coverage is intentionally limited to flake-safe output-shape assertions; filesystem, git, and real repair-flow coverage is deferred to future integration tests so `nix flake check` stays sandbox-safe.

## Dependency baseline

- `cli/Cargo.toml` currently declares: `anyhow`, `clap`, `clap_complete`, `comfy-table`, `dirs`, `hmac`, `inquire`, `opentelemetry`, `opentelemetry-otlp`, `opentelemetry_sdk`, `owo-colors`, `reqwest`, `serde`, `serde_json`, `sha2`, `tokio`, `tracing`, `tracing-opentelemetry`, `tracing-subscriber`, and `turso`.
- `tokio` is pinned with `default-features = false` and keeps a constrained runtime footprint for current-thread `Runtime::block_on` usage, plus timer-backed bounded retry/timeout behavior in resilience-wrapped operations.
- `cli/src/services/auth.rs` now includes both the T03 Device Authorization Flow runtime (`start_device_auth_flow`) and T04 token-refresh runtime (`ensure_valid_token`) for WorkOS: it requests device codes, polls `/oauth/device/token` at fixed API interval (adding 5 seconds on `slow_down`), maps RFC 8628 terminal errors to actionable `Try:` guidance, checks token expiry from persisted `stored_at_unix_seconds + expires_in` with a bounded skew guard, refreshes expired access tokens through `/oauth/token` using `grant_type=refresh_token`, retries transient refresh failures via the shared resilience wrapper, and persists rotated tokens via `cli/src/services/token_storage.rs`.

## Scope boundary for this phase

- This slice establishes compile-safe crate/module boundaries with implemented setup orchestration and deterministic messaging.
- Local Turso smoke wiring is implemented for `sync`, while broader runtime command implementations and cloud behavior remain intentionally deferred.
