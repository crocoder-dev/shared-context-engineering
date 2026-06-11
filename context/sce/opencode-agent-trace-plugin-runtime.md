# OpenCode agent-trace plugin runtime

Current TypeScript runtime source:

- `config/lib/agent-trace-plugin/opencode-sce-agent-trace-plugin.ts`

The Claude TypeScript agent-trace runtime was removed in T07 of the `claude-rust-diff-trace` plan. Claude now routes through generated `.claude/settings.json` command hooks that call `sce hooks` directly with raw hook event JSON on STDIN; Rust handles extraction, validation, and persistence without a TypeScript intermediary.

## Event capture baseline

- The plugin registers for `message.updated`, `message.part.updated`, `session.created`, and `session.updated` events.
- For every captured `message.updated` event, the plugin checks for `summary.diffs` via `buildPatchConversationTracePayloads`:
  - **When diffs exist**: builds a `-patch` conversation trace payload set (one `message.updated` with `message_id = "${id}-patch"` + per-diff `message.part.updated` payloads with `part_type: "patch"`) and dispatches all payloads concurrently via `Promise.all` to `sce hooks conversation-trace`. The original `message.updated` event is replaced — no original `message.updated` payload is sent.
  - **When no diffs exist**: builds a single `message.updated` typed batch envelope `{ type: "message.updated", payloads: [item] }` via `buildConversationTracePayload` and invokes `sce hooks conversation-trace` over STDIN JSON (original behavior preserved).
- For every captured `message.part.updated` event, the plugin builds a typed batch envelope `{ type: "message.part.updated", payloads: [item] }` via `buildMessagePartConversationTracePayload` and invokes `sce hooks conversation-trace` over STDIN JSON; only `text` and `reasoning` part types with non-empty `text` are dispatched.
- Existing diff-trace capture remains filtered to user messages with usable diffs.
- When diff extraction succeeds, the plugin invokes `sce hooks diff-trace` after conversation-trace handoff and sends `{ sessionID, diff, time, model_id, tool_name, tool_version }` over STDIN JSON (`tool_name` is always `"opencode"`; `tool_version` is captured from session lifecycle events when available).
- The plugin no longer writes diff-trace artifacts or database rows directly; the Rust `diff-trace` hook path owns AgentTraceDb insertion plus collision-safe timestamp+attempt artifact writes.

## In-memory dedup cache

The plugin maintains a `Set<string>` (`processedDiffsMessageIds`) in the `SceAgentTracePlugin` closure, keyed by `"${sessionID}:${messageID}"`. Only `message.updated` events that carry `summary.diffs` are checked against and added to the set. An event without diffs does not interact with the set at all — it is processed normally and does not block subsequent events for the same `(sessionID, messageID)` pair.

This prevents duplicate processing of diff-bearing `message.updated` events while allowing a non-diff event (e.g., initial `message.updated` without `summary.diffs`) to be followed by a later diff-bearing event for the same message. The set lives for the lifetime of the plugin instance and is not time-bounded — once a diff-bearing `(sessionID, messageID)` pair is processed, it is never re-processed.

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

## Patch conversation trace

The `buildPatchConversationTracePayloads(event)` helper processes `message.updated` events that carry `summary.diffs`. When the event has diff items with usable `patch` fields, it returns an array of `ConversationTracePayload` envelopes; when no diff items have usable patches, it returns `undefined`.

### Payload shape when diffs exist

1. One `message.updated` payload with:
   - `session_id` from `event.properties.info.sessionID`
   - `role` from `event.properties.info.role`
   - `message_id` = `${event.properties.info.id}-patch`
   - `generated_at_unix_ms = Date.now()`
2. For each diff item with a non-empty `patch` field, a `message.part.updated` payload with:
   - Same `session_id` as the parent `message.updated` payload
   - `message_id = "${id}-patch"` (same as the parent)
   - `part_type: "patch"`
   - `text: entryObj.patch`
   - `generated_at_unix_ms = Date.now()`

### Concurrent dispatch

`recordConversationTrace` dispatches all payloads (the parent `message.updated` and all per-diff `message.part.updated` payloads) **concurrently** using `await Promise.all(...)`.

### No-diff fallback

When `buildPatchConversationTracePayloads` returns `undefined` (no diff items have usable patches), `recordConversationTrace` falls back to sending the original `message.updated` payload unchanged via `buildConversationTracePayload`.

## Current usage boundary

