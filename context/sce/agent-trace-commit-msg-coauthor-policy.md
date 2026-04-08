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
  - `attribution_hooks_enabled = true`
  - `sce_disabled = false`
- Runtime gate source mapping:
  - `attribution_hooks_enabled` resolves from env `SCE_ATTRIBUTION_HOOKS_ENABLED` over config key `policies.attribution_hooks.enabled`, default `false`.
  - `sce_disabled` resolves from `SCE_DISABLED` truthy evaluation.
- When all gate conditions pass, output commit message MUST contain exactly one canonical SCE trailer.
- When any gate condition fails, commit message is returned unchanged.

## Behavior details
- Hook runtime reads commit message file content as UTF-8 and returns deterministic actionable errors for missing/non-file/non-UTF-8 paths.
- Canonical trailer dedupe removes duplicate canonical lines before final insertion.
- Trailer insertion is idempotent: applying the policy repeatedly yields the same message.
- Existing trailing newline is preserved when present.
- Commit-msg runtime writes the file only when policy gates pass and transformed content differs from original content.
- Human author/committer identity is not rewritten; only commit message trailer content is affected.
- The current positive path is gate-driven only: when attribution hooks are enabled, `commit-msg` appends the canonical trailer without depending on checkpoint files or other helper state.

## Verification evidence
- `nix flake check`
