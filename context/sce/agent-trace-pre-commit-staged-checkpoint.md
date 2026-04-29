# Agent Trace Pre-commit Staged Checkpoint

## Current status

This contract is no longer active. The current `cli/src/services/hooks/mod.rs` runtime keeps `sce hooks pre-commit` as a deterministic no-op and does not persist checkpoint artifacts.

## Current baseline

- Code location: `cli/src/services/hooks/mod.rs`
- Runtime entrypoint: `run_pre_commit_subcommand`
- Current behavior: returns deterministic no-op status text
- No checkpoint artifact is written
- No staged/unstaged diff collection runs
- No index/head tree anchors are captured

## Historical note

Task `agent-trace-attribution-no-git-wrapper` `T04` originally defined a staged-checkpoint contract for later commit binding. That contract is not active in the current runtime.
