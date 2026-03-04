# Plan: agent-trace-attribution-no-git-wrapper

## 1) Change summary
Implement a no-git-wrapper attribution platform that preserves normal developer Git workflows while producing commit-level Agent Trace records, storing line-level attribution ranges, and reconciling rewritten commits across local and hosted rewrite events.

## 2) Success criteria
- Generated and stored trace data is compliant with the Agent Trace RFC (`https://agent-trace.dev/`) for required structure and semantics.
- Every emitted trace record contains required fields (`version`, `id`, `timestamp`, `files`) and uses RFC 3339 timestamps plus UUID record IDs.
- `vcs` data is valid Agent Trace shape (`type`, `revision`) and local implementation pins `vcs.type = "git"`.
- File attribution shape matches spec nesting (`files[].conversations[].ranges[]`) with 1-indexed line ranges and valid contributor types (`human`, `ai`, `mixed`, `unknown`).
- Conversation links use URI-formatted `url` values; optional `related[]` links are preserved when present.
- AI contributor `model_id` values follow models.dev provider/model convention when available.
- Developers keep standard workflows (`git commit`, `git rebase`, IDE commits) without replacing `git` on `PATH`.
- Each finalized commit has one canonical Agent Trace record (`version = "0.1.0"`) attached to `refs/notes/agent-trace` and mirrored to backend storage.
- Local rewrite events (`rebase`, `amend`) remap trace attribution with auditable method/confidence metadata.
- Hosted rewrite events (GitHub/GitLab PR/MR updates and force-pushes) reconcile old/new commit identity with deterministic idempotency keys and replay-safe behavior.
- Co-author trailer behavior uses only canonical identity `Co-authored-by: SCE <sce@crocoder.dev>` when SCE contribution is present, with idempotent insertion.
- Persistence schema supports trace storage, flattened range analytics, reconciliation runs, and rewrite mappings with quality states (`final`, `partial`, `needs_review`).

## 3) Constraints and non-goals
- In scope: local hook-based capture/finalization, notes distribution, DB ingestion/indexing, hosted reconciliation worker, confidence policies, and operational observability.
- In scope: Agent Trace JSON as canonical interchange and source of truth for line-level attribution, with schema/field compliance to `https://agent-trace.dev/`.
- In scope: MIME and distribution alignment for trace payloads (`application/vnd.agent-trace.record+json` in notes and persisted records).
- In scope: one fixed SCE co-author identity for commit trailer UX metadata.
- Out of scope: legal ownership/copyright inference, model training provenance, and polished real-time IDE UX.
- Out of scope: replacing native git invocation, overriding human author/committer identity, or introducing multiple agent co-author identities.

## 4) Task stack (T01..T15)
- [x] T01: Finalize implementation contract baseline (status:done)
  - Task ID: T01
  - Goal: Translate architecture/hooks/identity/reconciler/schema into one implementation contract with strict invariants.
  - Boundaries (in/out of scope):
    - In: command contracts, metadata keys, confidence thresholds, failure policy, rollout acceptance gates, and Agent Trace field-level compliance matrix.
    - Out: production code changes.
  - Done when:
    - One contract artifact exists and removes cross-doc ambiguity.
    - Contract includes a normative mapping table from internal attribution structures to Agent Trace schema objects/fields.
  - Verification notes (commands or checks):
    - Structured contract checklist covering all source sections plus Agent Trace RFC required/optional field mapping.
    - Contract artifact: `context/sce/agent-trace-implementation-contract.md`.

- [x] T02: Define trace payload schema adapter and canonical metadata mapping (status:done)
  - Task ID: T02
  - Goal: Create a schema adapter that maps internal attribution structures to Agent Trace-compliant record shape.
  - Boundaries (in/out of scope):
    - In: `vcs` fields, metadata reverse-domain keys, quality status mapping, contributor enum rules, and canonical field mapping.
    - Out: runtime persistence and hook execution paths.
  - Done when:
    - A single adapter contract maps all required/optional Agent Trace fields used by this system.
    - Adapter output contract is deterministic and reusable by local finalize and rewrite flows.
  - Verification notes (commands or checks):
    - Mapping tests for required fields and extension metadata placement.
    - `cargo test --manifest-path cli/Cargo.toml`
    - `cargo fmt --manifest-path cli/Cargo.toml -- --check`
    - `cargo build --manifest-path cli/Cargo.toml`

- [x] T03: Implement trace payload builder and compliance validation suite (status:done)
  - Task ID: T03
  - Goal: Implement payload construction and schema-validation tests on top of the adapter.
  - Boundaries (in/out of scope):
    - In: deterministic serialization, URI/date-time formatting, model_id normalization, and JSON schema compliance checks.
    - Out: hook orchestration and DB/note write side effects.
  - Done when:
    - One builder path generates deterministic payloads for finalize and rewrite flows.
    - Builder output passes JSON schema validation against the published Agent Trace trace-record schema.
  - Verification notes (commands or checks):
    - Unit tests for serialization determinism and metadata correctness.
    - Schema-compliance tests for required fields, enum validation, URI/date-time format, and `files[].conversations[].ranges[]` nesting.
    - `cargo fmt --manifest-path cli/Cargo.toml -- --check`
    - `cargo test --manifest-path cli/Cargo.toml`
    - `cargo build --manifest-path cli/Cargo.toml`

