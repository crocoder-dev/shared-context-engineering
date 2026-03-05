# Plan: agent-trace-local-hooks-production-mvp
## 1) Change summary
Connect the existing Agent Trace service seams into a fully functional local Git-hook pipeline for production readiness: real `sce hooks` subcommand execution, end-to-end hook data flow (`pre-commit`, `commit-msg`, `post-commit`, `post-rewrite`), canonical notes + local DB persistence, retry recovery, and operator-facing rollout/validation guarantees.

## 2) Success criteria
- Local hooks MVP is production-functional: required hooks execute real behavior through `sce hooks <hook>` instead of placeholder output.
- End-to-end local commit flow is validated: staged-only pre-commit checkpointing, commit-msg co-author policy, post-commit canonical Agent Trace creation, and post-rewrite remap + rewrite-trace handling.
- Persistence contract is operational: canonical writes to `refs/notes/agent-trace` plus local DB persistence with deterministic idempotency and replay-safe retry behavior.
- Hard release gates pass for this scope: no dead-code warnings in Agent Trace/local-hooks production modules, deterministic tests for happy/failure/idempotency paths, and rollout docs/checklists are updated.

## 3) Constraints and non-goals
- In scope: local Git-hook productionization for Agent Trace (`pre-commit`, `commit-msg`, `post-commit`, `post-rewrite`), hook command wiring, local notes/DB persistence adapters, retry processing, doctor/setup alignment, and release validation artifacts.
- In scope: production decisions needed for local persistence/runtime policy (for example local DB path resolution policy, schema bootstrap timing, and hook runtime guard behavior).
- In scope: reducing current dead-code warnings by wiring currently isolated seams into executable production paths for this MVP slice.
- Out of scope: hosted webhook ingestion/orchestration and hosted reconciliation pipelines (T12+ equivalent behavior remains future scope).
- Out of scope: making `mcp` or cloud `sync` production-ready.
- Non-goal: broad architecture changes unrelated to local hooks attribution and persistence.

## 4) Task stack (T01..T10)
- [x] T01: Freeze Local Hooks MVP production contract and gap matrix (status:done)
  - Task ID: T01
  - Goal: Define the exact production MVP contract and map current seam-level implementation to missing runtime wiring/gates.
  - Boundaries (in/out of scope):
    - In: explicit local flow boundaries, required runtime guards, persistence policy decisions, and module ownership for hook runtime adapters.
    - Out: implementing code paths; this is contract/gap finalization only.
  - Done when:
    - A current-state contract artifact captures Local Hooks MVP behavior and acceptance boundaries.
    - A deterministic gap matrix lists each missing runtime piece needed to move from placeholder to functional behavior.
  - Verification notes (commands or checks):
    - Context review parity against `cli/src/services/{hooks,agent_trace,local_db,setup,doctor}.rs` and relevant context artifacts.

- [ ] T02: Implement real `sce hooks` command routing and hook argument handling (status:todo)
  - Task ID: T02
  - Goal: Replace placeholder-only hooks dispatch with concrete subcommand routing for `pre-commit`, `commit-msg`, `post-commit`, and `post-rewrite` execution.
  - Boundaries (in/out of scope):
    - In: parser/dispatch updates, deterministic error handling for invalid hook invocations, and wiring to concrete runtime handlers.
    - Out: deep persistence logic internals (handled in later tasks).
  - Done when:
    - `sce hooks <hook>` executes the corresponding production path instead of placeholder messaging.
    - Hook argument/STDIN contracts are validated and surfaced with actionable deterministic errors.
  - Verification notes (commands or checks):
    - `cargo test --manifest-path cli/Cargo.toml app::tests`
    - Focused hook command-surface tests for valid/invalid hook invocations.

- [ ] T03: Wire pre-commit runtime finalization to real staged attribution inputs (status:todo)
  - Task ID: T03
  - Goal: Connect `finalize_pre_commit_checkpoint` to real runtime data collection and deterministic checkpoint persistence handoff.
  - Boundaries (in/out of scope):
    - In: runtime guard evaluation, staged/unstaged extraction integration, anchor capture, and finalized checkpoint handoff/store seam.
    - Out: post-commit persistence and rewrite flow behavior.
  - Done when:
    - Pre-commit path produces staged-only finalized checkpoint artifacts for downstream commit binding.
    - No-op guard outcomes remain explicit and test-covered.
  - Verification notes (commands or checks):
    - `cargo test --manifest-path cli/Cargo.toml pre_commit`
    - End-to-end local repo fixture test proving unstaged ranges are excluded.

- [ ] T04: Wire commit-msg hook file mutation to canonical co-author policy (status:todo)
  - Task ID: T04
  - Goal: Connect `apply_commit_msg_coauthor_policy` to real commit message file IO in hook runtime with idempotent trailer handling.
  - Boundaries (in/out of scope):
    - In: commit message file read/transform/write flow, newline preservation, and policy gate wiring.
    - Out: author identity rewriting or non-canonical trailer behavior.
  - Done when:
    - Commit-msg runtime mutates message files only when policy gates pass and preserves idempotency/newline semantics.
    - Invalid message-file scenarios return deterministic actionable failures.
  - Verification notes (commands or checks):
    - `cargo test --manifest-path cli/Cargo.toml commit_msg_policy`
    - Hook-runtime integration test with on-disk commit message fixture.

