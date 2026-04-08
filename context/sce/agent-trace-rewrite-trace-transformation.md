# Agent Trace Rewrite Trace Transformation (Historical Reference)

## Current status

- This contract is no longer active in runtime.
- The current `cli/src/services/hooks.rs` keeps `sce hooks post-rewrite` as a deterministic no-op.

## Current baseline

- `sce hooks post-rewrite` does not transform rewritten commits into Agent Trace records
- No rewrite metadata is emitted
- No persistence or retry behavior is attached to `post-rewrite`

## Historical note

- Plan: `agent-trace-attribution-no-git-wrapper`
- Task: `T09`
- The rewrite-transformation contract described by the original task is retained only as historical reference.