- [x] T04: Implement `pre-commit` staged checkpoint finalization contract (status:done)
  - Task ID: T04
  - Goal: Bind pending checkpoints to staged content only and capture index/tree anchors.
  - Boundaries (in/out of scope):
    - In: no-op behavior for disabled/missing CLI/bare repo and staged-only attribution enforcement.
    - Out: commit-note writes.
  - Done when:
    - Unstaged edits cannot be attributed during commit finalization.
  - Verification notes (commands or checks):
    - Hook fixture tests with mixed staged/unstaged edits.
    - `cargo test --manifest-path cli/Cargo.toml pre_commit_finalization_uses_only_staged_ranges_and_captures_anchors`
    - `cargo test --manifest-path cli/Cargo.toml pre_commit_finalization_noops_when_sce_disabled`
    - `cargo build --manifest-path cli/Cargo.toml`

- [x] T05: Implement `commit-msg` canonical co-author trailer policy (status:done)
  - Task ID: T05
  - Goal: Add idempotent canonical SCE trailer injection when SCE-attributed staged changes exist.
  - Boundaries (in/out of scope):
    - In: `SCE_DISABLED`, `SCE_COAUTHOR_ENABLED`, dedupe behavior, canonical identity format.
    - Out: rewriting human author/committer identity.
  - Done when:
    - Exactly one canonical trailer appears in all allowed SCE cases.
  - Verification notes (commands or checks):
    - Identity acceptance checklist scenarios 1-5, 8, and 10.
    - `cargo fmt --manifest-path cli/Cargo.toml -- --check`
    - `cargo test --manifest-path cli/Cargo.toml commit_msg_policy`
    - `cargo build --manifest-path cli/Cargo.toml`

- [x] T06: Implement `post-commit` trace finalize and dual-write path (status:done)
  - Task ID: T06
  - Goal: Emit commit trace after commit creation and write to notes + DB (or queue fallback).
  - Boundaries (in/out of scope):
    - In: parent SHA handling, notes ref policy, emission idempotency, and MIME tagging (`application/vnd.agent-trace.record+json`).
    - Out: hosted reconciliation flow.
  - Done when:
    - New HEAD always produces a trace record with durable persistence semantics.
  - Verification notes (commands or checks):
    - End-to-end local commit tests including transient DB or notes outage.
    - `cargo test --manifest-path cli/Cargo.toml post_commit_finalization`
    - `cargo build --manifest-path cli/Cargo.toml`

- [x] T07: Add hook install and health validation (`sce doctor`) for local rollout (status:done)
  - Task ID: T07
  - Goal: Provide deterministic setup validation for per-repo and global hook-path installs.
  - Boundaries (in/out of scope):
    - In: hook presence/permissions/config checks and actionable diagnostics.
    - Out: hosted provider integration.
  - Done when:
    - Operators can verify hook readiness before enabling attribution enforcement.
  - Verification notes (commands or checks):
    - Doctor output tests for healthy, missing, and misconfigured hook states.
    - `cargo fmt --manifest-path cli/Cargo.toml -- --check`
    - `cargo test --manifest-path cli/Cargo.toml doctor_output_reports`
    - `cargo test --manifest-path cli/Cargo.toml doctor_command_exits_success`
    - `cargo build --manifest-path cli/Cargo.toml`

- [x] T08: Implement `post-rewrite` local remap ingestion pipeline (status:done)
  - Task ID: T08
  - Goal: Ingest old->new SHA pairs from rewrite events and trigger remap pipeline.
  - Boundaries (in/out of scope):
    - In: rewrite type capture, temporary pairs-file parsing, idempotent replay behavior.
    - Out: remote webhook event processing.
  - Done when:
    - Rebase/amend rewrites trigger deterministic remap processing without duplicate artifacts.
  - Verification notes (commands or checks):
    - Local rewrite fixture tests across amend and interactive/non-interactive rebase outcomes.
    - `cargo test --manifest-path cli/Cargo.toml post_rewrite_finalization`
    - `cargo fmt --manifest-path cli/Cargo.toml -- --check`
    - `cargo build --manifest-path cli/Cargo.toml`

- [x] T09: Implement rewrite trace transformation semantics (status:done)
  - Task ID: T09
  - Goal: Materialize new trace records for rewritten SHAs with explicit rewrite metadata.
  - Boundaries (in/out of scope):
    - In: new record ID/timestamp, `rewrite_from`, `rewrite_method`, `rewrite_confidence`, quality status logic, and preservation of RFC-compliant trace structure on rewritten commits.
    - Out: provider-specific mapping heuristics.
  - Done when:
    - Rewritten traces preserve attribution continuity and auditability.
  - Verification notes (commands or checks):
    - Integration tests asserting metadata integrity and notes/DB parity.
    - `cargo fmt --manifest-path cli/Cargo.toml -- --check`
    - `cargo test --manifest-path cli/Cargo.toml rewrite_trace_finalization`
    - `cargo build --manifest-path cli/Cargo.toml`

