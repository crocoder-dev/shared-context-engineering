# Overview

This repository maintains shared assistant configuration for OpenCode and Claude from a single canonical Pkl authoring source. It validates that generated outputs stay deterministic and in sync via `nix run .#pkl-check-generated` and `nix flake check`. It supports both manual and automated profile variants; the automated profile applies deterministic non-interactive behavior for CI/automation workflows.

It also includes a Rust CLI (`sce`) for Shared Context Engineering workflows: auth, config inspection, setup, doctor, agent-trace hooks, bash-policy evaluation, and trace database inspection. See `context/architecture.md` for module-level boundaries and `context/context-map.md` for the full domain file index.

## Key cross-cutting contracts

- **Exit codes:** `2` parse, `3` validation, `4` runtime, `5` dependency failure (see `context/sce/cli-exit-code-contract.md`).
- **Stderr diagnostics:** stable `SCE-ERR-{PARSE,VALIDATION,RUNTIME,DEPENDENCY}` codes with class-default `Try:` remediation (see `context/sce/cli-error-code-taxonomy.md`).
- **Stdout/stderr:** command payloads on stdout only; redacted diagnostics on stderr (see `context/sce/cli-stdout-stderr-contract.md`).
- **Observability:** config-resolved logging to stderr, optional `SCE_LOG_FILE` mirroring (see `context/sce/cli-observability-contract.md`).
- **Config precedence:** `flags > env > config file > defaults` (see `context/cli/config-precedence-contract.md`).
- **Attribution hooks:** enabled by default, gated by staged-diff AI-overlap preflight; `SCE_ATTRIBUTION_HOOKS_DISABLED` opt-out (see `context/sce/agent-trace-commit-msg-coauthor-policy.md`).
- **Install channels:** repo-flake Nix, Cargo, npm, and source-built Flatpak (`dev.crocoder.sce`); Homebrew deferred (see `context/sce/cli-first-install-channels-contract.md`).

## Repository model

- Author once in canonical Pkl content organized by concern: `config/pkl/base/shared-content-{common,plan,code,commit}.pkl` for manual profile and `config/pkl/base/shared-content-automated-{common,plan,code,commit}.pkl` for automated profile; aggregation surfaces `config/pkl/base/shared-content.pkl` and `config/pkl/base/shared-content-automated.pkl` import from these grouped modules for downstream renderers.
- Apply target-specific metadata/rendering in `config/pkl/renderers/`.
- Generate derived artifacts into `config/.opencode/**` (manual profile), `config/automated/.opencode/**` (automated profile), and `config/.claude/**` via `config/pkl/generate.pkl`.
- Treat generated outputs as build artifacts, not primary editing surfaces.

## Ownership boundaries

- Generation-owned paths are authored config artifacts under `config/.opencode/**`, `config/automated/.opencode/**`, and `config/.claude/**` (agents, commands, skills, shared runtime libraries, OpenCode plugin files, generated OpenCode package manifests, generated OpenCode `opencode.json` manifests including SCE plugin registration, and Claude hook/settings assets).
- Runtime/install artifacts are not generation-owned (for example `node_modules`, lockfiles, install outputs).
- Code and behavior changes must be made in canonical sources and renderer metadata, then regenerated.

## Core commands

- Regenerate outputs in place: `nix develop -c pkl eval -m . config/pkl/generate.pkl`
- Verify generated outputs are current: `nix run .#pkl-check-generated`
- Run repository flake checks (CLI tests, clippy, fmt, pkl-parity, workflow-actionlint): `nix flake check`

Lightweight post-task verification baseline (required after each completed task): run `nix run .#pkl-check-generated` and `nix flake check`.

## Navigation

- For module boundaries and current-state contracts: `context/architecture.md`
- For implementation conventions: `context/patterns.md`
- For the full domain file index and discoverability: `context/context-map.md`
- For active plan execution state: `context/plans/`
- For architecture decisions: `context/decisions/`
