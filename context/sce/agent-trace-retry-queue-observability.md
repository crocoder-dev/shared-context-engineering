# Agent Trace retry queue and observability metrics (Historical Reference)

## Current status

- This contract is no longer active in local hook runtime.
- The current `cli/src/services/hooks.rs` no longer runs retry replay from hook entrypoints.

## Current baseline

- The local hook runtime does not replay Agent Trace retries
- `post-commit` and `post-rewrite` no-op before any retry processing
- No local DB retry schema is part of the active runtime baseline
- No local-hook retry metrics are emitted

## Historical note

- Plan: `agent-trace-attribution-no-git-wrapper`
- Task: `T14`
- This file is retained only as historical reference for the removed retry/metrics design slice.