- [x] T10: Ship core schema migrations (`repositories`, `commits`, `trace_records`, `trace_ranges`) (status:done)
  - Task ID: T10
  - Goal: Establish foundational persistence tables, constraints, and indexes.
  - Boundaries (in/out of scope):
    - In: migration authoring and upgrade-safe execution.
    - Out: reconciliation-run tables and mapping pipeline logic.
  - Done when:
    - Core schema applies cleanly and supports local commit ingestion.
  - Verification notes (commands or checks):
    - Migration tests with empty and preexisting DB states.
    - `cargo test --manifest-path cli/Cargo.toml core_schema_migrations`
    - `cargo build --manifest-path cli/Cargo.toml`

- [x] T11: Ship reconciliation schema and ingestion (`reconciliation_runs`, `rewrite_mappings`, `conversations`) (status:done)
  - Task ID: T11
  - Goal: Add hosted rewrite persistence and idempotency-backed run bookkeeping.
  - Boundaries (in/out of scope):
    - In: run status lifecycle, mapping persistence, idempotency uniqueness, and indexes.
    - Out: provider webhook transport implementation.
  - Done when:
    - Reconciliation runs and mappings can be stored and queried reproducibly.
  - Verification notes (commands or checks):
    - Referential-integrity tests and representative mapping/replay query checks.
    - `cargo fmt --manifest-path cli/Cargo.toml -- --check`
    - `cargo test --manifest-path cli/Cargo.toml core_schema_migrations`
    - `cargo test --manifest-path cli/Cargo.toml reconciliation_schema_supports_replay_safe_runs_and_mapping_queries`
    - `cargo build --manifest-path cli/Cargo.toml`

- [x] T12: Implement hosted event intake and run orchestration (status:done)
  - Task ID: T12
  - Goal: Accept GitHub/GitLab webhook events, verify signatures, and create replay-safe runs.
  - Boundaries (in/out of scope):
    - In: provider event parsing, old/new head resolution, idempotency key generation.
    - Out: mapping heuristic internals.
  - Done when:
    - Duplicate events do not create duplicate side effects.
  - Verification notes (commands or checks):
    - Webhook signature and replay tests per provider.
    - `cargo test --manifest-path cli/Cargo.toml hosted_reconciliation`
    - `cargo fmt --manifest-path cli/Cargo.toml -- --check`
    - `cargo build --manifest-path cli/Cargo.toml`

- [x] T13: Implement mapping engine (patch-id, range-diff, fuzzy fallback) (status:done)
  - Task ID: T13
  - Goal: Map old commits to new commits using strict staged matching with confidence scoring.
  - Boundaries (in/out of scope):
    - In: patch-id exact, range-diff hints, fuzzy thresholding (`>= 0.60`) and unresolved handling.
    - Out: manual reviewer UI.
  - Done when:
    - Mapping outcomes are explainable, reproducible, and confidence-classified.
  - Verification notes (commands or checks):
    - Deterministic fixture tests for exact, ambiguous, unmatched, and low-confidence cases.
    - `cargo fmt --manifest-path cli/Cargo.toml -- --check`
    - `cargo test --manifest-path cli/Cargo.toml hosted_reconciliation`
    - `cargo build --manifest-path cli/Cargo.toml`

- [ ] T14: Implement notes write-back fallback, retry queue, and observability metrics (status:todo)
  - Task ID: T14
  - Goal: Guarantee no trace loss when notes pushes fail and expose reconciliation/runtime telemetry.
  - Boundaries (in/out of scope):
    - In: DB-first fallback queue, retry processor, run metrics (`mapped/unmapped`, histogram, runtime/error class).
    - Out: full operational dashboard productization.
  - Done when:
    - Failed notes pushes recover via retry and metrics expose operational state.
  - Verification notes (commands or checks):
    - Fault-injection and recovery tests with metric emission assertions.

- [ ] T15: Validation and cleanup (status:todo)
  - Task ID: T15
  - Goal: Run full-system validation, sync context/docs, and leave implementation evidence for handoff.
  - Boundaries (in/out of scope):
    - In: local commit + rewrite + hosted rewrite + outage/retry scenario verification and context sync.
    - Out: scope expansion beyond this architecture set.
  - Done when:
    - Every success criterion has evidence and no unresolved blocker remains.
    - Plan checkboxes/status and verification evidence are fully updated.
  - Verification notes (commands or checks):
    - End-to-end scenario runbook with idempotent replay and confidence policy validation.
    - Agent Trace compliance test report covering required fields, formats, nesting, enum constraints, and MIME expectations.
    - Context sync review across architecture/overview/glossary/patterns to match resulting code truth.

## 5) Open questions
- Agent Trace RFC page shows a potential version-format mismatch (`version` schema pattern appears two-segment while examples and document header use `0.1.0`); implementation currently plans to emit `0.1.0` and keep parser tolerant.
