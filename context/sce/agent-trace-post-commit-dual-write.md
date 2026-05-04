# Agent Trace post-commit persistence baseline

## Current status

- This contract is no longer active in runtime.
- The current `cli/src/services/hooks/mod.rs` no longer runs the retired dual-write/full-Agent-Trace behavior. `sce hooks post-commit` now only attempts latest-session `ParsedPatch` intersection persistence into AgentTraceDb when usable diff-trace rows exist.

## Current baseline

- Runtime entrypoint: `cli/src/services/hooks/mod.rs` -> `run_post_commit_subcommand`
- Current behavior: `sce hooks post-commit` preserves the `SCE_DISABLED` no-op; otherwise it selects the latest `diff_traces` session, captures `HEAD` SHA/patch, builds compact `ParsedPatch` intersection JSON, and inserts one `patch_intersections` row when source data exists
- No Agent Trace payload is built
- No git-notes write runs
- No local DB trace write runs
- No retry queue entry is produced
- Enabling the attribution-hooks gate does not control `post-commit`; the attribution gate remains specific to `commit-msg` trailer insertion

## Historical note

- Plan: `agent-trace-attribution-no-git-wrapper`
- Task: `T06`
- This file retains the removed dual-write task slice for reference only.