- `recordConversationTrace(repoRoot, event)` branches on event type:
  - For `message.updated` events: calls `buildPatchConversationTracePayloads` first.
    - If patch payloads are returned (diffs exist), dispatches them concurrently via `Promise.all` — the original `message.updated` payload is not sent.
    - If `undefined` (no diffs), sends the original `message.updated` payload unchanged via `buildConversationTracePayload`.
  - For `message.part.updated` events (only `text` and `reasoning` with non-empty `text`): uses `buildMessagePartConversationTracePayload` (unchanged).
- The `message.updated` conversation-trace batch (no-diff fallback) maps OpenCode event fields mechanically into a `payloads[0]` item with `session_id`, `message_id`, `role`, and `generated_at_unix_ms`; it does not emit message-level `agent` or `summary_diffs` fields and does not duplicate Rust hook validation.
- `buildMessagePartConversationTracePayload(event)` maps `event.properties.part.sessionID`, `messageID`, `type`, and `text` into a `payloads[0]` item with `session_id`, `message_id`, `part_type`, and `text`, and uses `Date.now()` for `generated_at_unix_ms`.
- The diff extraction seam is internal to the source module and is used by `buildTrace` at runtime.
- `buildTrace` exits early when extraction returns `undefined` (non-user role, empty diffs array, or no usable patch entries), so no diff-trace hook invocation occurs for those events.
- The plugin tracks OpenCode client version per session ID from `session.created` / `session.updated` events and forwards it as `tool_version` when available.
- When extraction succeeds, `buildTrace` forwards the extracted payload with required `tool_name="opencode"` and required `tool_version` (nullable when session version is unavailable) to `sce hooks diff-trace` via STDIN JSON; the Rust hook runtime validates required non-empty `sessionID`/`diff`/`tool_name`, optional `model_id`, required nullable/non-empty `tool_version`, plus required `time`, resolves missing/nullable attribution fields from `session_models` when available while preserving direct payload precedence, and persists DB-backed diff-trace fields through AgentTraceDb `diff_traces` insertion.

## Shared boundary with Claude runtime

- OpenCode uses a generated TypeScript event runtime as an event-shape adapter before handing normalized diff-trace payloads to the shared Rust hook intake.
- Claude registration uses generated `.claude/settings.json` command hooks that call `sce hooks` directly (no TypeScript runtime intermediary): `SessionStart` pipes the raw Claude hook event JSON to `sce hooks session-model`, and matched `PostToolUse Write|Edit|MultiEdit|NotebookEdit` pipes the raw hook event to `sce hooks diff-trace`.
- Rust `diff-trace` intake detects Claude payloads via `hook_event_name` and derives structured patches from the raw JSON with `payload_type="structured"`; OpenCode normalized payloads (no `hook_event_name`) are stored as `payload_type="patch"`.
- Rust `session-model` intake detects Claude `SessionStart` payloads via `hook_event_name`, extracts `session_id`/`model_id`/`time` from the raw Claude event format, uses explicit payload version fields (`tool_version`/`claude_version`/`version`) when present, and otherwise best-effort fills `tool_version` from trimmed `claude --version` stdout when available.
- The shared Rust boundary is `sce hooks diff-trace` and `sce hooks session-model`: Rust remains the only writer of parsed `context/tmp/*-diff-trace.json` artifacts and AgentTraceDb `diff_traces`/`session_models` rows.
- Claude attribution differs from OpenCode attribution: OpenCode reads provider/model data from the OpenCode event and includes `model_id` in the payload; for Claude `diff-trace`, Rust resolves missing `model_id` and `tool_version` from AgentTraceDb `session_models` at persistence time when available, otherwise persisting nullable attribution; for Claude `session-model`, Rust extracts `model_id` from the raw hook event, normalizes it with a `claude/` prefix, and stores nullable `tool_version` for later diff-trace fallback, including best-effort `claude --version` capture when payload version metadata is absent.

## Claude derivation golden tests

- Rust golden tests in `cli/src/services/structured_patch/tests.rs` (`claude_derivation_golden_tests`) own the Claude derivation fixture coverage.
- The test dynamically discovers the checked-in `cli/src/services/structured_patch/fixtures/` scenario directories, validates the expected eight-scenario set, then loads each `claude-post-tool-use.json` plus `expected.patch` pair.
- Each scenario calls `derive_claude_structured_patch(...)` with fixed time/tool-version inputs and asserts derived status, session ID, time, `tool_name="claude"`, tool version, exact golden diff parsed via `parse_patch`, and no emitted `model_id`.
- The former TypeScript `deriveClaudeDiffTracePayload(...)` seam and its Bun test were removed in T07 when the Claude TypeScript plugin source was deleted.
