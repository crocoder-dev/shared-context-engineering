# Agent Trace Post-Rewrite Local Remap Ingestion (Historical Reference)

## Current status

- This contract is no longer active in runtime.
- The current `cli/src/services/hooks/mod.rs` keeps `sce hooks post-rewrite` as a deterministic no-op.

## Current baseline

- Runtime entrypoint: `run_post_rewrite_subcommand`
- Current behavior: returns deterministic no-op status text and includes the provided rewrite-method argument in that status text
- No stdin rewrite-pair parsing runs
- No local remap ingestion runs
- Enabling the attribution-hooks gate does not change `post-rewrite`; it remains a no-op

## Historical note

- Plan: `agent-trace-attribution-no-git-wrapper`
- Task: `T08`
- The remap-ingestion contract described by the original task is not active in the current runtime.
