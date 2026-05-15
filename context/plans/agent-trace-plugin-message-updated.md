# Plan: Replace `session.diff` event capture with `message.updated` in agent-trace plugin

## Change summary

Update `config/lib/agent-trace-plugin/opencode-sce-agent-trace-plugin.ts` to capture
`message.updated` events (filtered to user messages with diffs) instead of `session.diff`
events. The extraction seam (`extractDiffTracePayload`) must be rewritten to read from
`properties.info.summary.diffs[].patch` on `UserMessage` payloads and return the same
`DiffTracePayload` shape (`{ sessionID, diff, time }`) consumed by `sce hooks diff-trace`.

## Success criteria

- Plugin registers interest in `message.updated` instead of `session.diff`.
- `extractDiffTracePayload` returns a valid `DiffTracePayload` for a `message.updated`
  event where `properties.info.role === "user"` and `summary.diffs` contains at least one
  non-empty `patch`.
- `extractDiffTracePayload` returns `undefined` for:
  - Non-`message.updated` events
  - `message.updated` events where `info.role` is not `"user"` (e.g., `"assistant"`)
  - `message.updated` user messages with no `summary` or no `summary.diffs`
  - `message.updated` user messages where all `diffs[].patch` entries are empty/missing
- `sessionID` falls back to `"unknown"` when `properties.info.sessionID` is absent or
  empty.
- `time` is set to `Date.now()` (extraction time, existing behavior).
- `diff` is formed by joining non-empty `patch` strings from each file-diff entry with
  `\n`.
- The context document `context/sce/opencode-agent-trace-plugin-runtime.md` is updated
  to reflect the new event contract.
- `nix flake check` and `nix run .#pkl-check-generated` pass after the change.

## Constraints and non-goals

- **In scope**: TS source, context doc.
- **Out of scope**: Rust CLI changes, schema changes, new database tables, Pkl generation
  changes, test file creation (the plugin has no test file yet — none was found).
- No external dependency changes.
- The `DiffTracePayload` type and `runDiffTraceHook` / `buildTrace` / `SceAgentTracePlugin`
  surfaces remain unchanged.
- The `session.diff` constant can be removed.

## Task stack

- [x] T01: `Rewrite extractDiffTracePayload and update event registration` (status:done)
  - Task ID: T01
  - Goal: Rewrite `extractDiffTracePayload` to extract from `message.updated` user-message
    events and update `REQUIRED_EVENTS` / `ALL_CAPTURED_EVENTS`.
  - Boundaries (in/out of scope):
    - In: Change `REQUIRED_EVENTS` from `session.diff` to `message.updated`.
    - In: Update `extractDiffTracePayload` to:
      - Check `event.type === "message.updated"`
      - Narrow the event properties to access `info` (the `Message`)
      - Filter to `info.role === "user"` (i.e., `UserMessage`)
      - Read `sessionID` from `info.sessionID` with fallback to `"unknown"`
      - Read `summary.diffs` from `info.summary.diffs` (handle optional chain:
        `info.summary?` → `summary.diffs?`)
      - Extract `patch` from each diff entry (the `FileDiff` shape has `patch?: string`
        alongside `additions`, `deletions`, etc.)
      - Join non-empty patches with `\n`; return `undefined` if no patches yield content
      - Use `Date.now()` for `time`
    - In: Remove `session.diff` references, leaving only `message.updated`.
    - In: Only the TS source file is modified.
    - Out: Test creation, Rust changes, Pkl changes, dependency changes.
  - Done when:
    - Source file compiles with `tsc --noEmit` (TypeScript strict mode).
    - All success criteria in the plan are met by the single source change.
  - Verification notes (commands or checks):
    - `cd config/lib/agent-trace-plugin && npx tsc --noEmit`
    - Visual review of the final `extractDiffTracePayload` logic covers all edge cases.
  - **Completed:** 2026-05-15
  - **Files changed:** `config/lib/agent-trace-plugin/opencode-sce-agent-trace-plugin.ts`
  - **Evidence:** `tsc --noEmit` passed with zero errors; visual review confirms all edge cases covered.
  - **Notes:** `REQUIRED_EVENTS` changed from `session.diff` to `message.updated`; `extractDiffTracePayload` reads from `properties.info` (Message shape) with role filter, optional-chain diffs access, and patch-only extraction (no `diff` fallback needed for FileDiff).

- [x] T02: `Update context document opencode-agent-trace-plugin-runtime.md` (status:done)
  - Task ID: T02
  - Goal: Update `context/sce/opencode-agent-trace-plugin-runtime.md` to document the
    new `message.updated` event capture baseline instead of `session.diff`.
  - Boundaries (in/out of scope):
    - In: Rewrite the "Event capture baseline" and "Diff extraction seam" sections to
      describe `message.updated` behavior.
    - In: Update extraction contract to list the new field access path
      (`properties.info.role === "user"`, `info.sessionID`, `info.summary?.diffs[].patch`).
    - In: Document that `session.diff` capture has been removed.
    - Out: Any file outside `context/`.
  - Done when: Context doc accurately describes the new plugin behavior and all old
    `session.diff` references are removed or replaced.
  - Verification notes (commands or checks): Read the file and verify alignment with
    the final T01 source.
  - **Completed:** 2026-05-15
  - **Files changed:** `context/sce/opencode-agent-trace-plugin-runtime.md`
  - **Evidence:** Content verified against T01 source; all session.diff references replaced except the intentional removal notice.
  - **Notes:** Sections rewritten to document message.updated capture, user-role filtering, info.sessionID fallback, info.summary?.diffs[].patch-only extraction, and session.diff removal.

- [x] T03: `Validation and cleanup` (status:done)
  - Task ID: T03
  - Goal: Run final repo-level checks and confirm everything is consistent.
  - Boundaries (in/out of scope):
    - In: `nix flake check`, `nix run .#pkl-check-generated`.
    - In: Confirm `git status` shows only the expected two changed files.
    - Out: Any code or context changes beyond validation.
  - Done when:
    - `nix flake check` passes.
    - `nix run .#pkl-check-generated` passes.
    - No unexpected modified or untracked files remain.
  - Verification notes (commands or checks):
    - `nix flake check`
    - `nix run .#pkl-check-generated`
    - `git status`
  - **Completed:** 2026-05-15
  - **Files changed:** `config/.opencode/plugins/sce-agent-trace.ts` (regenerated via Pkl), `config/automated/.opencode/plugins/sce-agent-trace.ts` (regenerated via Pkl)
  - **Evidence:** `nix flake check` passed (all 4 checks), `nix run .#pkl-check-generated` passed ("Generated outputs are up to date.")
  - **Notes:** Pkl regeneration was needed because T01/T02 modified the canonical source but did not regenerate generated outputs. Git status also shows unrelated untracked files (`poem.txt`, `secondPoem.txt`), pre-existing dependency bump side effects (`bun.lock`, `package.json`), and the expected `context-map.md` update from T02.

## Open questions

None. All clarifications resolved before planning.
