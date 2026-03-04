# Agent Trace Implementation Contract (No Git Wrapper)

## Status
- Plan: `agent-trace-attribution-no-git-wrapper`
- Task: `T01`
- Scope: implementation contract baseline only (no production code changes)
- Normative keywords: `MUST`, `SHOULD`, `MAY`

## 1. Objective
Define one canonical, implementation-ready contract for Agent Trace attribution in this repository so later tasks (`T02`..`T15`) execute against a single set of invariants.

## 2. Core invariants
- Native Git workflows are preserved. Developers MUST continue to use normal Git entrypoints (`git commit`, `git rebase`, IDE commit UIs). This system MUST NOT replace `git` on `PATH`.
- Canonical interchange is Agent Trace JSON. Local and hosted flows MUST treat Agent Trace records as the source of truth for line-level attribution.
- Local VCS identity is fixed. Emitted records MUST set `vcs.type = "git"`.
- One canonical finalized trace per commit. Each finalized commit SHA MUST map to one canonical Agent Trace record (`version = "0.1.0"`) attached to `refs/notes/agent-trace` and mirrored to backend persistence.
- Co-author behavior is metadata-only UX. Human author/committer identity MUST NOT be rewritten by this system.
- SCE co-author trailer, when applicable, MUST use exactly `Co-authored-by: SCE <sce@crocoder.dev>` with idempotent insertion.

## 3. Command and workflow contracts

### 3.1 Local hook contracts
- `pre-commit`
  - MUST finalize attribution checkpoints from staged content only.
  - MUST capture index/tree anchors needed for later commit binding.
  - MUST no-op safely when disabled, missing CLI, or bare repository conditions apply.
- `commit-msg`
  - MUST apply canonical trailer policy only when staged SCE-attributed changes exist.
  - MUST honor `SCE_DISABLED` and `SCE_COAUTHOR_ENABLED` controls.
  - MUST deduplicate canonical trailer entries.
- `post-commit`
  - MUST build and finalize a trace for new `HEAD`.
  - MUST dual-write to Git notes (`refs/notes/agent-trace`) and backend storage (or queue fallback on transient failures).
  - MUST emit canonical trace media type `application/vnd.agent-trace.record+json`.
- `post-rewrite`
  - MUST ingest old->new commit pairs from rewrite events.
  - MUST trigger deterministic remap processing with replay-safe idempotency.

### 3.2 Hosted reconciliation contracts
- Hosted intake (GitHub/GitLab PR/MR updates, force-push) MUST produce deterministic idempotency keys for replay-safe orchestration.
- Reconciliation runs MUST preserve auditable old->new identity mapping and emit explicit confidence and quality outcomes.
- Hosted rewrites MUST NOT mutate canonical attribution semantics beyond declared rewrite metadata fields.

## 4. Canonical metadata keys

All extension metadata keys MUST use reverse-domain namespaced keys under `dev.crocoder.sce.*`.

Reserved key set:
- `dev.crocoder.sce.quality_status` -> one of `final | partial | needs_review`
- `dev.crocoder.sce.rewrite_from` -> previous commit SHA when record is rewritten
- `dev.crocoder.sce.rewrite_method` -> rewrite method enum (for example `amend`, `rebase`, `force_push_reconcile`)
- `dev.crocoder.sce.rewrite_confidence` -> normalized score `0.00`..`1.00`
- `dev.crocoder.sce.idempotency_key` -> deterministic replay key for hosted/local remap orchestration
- `dev.crocoder.sce.notes_ref` -> `refs/notes/agent-trace` when persisted via Git notes
- `dev.crocoder.sce.content_type` -> `application/vnd.agent-trace.record+json`

Rules:
- Unknown `dev.crocoder.sce.*` keys MAY be added later but MUST be forward-compatible and ignored safely by consumers.
- `quality_status`, `rewrite_*`, and `idempotency_key` fields MUST be preserved end-to-end if present.

## 5. Confidence and quality policy

### 5.1 Confidence scoring thresholds
- `>= 0.90`: high confidence; eligible for `final` quality when all required invariants pass.
- `0.60..0.89`: medium confidence; default `partial` unless explicit strict mapping criteria are met.
- `< 0.60`: low confidence; MUST set quality `needs_review`.

### 5.2 Quality status contract
- `final`
  - Required fields valid.
  - Deterministic commit identity resolution complete.
  - Attribution ranges structurally valid.
- `partial`
  - Required fields valid, but one or more confidence or remap guarantees are incomplete.
- `needs_review`
  - Any unresolved/low-confidence mapping, structural anomaly, or policy violation requiring operator inspection.

## 6. Failure policy
- Never lose trace intent:
  - If notes write fails and DB write succeeds, system MUST enqueue retry for notes sync.
  - If DB write fails and notes write succeeds, system MUST enqueue DB ingest retry.
  - If both fail, system MUST persist retry intent with deterministic idempotency and emit operational error metrics.
- Commit flow behavior:
  - Attribution tooling SHOULD be fail-open for normal developer commit completion unless explicitly configured otherwise.
  - Failures MUST be observable and replayable.
