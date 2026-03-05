# SCE CLI Foundation

The repository now includes a Rust CLI crate at `cli/` for SCE automation work.

`cli/README.md` is the operator onboarding source for running current commands and understanding safety limitations.

## Current implemented slice

- Binary entrypoint: `cli/src/main.rs`
- Runtime shell: `cli/src/app.rs`
- Command contract catalog: `cli/src/command_surface.rs`
- Dependency contract snapshot: `cli/src/dependency_contract.rs`
- Local Turso adapter: `cli/src/services/local_db.rs`
- Service domains: `cli/src/services/{agent_trace,completion,config,setup,doctor,mcp,hooks,resilience,sync,version}.rs`
- Shared test temp-path helper: `cli/src/test_support.rs` (`TestTempDir`, test-only module)

## Onboarding documentation

- `cli/README.md` includes quick-start commands for `help`, `config`, `setup`, `doctor`, `mcp`, `hooks`, `sync`, and `completion`.
- The README explicitly distinguishes implemented behavior from placeholders and maps future work to module contracts.
- Verification guidance in the README uses crate-local `cargo check`, `cargo test`, and `cargo build` commands, plus release/install commands for current installability (`cargo build --manifest-path cli/Cargo.toml --release`, `cargo install --path cli --locked`).

## Nix release installability surface

- `cli/flake.nix` exposes `packages.sce` and `packages.default = packages.sce` for packaged release builds.
- `cli/flake.nix` exposes `apps.sce` pointing to `${packages.sce}/bin/sce` for runnable packaged CLI execution.
- Root `flake.nix` forwards nested CLI flake inputs (`nixpkgs`, `flake-utils`, `rust-overlay`) so repository-level `nix flake check` can evaluate CLI checks without nested-input resolution failures.
- Current verification commands for this surface are:
  - `nix build ./cli#default`
  - `nix run ./cli#sce -- --help`

## Cargo release and future crates.io posture

- `cli/Cargo.toml` includes crates.io-facing package metadata (`description`, `license`, `repository`, `homepage`, `documentation`, `readme`, `keywords`, `categories`) while keeping `publish = false`.
- Current local install contract is `cargo install --path cli --locked`.
- Current release build verification command is `cargo build --manifest-path cli/Cargo.toml --release`.
- Future crates.io publication is readiness-only in this phase: before first publish, flip publish posture intentionally and run `cargo publish --manifest-path cli/Cargo.toml --dry-run` as a gate.

## Command surface contract

`sce --help` lists command names with explicit implementation status:

- `help`: implemented
- `config`: implemented
- `setup`: implemented
- `doctor`: implemented
- `mcp`: placeholder
- `hooks`: implemented
- `sync`: placeholder
- `version`: implemented
- `completion`: implemented

Placeholder commands currently acknowledge planned behavior and do not claim production implementation.
`mcp` and `sync` route through explicit service-contract placeholders.
`hooks` routes through implemented subcommand parsing/dispatch for `pre-commit`, `commit-msg`, `post-commit`, and `post-rewrite`.
`config` exposes deterministic inspect/validate entrypoints (`sce config show`, `sce config validate`) with explicit precedence (`flags > env > config file > defaults`) and deterministic text/JSON output modes.
`version` exposes deterministic runtime identification output in text mode by default and JSON mode via `--format json`.
`completion` exposes deterministic shell completion generation via `sce completion --shell <bash|zsh|fish>`.
`setup` defaults to an `inquire` interactive target selection (OpenCode, Claude, Both) and accepts mutually-exclusive non-interactive target flags (`--opencode`, `--claude`, `--both`).
`setup`, `doctor`, `mcp`, `hooks`, `sync`, `version`, and `completion` all support command-local `--help`/`-h` usage output via top-level parser routing in `cli/src/app.rs`.
`setup` now also exposes compile-time embedded config assets for OpenCode/Claude targets, sourced from `config/.opencode/**` and `config/.claude/**` via `cli/build.rs` with normalized forward-slash relative paths and target-scoped iteration APIs.
`setup` additionally includes a repository-root install engine (`install_embedded_setup_assets`) that stages embedded files and applies backup-and-replace safety for `.opencode/`/`.claude/` with rollback restoration if staged swap fails.
`setup` now executes end-to-end and prints deterministic completion details including selected target(s), per-target install count, and backup actions.
`doctor` now executes end-to-end and reports hook rollout readiness by validating effective hook-path source plus required hook presence/executable permissions.
`sync` includes a local Turso smoke gate backed by a lazily initialized shared tokio current-thread runtime, bounded retry/timeout/backoff policy for the smoke operation, and a placeholder cloud-sync gateway plan.

