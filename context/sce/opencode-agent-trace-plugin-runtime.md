# OpenCode agent-trace plugin runtime

Current runtime source: `config/lib/agent-trace-plugin/opencode-sce-agent-trace-plugin.ts`.

## Event capture baseline

- The plugin captures `message.updated` events, filtered to user messages with diffs.
- When diff extraction succeeds, the plugin invokes `sce hooks diff-trace` and sends `{ sessionID, diff, time, model_id, tool_name, tool_version }` over STDIN JSON (`tool_name` is always `"opencode"`; `tool_version` is captured from session lifecycle events when available).
- The plugin no longer writes diff-trace artifacts or database rows directly; the Rust `diff-trace` hook path owns AgentTraceDb insertion plus collision-safe timestamp+attempt artifact writes.
- `session.diff` event capture has been removed.

## Diff extraction seam

The plugin defines `extractDiffTracePayload(event)` as a typed guard/extraction seam for diff-bearing `message.updated` user-message events.

### Extraction contract

Returns `{ sessionID, diff, time, model_id }` only when all checks pass:

1. `event.type === "message.updated"`
2. `event.properties.info.role === "user"` (assistant, system, and other roles are skipped)
3. `event.properties.info.summary?.diffs` exists and is non-empty
4. Each entry contributes only its `patch` field when present; entries without `patch` are skipped
5. If all entries are skipped (no usable patches), the helper returns `undefined`
6. `sessionID` is sourced directly from `event.properties.info.sessionID`
7. `time` is sourced from `Date.now()` (Unix epoch milliseconds at extraction time)
8. `model_id` is built directly as `providerID/modelID` from `event.properties.info.model.providerID` and `event.properties.info.model.modelID`

Otherwise, the helper returns `undefined`.

## Current usage boundary

- The extraction seam is internal to the source module and is used by `buildTrace` at runtime.
- `buildTrace` is now called only for captured event types and exits early unless the event is `message.updated`; if extraction returns `undefined` (non-user role, empty diffs array, or no usable patch entries), no hook invocation occurs.
- The plugin tracks OpenCode client version per session ID from `session.created` / `session.updated` events and forwards it as `tool_version` when available.
- When extraction succeeds, `buildTrace` forwards the extracted payload with `tool_name="opencode"` and optional `tool_version` to `sce hooks diff-trace` via STDIN JSON; the Rust hook runtime continues to validate required `sessionID`/`diff`/`model_id` plus `time` and persists those required fields through AgentTraceDb `diff_traces` insertion.
