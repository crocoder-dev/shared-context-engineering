# OpenCode agent-trace plugin runtime

Current runtime source: `config/lib/agent-trace-plugin/opencode-sce-agent-trace-plugin.ts`.

## Event capture baseline

- The plugin currently captures only `message.part.updated` events.
- For captured events, `buildTrace` writes the full wrapped payload to `context/tmp` as:

```json
{ "input": { ...event hook input... } }
```
- When diff extraction succeeds, the plugin invokes `sce hooks diff-trace` and sends `{ sessionID, diff, time }` over STDIN JSON.
- The plugin no longer writes `[timestamp]-diff-trace.json` directly; the Rust `diff-trace` hook path owns that artifact.

## Diff extraction seam

The plugin defines `extractDiffTracePayload(input)` as a typed guard/extraction seam for diff-bearing `message.part.updated` events.

### Extraction contract

Returns `{ sessionID, diff, time }` only when all checks pass:

1. `input.event.type === "message.part.updated"`
2. `input.event.properties.part.type === "tool"`
3. `input.event.properties.part.state.status === "completed"`
4. `input.event.properties.part.state.metadata.diff` exists and is a non-empty string
5. `input.event.properties.part.sessionID` is read and returned as `sessionID`
6. `input.event.properties.part.state.time.end` is read and returned as numeric `time` (Unix epoch milliseconds)

Otherwise, the helper returns `undefined`.

## Current usage boundary

- The extraction seam is internal preparation logic used by `buildTrace`.
- The write flow always persists the full-event snapshot artifact, then conditionally forwards the extracted payload to `sce hooks diff-trace` via STDIN JSON.