- [ ] T05: Implement post-commit production persistence adapters (notes + DB + ledger + queue) (status:todo)
  - Task ID: T05
  - Goal: Connect `finalize_post_commit_trace` to concrete production adapters for notes writes, DB writes, emission ledger, and retry queue enqueue.
  - Boundaries (in/out of scope):
    - In: notes write adapter, DB write adapter, idempotency ledger storage behavior, fallback queue enqueue path, and runtime error classification mapping.
    - Out: hosted reconciliation workflows.
  - Done when:
    - Post-commit path persists canonical records to both targets or deterministically enqueues failed-target fallback.
    - Duplicate commit emission is prevented by ledger checks.
  - Verification notes (commands or checks):
    - `cargo test --manifest-path cli/Cargo.toml post_commit_finalization`
    - Local integration test validating notes content type/ref and DB persistence parity.

- [ ] T06: Productionize local DB runtime policy and schema bootstrap (status:todo)
  - Task ID: T06
  - Goal: Establish and implement production local DB location/bootstrap policy for Linux and other supported local platforms, then wire schema migration lifecycle.
  - Boundaries (in/out of scope):
    - In: deterministic DB path policy, path creation/error handling, startup migration execution, and migration idempotency behavior.
    - Out: hosted database/service infrastructure.
  - Done when:
    - Hook runtime uses a deterministic persistent DB target (not in-memory) for production paths.
    - Core/reconciliation/retry schema migrations are automatically ensured before writes.
  - Verification notes (commands or checks):
    - `cargo test --manifest-path cli/Cargo.toml local_db::tests`
    - Integration test proving persisted data survives process restart with configured local DB path.

- [ ] T07: Wire post-rewrite runtime flow (remap ingestion + rewrite trace finalization) (status:todo)
  - Task ID: T07
  - Goal: Connect `post-rewrite` hook runtime input parsing and rewrite-method normalization to real remap ingestion and rewritten-trace emission paths.
  - Boundaries (in/out of scope):
    - In: old/new SHA pair input ingestion, rewrite method handling, confidence/quality mapping flow, and fallback queue behavior for rewritten traces.
    - Out: hosted webhook event intake.
  - Done when:
    - Local amend/rebase rewrite scenarios emit deterministic remap ingestion requests and rewritten trace records.
    - Malformed input and duplicate replay scenarios are deterministic and test-covered.
  - Verification notes (commands or checks):
    - `cargo test --manifest-path cli/Cargo.toml post_rewrite_finalization`
    - `cargo test --manifest-path cli/Cargo.toml rewrite_trace_finalization`

- [ ] T08: Wire retry replay processor into operational runtime and observability outputs (status:todo)
  - Task ID: T08
  - Goal: Ensure retry queue processing is invokable in production local workflow with deterministic metrics emission and target-scoped recovery.
  - Boundaries (in/out of scope):
    - In: retry trigger strategy for local runtime, queue dequeue/requeue lifecycle, and metrics sink output integration.
    - Out: external metrics backends beyond current local/runtime contract.
  - Done when:
    - Failed-target retries are processed and recovered/requeued as expected with emitted runtime/error metrics.
    - Replay loops avoid same-pass duplicate processing for identical trace IDs.
  - Verification notes (commands or checks):
    - `cargo test --manifest-path cli/Cargo.toml hooks::tests::retry_processor_recovers_failed_notes_write_and_emits_success_metric`
    - `cargo test --manifest-path cli/Cargo.toml hooks::tests::retry_processor_requeues_when_db_write_still_fails`

- [ ] T09: Hardening pass for production gates (warnings, docs, rollout/runbook) (status:todo)
  - Task ID: T09
  - Goal: Satisfy hard release gates by eliminating dead-code warnings in MVP modules through real wiring, tightening failure diagnostics, and updating operator docs.
  - Boundaries (in/out of scope):
    - In: dead-code cleanup for Local Hooks MVP modules, CLI/help/readme/doctor/setup docs updates, and rollout checklist updates.
    - Out: cleanup of unrelated placeholder domains not needed for this MVP release.
  - Done when:
    - `clippy` for the target crate no longer reports dead-code warnings for local hooks production modules.
    - Operator docs clearly specify install, health checks, expected artifacts, and failure recovery workflow.
  - Verification notes (commands or checks):
    - `nix run ./cli#clippy`
    - `cargo test --manifest-path cli/Cargo.toml`
    - Documentation parity review across `cli/README.md` and context artifacts.

- [ ] T10: Validation and cleanup (status:todo)
  - Task ID: T10
  - Goal: Execute final end-to-end validation, evidence capture, artifact cleanup, and context sync verification for production-readiness signoff.
  - Boundaries (in/out of scope):
    - In: full verification suite, temporary artifact cleanup, and context/code alignment checks for changed behavior.
    - Out: net-new feature additions after validation freeze.
  - Done when:
    - End-to-end local commit and rewrite flows pass with deterministic evidence for success and failure/retry scenarios.
    - Required checks pass and context is synchronized to current behavior.
    - Residual risks and deferred items are explicitly documented.
  - Verification notes (commands or checks):
    - `cargo fmt --manifest-path cli/Cargo.toml -- --check`
    - `cargo build --manifest-path cli/Cargo.toml`
    - `cargo test --manifest-path cli/Cargo.toml`
    - `nix run ./cli#clippy`
    - `nix run .#pkl-check-generated`
    - `nix flake check`

## 5) Open questions
- None.
