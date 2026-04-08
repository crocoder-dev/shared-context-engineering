# Agent Trace post-commit persistence baseline

## Current status
- This contract is no longer active in runtime.
- The current `cli/src/services/hooks.rs` keeps `sce hooks post-commit` as a deterministic no-op.

## Status
- Plan: `agent-trace-attribution-no-git-wrapper`
- Task: `T06`
- Implementation state: done

## Canonical contract
- Policy entrypoint: `cli/src/services/hooks.rs` -> `finalize_post_commit_trace`.
- Runtime entrypoint status: retained but not active in the current trace-removal baseline.
- Runtime no-op guards:
  - `sce_disabled = true` -> `NoOp(Disabled)`
  - `attribution_hooks_enabled = false` -> `NoOp(AttributionDisabled)`
  - `trace_side_effects_enabled = false` -> `NoOp(AttributionOnlyMode)`
  - `cli_available = false` -> `NoOp(CliUnavailable)`
  - `is_bare_repo = true` -> `NoOp(BareRepository)`
- Idempotency guard: `TraceEmissionLedger::has_emitted(commit_sha)` short-circuits to `NoOp(AlreadyFinalized)`.
- Emitted trace payload path uses `build_trace_payload` from `cli/src/services/agent_trace.rs` with:
  - `quality_status = final`
  - `metadata[dev.crocoder.sce.idempotency_key]` populated
  - optional `metadata[dev.crocoder.sce.parent_revision]` when a parent SHA is available
- Notes write policy is fixed to `refs/notes/agent-trace` with MIME `application/vnd.agent-trace.record+json`.

## Current persistence behavior
- Finalization still attempts both persistence targets in one pass:
  - notes write via `TraceNotesWriter`
  - local DB write via `TraceRecordStore`
- The production local DB adapter is now `NoOpTraceRecordStore`, so the DB target returns `AlreadyExists` without writing trace rows.
- Successful notes persistence plus the no-op DB result mark commit emission in `TraceEmissionLedger` and return `Persisted`.
- Any failed target (`PersistenceWriteResult::Failed`) enqueues one retry item via `TraceRetryQueue` with explicit failed target list and returns `QueuedFallback`.
- Retry queue entries carry the full trace record, MIME type, notes ref, and failed target list to support replay-safe recovery.

## Retained runtime wiring details
- Runtime entrypoint: `cli/src/services/hooks.rs` -> `run_post_commit_subcommand_in_repo`.
- Current runtime posture:
  - `post-commit` remains invocable but exits through deterministic no-op output before invoking trace persistence behavior.
  - Enabling the attribution-hooks gate does not reactivate notes writes, local DB writes, retry replay, or emission-ledger mutation.
- Runtime input assembly:
  - resolves `HEAD` + optional `HEAD^` via git
  - derives commit timestamp from `git show -s --format=%cI HEAD`
  - derives file attribution from the pre-commit checkpoint artifact first, then falls back to changed-file discovery (`git show --name-only HEAD`)
  - derives deterministic idempotency (`post-commit:<sha>`) and deterministic UUIDv4 trace IDs from commit/timestamp seed
- Production adapters currently bound in runtime:
  - notes adapter: `GitNotesTraceWriter` writes canonical JSON note payloads to `refs/notes/agent-trace`
  - local record store adapter: `NoOpTraceRecordStore` disconnects local DB trace persistence while keeping the finalize path compile-safe
  - emission ledger adapter: `FileTraceEmissionLedger` stores emitted commit SHAs at `sce/trace-emission-ledger.txt`
  - retry queue adapter: `JsonFileTraceRetryQueue` appends failed-target fallback entries to `sce/trace-retry-queue.jsonl`
- If later runtime work reactivates this path, `resolve_post_commit_runtime_paths` still points at the empty-file/open-only Agent Trace DB baseline rather than any schema-backed trace store.
- Runtime posture remains fail-open: operational errors return deterministic skip/fallback messages instead of aborting commit progression.

## Verification evidence
- `nix flake check`
