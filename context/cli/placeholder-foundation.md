# SCE CLI Foundation

The repository now includes a Rust CLI crate at `cli/` for SCE automation work.

`cli/README.md` is the operator onboarding source for running current commands and understanding safety limitations.

## Current implemented slice

- Binary entrypoint: `cli/src/main.rs`
- Runtime shell: `cli/src/app.rs`
- Command contract catalog: `cli/src/command_surface.rs`
- Dependency contract snapshot: `cli/src/dependency_contract.rs`
- Local Turso adapter: `cli/src/services/local_db.rs`
- Service domains: `cli/src/services/{agent_trace,setup,doctor,mcp,hooks,sync}.rs`
- Shared test temp-path helper: `cli/src/test_support.rs` (`TestTempDir`, test-only module)

## Onboarding documentation

- `cli/README.md` includes quick-start commands for `help`, `setup`, `doctor`, `mcp`, `hooks`, and `sync`.
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
- `setup`: implemented
- `doctor`: implemented
- `mcp`: placeholder
- `hooks`: placeholder
- `sync`: placeholder

Placeholder commands currently acknowledge planned behavior and do not claim production implementation.
`mcp`, `hooks`, and `sync` route through explicit service-contract placeholders.
`setup` defaults to an `inquire` interactive target selection (OpenCode, Claude, Both) and accepts mutually-exclusive non-interactive target flags (`--opencode`, `--claude`, `--both`).
`setup` now also exposes compile-time embedded config assets for OpenCode/Claude targets, sourced from `config/.opencode/**` and `config/.claude/**` via `cli/build.rs` with normalized forward-slash relative paths and target-scoped iteration APIs.
`setup` additionally includes a repository-root install engine (`install_embedded_setup_assets`) that stages embedded files and applies backup-and-replace safety for `.opencode/`/`.claude/` with rollback restoration if staged swap fails.
`setup` now executes end-to-end and prints deterministic completion details including selected target(s), per-target install count, and backup actions.
`doctor` now executes end-to-end and reports hook rollout readiness by validating effective hook-path source plus required hook presence/executable permissions.
`sync` includes a local Turso smoke gate backed by a lazily initialized shared tokio current-thread runtime and a placeholder cloud-sync gateway plan.

## Command loop and error model

- Argument parsing is handled by `lexopt` in `cli/src/app.rs`.
- Runtime errors are normalized through `anyhow` and rendered as `Error: ...` with exit code `2`.
- Unknown commands/options and extra positional arguments return deterministic, actionable guidance to run `sce --help`.
- `sce setup --help` returns setup-specific usage output with target-flag contract details.
- Interactive `sce setup` prompt cancellation/interrupt exits cleanly with: `Setup cancelled. No files were changed.`
- Command handlers return deterministic status messaging:
  - `setup`: `Setup completed successfully.` plus selected targets, per-target install destinations/counts, and backup status lines.
  - `doctor`: `SCE doctor: ready|not ready` plus hook-path source, required hook checks, and actionable diagnostics.
  - `TODO: 'mcp' is planned and not implemented yet. MCP file-cache surface defines 2 placeholder tool contract(s) with max 1024 entries.`
  - `TODO: 'hooks' is planned and not implemented yet. Hook event model reserves 3 git hook(s) with generated-region tracking placeholders, staged-only pre-commit checkpoint preview over 1 file(s), and commit-msg canonical trailer preview applied=true.`
  - `TODO: 'sync' cloud workflows are planned and not implemented yet. Local Turso smoke check succeeded (1) row inserted; cloud sync placeholder enumerates 3 phase(s) and plan holds 3 checkpoint(s).`

## Service contracts

