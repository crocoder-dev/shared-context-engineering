# Overview

This repository maintains shared assistant configuration for OpenCode and Claude from a single canonical authoring source, then validates that generated outputs stay deterministic and in sync.

It also includes an early Rust CLI foundation at `cli/` for Shared Context Engineering workflows.
The crate ships onboarding and usage documentation at `cli/README.md` that reflects current implemented vs placeholder behavior.

The CLI crate currently enforces a minimal dependency contract: `anyhow`, `inquire`, `lexopt`, `tokio`, and `turso`.
Its command loop is implemented with `lexopt` argument parsing and `anyhow` error handling, with real setup orchestration, implemented `doctor` rollout validation, and placeholder dispatch for deferred commands through explicit service contracts.
The `setup` command includes an `inquire`-backed target-selection flow: default interactive selection for OpenCode/Claude/both, explicit non-interactive target flags (`--opencode`, `--claude`, `--both`), deterministic mutually-exclusive validation, and non-destructive cancellation exits.
The CLI now compiles an embedded setup asset manifest from `config/.opencode/**` and `config/.claude/**` via `cli/build.rs`; `cli/src/services/setup.rs` exposes deterministic normalized relative paths plus file bytes and target-scoped iteration without runtime reads from `config/`.
The setup service also provides repository-root install orchestration: it resolves interactive or flag-based target selection, installs embedded assets, and reports deterministic completion details (selected target(s), installed file counts, and backup actions).
The `doctor` command now validates Agent Trace local rollout readiness by resolving effective git hook-path source (default, per-repo `core.hooksPath`, or global `core.hooksPath`) and checking required hook presence/executable permissions with actionable diagnostics.
The `mcp` placeholder contract is now scoped to future file-cache workflows (`cache-put`/`cache-get`) and remains intentionally non-runnable.
The `sync` placeholder performs a local Turso smoke check through a lazily initialized shared tokio current-thread runtime and then reports a deferred cloud-sync plan from a placeholder gateway contract.
The nested CLI flake (`cli/flake.nix`) now applies a Rust overlay-backed stable toolchain (with `rustfmt`) and uses that toolchain contract for CLI check/build derivations.
The nested CLI flake now also exposes release install/run outputs: `packages.sce` (with `packages.default = packages.sce`) and `apps.sce`, so `nix build ./cli#default` and `nix run ./cli#sce -- --help` execute against the packaged `sce` binary.
The CLI Cargo package metadata now includes crates.io-facing fields while keeping `publish = false`; local install/release flows are documented as `cargo install --path cli --locked` and `cargo build --manifest-path cli/Cargo.toml --release`.
The repository-root flake now keeps nested CLI flake input wiring coherent by passing through `nixpkgs`, `flake-utils`, and `rust-overlay`, so root-level `nix flake check` can evaluate CLI checks without missing-input failures.
Shared Context Plan and Shared Context Code remain separate agent roles by design; planning (`/change-to-plan`) and implementation (`/next-task`) stay split while shared baseline guidance is deduplicated via canonical skill-owned contracts.
Their shared baseline doctrine (core principles, `context/` authority, and quality posture) is defined once as canonical snippets in `config/pkl/base/shared-content.pkl` and composed into both agent bodies during generation.
The `/next-task` command body is intentionally thin orchestration: readiness gating + phase sequencing are command-owned, while detailed implementation/context-sync contracts are skill-owned (`sce-plan-review`, `sce-task-execution`, `sce-context-sync`).
Context sync now uses an important-change gate: cross-cutting/policy/architecture/terminology changes require root shared-file edits, while localized tasks run verify-only root checks without default churn.
The `/change-to-plan` command body is also intentionally thin orchestration: it delegates clarification and plan-shape contracts to `sce-plan-authoring` (including one-task/one-atomic-commit task slicing) while keeping wrapper-level plan output and handoff obligations explicit.
The `/commit` command body is intentionally thin orchestration: it retains staged-confirmation and proposal-only constraints while delegating commit grammar and atomic split guidance to `sce-atomic-commit`.
The no-git-wrapper Agent Trace initiative baseline contract is defined in `context/sce/agent-trace-implementation-contract.md`, including normative invariants, compliance matrix, and canonical internal-to-Agent-Trace mapping for downstream implementation tasks.
The CLI now includes a task-scoped Agent Trace schema adapter contract in `cli/src/services/agent_trace.rs`, with deterministic mapping of internal attribution input to Agent Trace-shaped record structures documented in `context/sce/agent-trace-schema-adapter.md`.
The Agent Trace service now also provides a deterministic payload-builder path (`build_trace_payload`) with AI `model_id` normalization and schema-compliance validation coverage documented in `context/sce/agent-trace-payload-builder-validation.md`.
The hooks service now includes a pre-commit staged checkpoint finalization contract (`finalize_pre_commit_checkpoint`) that enforces staged-only attribution, captures index/tree anchors, and no-ops for disabled/unavailable/bare-repo runtime states; this behavior is documented in `context/sce/agent-trace-pre-commit-staged-checkpoint.md`.
The hooks service now also exposes a `commit-msg` co-author trailer policy (`apply_commit_msg_coauthor_policy`) that conditionally injects exactly one canonical SCE trailer based on `SCE_DISABLED`, `SCE_COAUTHOR_ENABLED`, and staged-attribution presence, with idempotent deduplication behavior documented in `context/sce/agent-trace-commit-msg-coauthor-policy.md`.
The hooks service now also includes a post-commit trace finalization seam (`finalize_post_commit_trace`) that builds canonical Agent Trace payloads, enforces commit-level idempotency guards, performs notes + DB dual writes, and enqueues retry fallback metadata when persistence targets fail; this behavior is documented in `context/sce/agent-trace-post-commit-dual-write.md`.
The CLI now also includes a hook rollout doctor contract documented in `context/sce/agent-trace-hook-doctor.md`.

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
- Verify generated outputs are current: `nix run .#pkl-check-generated`
- Run staged destructive sync for `config/` and root `.opencode/`: `nix run .#sync-opencode-config`
- Run workflow token counting from repo root: `nix run .#token-count-workflows`
- Run repository flake checks (includes CLI setup command-surface checks): `nix flake check`

