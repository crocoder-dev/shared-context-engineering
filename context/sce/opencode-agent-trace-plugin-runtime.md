# OpenCode agent-trace plugin runtime

Current TypeScript runtime source:

- `config/lib/agent-trace-plugin/opencode-sce-agent-trace-plugin.ts`

The Claude TypeScript agent-trace runtime was removed in T07 of the `claude-rust-diff-trace` plan. Claude now routes through generated `.claude/settings.json` command hooks that call `.claude/hooks/run-sce-or-show-install-guidance.sh` before invoking `sce hooks` with raw hook event JSON on STDIN; Rust handles extraction, validation, and persistence without a TypeScript intermediary.

## Event capture baseline

- The plugin registers for `message`, `message.part`, `session.created`, and `session.updated` events.
- Conversation-trace handoff uses the current mixed-batch STDIN shape expected by Rust: `{ "payloads": [{ "type": "message" | "message.part", ... }] }`. The producer does not emit top-level `type` envelopes.
- For every captured `message` event, the plugin checks for `summary.diffs` via `buildPatchConversationTracePayload`:
  - **When diffs exist**: builds one mixed `-patch` conversation-trace envelope containing the synthetic parent `message` item with `message_id = "${id}-patch"` plus all per-diff `message.part` patch items, then invokes `sce hooks conversation-trace` once. The original `message` event is replaced — no original `message` payload is sent.
  - **When no diffs exist**: builds one mixed envelope containing a single `message` item via `buildConversationTracePayload` and invokes `sce hooks conversation-trace` over STDIN JSON.
- For captured `message.part` events, the plugin dispatches only supported part shapes to `sce hooks conversation-trace`: ordinary `text` and `reasoning` parts with non-empty `text`, plus completed OpenCode `question` tool parts emitted as first-class `part_type: "question"` payloads.
- Both `runConversationTraceHook` and `runDiffTraceHook` fail open at the plugin level: they ignore the child process stderr (`stdio: ["pipe", "ignore", "ignore"]`) and resolve unconditionally on error or close, so spawn errors, non-zero exits, and sce intake errors (connection refused, timeout, etc.) never produce unhandled promise rejections or leak sce stderr into the OpenCode TUI.
- Existing diff-trace capture remains filtered to user messages with usable diffs.
- When diff extraction succeeds, the plugin invokes `sce hooks diff-trace` after conversation-trace handoff and sends `{ sessionID, diff, time, model_id, tool_name, tool_version }` over STDIN JSON (`tool_name` is always `"opencode"`; `tool_version` is captured from session lifecycle events when available). `runDiffTraceHook` fails open at the plugin level (ignored stderr, unconditional resolve), so callers do not need try/catch.
- The plugin no longer writes diff-trace artifacts or database rows directly; the Rust `diff-trace` hook path owns AgentTraceDb insertion plus collision-safe timestamp+attempt artifact writes.

## In-memory dedup cache

The plugin maintains a `Set<string>` (`processedDiffsMessageIds`) in the `SceAgentTracePlugin` closure, keyed by `"${sessionID}:${messageID}"`. Only `message` events that carry `summary.diffs` are checked against and added to the set. An event without diffs does not interact with the set at all — it is processed normally and does not block subsequent events for the same `(sessionID, messageID)` pair.

This prevents duplicate processing of diff-bearing `message` events while allowing a non-diff event (e.g., initial `message` without `summary.diffs`) to be followed by a later diff-bearing event for the same message. The set lives for the lifetime of the plugin instance and is not time-bounded — once a diff-bearing `(sessionID, messageID)` pair is processed, it is never re-processed.

## Diff extraction seam

The plugin defines `extractDiffTracePayload(event)` as a typed guard/extraction seam for diff-bearing `message` user-message events.

### Extraction contract

Returns `{ sessionID, diff, time, model_id }` only when all checks pass:

