# Plan: Agent Trace Plugin — Switch to session.diff Event Capture

## Change Summary

Change the OpenCode agent-trace plugin (`config/lib/agent-trace-plugin/opencode-sce-agent-trace-plugin.ts`) to capture `session.diff` events instead of `message.part.updated` events, and extract diff payloads from the `session.diff` event shape. The plugin will no longer handle `message.part.updated` events at all.

User-confirmed decisions:

- **Time field**: Use `Date.now()` (wall-clock time) for the `time` field in `DiffTracePayload` when processing `session.diff` events, since the event structure doesn't carry its own timestamp.
- **Empty diffs**: Skip diff-trace hook invocation when `properties.diff` is an empty array — there's no meaningful diff content to forward.
- **Scope**: Only the canonical library source file and its generated copies are in scope. The Rust `diff-trace` hook contract (`{ sessionID, diff, time }` on STDIN) is unchanged.

## Success Criteria

1. The plugin captures `session.diff` events and drops `message.part.updated` events entirely.
2. `extractDiffTracePayload` correctly extracts `{ sessionID, diff, time }` from `session.diff` event properties:
   - `sessionID` from `properties.sessionID` (falling back to `"unknown"` when missing/empty)
   - `diff` from `properties.diff[]` — joining each entry's `patch` (or `diff`) field into a single string, skipping entries with no patch content
   - `time` from `Date.now()` (Unix epoch milliseconds)
3. Events with an empty `properties.diff` array are silently skipped (no hook invocation).
4. The `DiffTracePayload` type and `runDiffTraceHook` function remain unchanged — the Rust-side contract is preserved.
5. Generated plugin copies at `config/.opencode/plugins/sce-agent-trace.ts` and `config/automated/.opencode/plugins/sce-agent-trace.ts` are regenerated and content-aligned with the canonical source.
6. Pkl generation parity check passes (`nix run .#pkl-check-generated`).
7. Repository validation passes (`nix flake check`).
8. Context documentation for the agent-trace plugin runtime is updated to reflect the new event capture and extraction behavior.

## Constraints and Non-Goals

- In scope: canonical plugin source (`config/lib/agent-trace-plugin/opencode-sce-agent-trace-plugin.ts`), generated copies, Pkl regeneration, and context documentation updates.
- Out of scope: changes to the Rust `diff-trace` hook contract, `DiffTracePayload` shape, or CLI-side behavior.
- Out of scope: adding new test infrastructure for the agent-trace plugin (no tests currently exist for this plugin).
- Do not hand-edit generated outputs; regenerate from canonical source via Pkl.
- Keep task slicing atomic: each executable task must be one coherent commit unit.

## Task Stack

- [x] T01: `Switch agent-trace plugin to session.diff event capture and extraction` (status:done)
  - Task ID: T01
  - Goal: Rewrite the canonical plugin source to capture `session.diff` events, extract `{ sessionID, diff, time }` from the `session.diff` event shape, and drop `message.part.updated` handling entirely.
  - Boundaries (in/out of scope): In — `config/lib/agent-trace-plugin/opencode-sce-agent-trace-plugin.ts` source changes: update `REQUIRED_EVENTS`/`ALL_CAPTURED_EVENTS` to `session.diff`, replace `extractDiffTracePayload` with new extraction logic for `session.diff` properties, remove `extractDiffFromFilesMetadata` (replaced by inline extraction from `properties.diff[]`), use `Date.now()` for `time`, skip empty diff arrays. Out — Rust-side changes, `DiffTracePayload` type changes, `runDiffTraceHook` changes, generated output files, context docs.
  - Done when: the canonical plugin source captures only `session.diff` events, extracts `sessionID`/`diff`/`time` from the `session.diff` event shape, skips empty diff arrays, uses `Date.now()` for `time`, and no longer references `message.part.updated` or its nested `part`/`state`/`metadata` structure.
  - Verification notes (commands or checks): manual inspection of the canonical source; confirm `message.part.updated` is not referenced; confirm `session.diff` is the only captured event type; confirm `Date.now()` is used for `time`; confirm empty diff arrays produce no hook invocation.
  - **Status:** done
  - **Completed:** 2026-04-29
  - **Files changed:** `config/lib/agent-trace-plugin/opencode-sce-agent-trace-plugin.ts`
  - **Evidence:** `nix flake check` passed (all checks passed); `nix run .#pkl-check-generated` passed (generated outputs up to date); `grep "message.part.updated" config/lib/agent-trace-plugin/opencode-sce-agent-trace-plugin.ts` returned no matches; `session.diff` confirmed as only captured event type; `Date.now()` confirmed for `time`; empty diff arrays confirmed to produce no hook invocation.
  - **Notes:** Verify-only context sync pass — root context files unchanged; stale `message.part.updated` references in `context/glossary.md` and `context/context-map.md` are T03 scope.