- Idempotency:
  - All finalize/rewrite pipelines MUST be safe to retry without duplicate canonical records for the same `(repo, commit_sha, trace_version)` tuple.

## 7. Rollout acceptance gates

Before enforcement is considered enabled in a repository, the following MUST pass:
- Hook installation and health checks (`sce doctor`) report ready state.
- At least one local commit path demonstrates staged-only attribution and canonical trace creation.
- Notes + backend dual-write path is verified, including one forced transient outage scenario with retry success.
- Local rewrite (`amend` and `rebase`) remap evidence shows deterministic old->new mapping outcomes.
- Hosted replay/idempotency evidence demonstrates duplicate events do not produce duplicate side effects.
- Compliance validation confirms Agent Trace required field presence and structural nesting.

## 8. Agent Trace field-level compliance matrix

### 8.1 Required Agent Trace fields

| Agent Trace field | Requirement | Local contract rule |
| --- | --- | --- |
| `version` | required | MUST emit `0.1.0` |
| `id` | required | MUST be UUID |
| `timestamp` | required | MUST be RFC 3339 date-time |
| `files` | required | MUST be non-empty when attributed file changes exist |

### 8.2 VCS block

| Agent Trace field | Requirement | Local contract rule |
| --- | --- | --- |
| `vcs.type` | required (when `vcs` present) | MUST be `git` |
| `vcs.revision` | required (when `vcs` present) | MUST be finalized commit SHA |

### 8.3 File attribution nesting

| Agent Trace path | Requirement | Local contract rule |
| --- | --- | --- |
| `files[].conversations[]` | optional but used | SHOULD be present for attributed edits |
| `files[].conversations[].url` | required in conversation object | MUST be URI-formatted |
| `files[].conversations[].ranges[]` | required when conversation carries ranges | MUST exist and be valid |
| `files[].conversations[].ranges[].start_line` | required | MUST be 1-indexed integer >= 1 |
| `files[].conversations[].ranges[].end_line` | required | MUST be integer >= `start_line` |
| `files[].conversations[].ranges[].contributor.type` | required | MUST be one of `human|ai|mixed|unknown` |
| `files[].conversations[].ranges[].contributor.model_id` | conditional | AI entries SHOULD use `provider/model` (models.dev convention) when known |

### 8.4 Optional links

| Agent Trace field | Requirement | Local contract rule |
| --- | --- | --- |
| `related[]` | optional | MUST preserve when present |

## 9. Normative mapping: internal attribution model -> Agent Trace

| Internal model element | Agent Trace destination | Mapping rule |
| --- | --- | --- |
| `TraceDraft.version` | `version` | Constant `0.1.0` |
| `TraceDraft.record_uuid` | `id` | UUID v4 string |
| `TraceDraft.emitted_at` | `timestamp` | RFC 3339 UTC timestamp |
| `CommitIdentity.sha` | `vcs.revision` | Finalized commit SHA |
| `CommitIdentity.vcs_kind` | `vcs.type` | Constant `git` |
| `FileAttribution.path` | `files[].path` | Repository-relative normalized path |
| `ConversationRef.url` | `files[].conversations[].url` | Valid URI string |
| `ConversationRef.related_urls[]` | `related[]` (or conversation-level related field if schema supports) | Preserve order and values |
| `LineRange.start` | `files[].conversations[].ranges[].start_line` | 1-indexed integer |
| `LineRange.end` | `files[].conversations[].ranges[].end_line` | Inclusive integer >= start |
| `RangeAttribution.kind` | `files[].conversations[].ranges[].contributor.type` | Enum map: `human|ai|mixed|unknown` |
| `RangeAttribution.model` | `...contributor.model_id` | `provider/model` when available |
| `RewriteInfo.from_sha` | `metadata[dev.crocoder.sce.rewrite_from]` | Only for rewritten commits |
| `RewriteInfo.method` | `metadata[dev.crocoder.sce.rewrite_method]` | Enum string |
| `RewriteInfo.confidence` | `metadata[dev.crocoder.sce.rewrite_confidence]` | Decimal `0.00`..`1.00` |
| `QualityState` | `metadata[dev.crocoder.sce.quality_status]` | `final|partial|needs_review` |
| `TransportInfo.content_type` | `metadata[dev.crocoder.sce.content_type]` | Constant media type |
| `TransportInfo.notes_ref` | `metadata[dev.crocoder.sce.notes_ref]` | Constant `refs/notes/agent-trace` |
| `ReplayInfo.idempotency_key` | `metadata[dev.crocoder.sce.idempotency_key]` | Deterministic key |

## 10. Version-format interoperability note
- Known ambiguity: public RFC page currently shows a possible pattern/example mismatch for `version` formatting.
- Contract decision: emit `version = "0.1.0"` canonically, and keep readers tolerant to equivalent semver-like variants where needed.

## 11. Implementation sequencing implications
- `T02` MUST implement schema adapter outputs matching section 8 and section 9.
- `T03` MUST prove deterministic serialization + compliance validation against Agent Trace schema.
- `T04`..`T09` MUST preserve invariants in sections 2 through 7.
- `T10`..`T14` MUST persist metadata needed to support section 5 and section 6 without semantic loss.
