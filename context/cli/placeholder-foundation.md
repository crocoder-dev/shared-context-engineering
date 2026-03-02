# SCE CLI Placeholder Foundation

The repository now includes a placeholder Rust CLI crate at `cli/` for future SCE automation work.

`cli/README.md` is the operator onboarding source for running the placeholder commands and understanding current safety limitations.

## Current implemented slice

- Binary entrypoint: `cli/src/main.rs`
- Runtime shell: `cli/src/app.rs`
- Command contract catalog: `cli/src/command_surface.rs`
- Dependency contract snapshot: `cli/src/dependency_contract.rs`
- Local Turso adapter: `cli/src/services/local_db.rs`
- Service domains: `cli/src/services/{setup,mcp,hooks,sync}.rs`

## Onboarding documentation

- `cli/README.md` includes quick-start commands for `help`, `setup`, `mcp`, `hooks`, and `sync`.
- The README explicitly distinguishes implemented behavior from placeholders and maps future work to module contracts.
- Verification guidance in the README uses crate-local `cargo check`, `cargo test`, and `cargo build` commands.

## Command surface contract

`sce --help` lists command names with explicit implementation status:

- `help`: implemented
- `setup`: placeholder
- `mcp`: placeholder
- `hooks`: placeholder
- `sync`: placeholder

Placeholder commands currently acknowledge planned behavior and do not claim production implementation.
`setup`, `mcp`, and `hooks` now route through explicit service-contract placeholders.
`setup` defaults to interactive target selection and accepts mutually-exclusive non-interactive target flags (`--opencode`, `--claude`, `--both`).
`sync` includes a local Turso smoke gate and a placeholder cloud-sync gateway plan.

## Command loop and error model

- Argument parsing is handled by `lexopt` in `cli/src/app.rs`.
- Runtime errors are normalized through `anyhow` and rendered as `Error: ...` with exit code `2`.
- Unknown commands/options and extra positional arguments return deterministic, actionable guidance to run `sce --help`.
- `sce setup --help` returns setup-specific usage output with target-flag contract details.
- Placeholder command handlers return explicit TODO messaging:
  - `TODO: 'setup' is planned and not implemented yet. Setup mode '<interactive or --flag>' accepted; setup plan scaffolded with 3 deferred step(s).`
  - `TODO: 'mcp' is planned and not implemented yet. MCP file-cache surface defines 2 placeholder tool contract(s) with max 1024 entries.`
  - `TODO: 'hooks' is planned and not implemented yet. Hook event model reserves 2 git hook(s) with generated-region tracking placeholders.`
  - `TODO: 'sync' cloud workflows are planned and not implemented yet. Local Turso smoke check succeeded (1) row inserted; cloud sync plan holds 3 checkpoint(s).`

## Future feature contracts (T05)

- `cli/src/services/setup.rs` defines `SetupService`, `SetupRequest`, and `SetupPlan` as setup-planning seams with a non-executing placeholder implementation.
- `cli/src/services/mcp.rs` defines `McpService`, a `McpCapabilitySnapshot` model, and `CachePolicy` defaults for future file-cache workflows (`cache-put`/`cache-get`) with `runnable: false` placeholders.
- `cli/src/services/hooks.rs` defines `HookService` plus hook-event/generated-region event placeholders (`HookEventModel`, `HookEvent`, `GeneratedRegionEvent`).
- `cli/src/services/sync.rs` defines cloud-sync abstraction points (`CloudSyncGateway`, `CloudSyncRequest`, `CloudSyncPlan`) layered after the local Turso smoke gate.
- `cli/src/app.rs` dispatches `setup`, `mcp`, and `hooks` through service-level placeholder functions so runtime messages are sourced from domain modules instead of inline strings.

## Local Turso adapter behavior

- `cli/src/services/local_db.rs` provides `run_smoke_check(...)` with local target options:
  - in-memory (`:memory:`)
  - file-backed path (`Builder::new_local(<path>)`)
- The smoke path creates `sce_smoke`, inserts one row, and runs a query round-trip to confirm readable results.
- `cli/src/services/sync.rs` wraps this in a tokio current-thread runtime and returns placeholder-safe messaging when local checks pass.
- The same sync path now derives deferred cloud checkpoint messaging from `PlaceholderCloudSyncGateway`.

## Parser-focused tests

- `cli/src/app.rs` unit tests cover default-help behavior, known command routing, and failure paths for unknown commands/options and extra arguments.
- `cli/src/app.rs` additionally validates setup contract routing for interactive default, explicit target flags, and mutually-exclusive setup flag failures.
- `cli/src/services/local_db.rs` tests cover in-memory and file-backed local Turso initialization plus execute/query smoke checks.
- `cli/src/services/sync.rs` test confirms `sync` runs the local smoke gate and returns deterministic placeholder messaging.
- `cli/src/services/{setup,mcp,hooks,sync}.rs` include contract-focused tests for setup flag parsing/validation, placeholder wiring, and non-runnable capability/event plans.

## Dependency baseline

- `cli/Cargo.toml` declares only: `anyhow`, `lexopt`, `tokio`, and `turso`.
- `cli/src/dependency_contract.rs` keeps compile-time crate references centralized for this placeholder slice.

## Scope boundary for this phase

- This slice establishes compile-safe crate/module boundaries and deterministic placeholder messaging.
- Local Turso smoke wiring is implemented for `sync`, while broader runtime command implementations and cloud behavior remain intentionally deferred.
