# Context Map

Primary context files:
- `context/overview.md`
- `context/architecture.md`
- `context/patterns.md`
- `context/glossary.md`

Feature/domain context:
- `context/cli/cli-command-surface.md` (CLI command surface including slim top-level help visibility, setup install flow, WorkOS device authorization flow + token storage behavior, attribution-only hook routing, bounded resilience-wrapped sync/local-DB smoke behavior, nested flake release package/app installability, and Cargo local install + crates.io readiness policy)
- `context/cli/default-path-catalog.md` (canonical production CLI path-ownership contract centered on `cli/src/services/default_paths.rs`, including persisted, repo-relative, embedded-asset, install/runtime, hook, and context-path families plus the regression guard that keeps production path ownership centralized)
- `context/cli/styling-service.md` (CLI text-mode output styling with `owo-colors` and `comfy-table`, TTY/`NO_COLOR` policy, and shared helper API for human-facing surfaces)
- `context/cli/config-precedence-contract.md` (implemented `sce config` show/validate command contract, deterministic `flags > env > config file > defaults` resolution order, canonical `$schema` acceptance for startup-loaded `sce/config.json` files, shared auth-key env/config/optional baked-default support starting with `workos_client_id`, shared runtime resolution for flat logging plus nested `otel` observability keys, canonical Pkl-generated `sce/config.json` schema ownership plus CLI embedding/reuse contract, config-file selection order, `show` provenance output, trimmed `validate` output contract, and opt-in compiled-binary config-precedence E2E coverage contract)
- `context/sce/cli-observability-contract.md` (implemented config-backed runtime observability contract for the flat logging + nested `otel` config-file shape with env-over-config fallback, plus operator-facing `sce config show` observability reporting and the trimmed `sce config validate` status-only validation surface)
- `context/sce/shared-context-code-workflow.md`
- `context/sce/shared-context-plan-workflow.md` (canonical `/change-to-plan` workflow, clarification/readiness gate contract, and one-task/one-atomic-commit task-slicing policy)
- `context/sce/plan-code-overlap-map.md` (T01 overlap matrix for Shared Context Plan/Code, related commands, and core skill ownership/dedup targets)
- `context/sce/dedup-ownership-table.md` (current-state canonical owner-vs-consumer matrix for shared SCE behavior domains and thin-command ownership boundaries)

