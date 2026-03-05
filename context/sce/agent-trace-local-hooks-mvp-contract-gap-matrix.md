# Agent Trace Local Hooks MVP Contract and Gap Matrix

## Status
- Plan: `agent-trace-local-hooks-production-mvp`
- Task: `T01`
- Scope: contract and gap freeze only (no production code changes)
- Normative keywords: `MUST`, `SHOULD`, `MAY`

## Objective
Freeze one implementation-ready contract for Local Hooks MVP productionization and map current code-truth seams to missing runtime wiring required by tasks `T02`..`T10`.

## Local MVP boundary
- In scope: `sce hooks` runtime command flow for `pre-commit`, `commit-msg`, `post-commit`, `post-rewrite`; local notes + DB persistence; retry replay; rollout readiness alignment with `sce setup --hooks` and `sce doctor`.
- In scope: local runtime guard behavior (`SCE_DISABLED`, CLI availability, bare-repo safety), deterministic idempotency, and actionable failure diagnostics.
- Out of scope: hosted webhook ingestion/reconciliation execution (`T12+` equivalent scope), MCP productionization, cloud sync productionization.

## Production contract (frozen for T02..T10)

### 1) Command/runtime entrypoints
- `sce hooks` MUST become an implemented command surface and MUST route to concrete hook subcommands: `pre-commit`, `commit-msg`, `post-commit`, `post-rewrite`.
- Invalid hook invocations MUST return deterministic actionable usage errors.
- Hook runtime handlers MUST support Git hook argument/STDIN contracts without placeholder output.

### 2) Pre-commit contract
- Runtime MUST collect staged-vs-unstaged attribution inputs and pass staged-only data into `finalize_pre_commit_checkpoint`.
- Runtime MUST capture index/head tree anchors and persist finalized checkpoint artifacts for downstream binding.
- Runtime MUST preserve explicit no-op outcomes for disabled, CLI-unavailable, and bare-repo states.

### 3) Commit-msg contract
- Runtime MUST read, transform, and write the real commit message file path passed by Git.
- Runtime MUST apply canonical trailer policy only when gates pass (`SCE_DISABLED`, `SCE_COAUTHOR_ENABLED`, staged-attribution present).
- Runtime MUST preserve idempotency and newline semantics during file mutation.

### 4) Post-commit contract
- Runtime MUST materialize canonical Agent Trace payloads through `build_trace_payload` and finalize via notes + DB dual-write adapters.
- Runtime MUST gate duplicate emission through commit-level ledger checks.
- Runtime MUST enqueue target-scoped retry entries when either notes or DB write fails.

### 5) Post-rewrite contract
- Runtime MUST parse `post-rewrite` old/new SHA input, normalize rewrite method, and ingest deterministic remap requests.
- Runtime MUST emit rewritten-SHA trace finalization with rewrite metadata and confidence-to-quality mapping.
- Runtime MUST preserve idempotent replay behavior for duplicate rewrite events.

### 6) Persistence and schema contract
- Local production runtime MUST use a deterministic persistent DB path policy (not in-memory).
- Schema bootstrap (`apply_core_schema_migrations`) MUST run before production write paths.
- Notes persistence target MUST remain `refs/notes/agent-trace` with content type `application/vnd.agent-trace.record+json`.

### 7) Retry/observability contract
- Retry processor MUST be invokable in local production workflow and MUST recover only failed targets per entry.
- Retry processing MUST emit per-attempt runtime/error-class metrics.
- Same-pass duplicate trace processing MUST be prevented.

### 8) Rollout/health contract
- `sce setup --hooks` remains canonical install path for required hook scripts.
- `sce doctor` remains canonical health/readiness validator for required hook files and executable state.
- Operator docs MUST describe install, health verification, expected artifacts, and recovery workflow.

