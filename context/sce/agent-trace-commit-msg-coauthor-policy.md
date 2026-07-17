# Agent Trace commit-msg co-author policy

## Status
- Plan: `commit-msg-coauthor-gated-by-ai-trace`
- Task: `T07`
- Implementation state: done
- Runtime hook wiring: `agent-trace-local-hooks-production-mvp` `T04` (done)

## Canonical contract
- Policy entrypoint: `cli/src/services/hooks/mod.rs` -> `apply_commit_msg_coauthor_policy`.
- Runtime entrypoint: `cli/src/services/hooks/mod.rs` -> `run_commit_msg_subcommand` / `run_commit_msg_subcommand_in_repo`.
- Canonical trailer string: `Co-authored-by: SCE <sce@crocoder.dev>`.
- Runtime gating conditions (all must pass for trailer insertion):
  - `attribution_hooks_enabled = true` (opt-out default; resolved from `SCE_ATTRIBUTION_HOOKS_DISABLED` env over `policies.attribution_hooks.enabled` config, default `true`)
  - `sce_disabled = false` (resolved from `SCE_DISABLED` truthy evaluation)
  - `ai_contribution_present = true` (resolved from staged-diff AI-overlap preflight)
- Runtime gate source mapping:
  - `attribution_hooks_enabled` resolves from opt-out env `SCE_ATTRIBUTION_HOOKS_DISABLED` over config key `policies.attribution_hooks.enabled`, default `true`; the env value is inverted on read, so truthy disables attribution.
  - `sce_disabled` resolves from `SCE_DISABLED` truthy evaluation.
  - `ai_contribution_present` resolves from `staged_diff_has_ai_overlap(repository_root, logger)`, which returns `StagedDiffAiOverlapResult::Overlap` when the staged diff overlaps with at least one recent AI/editor diff trace, `NoOverlap` when no overlap is found, or `Error` when any preflight error occurs. Both `NoOverlap` and `Error` map to `ai_contribution_present = false`.
- When all gate conditions pass, output commit message MUST contain exactly one canonical SCE trailer.
- When any gate condition fails, commit message is returned unchanged.

## Behavior details
- Hook runtime reads commit message file content as UTF-8 and returns deterministic actionable errors for missing/non-file/non-UTF-8 paths.
- Canonical trailer dedupe removes duplicate canonical lines before final insertion.
- Trailer insertion is idempotent: applying the policy repeatedly yields the same message.
- Existing trailing newline is preserved when present.
- Commit-msg runtime writes the file only when policy gates pass and transformed content differs from original content.
- Human author/committer identity is not rewritten; only commit message trailer content is affected.
- The preflight is invoked only when the policy gate passes (`attribution_hooks_enabled && !sce_disabled`); when the gate does not pass, no DB read or staged-diff capture occurs.
- Errors during the preflight (DB open failure, schema not ready, query error, staged diff read failure, malformed/no rows, zero overlap) are collapsed to `ai_contribution_present = false` and the trailer is never appended. Errors are logged via `sce.hooks.commit_msg.ai_overlap_error` for diagnostics but never escalate to applying the trailer.

## Staged AI-overlap evidence gate

- `cli/src/services/agent_trace.rs` owns the pure patch-overlap helper (`patches_have_overlap`) for Agent Trace evidence checks; fixture-backed unit coverage in `cli/src/services/agent_trace/tests.rs` covers overlap, no-overlap, empty/untouched patches, and Claude structured-patch-derived input.
- `cli/src/services/hooks/mod.rs` owns the staged-diff AI-overlap evidence gate, which is now wired into `run_commit_msg_subcommand_in_repo`:
  - `StagedDiffAiOverlapResult` enum (`Overlap`/`NoOverlap`/`Error`) is the three-valued result from the injectable `_with` variant, enabling testable branch coverage and caller-side error logging.
  - `staged_diff_has_ai_overlap_with` is the injectable variant that accepts staged-patch/time/recent-trace dependencies and returns `StagedDiffAiOverlapResult`; available for future test coverage.
  - `staged_diff_has_ai_overlap` is the live wrapper that opens Agent Trace DB through the no-migration hook path, delegates to `_with`, and logs `sce.hooks.commit_msg.ai_overlap_error` on `Error` results.
- Live helper path:
  - opens Agent Trace DB through `RepositoryAgentTraceDb::open_for_hooks_without_migrations_at()` and `ensure_schema_ready_for_hooks()`;
  - captures the staged patch with `git diff --cached --patch --no-ext-diff`;
  - queries recent diff traces using the same bounded 7-day window as post-commit;
  - combines each recent patch and checks overlap through `agent_trace::patches_have_overlap`, which uses the existing patch intersection primitive;
  - short-circuits on the first positive overlap.
- No-evidence/error posture: DB open/readiness failure, staged-diff capture/parse failure, clock/query failure, empty staged diff, no recent rows, malformed-only rows, or zero overlap all return `StagedDiffAiOverlapResult::Error` or `NoOverlap`, both mapping to `ai_contribution_present = false`. There is no fail-open mode.

## Verification evidence
- `nix flake check`