1. `event.type === "message"`
2. `event.properties.info.role === "user"` (assistant, system, and other roles are skipped)
3. `event.properties.info.summary?.diffs` exists and is non-empty
4. Each entry contributes only its `patch` field when present; entries without `patch` are skipped
5. If all entries are skipped (no usable patches), the helper returns `undefined`
6. `sessionID` is sourced directly from `event.properties.info.sessionID`
7. `time` is sourced from `Date.now()` (Unix epoch milliseconds at extraction time)
8. `model_id` is built directly as `providerID/modelID` from `event.properties.info.model.providerID` and `event.properties.info.model.modelID`

Otherwise, the helper returns `undefined`.

## Patch conversation trace

The `buildPatchConversationTracePayload(event)` helper processes `message` events that carry `summary.diffs`. When the event has diff items, it returns one mixed `ConversationTracePayload` envelope containing the synthetic parent item plus any usable patch part items; when no diff entries are present, it returns `undefined`.

### Payload shape when diffs exist

1. One `message` item with:
   - `type: "message"`
   - `session_id` from `event.properties.info.sessionID`
   - `role` from `event.properties.info.role`
   - `message_id` = `${event.properties.info.id}-patch`
   - `generated_at_unix_ms = Date.now()`
2. For each diff item with a string `patch` field, a `message.part` item with:
   - `type: "message.part"`
   - Same `session_id` as the parent `message` payload
   - `message_id = "${id}-patch"` (same as the parent)
   - `part_type: "patch"`
   - `text: entryObj.patch`
   - `generated_at_unix_ms = Date.now()`

### Single mixed dispatch

`recordConversationTrace` dispatches the parent `message` and all per-diff `message.part` items as one mixed envelope through one `sce hooks conversation-trace` invocation.

### No-diff fallback

When `buildPatchConversationTracePayload` returns `undefined` (no diff entries), `recordConversationTrace` falls back to sending the original `message` payload as one mixed envelope via `buildConversationTracePayload`.

## Question tool conversation trace

Completed OpenCode question-tool results are captured through the existing `message.part` mixed-batch path as first-class `part_type: "question"` items.

Capture requires all of these guards:

1. `event.type === "message.part.updated"`
2. `event.properties.part.type === "tool"`
3. `event.properties.part.tool === "question"`
4. `event.properties.part.state.status === "completed"`

`extractQuestionToolAnswers(eventPart)` accepts the narrowed OpenCode tool-part type and reads the question-tool state directly. It uses `state.input.questions` only when that property exists and is an array, and uses `state.metadata.answers` only when that property exists and is an array. Empty or length-mismatched question/answer arrays return `undefined`. Otherwise, entries are paired by index: entries with a string `question` field are retained, missing/non-string question entries are skipped, and each answer entry is joined with `", "` when it is an array (non-array answer entries become an empty answer string). Unrelated tool events are rejected by the earlier `message.part.updated` + `type === "tool"` + `tool === "question"` guards and are not dispatched.

When extraction succeeds, `buildQuestionToolConversationTracePayload(eventPart)` emits one `message.part` item:

- `type: "message.part"`
- `session_id` from `event.properties.part.sessionID`
- `message_id` from `event.properties.part.messageID`
- `part_type: "question"`
- `text: JSON.stringify(Array<{ question: string; answer: string }>)`
- `generated_at_unix_ms = Date.now()`

## Current usage boundary

- The plugin-level fail-open applies to both `runDiffTraceHook` and `runConversationTraceHook`: both functions ignore the child process stderr and resolve unconditionally on any outcome. This prevents sce intake errors from appearing as messages in the OpenCode TUI, since callers await these functions without try/catch.
- `recordConversationTrace(repoRoot, event)` branches on event type:
  - For `message` events: calls `buildPatchConversationTracePayload` first.
    - If a patch payload is returned (diff entries exist), dispatches it once — the original `message` payload is not sent.
    - If `undefined` (no diffs), sends the original `message` payload as one mixed envelope via `buildConversationTracePayload`.
  - For `message.part` question-tool events: checks the `tool === "question"` guard before attempting `buildQuestionToolConversationTracePayload`; completed well-formed results dispatch as one `part_type: "question"` item and skipped results fall through without dispatch.
  - For `message.part` text/reasoning events (only `text` and `reasoning` with non-empty `text`): uses `buildMessagePartConversationTracePayload`.
