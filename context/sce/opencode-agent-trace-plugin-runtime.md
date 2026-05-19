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
6. `info.summary?.diffs` is a non-empty array; non-object entries are skipped
7. Each object entry contributes its `patch` value or an empty string, and entries are joined with `\n` to form the `diff` output string (no `diff` field fallback; only `patch` is used)
8. If no object entries are present, the helper returns `undefined`; all-empty patch values still produce a payload and are left to the Rust `diff-trace` hook validation
9. `time` is sourced from `Date.now()` (Unix epoch milliseconds at extraction time)
10. `model_id` is built directly as `providerID/modelID` from `info.model.providerID` and `info.model.modelID`

Otherwise, the helper returns `undefined`.

## Current usage boundary

- The extraction seam is internal to the source module and is used by `buildTrace` at runtime.
- `buildTrace` calls `extractDiffTracePayload`; if the result is `undefined` (non-`message.updated` event, non-user role, empty diffs array, or no object diff entries), no hook invocation occurs.
- When extraction succeeds, `buildTrace` forwards the extracted payload to `sce hooks diff-trace` via STDIN JSON; the Rust hook runtime validates required `sessionID`/`diff`/`model_id` plus `time` and persists those fields through AgentTraceDb `diff_traces` insertion.
