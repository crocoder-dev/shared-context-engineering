# Context Map

Primary context files:
- `context/overview.md`
- `context/architecture.md`
- `context/patterns.md`
- `context/glossary.md`

Feature/domain context:
- `context/cli/cli-command-surface.md` (CLI command surface including slim top-level help visibility, setup install flow, WorkOS device authorization flow + token storage behavior, bounded resilience-wrapped sync/local-DB smoke and bootstrap behavior, nested flake release package/app installability, and Cargo local install + crates.io readiness policy)
- `context/cli/default-path-catalog.md` (canonical production CLI path-ownership contract centered on `cli/src/services/default_paths.rs`, including persisted, repo-relative, embedded-asset, install/runtime, hook, and context-path families plus the regression guard that keeps production path ownership centralized)
- `context/cli/styling-service.md` (CLI text-mode output styling with `owo-colors` and `comfy-table`, TTY/`NO_COLOR` policy, and shared helper API for human-facing surfaces)
- `context/cli/config-precedence-contract.md` (implemented `sce config` show/validate command contract, deterministic `flags > env > config file > defaults` resolution order, canonical `$schema` acceptance for startup-loaded `sce/config.json` files, shared auth-key env/config/optional baked-default support starting with `workos_client_id`, shared runtime resolution for flat logging plus nested `otel` observability keys, canonical Pkl-generated `sce/config.json` schema ownership plus CLI embedding/reuse contract, config-file selection order, `show` provenance output, trimmed `validate` output contract, and opt-in compiled-binary config-precedence E2E coverage contract)
- `context/sce/cli-observability-contract.md` (implemented config-backed runtime observability contract for the flat logging + nested `otel` config-file shape with env-over-config fallback, plus operator-facing `sce config show` observability reporting and the trimmed `sce config validate` status-only validation surface)
- `context/sce/shared-context-code-workflow.md`
- `context/sce/shared-context-plan-workflow.md` (canonical `/change-to-plan` workflow, clarification/readiness gate contract, and one-task/one-atomic-commit task-slicing policy)
- `context/sce/plan-code-overlap-map.md` (T01 overlap matrix for Shared Context Plan/Code, related commands, and core skill ownership/dedup targets)
- `context/sce/dedup-ownership-table.md` (current-state canonical owner-vs-consumer matrix for shared SCE behavior domains and thin-command ownership boundaries)