- [x] T02: `Regenerate plugin copies and verify parity` (status:done)
  - Task ID: T02
  - Goal: Regenerate both generated plugin copies from the updated canonical source and verify generation parity.
  - Boundaries (in/out of scope): In — running Pkl generation to update `config/.opencode/plugins/sce-agent-trace.ts` and `config/automated/.opencode/plugins/sce-agent-trace.ts`, plus `nix run .#pkl-check-generated` verification. Out — source logic changes, context docs.
  - Done when: both generated plugin files are content-aligned with the canonical source and `nix run .#pkl-check-generated` passes.
  - Verification notes (commands or checks): `nix develop -c pkl eval -m . config/pkl/generate.pkl` then `nix run .#pkl-check-generated`; diff the generated files against the canonical source to confirm content alignment.
  - **Status:** done
  - **Completed:** 2026-04-29
  - **Files changed:** `config/.opencode/plugins/sce-agent-trace.ts`, `config/automated/.opencode/plugins/sce-agent-trace.ts`
  - **Evidence:** `nix run .#pkl-check-generated` passed ("Generated outputs are up to date"); `nix flake check` passed (all checks passed); both generated files are content-identical to the canonical source (136 lines each, `session.diff` event capture, `Date.now()` for `time`, no `message.part.updated` references).
  - **Notes:** Verify-only context sync expected — this task only regenerated outputs from an already-updated canonical source; no root context changes needed.

- [x] T03: `Update context documentation for agent-trace plugin runtime` (status:done)
  - Task ID: T03
  - Goal: Update `context/sce/opencode-agent-trace-plugin-runtime.md` and relevant glossary/context-map entries to reflect the new `session.diff` event capture and extraction behavior.
  - Boundaries (in/out of scope): In — `context/sce/opencode-agent-trace-plugin-runtime.md`, `context/glossary.md` (agent-trace plugin entries), `context/context-map.md` (agent-trace plugin runtime entry). Out — code changes, generated outputs.
  - Done when: context files accurately describe `session.diff` event capture, the new extraction contract (sessionID from properties, diff from properties.diff[] array, time from Date.now(), empty-diff skip behavior), and no longer reference `message.part.updated` as a captured event.
  - Verification notes (commands or checks): manual review of updated context files for accuracy against the canonical source.
  - **Status:** done
  - **Completed:** 2026-04-29
  - **Files changed:** `context/sce/opencode-agent-trace-plugin-runtime.md`, `context/glossary.md`, `context/context-map.md`
  - **Evidence:** Manual review confirms all three context files now describe `session.diff` event capture, `Date.now()` for time, empty-diff skip behavior, and `properties.diff[]` array extraction; no stale `message.part.updated` references remain in the updated context files as a captured event type.
  - **Notes:** This was a context-only task; no code or generated outputs were changed.

