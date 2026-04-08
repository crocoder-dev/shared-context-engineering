# Agent Trace post-commit persistence baseline

## Current status

- This contract is no longer active in runtime.
- The current `cli/src/services/hooks.rs` keeps `sce hooks post-commit` as a deterministic no-op.

## Current baseline

- Runtime entrypoint: `cli/src/services/hooks.rs` -> `run_post_commit_subcommand`
- Current behavior: `sce hooks post-commit` returns deterministic no-op status text
- No Agent Trace payload is built
- No git-notes write runs
- No local DB trace write runs
- No retry queue entry is produced
- Enabling the attribution-hooks gate does not change `post-commit`; the active gated behavior remains `commit-msg` only

## Historical note

- Plan: `agent-trace-attribution-no-git-wrapper`
- Task: `T06`
- This file retains the removed dual-write task slice for reference only.