- `cli/src/services/setup.rs` defines setup parsing/selection contracts plus runtime install orchestration (`run_setup_for_mode`) over the embedded asset install engine.
- `cli/src/services/doctor.rs` defines hook rollout health validation (`run_doctor`) with path-source detection (default/local/global) and required-hook presence/executable checks.
- `cli/src/services/agent_trace.rs` defines the task-scoped schema adapter contract (`adapt_trace_payload`) from internal attribution input structs to Agent Trace-shaped record structs, including fixed git `vcs` mapping, contributor type mapping, and reserved `dev.crocoder.sce.*` metadata placement.
- `cli/src/services/mcp.rs` defines `McpService`, a `McpCapabilitySnapshot` model (primary + supported transports), and `CachePolicy` defaults for future file-cache workflows (`cache-put`/`cache-get`) with `runnable: false` placeholders.
- `cli/src/services/hooks.rs` defines `HookService` plus hook-event/generated-region event placeholders (`HookEventModel`, `HookEvent`, `GeneratedRegionEvent`) and keeps placeholder recording path compile-safe by consuming hook/lifecycle variants without enabling production hook actions.
- `cli/src/services/sync.rs` defines cloud-sync abstraction points (`CloudSyncGateway`, `CloudSyncRequest`, `CloudSyncPlan`) layered after the local Turso smoke gate.
- `cli/src/app.rs` dispatches `setup`, `doctor`, `mcp`, and `hooks` through service-level modules so runtime messages are sourced from domain modules instead of inline strings.

## Local Turso adapter behavior

- `cli/src/services/local_db.rs` provides `run_smoke_check(...)` with local target options:
  - in-memory (`:memory:`)
  - file-backed path (`Builder::new_local(<path>)`)
- The smoke path creates `sce_smoke`, inserts one row, and runs a query round-trip to confirm readable results.
- `cli/src/services/sync.rs` wraps this in a lazily initialized shared tokio current-thread runtime and returns placeholder-safe messaging when local checks pass.
- The same sync path now derives deferred cloud checkpoint messaging from `PlaceholderCloudSyncGateway`.

## Parser-focused tests

- `cli/src/app.rs` unit tests cover default-help behavior, known command routing, and failure paths for unknown commands/options and extra arguments.
- `cli/src/app.rs` additionally validates setup contract routing for interactive default, explicit target flags, and mutually-exclusive setup flag failures.
- `cli/src/services/local_db.rs` tests cover in-memory and file-backed local Turso initialization plus execute/query smoke checks.
- `cli/src/services/sync.rs` test confirms `sync` runs the local smoke gate and returns deterministic placeholder messaging.
- `cli/src/services/{setup,mcp,hooks,sync}.rs` include contract-focused tests for setup flag parsing/validation, interactive selection/cancellation dispatch, setup run messaging, and non-runnable capability/event plans.
- `cli/src/services/agent_trace.rs` includes adapter mapping tests for required field projection, contributor enum/model_id handling, and extension metadata placement under reserved reverse-domain keys.
- `cli/src/services/setup.rs` tests also verify embedded-manifest completeness against runtime `config/` trees, deterministic sorted path normalization, target-scoped iterator behavior (`OpenCode`, `Claude`, `Both`), install backup creation/replacement, and rollback restoration after injected swap failures.
- `cli/src/services/setup.rs` and `cli/src/services/local_db.rs` now share temporary path setup through `crate::test_support::TestTempDir` to keep filesystem test fixtures consistent and cleanup deterministic.

## Dependency baseline

- `cli/Cargo.toml` declares only: `anyhow`, `inquire`, `lexopt`, `tokio`, and `turso`.
- `tokio` is pinned with `default-features = false` and `features = ["rt"]` to match current runtime usage (current-thread runtime builder and `Runtime::block_on` without broader async feature surface).
- `cli/src/dependency_contract.rs` keeps compile-time crate references centralized for this placeholder slice.

## Scope boundary for this phase

- This slice establishes compile-safe crate/module boundaries with implemented setup orchestration and deterministic messaging.
- Local Turso smoke wiring is implemented for `sync`, while broader runtime command implementations and cloud behavior remain intentionally deferred.
