# Overview

This repository maintains shared assistant configuration for OpenCode and Claude from a single canonical authoring source, then validates that generated outputs stay deterministic and in sync.
It now supports both manual and automated profile variants: the manual profile preserves interactive approval gates, while the automated profile applies deterministic non-interactive behavior for CI/automation workflows.

It also includes an early Rust CLI foundation at `cli/` for Shared Context Engineering workflows.
Operator-facing CLI usage currently comes from `sce --help`, command-local `--help` output, and focused context files under `context/cli/` and `context/sce/`.

The CLI crate currently depends on `anyhow`, `clap`, `clap_complete`, `dirs`, `hmac`, `inquire`, `opentelemetry`, `opentelemetry-otlp`, `opentelemetry_sdk`, `reqwest`, `serde`, `serde_json`, `sha2`, `tokio`, `tracing`, `tracing-opentelemetry`, `tracing-subscriber`, and `turso`.
Its command loop is implemented with `clap` derive-based argument parsing and `anyhow` error handling, with implemented auth flows (`auth login|logout|status`), implemented config inspection/validation (`config show`/`config validate`), real setup orchestration, implemented `doctor` rollout validation, implemented `hooks` subcommand routing/validation entrypoints, implemented machine-readable runtime identification (`version`), implemented shell completion script generation via `clap_complete` (`completion --shell <bash|zsh|fish>`), and placeholder dispatch for deferred commands (`mcp`, `sync`) through explicit service contracts.
The command loop now enforces a stable exit-code contract in `cli/src/app.rs`: `2` parse failures, `3` invocation validation failures, `4` runtime failures, and `5` dependency startup failures.
The same runtime also emits stable user-facing stderr error classes (`SCE-ERR-PARSE`, `SCE-ERR-VALIDATION`, `SCE-ERR-RUNTIME`, `SCE-ERR-DEPENDENCY`) using deterministic `Error [<code>]: ...` diagnostics with class-default `Try:` remediation appended when missing.
The app runtime now also includes a structured observability baseline in `cli/src/services/observability.rs`: deterministic env-controlled log threshold/format (`SCE_LOG_LEVEL`, `SCE_LOG_FORMAT`), optional file sink controls (`SCE_LOG_FILE`, `SCE_LOG_FILE_MODE` with deterministic `truncate` default), optional OpenTelemetry export bootstrap (`SCE_OTEL_ENABLED`, `OTEL_EXPORTER_OTLP_ENDPOINT`, `OTEL_EXPORTER_OTLP_PROTOCOL`), stable lifecycle event IDs, and stderr-only primary emission so stdout command payloads remain pipe-safe.
The app command dispatcher now enforces a centralized stdout/stderr stream contract in `cli/src/app.rs`: command success payloads are emitted on stdout only, while redacted user-facing diagnostics are emitted on stderr.
The CLI now also enforces a shared output-format parser contract in `cli/src/services/output_format.rs`, with canonical `--format <text|json>` parsing and command-specific actionable invalid-value guidance reused by `config` and `version` services.
The `setup` command includes an `inquire`-backed target-selection flow: default interactive selection for OpenCode/Claude/both with required-hook installation in the same run, explicit non-interactive target flags (`--opencode`, `--claude`, `--both`), deterministic mutually-exclusive validation, and non-destructive cancellation exits.
The CLI now compiles an embedded setup asset manifest from `config/.opencode/**`, `config/.claude/**`, and `cli/assets/hooks/**` via `cli/build.rs`; `cli/src/services/setup.rs` exposes deterministic normalized relative paths plus file bytes and target-scoped iteration without runtime reads from `config/`.
The setup service also provides repository-root install orchestration: it resolves interactive or flag-based target selection, installs embedded assets, and reports deterministic completion details (selected target(s), installed file counts, and backup actions).
The CLI now also applies baseline security hardening for reliability-driven automation: diagnostics/logging paths use deterministic secret redaction, `sce setup --hooks --repo <path>` canonicalizes and validates repository paths before execution, and setup write flows run explicit directory write-permission probes before staging/swap operations.
The config service now provides deterministic runtime config resolution with explicit precedence (`flags > env > config file > defaults`), strict config-file validation (`log_level`, `timeout_ms`, `workos_client_id`), deterministic default discovery/merge of global+local config files (`${state_root}/sce/config.json` then `.sce/config.json` with local override), shared auth-key resolution with optional baked defaults starting at `workos_client_id`, and deterministic text/JSON output contracts for `sce config show` and `sce config validate`.
The repository root flake now also exposes an opt-in compiled-binary config-precedence integration entrypoint, `nix run .#cli-config-precedence-integration-tests`, which runs `cli/tests/config_precedence_integration.rs` outside the default `nix flake check` path while `nix run .#cli-integration-tests` remains setup-only.
The `doctor` command now validates Agent Trace local rollout readiness by resolving effective git hook-path source (default, per-repo `core.hooksPath`, or global `core.hooksPath`) and checking required hook presence/executable permissions with actionable diagnostics.
The `mcp` placeholder contract is now scoped to future file-cache workflows (`cache-put`/`cache-get`) and remains intentionally non-runnable.
The `sync` placeholder performs a local Turso smoke check through a lazily initialized shared tokio current-thread runtime with bounded retry/timeout/backoff controls, then reports a deferred cloud-sync plan from a placeholder gateway contract; persistent local DB schema bootstrap now uses the same bounded resilience wrapper.
The nested CLI flake (`cli/flake.nix`) now applies a Rust overlay-backed stable toolchain (with `rustfmt`) and uses that toolchain contract for CLI check/build derivations.
The nested CLI flake now also exposes release install/run outputs: `packages.sce` (with `packages.default = packages.sce`) and `apps.sce`, so `nix build ./cli#default` and `nix run ./cli#sce -- --help` execute against the packaged `sce` binary.
The CLI Cargo package metadata now includes crates.io-facing fields while keeping `publish = false`; local install/release flows are documented as `cargo install --path cli --locked` and `cargo build --manifest-path cli/Cargo.toml --release`.
The repository-root flake now keeps nested CLI flake input wiring coherent by passing through `nixpkgs`, `flake-utils`, and `rust-overlay`, so root-level `nix flake check` can evaluate CLI checks (including setup command-surface and setup integration slices) without missing-input failures.
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
The hooks service now also includes a post-commit trace finalization seam (`finalize_post_commit_trace`) that builds canonical Agent Trace payloads, enforces commit-level idempotency guards, performs notes + DB dual writes, and enqueues retry fallback metadata when persistence targets fail; post-commit runtime now also enforces persistent local DB readiness (`.../sce/agent-trace/local.db`) with automatic schema bootstrap before DB writes, documented in `context/sce/agent-trace-post-commit-dual-write.md`.
The CLI now also includes a hook rollout doctor contract documented in `context/sce/agent-trace-hook-doctor.md`.
The hooks service now also includes a post-rewrite local remap ingestion seam (`finalize_post_rewrite_remap`) that parses `post-rewrite` old->new SHA pairs, normalizes rewrite method capture, and derives deterministic per-pair idempotency keys before remap dispatch; this behavior is documented in `context/sce/agent-trace-post-rewrite-local-remap-ingestion.md`.
The hooks service now also includes rewrite trace transformation finalization (`finalize_rewrite_trace`) that materializes rewritten-SHA Agent Trace records with `rewrite_from`/`rewrite_method`/`rewrite_confidence` metadata, confidence-threshold quality mapping (`final`/`partial`/`needs_review`), and notes+DB persistence parity with retry fallback; this behavior is documented in `context/sce/agent-trace-rewrite-trace-transformation.md`.
The local DB service now includes core Agent Trace persistence schema migrations (`apply_core_schema_migrations`) that install idempotent foundational tables and indexes for `repositories`, `commits`, `trace_records`, and `trace_ranges`; this behavior is documented in `context/sce/agent-trace-core-schema-migrations.md`.
The local DB service now also includes reconciliation persistence schema coverage in the same migration entrypoint for hosted rewrite bookkeeping tables (`reconciliation_runs`, `rewrite_mappings`, `conversations`) and replay/query indexes; this behavior is documented in `context/sce/agent-trace-reconciliation-schema-ingestion.md`.
The CLI now also includes a hosted event intake/orchestration seam in `cli/src/services/hosted_reconciliation.rs` that verifies provider signatures, resolves old/new commit heads from GitHub/GitLab payloads, and creates deterministic replay-safe reconciliation run requests; this behavior is documented in `context/sce/agent-trace-hosted-event-intake-orchestration.md`.
The hosted reconciliation service now also includes a deterministic rewrite mapping engine (`map_rewritten_commit`) that resolves old->new commit identity using patch-id exact precedence, then range-diff hints, then fuzzy fallback with a `>= 0.60` mapping threshold and explicit ambiguous/unmatched/low-confidence unresolved outcomes; this behavior is documented in `context/sce/agent-trace-rewrite-mapping-engine.md`.
The hooks service now also includes operational retry-queue replay processing (`process_trace_retry_queue`) invoked from post-commit and post-rewrite runtime flows with bounded same-pass replay and deterministic retry summary output, plus per-attempt runtime/error-class metric emission; the hosted reconciliation service now includes mapped/unmapped + confidence histogram metric snapshots (`summarize_reconciliation_metrics`), with DB-first queue/metrics schema coverage in `apply_core_schema_migrations`; this behavior is documented in `context/sce/agent-trace-retry-queue-observability.md`.
The hooks command surface now also supports concrete runtime subcommand routing (`pre-commit`, `commit-msg`, `post-commit`, `post-rewrite`) with deterministic argument/STDIN validation and production post-rewrite runtime wiring (local remap ingestion plus rewritten-trace finalization through notes+DB adapters) owned by `cli/src/services/hooks.rs`; this behavior is documented in `context/sce/agent-trace-hooks-command-routing.md`.
The setup service now also exposes deterministic required-hook embedded asset accessors (`iter_required_hook_assets`, `get_required_hook_asset`) backed by canonical templates in `cli/assets/hooks/` for `pre-commit`, `commit-msg`, and `post-commit`; this behavior is documented in `context/sce/setup-githooks-hook-asset-packaging.md`.
The setup service now also includes required-hook install orchestration (`install_required_git_hooks`) that resolves repository root and effective hooks path from git truth, enforces deterministic per-hook outcomes (`Installed`/`Updated`/`Skipped`), and performs backup-and-restore rollback on swap failures; this behavior is documented in `context/sce/setup-githooks-install-flow.md`.
The setup command parser/dispatch now also supports composable setup+hooks runs (`sce setup --opencode|--claude|--both --hooks`) plus hooks-only mode (`sce setup --hooks` with optional `--repo <path>`), enforces deterministic compatibility validation (`--repo` requires `--hooks`; target flags remain mutually exclusive), and emits deterministic setup/hook outcome messaging (`installed`/`updated`/`skipped` with backup status); this behavior is documented in `context/sce/setup-githooks-cli-ux.md`.

