# sce CLI (placeholder foundation)

This crate provides the early command-surface scaffold for the Shared Context
Engineering CLI (`sce`).

Current scope is intentionally narrow: deterministic command dispatch, explicit
placeholder messaging, and service contracts that reserve future implementation
seams.

## Quick start

```bash
cargo run --manifest-path cli/Cargo.toml -- --help
cargo run --manifest-path cli/Cargo.toml -- setup
cargo run --manifest-path cli/Cargo.toml -- mcp
cargo run --manifest-path cli/Cargo.toml -- hooks
cargo run --manifest-path cli/Cargo.toml -- sync
```

## Current behavior

- `help` is implemented and prints the current command surface.
- `setup` is a placeholder that returns a deferred setup-plan message.
- `mcp` is a placeholder for future file-cache tooling contracts
  (`cache-put`/`cache-get`).
- `hooks` is a placeholder for future git hook event and generated-region
  tracking integration.
- `sync` is a placeholder that runs a local Turso smoke check, then reports a
  deferred cloud-sync plan.

## Safety and limitations

- Placeholder commands do not perform repository setup, MCP transport, hook
  installation, or cloud sync.
- `sync` only validates local adapter wiring and does not require remote auth.
- This crate is scaffolding for incremental delivery and should not be treated
  as production-ready workflow automation.

## Near-term roadmap mapping

- Repository setup automation seam: `cli/src/services/setup.rs`
- MCP file-cache seam: `cli/src/services/mcp.rs`
- Hook event and generated-region seam: `cli/src/services/hooks.rs`
- Cloud sync seam + local Turso gate: `cli/src/services/sync.rs`
- Command catalog and placeholder status: `cli/src/command_surface.rs`

## Verification commands

Run crate-local checks:

```bash
cargo check --manifest-path cli/Cargo.toml
cargo test --manifest-path cli/Cargo.toml
cargo build --manifest-path cli/Cargo.toml
```