- The `message` conversation-trace batch (no-diff fallback) maps OpenCode event fields mechanically into a `payloads[0]` item with `type: "message"`, `session_id`, `message_id`, `role`, and `generated_at_unix_ms`; it does not emit message-level `agent` or `summary_diffs` fields and does not duplicate Rust hook validation.
- `buildMessagePartConversationTracePayload(event)` maps `event.properties.part.sessionID`, `messageID`, `type`, and `text` into a `payloads[0]` item with `type: "message.part"`, `session_id`, `message_id`, `part_type`, and `text`, and uses `Date.now()` for `generated_at_unix_ms`.
- The diff extraction seam is internal to the source module and is used by `buildTrace` at runtime.
- `buildTrace` exits early when extraction returns `undefined` (non-user role, empty diffs array, or no usable patch entries), so no diff-trace hook invocation occurs for those events.
- The plugin tracks OpenCode client version per session ID from `session.created` / `session.updated` events and forwards it as `tool_version` when available.
- When extraction succeeds, `buildTrace` forwards the extracted payload with required `tool_name="opencode"` and required `tool_version` (nullable when session version is unavailable) to `sce hooks diff-trace` via STDIN JSON; the Rust hook runtime validates required non-empty `sessionID`/`diff`/`tool_name`, optional `model_id`, required nullable/non-empty `tool_version`, plus required `time`, persists nullable `model_id` / `tool_version` attribution without session fallback, and fails open for runtime intake failures by logging `sce.hooks.diff_trace.error` while returning hook success to the producer.

## Shared boundary with Claude runtime

- OpenCode uses a generated TypeScript event runtime as an event-shape adapter before handing normalized diff-trace payloads to the shared Rust hook intake.
- Claude registration uses generated `.claude/settings.json` command hooks that call `.claude/hooks/run-sce-or-show-install-guidance.sh` before `sce hooks` (no TypeScript runtime intermediary): matched `PostToolUse Write|Edit|MultiEdit|NotebookEdit` pipes the raw hook event to `sce hooks diff-trace`, and supported conversation events pipe raw hook events to `sce hooks conversation-trace`; `SessionStart` is no longer registered.
- Rust `diff-trace` intake detects Claude payloads via `hook_event_name` and derives structured patches from the raw JSON with `payload_type="structured"`; OpenCode normalized payloads (no `hook_event_name`) are stored as `payload_type="patch"`.
- `sce hooks session-model` is no longer a supported shared Rust boundary. Rust remains the only writer of AgentTraceDb `diff_traces` rows; no active runtime path reads or writes `session_models` and no parsed `context/tmp/*-diff-trace.json` artifacts are written by the TypeScript runtime.
- Claude attribution differs from OpenCode attribution: OpenCode reads provider/model data from the OpenCode event and includes `model_id` in the payload, while Claude `diff-trace` best-effort extracts direct model metadata from the raw `PostToolUse` payload and leaves `model_id` nullable when Claude omits it.

## Claude derivation golden tests

- Rust golden tests in `cli/src/services/structured_patch/tests.rs` (`claude_derivation_golden_tests`) own the Claude derivation fixture coverage.
- The test dynamically discovers the checked-in `cli/src/services/structured_patch/fixtures/` scenario directories, validates the expected eight-scenario set, then loads each `claude-post-tool-use.json` plus `expected.patch` pair.
- Each scenario calls `derive_claude_structured_patch(...)` with fixed time/tool-version inputs and asserts derived status, session ID, time, `tool_name="claude"`, tool version, exact golden diff parsed via `parse_patch`, and no emitted `model_id`.
- The former TypeScript `deriveClaudeDiffTracePayload(...)` seam and its Bun test were removed in T07 when the Claude TypeScript plugin source was deleted.
