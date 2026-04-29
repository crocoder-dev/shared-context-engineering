# OpenCode agent-trace plugin runtime

Current runtime source: `config/lib/agent-trace-plugin/opencode-sce-agent-trace-plugin.ts`.

## Event capture baseline

- The plugin captures only `session.diff` events.
- When diff extraction succeeds, the plugin invokes `sce hooks diff-trace` and sends `{ sessionID, diff, time }` over STDIN JSON.
- The plugin no longer writes diff-trace artifacts directly; the Rust `diff-trace` hook path owns collision-safe timestamp+attempt artifact writes plus additive `diff_traces` insertion into `agent-trace.db`.

## Diff extraction seam

The plugin defines `extractDiffTracePayload(input)` as a typed guard/extraction seam for diff-bearing `session.diff` events.

### Extraction contract

Returns `{ sessionID, diff, time }` only when all checks pass:

1. `input.event.type === "session.diff"`
2. `input.event.properties` is a non-null object
3. `properties.sessionID` is read and returned as `sessionID`, falling back to `"unknown"` when OpenCode omits or empties the field
4. `properties.diff` is an array with at least one entry; entries without `patch` or `diff` string content are skipped
5. Each entry's `patch` field is preferred; `diff` field is used as fallback when `patch` is absent or non-string
6. Non-empty patch strings are joined with `\n` to form the `diff` output string
7. If no entries yield non-empty patch content, the helper returns `undefined` (empty-diff skip)
8. `time` is sourced from `Date.now()` (Unix epoch milliseconds at extraction time)

Otherwise, the helper returns `undefined`.

## Current usage boundary

- The extraction seam is internal preparation logic used by `buildTrace`.
- `buildTrace` calls `extractDiffTracePayload`; if the result is `undefined` (non-`session.diff` event, empty diff array, or no patch content), no hook invocation occurs.
- When extraction succeeds, `buildTrace` forwards the extracted payload to `sce hooks diff-trace` via STDIN JSON so the Rust hook runtime owns accepted diff-trace artifact writes and `agent-trace.db` persistence.
