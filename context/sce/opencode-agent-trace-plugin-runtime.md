# OpenCode agent-trace plugin runtime

Current runtime source: `config/lib/agent-trace-plugin/opencode-sce-agent-trace-plugin.ts`.

## Event capture baseline

- The plugin captures `message.updated` events for conversation-trace handoff before any diff-trace extraction.
- The plugin also captures `message.part.updated` events for conversation-trace handoff; part events do not invoke diff-trace.
- For every captured `message.updated` event, the plugin builds a normalized snake_case `message.updated` envelope and invokes `sce hooks conversation-trace` over STDIN JSON; the Rust hook owns value validation and AgentTraceDb persistence.
- For every captured `message.part.updated` event, the plugin builds a normalized snake_case `message.part.updated` envelope and invokes `sce hooks conversation-trace` over STDIN JSON with the same subprocess behavior.
- Existing diff-trace capture remains filtered to user messages with usable diffs.
- When diff extraction succeeds, the plugin invokes `sce hooks diff-trace` after conversation-trace handoff and sends `{ sessionID, diff, time, model_id, tool_name, tool_version }` over STDIN JSON (`tool_name` is always `"opencode"`; `tool_version` is captured from session lifecycle events when available).
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

- `recordConversationTrace(repoRoot, event)` builds and sends the conversation-trace payload for both `message.updated` and `message.part.updated` variants; for `message.updated`, it still runs before `buildTrace`.
- The `message.updated` conversation-trace payload maps OpenCode event fields mechanically to `type`, `session_id`, `message_id`, `role`, and `generated_at_unix_ms`; it does not emit message-level `agent` or `summary_diffs` fields and does not duplicate Rust hook validation.
- `buildMessagePartConversationTracePayload(event)` maps `event.properties.part.sessionID`, `messageID`, `type`, and `text` into `session_id`, `message_id`, `part_type`, and `text`, and uses `Date.now()` for `generated_at_unix_ms`.
- The diff extraction seam is internal to the source module and is used by `buildTrace` at runtime.
- `buildTrace` exits early when extraction returns `undefined` (non-user role, empty diffs array, or no usable patch entries), so no diff-trace hook invocation occurs for those events.
- The plugin tracks OpenCode client version per session ID from `session.created` / `session.updated` events and forwards it as `tool_version` when available.
- When extraction succeeds, `buildTrace` forwards the extracted payload with required `tool_name="opencode"` and required `tool_version` (nullable when session version is unavailable) to `sce hooks diff-trace` via STDIN JSON; the Rust hook runtime validates required non-empty `sessionID`/`diff`/`model_id`/`tool_name`, required nullable/non-empty `tool_version`, plus required `time`, and persists the DB-backed diff-trace fields through AgentTraceDb `diff_traces` insertion.