- [x] T04: `Validation and cleanup` (status:done)
  - Task ID: T04
  - Goal: Run full repository validation and confirm no residual references to the old `message.part.updated` capture behavior remain in the plugin source or generated copies.
  - Boundaries (in/out of scope): In — `nix flake check`, `nix run .#pkl-check-generated`, grep for stale `message.part.updated` references in plugin files, plan evidence updates. Out — new functional behavior.
  - Done when: `nix flake check` passes, `nix run .#pkl-check-generated` passes, no stale `message.part.updated` references remain in the plugin source or generated copies, and plan task statuses are updated.
  - Verification notes (commands or checks): `nix flake check`; `nix run .#pkl-check-generated`; `grep -r "message.part.updated" config/lib/agent-trace-plugin/ config/.opencode/plugins/sce-agent-trace.ts config/automated/.opencode/plugins/sce-agent-trace.ts` should return no matches.
  - **Status:** done
  - **Completed:** 2026-04-29
  - **Files changed:** `context/plans/agent-trace-plugin-session-diff.md` (status update only)
  - **Evidence:** `nix flake check` passed (all checks passed); `nix run .#pkl-check-generated` passed ("Generated outputs are up to date"); `grep -r "message.part.updated"` in plugin source and generated copies returned only SDK type definitions in `node_modules/@opencode-ai/sdk/` (not our plugin code) — no stale references in our plugin source or generated copies.
  - **Notes:** All four tasks complete. Plan is fully validated.

## Open Questions

None — all ambiguities resolved through user confirmation.

## Validation Report

### Commands run
- `nix flake check` → exit 0 (all checks passed: cli-tests, cli-clippy, cli-fmt, integrations-install-tests, integrations-install-clippy, integrations-install-fmt, pkl-parity, npm-bun-tests, npm-biome-check, npm-biome-format, config-lib-bun-tests, config-lib-biome-check, config-lib-biome-format)
- `nix run .#pkl-check-generated` → exit 0 ("Generated outputs are up to date")
- `grep -c "message.part.updated" config/lib/agent-trace-plugin/opencode-sce-agent-trace-plugin.ts config/.opencode/plugins/sce-agent-trace.ts config/automated/.opencode/plugins/sce-agent-trace.ts` → 0 matches in all three files
- `grep -c "session.diff" config/lib/agent-trace-plugin/opencode-sce-agent-trace-plugin.ts config/.opencode/plugins/sce-agent-trace.ts config/automated/.opencode/plugins/sce-agent-trace.ts` → 2 matches in each file (REQUIRED_EVENTS + extractDiffTracePayload guard)
- `grep -c "Date.now()" config/lib/agent-trace-plugin/opencode-sce-agent-trace-plugin.ts config/.opencode/plugins/sce-agent-trace.ts config/automated/.opencode/plugins/sce-agent-trace.ts` → 1 match in each file (time field)

### Temporary scaffolding removed
- None introduced.

### Success-criteria verification
- [x] SC1: The plugin captures `session.diff` events and drops `message.part.updated` events entirely → confirmed: 0 `message.part.updated` references in plugin source/generated; 2 `session.diff` references per file
- [x] SC2: `extractDiffTracePayload` correctly extracts `{ sessionID, diff, time }` from `session.diff` event properties → confirmed: canonical source uses `properties.sessionID` with `"unknown"` fallback, `properties.diff[]` array with `patch`/`diff` field joining, and `Date.now()` for time
- [x] SC3: Events with empty `properties.diff` array are silently skipped → confirmed: canonical source returns `undefined` when `diffEntries.length === 0`
- [x] SC4: `DiffTracePayload` type and `runDiffTraceHook` function remain unchanged → confirmed: no changes to these in canonical source
- [x] SC5: Generated plugin copies are content-aligned with canonical source → confirmed: `nix run .#pkl-check-generated` passed; all three files have identical `session.diff`/`Date.now()`/empty-diff-skip behavior
- [x] SC6: Pkl generation parity check passes → confirmed: `nix run .#pkl-check-generated` passed
- [x] SC7: Repository validation passes → confirmed: `nix flake check` passed
- [x] SC8: Context documentation updated → confirmed: `context/sce/opencode-agent-trace-plugin-runtime.md`, `context/glossary.md`, and `context/context-map.md` all reflect `session.diff` event capture

### Residual risks
- None identified.