## Command loop and error model

- Argument parsing is handled by `lexopt` in `cli/src/app.rs`.
- Runtime errors are normalized through `anyhow` and rendered as `Error: ...` with exit code `2`.
- Unknown commands/options and extra positional arguments return deterministic, actionable guidance to run `sce --help`.
- `sce setup --help` returns setup-specific usage output with target-flag contract details and deterministic examples.
- `sce doctor --help`, `sce mcp --help`, `sce hooks --help`, and `sce sync --help` return command-local usage output and deterministic copy-ready examples.
- Interactive `sce setup` prompt cancellation/interrupt exits cleanly with: `Setup cancelled. No files were changed.`
- Command handlers return deterministic status messaging:
  - `setup`: `Setup completed successfully.` plus selected targets, per-target install destinations/counts, and backup status lines.
  - `doctor`: `SCE doctor: ready|not ready` plus hook-path source, required hook checks, and actionable diagnostics.
  - `TODO: 'mcp' is planned and not implemented yet. MCP file-cache surface defines 2 placeholder tool contract(s) with max 1024 entries.`
  - `hooks`: deterministic hook subcommand status messaging for runtime entrypoint invocation and argument/STDIN contract validation.
  - `TODO: 'sync' cloud workflows are planned and not implemented yet. Local Turso smoke check succeeded (1) row inserted; cloud sync placeholder enumerates 3 phase(s) and plan holds 3 checkpoint(s).`

## Service contracts

- `cli/src/services/setup.rs` defines setup parsing/selection contracts plus runtime install orchestration (`run_setup_for_mode`) over the embedded asset install engine.
- `cli/src/services/config.rs` defines config parser/runtime contracts (`show`, `validate`, `--help`), strict config-file key/type validation, and deterministic text/JSON rendering.
- `cli/src/services/doctor.rs` defines hook rollout health validation (`run_doctor`) with path-source detection (default/local/global), required-hook presence/executable checks, and command-local usage text (`doctor_usage_text`).
- `cli/src/services/agent_trace.rs` defines the task-scoped schema adapter contract (`adapt_trace_payload`) from internal attribution input structs to Agent Trace-shaped record structs, including fixed git `vcs` mapping, contributor type mapping, and reserved `dev.crocoder.sce.*` metadata placement.
- `cli/src/services/mcp.rs` defines `McpService`, a `McpCapabilitySnapshot` model (primary + supported transports), `CachePolicy` defaults for future file-cache workflows (`cache-put`/`cache-get`) with `runnable: false` placeholders, and command-local usage text (`mcp_usage_text`).
- `cli/src/services/version.rs` defines the version parser/output contract (`parse_version_request`, `render_version`) with deterministic text/JSON output modes.
- `cli/src/services/completion.rs` defines the completion parser/output contract (`parse_completion_request`, `render_completion`) with deterministic shell scripts for Bash, Zsh, and Fish.
- `cli/src/services/hooks.rs` defines production local hook runtime parsing/dispatch (`HookSubcommand`, `parse_hooks_subcommand`, `run_hooks_subcommand`) for `pre-commit`, `commit-msg`, `post-commit`, and `post-rewrite`, plus checkpoint/persistence/retry finalization seams used by hook entrypoints.
- `cli/src/services/resilience.rs` defines shared bounded retry/timeout/backoff execution policy (`RetryPolicy`, `run_with_retry`) with deterministic failure messaging and retry observability hooks.
- `cli/src/services/sync.rs` defines cloud-sync abstraction points (`CloudSyncGateway`, `CloudSyncRequest`, `CloudSyncPlan`) layered after the local Turso smoke gate, plus command-local usage text (`sync_usage_text`).
- `cli/src/app.rs` dispatches `config`, `setup`, `doctor`, `mcp`, `hooks`, `sync`, `version`, and `completion` through service-level modules so runtime messages are sourced from domain modules instead of inline strings.