## Repository model

- Author once in canonical Pkl content (`config/pkl/base/shared-content.pkl` for manual profile, `config/pkl/base/shared-content-automated.pkl` for automated profile).
- Apply target-specific metadata/rendering in `config/pkl/renderers/`.
- Generate derived artifacts into `config/.opencode/**` (manual profile), `config/automated/.opencode/**` (automated profile), and `config/.claude/**` via `config/pkl/generate.pkl`.
- Treat generated outputs as build artifacts, not primary editing surfaces.

## Ownership boundaries

- Generation-owned paths are authored config artifacts under `config/.opencode/**`, `config/automated/.opencode/**`, and `config/.claude/**` (agents, commands, skills, shared drift library).
- Runtime/install artifacts are not generation-owned (for example `node_modules`, lockfiles, install outputs).
- Code and behavior changes must be made in canonical sources and renderer metadata, then regenerated.

## Core commands

- Regenerate outputs in place: `nix develop -c pkl eval -m . config/pkl/generate.pkl`
- Verify generated outputs are current: `nix run .#pkl-check-generated`
- Run staged destructive sync for `config/` and root `.opencode/`: `nix run .#sync-opencode-config`
- Run workflow token counting from repo root: `nix run .#token-count-workflows`
- Run setup integration tests through the deterministic flake app entrypoint: `nix run .#cli-integration-tests`
- Run opt-in config-precedence binary integration tests: `nix run .#cli-config-precedence-integration-tests`
- Run repository flake checks (includes CLI setup command-surface and setup integration checks): `nix flake check`

