# Agent Trace commit-msg co-author policy

## Status
- Plan: `agent-trace-attribution-no-git-wrapper`
- Task: `T05`
- Implementation state: done
- Runtime hook wiring: `agent-trace-local-hooks-production-mvp` `T04` (done)

## Canonical contract
- Policy entrypoint: `cli/src/services/hooks/mod.rs` -> `apply_commit_msg_coauthor_policy`.
- Runtime entrypoint: `cli/src/services/hooks/mod.rs` -> `run_commit_msg_subcommand` / `run_commit_msg_subcommand_in_repo`.
- Canonical trailer string: `Co-authored-by: SCE <sce@crocoder.dev>`.
- Runtime gating conditions:
  - `attribution_hooks_enabled = true`
  - `sce_disabled = false`
- Runtime gate source mapping:
  - `attribution_hooks_enabled` resolves from opt-out env `SCE_ATTRIBUTION_HOOKS_DISABLED` over config key `policies.attribution_hooks.enabled`, default `true`; the env value is inverted on read, so truthy disables attribution.
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

## Staged AI-overlap helper seam

- `cli/src/services/agent_trace.rs` owns the pure patch-overlap helper (`patches_have_overlap`) for Agent Trace evidence checks; this is the seam intended for future golden fixture coverage.
- `cli/src/services/hooks/mod.rs` includes a hooks-owned, bool-shaped staged-diff overlap helper for a later commit-msg gate wiring task and delegates pure overlap classification to `agent_trace.rs`.
- The helper is intentionally not invoked by `run_commit_msg_subcommand_in_repo` yet, so runtime commit-msg behavior is unchanged until the wiring task lands.
- Live helper path:
  - opens Agent Trace DB through `AgentTraceDb::open_for_hooks_without_migrations()` and `ensure_schema_ready_for_hooks()`;
  - captures the staged patch with `git diff --cached --patch --no-ext-diff`;
  - queries recent diff traces using the same bounded 7-day window as post-commit;
  - combines each recent patch and checks overlap through `agent_trace::patches_have_overlap`, which uses the existing patch intersection primitive;
  - short-circuits on the first positive overlap.
- No-evidence/error posture: DB open/readiness failure, staged-diff capture/parse failure, clock/query failure, empty staged diff, no recent rows, malformed-only rows, or zero overlap all return `false`.

## Verification evidence
- `nix flake check`
