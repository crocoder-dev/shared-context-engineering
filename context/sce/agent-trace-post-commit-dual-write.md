# Agent Trace post-commit dual-write finalization

## Status
- Plan: `agent-trace-attribution-no-git-wrapper`
- Task: `T06`
- Implementation state: done

## Canonical contract
- Policy entrypoint: `cli/src/services/hooks.rs` -> `finalize_post_commit_trace`.
- Runtime no-op guards:
  - `sce_disabled = true` -> `NoOp(Disabled)`
  - `cli_available = false` -> `NoOp(CliUnavailable)`
  - `is_bare_repo = true` -> `NoOp(BareRepository)`
- Idempotency guard: `TraceEmissionLedger::has_emitted(commit_sha)` short-circuits to `NoOp(AlreadyFinalized)`.
- Emitted trace payload path uses `build_trace_payload` from `cli/src/services/agent_trace.rs` with:
  - `quality_status = final`
  - `metadata[dev.crocoder.sce.idempotency_key]` populated
  - optional `metadata[dev.crocoder.sce.parent_revision]` when a parent SHA is available
- Notes write policy is fixed to `refs/notes/agent-trace` with MIME `application/vnd.agent-trace.record+json`.

## Dual-write and fallback behavior
- Finalization attempts both targets in one pass:
  - notes write via `TraceNotesWriter`
  - DB persistence via `TraceRecordStore`
- Successful writes (`Written` or `AlreadyExists`) on both targets mark commit emission in `TraceEmissionLedger` and return `Persisted`.
- Any failed target (`PersistenceWriteResult::Failed`) enqueues one retry item via `TraceRetryQueue` with explicit failed target list and returns `QueuedFallback`.
- Retry queue entries carry the full trace record, MIME type, notes ref, and failed target list to support replay-safe recovery.

## Verification evidence
- `cargo test --manifest-path cli/Cargo.toml post_commit_finalization`
- `cargo build --manifest-path cli/Cargo.toml`
