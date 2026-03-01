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

## Dependency baseline

- `cli/Cargo.toml` declares only: `anyhow`, `lexopt`, `tokio`, and `turso`.
- `cli/src/dependency_contract.rs` keeps compile-time crate references centralized for this placeholder slice.

## Scope boundary for this phase

- This slice establishes compile-safe crate/module boundaries and deterministic placeholder messaging.
- Runtime command implementations and live Turso wiring are intentionally deferred to later plan tasks.
