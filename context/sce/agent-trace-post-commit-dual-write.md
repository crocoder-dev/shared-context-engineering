# Agent Trace post-commit persistence baseline

## Current status

- This contract is no longer active in runtime.
- The current `cli/src/services/hooks/mod.rs` has replaced the earlier no-op with an active intersection entrypoint (see `agent-trace-hooks-command-routing.md`).

## Historical baseline (pre-T04)

- Runtime entrypoint: `cli/src/services/hooks/mod.rs` -> `run_post_commit_subcommand`
- Pre-T04 behavior: `sce hooks post-commit` was a deterministic no-op
- No Agent Trace payload was built
- No git-notes write ran
- No local DB trace write ran
- No retry queue entry was produced
- Enabling the attribution-hooks gate did not change `post-commit`; the active gated behavior was `commit-msg` only

## Historical note

- Plan: `agent-trace-attribution-no-git-wrapper`
- Task: `T06`
- This file retains the removed dual-write task slice for reference only.