## Local Turso adapter behavior

- `cli/src/services/local_db.rs` provides `run_smoke_check(...)` with local target options:
  - in-memory (`:memory:`)
  - file-backed path (`Builder::new_local(<path>)`)
- The smoke path creates `sce_smoke`, inserts one row, and runs a query round-trip to confirm readable results.
- `cli/src/services/sync.rs` wraps this in a lazily initialized shared tokio current-thread runtime and applies bounded retries (3 attempts), operation timeout (2000ms), and capped backoff (100-400ms) before returning placeholder-safe messaging.
- The same sync path now derives deferred cloud checkpoint messaging from `PlaceholderCloudSyncGateway`.
- `cli/src/services/local_db.rs` applies the same resilience wrapper when bootstrapping persistent Agent Trace schema migrations (`ensure_agent_trace_local_db_ready_blocking`) with deterministic retries/timeouts/backoff and actionable terminal failure hints.

## Parser-focused tests

- `cli/src/app.rs` unit tests cover default-help behavior, known command routing, command-local `--help` routing for `doctor`/`mcp`/`hooks`/`sync`, and failure paths for unknown commands/options and extra arguments.
- `cli/src/app.rs` additionally validates setup contract routing for interactive default, explicit target flags, and mutually-exclusive setup flag failures.
- `cli/src/services/local_db.rs` tests cover in-memory and file-backed local Turso initialization plus execute/query smoke checks.
- `cli/src/services/resilience.rs` tests lock deterministic retry behavior for transient failures, timeout exhaustion, and actionable terminal error messaging.
- `cli/src/services/sync.rs` test confirms `sync` runs the local smoke gate and returns deterministic placeholder messaging.
- `cli/src/services/{setup,mcp,hooks,sync}.rs` include contract-focused tests for setup flag parsing/validation, interactive selection/cancellation dispatch, setup run messaging, and hook runtime argument/IO/finalization behavior.
- `cli/src/services/agent_trace.rs` includes adapter mapping tests for required field projection, contributor enum/model_id handling, and extension metadata placement under reserved reverse-domain keys.
- `cli/src/services/setup.rs` tests also verify embedded-manifest completeness against runtime `config/` trees, deterministic sorted path normalization, target-scoped iterator behavior (`OpenCode`, `Claude`, `Both`), install backup creation/replacement, and rollback restoration after injected swap failures.
- `cli/src/services/setup.rs` and `cli/src/services/local_db.rs` now share temporary path setup through `crate::test_support::TestTempDir` to keep filesystem test fixtures consistent and cleanup deterministic.

## Dependency baseline

- `cli/Cargo.toml` declares only: `anyhow`, `hmac`, `inquire`, `lexopt`, `serde_json`, `sha2`, `tokio`, and `turso`.
- `tokio` is pinned with `default-features = false` and keeps a constrained runtime footprint for current-thread `Runtime::block_on` usage, plus timer-backed bounded retry/timeout behavior in resilience-wrapped operations.
- `cli/src/dependency_contract.rs` keeps compile-time crate references centralized for this placeholder slice.

## Scope boundary for this phase

- This slice establishes compile-safe crate/module boundaries with implemented setup orchestration and deterministic messaging.
- Local Turso smoke wiring is implemented for `sync`, while broader runtime command implementations and cloud behavior remain intentionally deferred.