- `context/sce/atomic-commit-workflow.md` (canonical `/commit` command + `sce-atomic-commit` skill contract and naming decision)
- `context/sce/agent-trace-implementation-contract.md` (normative T01 implementation contract for no-git-wrapper Agent Trace attribution invariants, compliance matrix, and internal-to-Agent-Trace mapping)
- `context/sce/agent-trace-schema-adapter.md` (T02 schema adapter contract and code-level mapping surface in `cli/src/services/agent_trace.rs`)
- `context/sce/agent-trace-payload-builder-validation.md` (T03 deterministic payload-builder path, model-id normalization behavior, and Agent Trace schema validation suite)
- `context/sce/agent-trace-pre-commit-staged-checkpoint.md` (T04 pre-commit staged-only finalization contract with no-op guards and index/tree anchor capture)
- `context/sce/agent-trace-commit-msg-coauthor-policy.md` (T05 commit-msg canonical co-author trailer policy with env-gated injection and idempotent dedupe)
- `context/sce/agent-trace-post-commit-dual-write.md` (T06 post-commit trace finalization contract, persistent local DB bootstrap/path policy, notes+DB dual-write behavior, idempotency ledger guard, and retry-queue fallback semantics)
- `context/sce/agent-trace-hook-doctor.md` (approved operator-environment contract for broadening `sce doctor` into the canonical health-and-repair entrypoint, including stable problem taxonomy, `--fix` semantics, setup-to-doctor alignment rules, and the approved downstream human text-mode layout/status/integration contract; current implementation baseline is captured inside the file)
- `context/sce/setup-githooks-install-contract.md` (T01 canonical `sce setup --hooks` install contract for target-path resolution, idempotent outcomes, backup/rollback, and doctor-readiness alignment)
- `context/sce/setup-no-backup-policy-seam.md` (implemented shared `SetupBackupPolicy` seam that classifies git-backed vs non-git-backed setup targets and feeds both config-install and required-hook install flows)
- `context/sce/setup-githooks-hook-asset-packaging.md` (T02 compile-time `sce setup --hooks` required-hook template packaging contract and setup-service accessor surface)
- `context/sce/setup-githooks-install-flow.md` (T03 setup-service required-hook install orchestration with git-truth hooks-path resolution, per-hook installed/updated/skipped outcomes, shared git-backed no-backup policy branching, and recovery guidance semantics)
- `context/sce/setup-githooks-cli-ux.md` (T04 composable `sce setup` target+`--hooks` / `--repo` command-surface contract, option compatibility validation, and deterministic setup/hook output semantics)
- `context/sce/cli-security-hardening-contract.md` (T06 CLI redaction contract, setup `--repo` canonicalization/validation, and setup write-permission probe behavior)
- `context/sce/agent-trace-post-rewrite-local-remap-ingestion.md` (T08 `post-rewrite` local remap ingestion contract with strict pair parsing, rewrite-method normalization, and deterministic replay-key derivation)
- `context/sce/agent-trace-rewrite-trace-transformation.md` (T09 rewritten-SHA trace transformation contract with rewrite metadata, confidence-to-quality mapping, and notes+DB persistence parity)
- `context/sce/agent-trace-core-schema-migrations.md` (T10 core local schema migration contract for `repositories`, `commits`, `trace_records`, and `trace_ranges` with upgrade-safe idempotent create semantics)
- `context/sce/agent-trace-reconciliation-schema-ingestion.md` (T11 reconciliation persistence schema for `reconciliation_runs`, `rewrite_mappings`, and `conversations` with replay-safe idempotency and query indexes)
- `context/sce/agent-trace-retry-queue-observability.md` (T14 retry queue recovery contract plus reconciliation/runtime observability metrics and DB-first queue schema additions)
- `context/sce/agent-trace-local-hooks-mvp-contract-gap-matrix.md` (T01 Local Hooks MVP production contract freeze and deterministic gap matrix for `agent-trace-local-hooks-production-mvp`)
- `context/sce/agent-trace-hooks-command-routing.md` (implemented `sce hooks` command routing plus current runtime entrypoint behavior, including commit-msg policy gating/file mutation and post-rewrite remap+rewrite finalization wiring)
- `context/sce/automated-profile-contract.md` (deterministic gate policy for automated OpenCode profile, including 10 gate categories, permission mappings, and automated profile constraints)
- `context/sce/bash-tool-policy-enforcement-contract.md` (approved bash-tool blocking contract plus the implementation target for generated OpenCode enforcement, including config schema, argv-prefix matching, fixed preset catalog/messages, and precedence rules)
- `context/sce/generated-opencode-plugin-registration.md` (current generated OpenCode plugin-registration contract, canonical Pkl ownership, generated manifest/plugin paths, and TypeScript source ownership; Claude bash-policy enforcement has been removed from generated outputs)
- `context/sce/cli-first-install-channels-contract.md` (current first-wave `sce` install/distribution contract covering supported channels, canonical naming, `.version` release authority, and Nix-owned build policy)
- `context/sce/cli-release-artifact-contract.md` (shared `sce` release artifact naming, checksum/manifest outputs, GitHub Releases as the canonical artifact publication surface, and the current four-target Linux/macOS release workflow topology including Linux ARM)
- `context/sce/cli-npm-distribution-contract.md` (implemented `sce` npm launcher package, release-manifest/checksum-verified native binary install flow, the supported darwin/linux x64+arm64 npm platform matrix, and dedicated `.github/workflows/publish-npm.yml` downstream npm publish-stage contract)
- `context/sce/cli-cargo-distribution-contract.md` (implemented `sce` Cargo publication posture plus supported crates.io, git, and local checkout install guidance, dedicated crates.io publish workflow, and ephemeral crate-local generated-asset mirror requirement for published builds)

Working areas:
- `context/plans/` (active plan execution artifacts, not durable history)
- `context/handovers/`
- `context/decisions/`
- `context/tmp/`

Supporting repo docs:
- `AGENTS.md` (repo-specific agent workflow guidance, including optional local Nix tuning recommendations for user-level `~/.config/nix/nix.conf` and the explicit system-level-only boundary for `auto-optimise-store`)

Recent decision records:
- `context/decisions/2026-02-28-pkl-generation-architecture.md`
- `context/decisions/2026-03-03-plan-code-agent-separation.md`
- `context/decisions/2026-03-09-migrate-lexopt-to-clap.md` (CLI argument parsing migration from lexopt to clap derive macros)
- `context/decisions/2026-03-25-first-install-channels.md` (approved first-wave install/distribution scope for `sce`, canonical naming, and Nix-owned build policy)