Lightweight post-task verification baseline (required after each completed task): run `nix run .#pkl-check-generated` and `nix flake check`.

## CI contracts

- `.github/workflows/pkl-generated-parity.yml` runs parity checks on pushes to `main` and pull requests targeting `main`.
- `.github/workflows/agnix-config-validate-report.yml` runs `agnix validate` from `config/`, fails on non-info findings, and uploads a deterministic report artifact when findings are present.
- `.github/workflows/workflow-token-count.yml` runs `nix run .#token-count-workflows` on pushes to `main` and pull requests targeting `main`, then uploads token-footprint artifacts from `context/tmp/token-footprint/`.
- `.github/workflows/cli-integration-tests.yml` runs `nix run .#cli-integration-tests` on pushes to `main` and pull requests targeting `main`.

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
- Use `context/sce/agent-trace-post-commit-dual-write.md` for the implemented T06 post-commit trace finalization and dual-write + queue-fallback behavior, including persistent local DB path/bootstrap policy for runtime writes.
- Use `context/sce/agent-trace-hook-doctor.md` for the implemented T07 hook install and health validation behavior (`sce doctor`) across default/per-repo/global hook-path installs.
- Use `context/sce/agent-trace-post-rewrite-local-remap-ingestion.md` for the implemented T08 post-rewrite local remap ingestion pipeline (`post-rewrite` pair parsing, rewrite-method normalization, and deterministic idempotency-key derivation).
- Use `context/sce/agent-trace-rewrite-trace-transformation.md` for the implemented T09 rewritten-SHA trace transformation path (`finalize_rewrite_trace`), confidence-based quality status mapping, and rewrite metadata persistence semantics.
- Use `context/sce/agent-trace-core-schema-migrations.md` for the implemented T10 core local schema migration contract (`apply_core_schema_migrations`) and table/index ownership across foundational Agent Trace persistence entities.
- Use `context/sce/agent-trace-reconciliation-schema-ingestion.md` for the implemented T11 reconciliation schema contract (`reconciliation_runs`, `rewrite_mappings`, `conversations`) and replay-safe idempotency/index coverage.
- Use `context/sce/agent-trace-hosted-event-intake-orchestration.md` for the implemented T12 hosted intake contract (GitHub/GitLab signature verification, old/new head resolution, deterministic reconciliation-run idempotency keys, and replay-safe run insertion outcomes).
- Use `context/sce/agent-trace-rewrite-mapping-engine.md` for the implemented T13 hosted mapping engine contract (patch-id exact matching, range-diff/fuzzy scoring precedence, confidence thresholds, and deterministic unresolved handling).
- Use `context/sce/agent-trace-retry-queue-observability.md` for the implemented T14 retry replay contract (notes/DB target-scoped recovery, per-attempt runtime/error-class metrics, reconciliation mapped/unmapped + confidence histogram snapshots, and DB-first retry/metrics schema additions).
- Use `context/sce/agent-trace-local-hooks-mvp-contract-gap-matrix.md` for the frozen T01 Local Hooks MVP production contract and deterministic gap matrix that maps current seam-level code truth to the remaining implementation stack (`T02`..`T10`).
- Use `context/sce/agent-trace-hooks-command-routing.md` for the implemented T02 `sce hooks` command routing contract (subcommand parsing, deterministic invocation errors, and initial runtime entrypoint behavior).
- Use `context/sce/setup-githooks-hook-asset-packaging.md` for the implemented `sce-setup-githooks-any-repo` T02 compile-time hook-template packaging contract and setup-service required-hook embedded accessor surface.
- Use `context/sce/setup-githooks-install-flow.md` for the implemented `sce-setup-githooks-any-repo` T03 required-hook install orchestration contract (git-truth hooks-path resolution, per-hook installed/updated/skipped outcomes, and backup/rollback behavior).
- Use `context/sce/setup-githooks-cli-ux.md` for the implemented `sce-setup-githooks-any-repo` T04 setup command-surface contract (`--hooks`, optional `--repo`), compatibility validation rules, and deterministic hook setup messaging.
- Use `context/sce/automated-profile-contract.md` for the automated OpenCode profile deterministic gate policy (10 gate categories, permission mappings, and automated profile constraints for non-interactive SCE workflows).