Lightweight post-task verification baseline (required after each completed task): run `nix run .#pkl-check-generated` and `nix flake check`.

## CI contracts

- `.github/workflows/pkl-generated-parity.yml` runs parity checks on pushes to `main` and pull requests targeting `main`.
- `.github/workflows/agnix-config-validate-report.yml` runs `agnix validate` from `config/`, fails on non-info findings, and uploads a deterministic report artifact when findings are present.
- `.github/workflows/workflow-token-count.yml` runs `nix run .#token-count-workflows` on pushes to `main` and pull requests targeting `main`, then uploads token-footprint artifacts from `context/tmp/token-footprint/`.

## Cross-target parity

- OpenCode and Claude are generated from the same canonical content with per-target capability mapping.
- When capabilities differ, parity is implemented by supported target-specific behavior rather than forcing unsupported fields.

## Context navigation

- Use `context/architecture.md` for component boundaries and current-state contracts.
- Use `context/patterns.md` for implementation and operational conventions.
- Use `context/decisions/` for explicit architecture decisions.
- Use `context/plans/` for active plan execution state and task handoff continuity.
- Use `context/cli/placeholder-foundation.md` for current command-surface, local Turso adapter behavior, and module-boundary details of the `sce` placeholder crate.
- Use `context/sce/shared-context-plan-workflow.md` for the canonical planning-session workflow (`/change-to-plan`) including clarification gating and `/next-task` handoff contract.
- Use `context/sce/plan-code-overlap-map.md` for the current overlap/dedup inventory across Shared Context Plan/Code agents, related commands, and core skills.
- Use `context/sce/dedup-ownership-table.md` for canonical owner-vs-consumer boundaries and keep-vs-dedup labels used by the dedup implementation plan.
- Use `context/sce/workflow-token-footprint-inventory.md` for the canonical participant inventory of `/change-to-plan` and `/next-task` workflows, T02 ranked token-hotspot classification, and the T03 static token-accounting method/report template used by token-footprint analysis tasks.
- Use `context/sce/workflow-token-footprint-manifest.json` for the canonical machine-readable T05 manifest consumed by workflow token-count tooling (`surface_id`, workflow class, extraction scope rules, and conditional flags).
- Use `context/sce/workflow-token-count-workflow.md` for the root flake app contract (`nix run .#token-count-workflows`) and runtime wiring to the evals token-count script.
- Use `evals/token-count-workflows.ts` (run via `nix run .#token-count-workflows` from repo root, or `bun run token-count-workflows` from `evals/`) for T06 static workflow token counting that emits deterministic reports to `context/tmp/token-footprint/`.
- Use `context/sce/atomic-commit-workflow.md` for canonical `/commit` behavior, `sce-atomic-commit` naming, and proposal-only commit planning constraints.
- Use `context/sce/agent-trace-implementation-contract.md` for canonical Agent Trace implementation invariants and field-level mapping guidance (`agent-trace-attribution-no-git-wrapper` T01 baseline).
- Use `context/sce/agent-trace-schema-adapter.md` for the implemented T02 adapter contract and canonical mapping surface in `cli/src/services/agent_trace.rs`.
- Use `context/sce/agent-trace-payload-builder-validation.md` for the implemented T03 builder path, normalization policy, and schema-validation behavior.
- Use `context/sce/agent-trace-pre-commit-staged-checkpoint.md` for the implemented T04 pre-commit staged-only finalization contract and runtime no-op guards.
- Use `context/sce/agent-trace-commit-msg-coauthor-policy.md` for the implemented T05 commit-msg canonical co-author trailer policy and idempotent dedupe behavior.
- Use `context/sce/agent-trace-post-commit-dual-write.md` for the implemented T06 post-commit trace finalization and dual-write + queue-fallback behavior.
- Use `context/sce/agent-trace-hook-doctor.md` for the implemented T07 hook install and health validation behavior (`sce doctor`) across default/per-repo/global hook-path installs.
