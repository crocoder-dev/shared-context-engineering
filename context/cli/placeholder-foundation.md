# SCE CLI Placeholder Foundation

The repository now includes a placeholder Rust CLI crate at `cli/` for future SCE automation work.

## Current implemented slice

- Binary entrypoint: `cli/src/main.rs`
- Runtime shell: `cli/src/app.rs`
- Command contract catalog: `cli/src/command_surface.rs`
- Dependency contract snapshot: `cli/src/dependency_contract.rs`
- Future service domains (module map only): `cli/src/services/{setup,mcp,hooks,sync}.rs`

## Command surface contract

`sce --help` lists command names with explicit implementation status:

- `help`: implemented
- `setup`: placeholder
- `mcp`: placeholder
- `hooks`: placeholder
- `sync`: placeholder

Placeholder commands currently acknowledge planned behavior and do not claim production implementation.

## Command loop and error model

- Argument parsing is handled by `lexopt` in `cli/src/app.rs`.
- Runtime errors are normalized through `anyhow` and rendered as `Error: ...` with exit code `2`.
- Unknown commands/options and extra positional arguments return deterministic, actionable guidance to run `sce --help`.
- Placeholder command handlers return explicit TODO messaging:
  - `TODO: 'setup' is planned and not implemented yet.`
  - `TODO: 'mcp' is planned and not implemented yet.`
  - `TODO: 'hooks' is planned and not implemented yet.`
  - `TODO: 'sync' is planned and not implemented yet.`

## Parser-focused tests

- `cli/src/app.rs` unit tests cover default-help behavior, known command routing, and failure paths for unknown commands/options and extra arguments.

## Dependency baseline

- `cli/Cargo.toml` declares only: `anyhow`, `lexopt`, `tokio`, and `turso`.
- `cli/src/dependency_contract.rs` keeps compile-time crate references centralized for this placeholder slice.

## Scope boundary for this phase

- This slice establishes compile-safe crate/module boundaries and deterministic placeholder messaging.
- Runtime command implementations and live Turso wiring are intentionally deferred to later plan tasks.
