# Context Map

Primary context files:
- `context/overview.md`
- `context/architecture.md`
- `context/patterns.md`
- `context/glossary.md`

Feature/domain context:
- `context/cli/placeholder-foundation.md` (CLI command surface, setup install flow, WorkOS device authorization flow + token storage behavior, bounded resilience-wrapped sync/local-DB smoke and bootstrap behavior, nested flake release package/app installability, and Cargo local install + crates.io readiness policy)
- `context/cli/config-precedence-contract.md` (implemented `sce config` show/validate command contract, deterministic `flags > env > config file > defaults` resolution order, config-file selection order, and text/JSON output schema)
- `context/sce/shared-context-code-workflow.md`
- `context/sce/shared-context-plan-workflow.md` (canonical `/change-to-plan` workflow, clarification/readiness gate contract, and one-task/one-atomic-commit task-slicing policy)
- `context/sce/plan-code-overlap-map.md` (T01 overlap matrix for Shared Context Plan/Code, related commands, and core skill ownership/dedup targets)
- `context/sce/dedup-ownership-table.md` (current-state canonical owner-vs-consumer matrix for shared SCE behavior domains and thin-command ownership boundaries)
- `context/sce/workflow-token-footprint-inventory.md` (canonical Plan/Execute workflow participant inventory, T02 ranked token-hotspot table, T03 static token-accounting method, and T06 implemented token-count script behavior/usage contract)
- `context/sce/workflow-token-footprint-manifest.json` (T05 canonical machine-readable surface manifest for workflow token counting, including scope extraction rules and conditional flags)
- `context/sce/workflow-token-count-workflow.md` (root flake app contract for workflow token counting and its runtime wiring to evals script execution)
- `context/sce/atomic-commit-workflow.md` (canonical `/commit` command + `sce-atomic-commit` skill contract and naming decision)
- `context/sce/agent-trace-implementation-contract.md` (normative T01 implementation contract for no-git-wrapper Agent Trace attribution invariants, compliance matrix, and internal-to-Agent-Trace mapping)
- `context/sce/agent-trace-schema-adapter.md` (T02 schema adapter contract and code-level mapping surface in `cli/src/services/agent_trace.rs`)
- `context/sce/agent-trace-payload-builder-validation.md` (T03 deterministic payload-builder path, model-id normalization behavior, and Agent Trace schema validation suite)
- `context/sce/agent-trace-pre-commit-staged-checkpoint.md` (T04 pre-commit staged-only finalization contract with no-op guards and index/tree anchor capture)
- `context/sce/agent-trace-commit-msg-coauthor-policy.md` (T05 commit-msg canonical co-author trailer policy with env-gated injection and idempotent dedupe)
- `context/sce/agent-trace-post-commit-dual-write.md` (T06 post-commit trace finalization contract, persistent local DB bootstrap/path policy, notes+DB dual-write behavior, idempotency ledger guard, and retry-queue fallback semantics)
- `context/sce/agent-trace-hook-doctor.md` (T07 `sce doctor` hook install/health validation contract for default, per-repo, and global hook-path rollout)
- `context/sce/setup-githooks-install-contract.md` (T01 canonical `sce setup --hooks` install contract for target-path resolution, idempotent outcomes, backup/rollback, and doctor-readiness alignment)
- `context/sce/setup-githooks-hook-asset-packaging.md` (T02 compile-time `sce setup --hooks` required-hook template packaging contract and setup-service accessor surface)
- `context/sce/setup-githooks-install-flow.md` (T03 setup-service required-hook install orchestration with git-truth hooks-path resolution, per-hook installed/updated/skipped outcomes, and backup/rollback semantics)
- `context/sce/setup-githooks-cli-ux.md` (T04 composable `sce setup` target+`--hooks` / `--repo` command-surface contract, option compatibility validation, and deterministic setup/hook output semantics)
- `context/sce/setup-nix-integration-test-contract.md` (T01 canonical setup integration-test scenario matrix and deterministic assertion policy for Nix-run binary-driven tests)
- `context/sce/cli-security-hardening-contract.md` (T06 CLI redaction contract, setup `--repo` canonicalization/validation, and setup write-permission probe behavior)
- `context/sce/agent-trace-post-rewrite-local-remap-ingestion.md` (T08 `post-rewrite` local remap ingestion contract with strict pair parsing, rewrite-method normalization, and deterministic replay-key derivation)
- `context/sce/agent-trace-rewrite-trace-transformation.md` (T09 rewritten-SHA trace transformation contract with rewrite metadata, confidence-to-quality mapping, and notes+DB persistence parity)
- `context/sce/agent-trace-core-schema-migrations.md` (T10 core local schema migration contract for `repositories`, `commits`, `trace_records`, and `trace_ranges` with upgrade-safe idempotent create semantics)
- `context/sce/agent-trace-reconciliation-schema-ingestion.md` (T11 reconciliation persistence schema for `reconciliation_runs`, `rewrite_mappings`, and `conversations` with replay-safe idempotency and query indexes)
- `context/sce/agent-trace-hosted-event-intake-orchestration.md` (T12 hosted GitHub/GitLab event intake contract with signature verification, old/new head resolution, and deterministic reconciliation-run idempotency keys)
- `context/sce/agent-trace-rewrite-mapping-engine.md` (T13 hosted rewrite mapping engine contract with patch-id exact precedence, range-diff/fuzzy scoring, and deterministic unresolved outcomes)
- `context/sce/agent-trace-retry-queue-observability.md` (T14 retry queue recovery contract plus reconciliation/runtime observability metrics and DB-first queue schema additions)
- `context/sce/agent-trace-local-hooks-mvp-contract-gap-matrix.md` (T01 Local Hooks MVP production contract freeze and deterministic gap matrix for `agent-trace-local-hooks-production-mvp`)
- `context/sce/agent-trace-hooks-command-routing.md` (implemented `sce hooks` command routing plus current runtime entrypoint behavior, including commit-msg policy gating/file mutation and post-rewrite remap+rewrite finalization wiring)
- `context/sce/automated-profile-contract.md` (deterministic gate policy for automated OpenCode profile, including 10 gate categories, permission mappings, and automated profile constraints)

Working areas:
- `context/plans/` (active plan execution artifacts, not durable history)
- `context/handovers/`
- `context/decisions/`
- `context/tmp/`

Recent decision records:
- `context/decisions/2026-02-28-pkl-generation-architecture.md`
- `context/decisions/2026-03-03-plan-code-agent-separation.md`
