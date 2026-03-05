# sce CLI (foundation)

This crate provides the early command-surface scaffold for the Shared Context
Engineering CLI (`sce`).

Current scope is intentionally narrow: deterministic command dispatch,
implemented repository `setup` flows (including hook installation),
implemented local rollout health checks via `doctor`, production local
`hooks` runtime execution, and explicit placeholders for commands that are
still deferred.

## Quick start

```bash
cargo run --manifest-path cli/Cargo.toml -- --help
cargo run --manifest-path cli/Cargo.toml -- setup
cargo run --manifest-path cli/Cargo.toml -- doctor
cargo run --manifest-path cli/Cargo.toml -- mcp
cargo run --manifest-path cli/Cargo.toml -- hooks pre-commit
cargo run --manifest-path cli/Cargo.toml -- hooks commit-msg .git/COMMIT_EDITMSG
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
  - required local hooks can be installed with `sce setup --hooks` (optionally
    `--repo <path>`) with deterministic per-hook
    `installed`/`updated`/`skipped` outcomes
- `doctor` is implemented and validates hook rollout readiness:
  - detects effective hooks directory for default, per-repo `core.hooksPath`,
    and global `core.hooksPath` installs
  - validates required hooks (`pre-commit`, `commit-msg`, `post-commit`) for
    presence and executable permissions
  - reports actionable diagnostics for missing or misconfigured hooks
- `mcp` is a placeholder for future file-cache tooling contracts
  (`cache-put`/`cache-get`).
- `hooks` is implemented for local Git hook execution:
  - `sce hooks pre-commit` captures staged-only checkpoint attribution
  - `sce hooks commit-msg <message-file>` enforces canonical co-author trailer
    policy when runtime gates pass
  - `sce hooks post-commit` finalizes Agent Trace records and performs
    notes+DB persistence with retry fallback
  - `sce hooks post-rewrite <amend|rebase|other>` ingests rewrite pairs from
    STDIN, applies rewrite remap + rewritten-trace finalization, and runs
    bounded retry replay
- `sync` is a placeholder that runs a local Turso smoke check, then reports a
  deferred cloud-sync plan.

## Safety and limitations

- `mcp` and `sync` remain placeholders and do not perform MCP transport or
  cloud sync.
- `sync` only validates local adapter wiring and does not require remote auth.
- Hosted reconciliation intake/mapping paths are not wired to public CLI
  commands yet.

## Near-term roadmap mapping

- Repository setup automation seam: `cli/src/services/setup.rs`
- Hook install health validation seam: `cli/src/services/doctor.rs`
- MCP file-cache seam: `cli/src/services/mcp.rs`
- Local hook runtime + persistence seam: `cli/src/services/hooks.rs`
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
