# SCE CLI Placeholder Foundation

The repository now includes a placeholder Rust CLI crate at `cli/` for future SCE automation work.

## Current implemented slice

- Binary entrypoint: `cli/src/main.rs`
- Runtime shell: `cli/src/app.rs`
- Command contract catalog: `cli/src/command_surface.rs`
- Dependency contract snapshot: `cli/src/dependency_contract.rs`
- Local Turso adapter: `cli/src/services/local_db.rs`
- Service domains: `cli/src/services/{setup,mcp,hooks,sync}.rs`

## Command surface contract

`sce --help` lists command names with explicit implementation status:

- `help`: implemented
- `setup`: placeholder
- `mcp`: placeholder
- `hooks`: placeholder
- `sync`: placeholder

Placeholder commands currently acknowledge planned behavior and do not claim production implementation.
`sync` includes a local Turso smoke gate but remains placeholder for cloud workflows.

## Command loop and error model

- Argument parsing is handled by `lexopt` in `cli/src/app.rs`.
- Runtime errors are normalized through `anyhow` and rendered as `Error: ...` with exit code `2`.
- Unknown commands/options and extra positional arguments return deterministic, actionable guidance to run `sce --help`.
- Placeholder command handlers return explicit TODO messaging:
  - `TODO: 'setup' is planned and not implemented yet.`
  - `TODO: 'mcp' is planned and not implemented yet.`
  - `TODO: 'hooks' is planned and not implemented yet.`
  - `TODO: 'sync' cloud workflows are planned and not implemented yet. Local Turso smoke check succeeded (1) row inserted.`

## Local Turso adapter behavior

- `cli/src/services/local_db.rs` provides `run_smoke_check(...)` with local target options:
  - in-memory (`:memory:`)
  - file-backed path (`Builder::new_local(<path>)`)
- The smoke path creates `sce_smoke`, inserts one row, and runs a query round-trip to confirm readable results.
- `cli/src/services/sync.rs` wraps this in a tokio current-thread runtime and returns placeholder-safe messaging when local checks pass.

## Parser-focused tests

- `cli/src/app.rs` unit tests cover default-help behavior, known command routing, and failure paths for unknown commands/options and extra arguments.
- `cli/src/services/local_db.rs` tests cover in-memory and file-backed local Turso initialization plus execute/query smoke checks.
- `cli/src/services/sync.rs` test confirms `sync` runs the local smoke gate and returns deterministic placeholder messaging.

## Dependency baseline

- `cli/Cargo.toml` declares only: `anyhow`, `lexopt`, `tokio`, and `turso`.
- `cli/src/dependency_contract.rs` keeps compile-time crate references centralized for this placeholder slice.

## Scope boundary for this phase

- This slice establishes compile-safe crate/module boundaries and deterministic placeholder messaging.
- Local Turso smoke wiring is implemented for `sync`, while broader runtime command implementations and cloud behavior remain intentionally deferred.
