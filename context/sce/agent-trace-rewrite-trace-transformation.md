# Agent Trace Rewrite Trace Transformation (T09)

## Status
- Plan: `agent-trace-attribution-no-git-wrapper`
- Task: `T09`
- Scope: rewrite trace transformation semantics for rewritten SHAs

## Implemented surface
- Code: `cli/src/services/hooks.rs`
- Primary entrypoint: `finalize_rewrite_trace`
- Purpose: materialize rewritten-commit Agent Trace records with explicit rewrite metadata and deterministic quality classification.

## Runtime gating and idempotency

`finalize_rewrite_trace` returns `NoOp` without persistence when any guard applies:

- `sce_disabled = true`
- `cli_available = false`
- `is_bare_repo = true`
- rewritten commit SHA is already marked emitted in `TraceEmissionLedger`

## Rewrite record transformation contract

- Rewritten traces are emitted through the canonical builder path (`build_trace_payload`) to preserve Agent Trace-required structure.
- The rewritten commit identity maps to `vcs.revision = <rewritten_commit_sha>`.
- Rewrite lineage metadata is always attached via reserved keys:
  - `dev.crocoder.sce.rewrite_from`
  - `dev.crocoder.sce.rewrite_method`
  - `dev.crocoder.sce.rewrite_confidence`
- The method value uses canonical labels from `RewriteMethod` (`amend`, `rebase`, lowercase passthrough for `Other`).

## Confidence and quality logic

- Confidence input must be finite and inside `[0.0, 1.0]`; otherwise finalization errors before writes.
- Confidence is normalized to a fixed two-decimal metadata string (`0.00`..`1.00`).
- Quality status mapping:
  - `>= 0.90` -> `final`
  - `0.60..0.89` -> `partial`
  - `< 0.60` -> `needs_review`

## Persistence semantics

- Rewritten trace finalization follows the same current notes-plus-no-op-DB baseline as post-commit traces.
- On success:
  - commit SHA is marked emitted in `TraceEmissionLedger`
  - outcome is `RewriteTraceFinalization::Persisted`
- On any target failure:
  - failed targets are captured in a retry queue entry
  - outcome is `RewriteTraceFinalization::QueuedFallback`

## Verification evidence

- `nix flake check`

## Tests added

- Metadata integrity and current notes/no-op-DB persistence behavior for rewritten traces.
- Confidence-threshold quality mapping (`final`, `partial`, `needs_review`).
- Confidence range validation errors for out-of-range input.
- No-op behavior when rewritten commit was already finalized.
