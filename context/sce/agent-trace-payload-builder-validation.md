# Agent Trace Payload Builder And Validation (Historical Reference)

## Current status

- This document is retained as historical reference only.
- The design below is superseded. The current `post-commit` hook builds, validates, and persists Agent Trace payloads through `cli/src/services/agent_trace.rs`; see `agent-trace-hooks-command-routing.md` and `agent-trace-minimal-generator.md`.

## Historical scope

- Plan/task: `agent-trace-attribution-no-git-wrapper` / `T03`
- Purpose at the time: define a deterministic payload-builder path and schema-validation coverage for Agent Trace records.

## Current guidance

- Do not use this file as current runtime guidance.
- For current local-hook behavior, use `context/sce/agent-trace-hooks-command-routing.md`.