## Deterministic policy decisions frozen in T01
- DB location policy target for production local writes: platform state-data location under `sce/agent-trace/local.db` (Linux baseline: `${XDG_STATE_HOME:-~/.local/state}/sce/agent-trace/local.db`); non-Linux follows equivalent per-user state-data root.
- Runtime failure posture for local hooks: fail-open for commit progression by default, while preserving retry-safe persistence intent and diagnostics.
- Idempotency unit for finalized local commit traces: one canonical finalized record per commit SHA.

## Module ownership map (code truth)
- CLI command parsing/dispatch: `cli/src/app.rs`
- Command surface status/help text: `cli/src/command_surface.rs`
- Hook-domain contracts/finalizers/retry processor: `cli/src/services/hooks.rs`
- Agent Trace payload adapter/builder/schema contract: `cli/src/services/agent_trace.rs`
- Local DB connection/migrations/smoke helpers: `cli/src/services/local_db.rs`
- Hook installation orchestration: `cli/src/services/setup.rs`
- Hook readiness diagnostics: `cli/src/services/doctor.rs`

## Gap matrix (current code truth -> required runtime completion)

| MVP area | Current state (code truth) | Required completion target | Planned task(s) |
| --- | --- | --- | --- |
| `sce hooks` command routing | `hooks` command dispatches `run_placeholder_hooks()` and rejects extra args as plain subcommand extras (`cli/src/app.rs`, `cli/src/command_surface.rs`). | Implement concrete subcommand parser/dispatcher for `pre-commit`/`commit-msg`/`post-commit`/`post-rewrite` with deterministic errors and no placeholder messaging. | `T02` |
| Pre-commit runtime wiring | Finalizer exists but has no real Git runtime data collection or persistence handoff (`cli/src/services/hooks.rs`). | Add runtime staged-data collection, anchor capture from Git state, and finalized checkpoint storage handoff for downstream commit binding. | `T03` |
| Commit-msg file IO wiring | Policy transformer exists as pure string function only (`apply_commit_msg_coauthor_policy`). | Wire real commit message file read/transform/write flow with newline/idempotency guarantees and deterministic file-path failures. | `T04` |
| Post-commit persistence adapters | Finalizer contracts exist as traits/in-memory seams; no production adapters bound to git notes/local DB/ledger queue in command runtime path. | Implement production adapters for notes write, DB write, emission ledger, queue enqueue, and runtime error-class mapping. | `T05` |
| Local DB persistent runtime policy | DB helpers support in-memory/path targets and migrations, but no production path policy/bootstrap lifecycle wiring for hooks runtime. | Add deterministic persistent path resolution + directory creation and run schema migrations before write paths. | `T06` |
| Post-rewrite runtime orchestration | Remap and rewrite finalizers exist, but no implemented hook command path binds STDIN/method args to these flows. | Implement runtime ingestion of Git `post-rewrite` inputs and wire remap + rewritten-trace finalization path end to end. | `T07` |
| Retry replay operational trigger | Retry processor exists but not integrated into operational hook runtime trigger strategy. | Wire retry execution into local workflow with bounded batch processing and metrics outputs. | `T08` |
| Release hardening gates | Placeholder command/help text and dead-code suppression (`#[allow(dead_code)]` in local DB target enum) indicate incomplete production wiring. | Remove local-hooks-module dead-code warnings through real wiring, tighten diagnostics, and update operator docs/runbooks. | `T09` |
| End-to-end validation signoff | No full local commit/rewrite production evidence bundle yet for this MVP scope. | Execute full verification suite, capture deterministic evidence, clean temporary artifacts, and sync context to final code truth. | `T10` |

## Acceptance checklist for T01
- [x] One current-state contract artifact defines Local Hooks MVP production boundaries and behavioral requirements.
- [x] Deterministic gap matrix maps code-truth seams to remaining runtime work for `T02`..`T10`.
- [x] Ownership mapping is explicit for command/runtime/persistence/doctor/setup modules.
