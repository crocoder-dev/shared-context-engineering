# Overview

This repository maintains shared assistant configuration for OpenCode and Claude from a single canonical authoring source, then validates that generated outputs stay deterministic and in sync.

It also includes an early placeholder Rust CLI foundation at `cli/` for future Shared Context Engineering workflows.
The crate ships onboarding and usage documentation at `cli/README.md` that reflects current implemented vs placeholder behavior.

The CLI crate currently enforces a minimal dependency contract: `anyhow`, `inquire`, `lexopt`, `tokio`, and `turso`.
Its command loop is implemented with `lexopt` argument parsing and `anyhow` error handling, with deterministic placeholder dispatch for `setup`, `mcp`, and `hooks` through explicit service contracts.
The `setup` placeholder now includes an `inquire`-backed target-selection flow: default interactive selection for OpenCode/Claude/both, explicit non-interactive target flags (`--opencode`, `--claude`, `--both`), deterministic mutually-exclusive validation, and non-destructive cancellation exits.
The CLI now compiles an embedded setup asset manifest from `config/.opencode/**` and `config/.claude/**` via `cli/build.rs`; `cli/src/services/setup.rs` exposes deterministic normalized relative paths plus file bytes and target-scoped iteration without runtime reads from `config/`.
The setup service now also provides a repository-root install engine that stages embedded assets and performs backup-and-replace for `.opencode/` and `.claude/` with rollback restoration on swap failure; CLI end-to-end orchestration and messaging remain deferred to follow-on setup wiring.
The `mcp` placeholder contract is now scoped to future file-cache workflows (`cache-put`/`cache-get`) and remains intentionally non-runnable.
The `sync` placeholder performs a local Turso smoke check through a tokio-backed adapter and then reports a deferred cloud-sync plan from a placeholder gateway contract.

## Repository model

- Author once in canonical Pkl content (`config/pkl/base/shared-content.pkl`).
- Apply target-specific metadata/rendering in `config/pkl/renderers/`.
- Generate derived artifacts into `config/.opencode/**` and `config/.claude/**` via `config/pkl/generate.pkl`.
- Treat generated outputs as build artifacts, not primary editing surfaces.

## Ownership boundaries

- Generation-owned paths are authored config artifacts under `config/.opencode/**` and `config/.claude/**` (agents, commands, skills, shared drift library).
- Runtime/install artifacts are not generation-owned (for example `node_modules`, lockfiles, install outputs).
- Code and behavior changes must be made in canonical sources and renderer metadata, then regenerated.

## Core commands

- Regenerate outputs in place: `nix develop -c pkl eval -m . config/pkl/generate.pkl`
- Verify generated outputs are current: `nix develop -c ./config/pkl/check-generated.sh`
- Run staged destructive sync for `config/` and root `.opencode/`: `nix run .#sync-opencode-config`

## CI contracts

- `.github/workflows/pkl-generated-parity.yml` runs parity checks on pushes to `main` and pull requests targeting `main`.
- `.github/workflows/agnix-config-validate-report.yml` runs `agnix validate` from `config/`, fails on non-info findings, and uploads a deterministic report artifact when findings are present.

## Cross-target parity

- OpenCode and Claude are generated from the same canonical content with per-target capability mapping.
- When capabilities differ, parity is implemented by supported target-specific behavior rather than forcing unsupported fields.

## Context navigation

- Use `context/architecture.md` for component boundaries and current-state contracts.
- Use `context/patterns.md` for implementation and operational conventions.
- Use `context/decisions/` for explicit architecture decisions.
- Use `context/plans/` for task history and verification evidence.
- Use `context/cli/placeholder-foundation.md` for current command-surface, local Turso adapter behavior, and module-boundary details of the `sce` placeholder crate.
