# OpenCode agent-trace plugin runtime

Current runtime source: `config/lib/agent-trace-plugin/opencode-sce-agent-trace-plugin.ts`.

## Event capture baseline

- The plugin captures `message.updated` events, filtered to user messages with diffs.
- When diff extraction succeeds, the plugin invokes `sce hooks diff-trace` and sends `{ sessionID, diff, time, model_id }` over STDIN JSON.
- The plugin no longer writes diff-trace artifacts or database rows directly; the Rust `diff-trace` hook path owns AgentTraceDb insertion plus collision-safe timestamp+attempt artifact writes.
- `session.diff` event capture has been removed.

## Diff extraction seam

The plugin defines `extractDiffTracePayload(input)` as a typed guard/extraction seam for diff-bearing `message.updated` user-message events.

### Extraction contract

Returns `{ sessionID, diff, time, model_id }` only when all checks pass:

1. `input.event.type === "message.updated"`
2. `input.event.properties` is a non-null object
3. `properties.info` is a non-null object (the `Message` object)
4. `info.role === "user"` (assistant, system, and other roles are skipped)
5. `info.sessionID` is read and returned as `sessionID`, falling back to `"unknown"` when OpenCode omits or empties the field
6. `info.summary?.diffs` is a non-empty array; entries without `patch` string content are skipped
7. Non-empty `patch` strings are joined with `\n` to form the `diff` output string (no `diff` field fallback; only `patch` is used)
8. If no entries yield non-empty patch content, the helper returns `undefined` (empty-diff skip)
9. `time` is sourced from `Date.now()` (Unix epoch milliseconds at extraction time)
10. `model_id` is built as `providerID/modelID` from `info.model.providerID` and `info.model.modelID`, with each missing or empty component falling back to `"unknown"`

Otherwise, the helper returns `undefined`.

## Current usage boundary

- The extraction seam is exported from the source module for focused Bun unit coverage and is used by `buildTrace` at runtime.
- `buildTrace` calls `extractDiffTracePayload`; if the result is `undefined` (non-`message.updated` event, non-user role, empty diffs, or no patch content), no hook invocation occurs.
- When extraction succeeds, `buildTrace` forwards the extracted payload to `sce hooks diff-trace` via STDIN JSON; the current Rust hook runtime still validates and persists only the existing required `sessionID`/`diff`/`time` fields until the downstream Rust payload/storage tasks are implemented.
