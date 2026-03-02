# sce CLI (placeholder foundation)

This crate provides the early command-surface scaffold for the Shared Context
Engineering CLI (`sce`).

Current scope is intentionally narrow: deterministic command dispatch, an
implemented repository `setup` flow, and explicit placeholders for commands
that are still deferred.

## Quick start

```bash
cargo run --manifest-path cli/Cargo.toml -- --help
cargo run --manifest-path cli/Cargo.toml -- setup
cargo run --manifest-path cli/Cargo.toml -- mcp
cargo run --manifest-path cli/Cargo.toml -- hooks
cargo run --manifest-path cli/Cargo.toml -- sync
```

## Install and release paths

Local install from this repository:

```bash
cargo install --path cli --locked
```

Release build from Cargo:

```bash
cargo build --manifest-path cli/Cargo.toml --release
```

Release build and run from the nested CLI flake:

```bash
nix build ./cli#default
nix run ./cli#sce -- --help
```

Crates.io is prepared but intentionally disabled in this phase.

- `cli/Cargo.toml` keeps `publish = false` until first-publish prerequisites are complete.
- Before the first publish, flip `publish` to allowed, confirm package metadata is still accurate, and run a publish dry run (`cargo publish --manifest-path cli/Cargo.toml --dry-run`).
- Publishing itself (`cargo publish`) is intentionally out of scope for the current plan task.

## Current behavior

- `help` is implemented and prints the current command surface.
- `setup` is implemented:
  - default mode is interactive target selection (`OpenCode`, `Claude`,
    `Both`) via `inquire`
  - non-interactive mode is available with one mutually-exclusive flag:
    `--opencode`, `--claude`, or `--both`
  - setup assets are embedded at compile time from `config/.opencode/**` and
    `config/.claude/**`
  - installation writes to repository-root `.opencode/` and/or `.claude/`
    using backup-and-replace safety with rollback on swap failures
- `mcp` is a placeholder for future file-cache tooling contracts
  (`cache-put`/`cache-get`).
- `hooks` is a placeholder for future git hook event and generated-region
  tracking integration.
- `sync` is a placeholder that runs a local Turso smoke check, then reports a
  deferred cloud-sync plan.

## Safety and limitations

- `mcp`, `hooks`, and `sync` remain placeholders and do not perform MCP
  transport, hook installation, or cloud sync.
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

Run repository flake checks (includes targeted setup command-surface checks from
`cli/`):

```bash
nix flake check
```
