# Overview

This repository maintains shared assistant configuration for OpenCode and Claude from a single canonical authoring source, then validates that generated outputs stay deterministic and in sync.

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
