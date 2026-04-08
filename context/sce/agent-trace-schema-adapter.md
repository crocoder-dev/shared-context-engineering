# Agent Trace Schema Adapter (Historical Reference)

## Current status

- This document is retained as historical reference only.
- `cli/src/services/agent_trace.rs` is not part of the active runtime surface.
- The current local-hook baseline does not build Agent Trace payloads.

## Historical scope

- Plan/task: `agent-trace-attribution-no-git-wrapper` / `T02`
- Purpose at the time: define a deterministic adapter contract that mapped internal attribution inputs to Agent Trace record shape without persistence or hook side effects.

## Current guidance

- Do not use this file as current implementation guidance.
- For current hook behavior, use `context/sce/agent-trace-hooks-command-routing.md`.
