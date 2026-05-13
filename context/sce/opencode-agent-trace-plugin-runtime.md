# OpenCode agent-trace plugin runtime

Current runtime source: `config/lib/agent-trace-plugin/opencode-sce-agent-trace-plugin.ts`.

## Event capture baseline

- The plugin treats `session.diff` as the diff-only event source for patch capture; model metadata is not sourced from the `session.diff` event.
- The plugin also observes `chat.message` and `chat.params` hooks only to cache model metadata by `sessionID`; those hooks do not invoke `sce` or persist data directly.
- When diff extraction succeeds, the plugin invokes `sce hooks diff-trace` and sends `{ sessionID, diff, time, model_id? }` over STDIN JSON.
- The plugin no longer writes diff-trace artifacts or database rows directly; the Rust `diff-trace` hook path owns AgentTraceDb insertion plus collision-safe timestamp+attempt artifact writes.

## Model metadata cache

- Runtime cache shape: in-memory `Map<string, string>` keyed by non-empty `sessionID`.
- Primary source: `chat.message` input `model.providerID` + `model.modelID`.
- Fallback source: `chat.params` input `provider.info.id` or `provider.info.providerID` plus `model.id` or `model.modelID`.
- Stored value shape: `<providerID>/<modelID>`; incomplete or empty model metadata is ignored.
- The cache is process-local and non-durable. Sessions without cached metadata still emit legacy-compatible diff-trace payloads without `model_id`.

## Diff extraction seam

The plugin defines `extractDiffTracePayload(input, modelIdsBySessionID)` as a typed guard/extraction seam for diff-bearing `session.diff` events.

### Extraction contract

Returns `{ sessionID, diff, time, model_id? }` only when all checks pass:

1. `input.event.type === "session.diff"`
2. `input.event.properties` is a non-null object
3. `properties.sessionID` is read and returned as `sessionID`, falling back to `"unknown"` when OpenCode omits or empties the field
4. `properties.diff` is an array with at least one entry; entries without `patch` or `diff` string content are skipped
5. Each entry's `patch` field is preferred; `diff` field is used as fallback when `patch` is absent or non-string
6. Non-empty patch strings are joined with `\n` to form the `diff` output string
7. If no entries yield non-empty patch content, the helper returns `undefined` (empty-diff skip)
8. `time` is sourced from `Date.now()` (Unix epoch milliseconds at extraction time)
9. `model_id` is included only when the session cache has a value for the extracted `sessionID`

Otherwise, the helper returns `undefined`.

## Current usage boundary

- The extraction seam is internal preparation logic used by `buildTrace` and exported for package-local unit tests.
- `buildTrace` calls `extractDiffTracePayload` with the current session model cache; if the result is `undefined` (non-`session.diff` event, empty diff array, or no patch content), no hook invocation occurs.
- When extraction succeeds, `buildTrace` forwards the extracted payload to `sce hooks diff-trace` via STDIN JSON; the Rust hook runtime owns validation and dual persistence for both legacy payloads and payloads with optional `model_id`.