- `context/sce/atomic-commit-workflow.md` (canonical manual-vs-automated `/commit` contract, including the single-message rule, the staged-plan commit-body requirement to cite affected plan slug(s) + updated task ID(s), and automated single-commit execution behavior)
- `context/sce/agent-trace-implementation-contract.md` (historical no-git-wrapper Agent Trace design contract; not active runtime behavior)
- `context/sce/agent-trace-schema-adapter.md` (historical Agent Trace adapter reference for the removed `cli/src/services/agent_trace.rs` surface)
- `context/sce/agent-trace-payload-builder-validation.md` (historical Agent Trace builder/validation reference for the removed runtime surface)
- `context/sce/agent-trace-pre-commit-staged-checkpoint.md` (historical pre-commit staged-checkpoint contract; current runtime baseline has replaced this path with a deterministic no-op)
- `context/sce/agent-trace-commit-msg-coauthor-policy.md` (current commit-msg canonical co-author trailer policy with attribution-hooks + co-author gating and idempotent dedupe)
- `context/sce/agent-trace-post-commit-dual-write.md` (current post-commit no-op baseline plus historical dual-write reference)
- `context/sce/agent-trace-hook-doctor.md` (approved operator-environment contract for broadening `sce doctor` into the canonical health-and-repair entrypoint, including stable problem taxonomy, `--fix` semantics, setup-to-doctor alignment rules, the current neutral local-DB baseline, and the approved downstream human text-mode layout/status/integration contract)
- `context/sce/setup-githooks-install-contract.md` (T01 canonical `sce setup --hooks` install contract for target-path resolution, idempotent outcomes, remove-and-replace behavior, and doctor-readiness alignment)
- `context/sce/setup-no-backup-policy-seam.md` (implemented unified remove-and-replace install policy for both config-install and required-hook install flows, with no backup creation and deterministic recovery guidance on swap failure)
- `context/sce/setup-githooks-hook-asset-packaging.md` (T02 compile-time `sce setup --hooks` required-hook template packaging contract and setup-service accessor surface)
- `context/sce/setup-githooks-install-flow.md` (T03 setup-service required-hook install orchestration with git-truth hooks-path resolution, per-hook installed/updated/skipped outcomes, and remove-and-replace behavior with recovery guidance)
- `context/sce/setup-githooks-cli-ux.md` (T04 composable `sce setup` target+`--hooks` / `--repo` command-surface contract, option compatibility validation, and deterministic setup/hook output semantics)
- `context/sce/setup-repo-local-config-bootstrap.md` (T02 repo-local `.sce/config.json` bootstrap behavior: create-if-missing with canonical schema-only payload, leave-if-present, applies to all setup modes)
- `context/sce/cli-security-hardening-contract.md` (T06 CLI redaction contract, setup `--repo` canonicalization/validation, and setup write-permission probe behavior)
- `context/sce/agent-trace-post-rewrite-local-remap-ingestion.md` (current post-rewrite no-op baseline plus historical remap-ingestion reference)
- `context/sce/agent-trace-rewrite-trace-transformation.md` (current post-rewrite no-op baseline plus historical rewrite-transformation reference)
- `context/sce/agent-trace-core-schema-migrations.md` (current neutral local DB empty-file baseline with create/open-only runtime behavior and no schema bootstrap)
- `context/sce/agent-trace-retry-queue-observability.md` (inactive local-hook retry path plus historical retry/metrics reference)
- `context/sce/agent-trace-local-hooks-mvp-contract-gap-matrix.md` (T01 Local Hooks MVP production contract freeze and deterministic gap matrix for `agent-trace-local-hooks-production-mvp`)
- `context/sce/agent-trace-hooks-command-routing.md` (implemented `sce hooks` command routing plus the current minimal runtime behavior: disabled-default commit-msg attribution and no-op `pre-commit`/`post-commit`/`post-rewrite` entrypoints)
- `context/sce/automated-profile-contract.md` (deterministic gate policy for automated OpenCode profile, including 10 gate categories, permission mappings, automated `/commit` single-commit execution behavior, and automated profile constraints)
- `context/sce/bash-tool-policy-enforcement-contract.md` (approved bash-tool blocking contract plus the implementation target for generated OpenCode enforcement, including config schema, argv-prefix matching, fixed preset catalog/messages, and precedence rules)
- `context/sce/generated-opencode-plugin-registration.md` (current generated OpenCode plugin-registration contract, canonical Pkl ownership, generated manifest/plugin paths, and TypeScript source ownership; Claude bash-policy enforcement has been removed from generated outputs)
- `context/sce/cli-first-install-channels-contract.md` (current first-wave `sce` install/distribution contract covering supported channels, canonical naming, `.version` release authority, and Nix-owned build policy)
- `context/sce/optional-install-channel-integration-test-entrypoint.md` (current opt-in flake app contract for install-channel integration coverage, including thin flake delegation to the Rust runner, shared harness ownership, real npm+Bun+Cargo install flows, channel selector semantics, and the explicit non-default execution boundary)
- `context/sce/cli-release-artifact-contract.md` (shared `sce` release artifact naming, checksum/manifest outputs, GitHub Releases as the canonical artifact publication surface, and the current three-target Linux/macOS release workflow topology)
- `context/sce/cli-npm-distribution-contract.md` (implemented `sce` npm launcher package, release-manifest/checksum-verified native binary install flow, the supported darwin/arm64 plus linux x64+arm64 npm platform matrix, and dedicated `.github/workflows/publish-npm.yml` downstream npm publish-stage contract)
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
