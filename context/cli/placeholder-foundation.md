# SCE CLI Placeholder Foundation

The repository now includes a placeholder Rust CLI crate at `cli/` for future SCE automation work.

## Current implemented slice

- Binary entrypoint: `cli/src/main.rs`
- Runtime shell: `cli/src/app.rs`
- Command contract catalog: `cli/src/command_surface.rs`
- Future service domains (module map only): `cli/src/services/{setup,mcp,hooks,sync}.rs`

## Command surface contract

`sce --help` lists command names with explicit implementation status:

- `help`: implemented
- `setup`: placeholder
- `mcp`: placeholder
- `hooks`: placeholder
- `sync`: placeholder

Placeholder commands currently acknowledge planned behavior and do not claim production implementation.

## Scope boundary for this phase

- This slice establishes compile-safe crate/module boundaries and deterministic placeholder messaging.
- Dependency contract expansion (`anyhow`, `tokio`, `turso`, `lexopt`) is intentionally deferred to plan task `T02`.
