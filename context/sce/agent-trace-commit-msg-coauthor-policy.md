# Agent Trace commit-msg co-author policy

## Status
- Plan: `agent-trace-attribution-no-git-wrapper`
- Task: `T05`
- Implementation state: done
- Runtime hook wiring: `agent-trace-local-hooks-production-mvp` `T04` (done)

## Canonical contract
- Policy entrypoint: `cli/src/services/hooks.rs` -> `apply_commit_msg_coauthor_policy`.
- Runtime entrypoint: `cli/src/services/hooks.rs` -> `run_commit_msg_subcommand` / `run_commit_msg_subcommand_in_repo`.
- Canonical trailer string: `Co-authored-by: SCE <sce@crocoder.dev>`.
- Runtime gating conditions:
  - `sce_disabled = false`
  - `sce_coauthor_enabled = true`
  - `has_staged_sce_attribution = true`
- Runtime gate source mapping:
  - `sce_disabled` resolves from `SCE_DISABLED` truthy evaluation.
  - `sce_coauthor_enabled` resolves from `SCE_COAUTHOR_ENABLED` with enabled-by-default semantics.
  - `has_staged_sce_attribution` resolves from staged pre-commit checkpoint artifact content only when at least one file has both non-empty `ranges[]` and `has_sce_attribution = true`.
- When all gate conditions pass, output commit message MUST contain exactly one canonical SCE trailer.
- When any gate condition fails, commit message is returned unchanged.

## Behavior details
- Hook runtime reads commit message file content as UTF-8 and returns deterministic actionable errors for missing/non-file/non-UTF-8 paths.
- Canonical trailer dedupe removes duplicate canonical lines before final insertion.
- Trailer insertion is idempotent: applying the policy repeatedly yields the same message.
- Existing trailing newline is preserved when present.
- Commit-msg runtime writes the file only when policy gates pass and transformed content differs from original content.
- Human author/committer identity is not rewritten; only commit message trailer content is affected.
- Missing or `false` `has_sce_attribution` markers fail the gate even when staged ranges are present, so generic human-only staged diffs do not trigger trailer insertion.
  - **TEMPORARY (v0.1.x)**: Currently defaults to `true` for all staged files (see TODO(0.3.0) in `cli/src/services/hooks.rs:collect_pending_checkpoint`).
  - **PLANNED (v0.3.0)**: Will default to `false` and require explicit attribution marking.
- The positive path remains explicit-marker driven: commit-msg appends the canonical trailer when an attribution-aware checkpoint producer marks staged ranges as SCE-attributed.

## Verification evidence
- `nix flake check`
