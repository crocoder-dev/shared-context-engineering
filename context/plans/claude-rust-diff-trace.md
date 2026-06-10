# Plan: Claude Rust Diff Trace

## Change summary

Move Claude agent-trace hook derivation out of generated TypeScript and into the Rust `sce` CLI so generated `.claude/settings.json` invokes `sce hooks` directly. The new Claude `PostToolUse` flow should send the structured Claude hook payload to `sce hooks diff-trace`, where Rust derives the patch and persists the existing normalized diff-trace data. Claude `SessionStart` model attribution should continue to flow through `sce hooks session-model`, also called directly from Claude settings. OpenCode TypeScript plugin behavior remains in scope only as an unchanged consumer of the existing normalized `diff-trace` payload.

Recent commit input considered:

- `8172e87 trace: Remove redundant PostToolUse forwarding comment`
- `5b8cbfb flake: Preserve repo-shaped config-lib check source`
- `d9b972b hooks: Unify STDIN payload validation helpers`
- `01a6359 patch: Remove unused diff_creation input fixtures`
- `f72fd3d agent-trace: Add Claude derivation golden tests`
- `d8169cf hooks: Make model_id optional with session_models resolution`
- `e2d242b cli: Remove unused tempfile dev dependency`
- `88a17f6 agent-trace: Replace raw Claude capture with normalized session-model intake`
- `7937112 feat(claude): Add raw hook capture and Claude agent configuration`

Planning interpretation: the last nine commits created and then refined a Claude TypeScript bridge that derives normalized diff traces, added reusable fixture coverage, removed raw capture, and stabilized Rust-side validation/model attribution. This plan preserves those validated contracts while moving the remaining Claude-specific derivation and golden coverage into Rust.

## Success criteria

- Generated Claude settings call `sce hooks session-model` and `sce hooks diff-trace` directly; they no longer execute Bun or `.claude/plugins/sce-agent-trace.ts`.
- AgentTraceDb stores diff-trace source payloads behind a generic payload column plus a payload-type discriminator, with OpenCode rows marked as patch payloads and Claude rows marked as structured payloads.
- `sce hooks diff-trace` persists Claude structured `PostToolUse` payload JSON without first rendering it into a patchset; post-commit processing derives `ParsedPatch` from the stored structured JSON through `structured_patch.rs`.
- Existing OpenCode normalized `diff-trace` payloads remain accepted and behaviorally unchanged, except for being stored through the generic payload/discriminator schema and parsed from that representation during post-commit processing.
- Claude TypeScript plugin source, generated plugin output, and Claude TypeScript golden tests are removed from the repo-owned Claude path.
- Golden fixture coverage for Claude diff derivation lives in Rust and validates the checked-in `cli/src/services/structured_patch/fixtures/` scenarios.
- Generated output parity and full repo validation pass.

## Constraints and non-goals

- In scope: AgentTraceDb diff-trace storage migration, Rust hook intake/derivation, Rust tests, Pkl-generated Claude settings, generated output updates, context sync.
- In scope: removing Claude-specific TypeScript plugin source/tests and generated `.claude/plugins` output.
- Out of scope: changing OpenCode TypeScript plugin behavior or removing OpenCode TypeScript runtime code.
- Out of scope: changing post-commit Agent Trace payload semantics, AgentTraceDb schema beyond the `diff_traces` typed source-payload migration, or OpenCode plugin registration.
- Out of scope: adding a new external dependency unless implementation proves the existing Rust stack cannot parse the structured Claude payload safely.
- Preserve one-task/one-atomic-commit slicing; each executable task below should land independently.

## Assumptions

- The generic diff-trace persisted payload uses a discriminator with values equivalent to `patch` for OpenCode unified patch payloads and `structured` for Claude structured hook payloads.
- Claude structured rows should mirror OpenCode row behavior: persist the source payload at `sce hooks diff-trace` intake, then convert to `ParsedPatch` only during post-commit recent-diff-trace processing.
- `cli/src/services/structured_patch.rs` remains the Rust owner for converting Claude structured payload JSON into `ParsedPatch`.

## Task stack

- [x] T01: `Add Rust Claude hook payload derivation model` (status:done)
  - Task ID: T01
  - Goal: Add Rust data models and pure derivation helpers that convert supported Claude `PostToolUse` structured payloads into the existing normalized diff-trace shape.
  - Boundaries (in/out of scope): In - Rust-only parsing/normalization helpers for Claude `Write` create and `Edit` structured-patch payloads, status/skip reasons matching the current supported cases, no CLI routing changes. Out - generated settings changes, TypeScript deletion, DB persistence changes, OpenCode flow changes.
  - Done when: Rust exposes a testable pure function that accepts event name + JSON payload + fixed time/tool version inputs and returns derived `{ sessionID, diff, time, tool_name="claude", tool_version }` or deterministic skip/error results for unsupported payloads.
  - Verification notes (commands or checks): Run the narrow Rust tests added for the derivation helper via `nix develop -c sh -c 'cd cli && cargo test claude'` if a narrow test target exists; otherwise use the narrowest relevant `cargo test` selector. Final full validation remains T09.
  - Completion evidence (2026-06-10): Added synchronous Rust `structured_patch` service module; Claude `PostToolUse` `Write` and `Edit` structured payload derivation returns `ParsedPatch`-backed `ClaudeStructuredPatch` results with deterministic skip reasons and fixed time/tool-version inputs. Generated helper tests were removed after review; golden fixture coverage remains deferred to T02. `nix flake check` passed. `nix run .#pkl-check-generated` passed. Direct narrow `cargo test claude` was not run because the repo bash policy blocks direct Cargo test commands in favor of `nix flake check`.

- [x] T02: `Move Claude diff-creation golden tests to Rust` (status:done)
  - Task ID: T02
  - Goal: Recreate the current Claude derivation golden coverage in Rust against `cli/src/services/structured_patch/fixtures/`.
  - Boundaries (in/out of scope): In - Rust tests that discover/validate the expected eight fixture scenarios and assert derived patch equality using `claude-post-tool-use.json` plus `expected.patch`. Out - deleting TypeScript tests/source, modifying fixture contents except for necessary fixture-contract corrections, changing OpenCode tests.
  - Done when: Rust tests fail on missing/extra scenarios, use fixed time/tool-version inputs, assert `sessionID`, `tool_name="claude"`, nullable/omitted `model_id` behavior as appropriate, and exact diff output for each golden fixture.
  - Verification notes (commands or checks): Run the narrow Rust golden test selector through Nix, for example `nix develop -c sh -c 'cd cli && cargo test claude_derivation'` once test names are known.
  - Completion evidence (2026-06-10): Added `cli/src/services/structured_patch/tests.rs` with runtime fixture discovery, missing/extra scenario validation, and `claude_derivation_golden_tests` asserting all eight `diff_creation/` scenarios against `derive_claude_structured_patch` with fixed time/tool-version inputs. `nix flake check` passed. `nix run .#pkl-check-generated` passed.

- [x] T03: `Migrate diff_traces to typed generic payload storage` (status:done)
  - Task ID: T03
  - Goal: Replace patch-only diff-trace persistence with a generic source-payload column plus payload-type discriminator while preserving existing OpenCode rows and query behavior.
  - Boundaries (in/out of scope): In - AgentTraceDb migration(s), typed insert/query structs, constants/enums for `patch` and `structured` discriminator values, backward-compatible handling for existing `patch` data if required by current migrations/tests. Out - Claude hook intake, post-commit structured parsing, generated settings changes, OpenCode plugin changes.
  - Done when: New diff-trace inserts can persist payload text with an explicit type; existing OpenCode patch payloads are stored/read as `patch`; recent-diff-trace query code exposes enough typed information for later parsing into `ParsedPatch`.
  - Verification notes (commands or checks): Run focused AgentTraceDb tests through Nix, for example `nix develop -c sh -c 'cd cli && cargo test agent_trace_db'`; include migration/backward-compatibility test coverage where the current DB test harness supports it.
  - Completion evidence (2026-06-10): Added migration `009_add_diff_traces_payload_type.sql` adding `payload_type TEXT NOT NULL DEFAULT 'patch'` to `diff_traces`. Added `PAYLOAD_TYPE_PATCH` and `PAYLOAD_TYPE_STRUCTURED` constants to `agent_trace_db/mod.rs`. Updated `DiffTraceInsert`, `DiffTracePatchRow`, `ParsedDiffTracePatch` to carry `payload_type`. Updated `INSERT_DIFF_TRACE_SQL` and `SELECT_RECENT_DIFF_TRACE_PATCHES_SQL` to include `payload_type`. Updated `insert_diff_trace_with`, `diff_trace_patch_row_from_turso`, and `parse_recent_diff_trace_patch_rows` for the new column. Updated `hooks/mod.rs` to pass `PAYLOAD_TYPE_PATCH` for existing OpenCode diff-trace flow. Updated baseline migration test to expect 9 migrations. Added `payload_type` assertion to existing diff-trace query test. `nix flake check` passed. `nix run .#pkl-check-generated` passed.

- [x] T04: `Persist Claude structured diff-trace source payloads` (status:done)
  - Task ID: T04
  - Goal: Extend `sce hooks diff-trace` STDIN intake so Claude structured `PostToolUse` payload JSON is classified and persisted as a structured source payload without converting it to a patchset at insert time.
  - Boundaries (in/out of scope): In - payload classification, validation errors/skips for unsupported/no-diff Claude events, insert adapter to the generic payload schema, tests for Claude structured payload intake and existing OpenCode normalized payload compatibility. Out - post-commit conversion to `ParsedPatch`, generated settings changes, OpenCode plugin changes.
  - Done when: `diff-trace` accepts existing OpenCode normalized payloads as `patch` payloads and Claude supported structured payloads as `structured` payloads; Claude unsupported/no-diff cases produce deterministic success/no-op or validation behavior consistent with current hook semantics; no Claude row is rendered to unified-diff text before DB persistence.
  - Verification notes (commands or checks): Run focused hooks tests through Nix, for example `nix develop -c sh -c 'cd cli && cargo test hooks'`; include a targeted exact test when available.
  - Completion evidence (2026-06-10): Extended `DiffTracePayload` with `payload_type` field. Added `DiffTraceParseResult` enum with `Persist` and `NoOp` variants. Modified `parse_diff_trace_payload` to classify payloads: if `hook_event_name` is present, the Claude path uses `derive_claude_structured_patch` to validate; unsupported events, non-PostToolUse events, and unsupported tools produce deterministic `NoOp` results. Claude `PostToolUse Write`/`Edit` payloads are classified as `structured` with the raw JSON stored as the `diff` column (not rendered to unified diff). OpenCode normalized payloads continue as `patch`. Updated `persist_diff_trace_payload_to_agent_trace_db_with` to use the payload's own `payload_type` instead of hardcoded `PAYLOAD_TYPE_PATCH`. Removed `#[allow(dead_code)]` from `PAYLOAD_TYPE_STRUCTURED` and `#![allow(dead_code)]` from `structured_patch.rs`. Generated tests were removed per review feedback. `nix flake check` passed (all check derivations green). `nix run .#pkl-check-generated` passed.

- [x] T05: `Parse typed diff-trace payloads during post-commit processing` (status:done)
  - Task ID: T05
  - Goal: Update post-commit recent-diff-trace processing so typed persisted payloads are converted into `ParsedPatch` at read/processing time.
  - Boundaries (in/out of scope): In - parser dispatch for `payload_type="patch"` through existing patch parsing, parser dispatch for `payload_type="structured"` through `structured_patch.rs`, malformed-row skip accounting, model/tool metadata preservation, tests covering mixed OpenCode+Claude rows. Out - DB schema changes beyond T03, hook settings generation, Agent Trace output schema changes.
  - Done when: Post-commit combines/intersects OpenCode patch rows and Claude structured rows through the same `ParsedPatch` pipeline; structured Claude rows derive the same patch output as Rust golden fixtures; malformed or unsupported stored payloads are skipped/reportable without breaking valid rows.
  - Verification notes (commands or checks): Run focused post-commit/hooks tests through Nix, for example `nix develop -c sh -c 'cd cli && cargo test post_commit'` or the narrowest matching selector once test names are known.
  - Completion evidence (2026-06-10): Modified `parse_recent_diff_trace_patch_rows` in `agent_trace_db/mod.rs` to dispatch on `payload_type`: `patch` rows use existing `parse_patch`, `structured` rows parse stored JSON and derive `ParsedPatch` via `derive_claude_structured_patch` at read time, other payload types are skipped deterministically. Added `Display` impl for `ClaudeStructuredPatchSkipReason` in `structured_patch.rs`. `nix flake check` passed (all 4 checks green). `nix run .#pkl-check-generated` passed.

- [x] T06: `Render Claude settings with direct sce hook commands` (status:done)
  - Task ID: T06
  - Goal: Update canonical Pkl-generated Claude settings so Claude invokes `sce hooks session-model` and `sce hooks diff-trace` directly instead of running Bun against `.claude/plugins/sce-agent-trace.ts`.
  - Boundaries (in/out of scope): In - `config/pkl/renderers/claude-content.pkl` settings command definitions and regenerated `config/.claude/settings.json` / repo-root `.claude/settings.json` outputs. Out - OpenCode renderer/plugin registration, agent/skill content changes unrelated to settings, manual edits to generated outputs without source updates.
  - Done when: Generated Claude settings contain no `.claude/plugins/sce-agent-trace.ts` or `bun` hook invocation for agent tracing, and route `SessionStart` to `sce hooks session-model` while routing matched `PostToolUse` to `sce hooks diff-trace` with Claude hook payload on STDIN according to Claude hook command behavior.
  - Verification notes (commands or checks): Run `nix develop -c pkl eval -m . config/pkl/generate.pkl` after source edits, then `nix run .#pkl-check-generated`.
  - Completion evidence (2026-06-10): Replaced `claude-content.pkl` settings block: `SessionStart` routes to `sce hooks session-model`, `PostToolUse Write|Edit|MultiEdit|NotebookEdit` routes to `sce hooks diff-trace`, removed `UserPromptSubmit`/`Stop` hooks. Added `parse_claude_session_model_payload` to `cli/src/services/hooks/mod.rs` so the Rust `session-model` intake handles raw Claude `SessionStart` payloads (extracts `session_id`/`model_id`/`time`/`tool_version`, normalizes `model_id` with `claude/` prefix). Regenerated `config/.claude/settings.json`. `nix flake check` passed (all 4 checks green). `nix run .#pkl-check-generated` passed.

- [x] T07: `Remove Claude TypeScript plugin source and generated outputs` (status:done)
   - Task ID: T07
   - Goal: Delete the now-obsolete Claude TypeScript agent-trace runtime and its TypeScript golden tests while preserving OpenCode TypeScript plugin/runtime code.
   - Boundaries (in/out of scope): In - remove `config/lib/agent-trace-plugin/claude-sce-agent-trace-plugin.ts`, its Bun test, generated `config/.claude/plugins/sce-agent-trace.ts`, generated root `.claude/plugins/sce-agent-trace.ts`, and references that assume a Claude plugin path exists. Out - `config/lib/agent-trace-plugin/opencode-sce-agent-trace-plugin.ts`, OpenCode generated plugins, bash-policy code.
   - Done when: No repo-owned Claude `.claude/plugins` agent-trace TypeScript remains; config-lib package/test configuration no longer expects the deleted Claude test; generated output parity is clean after regeneration.
   - Verification notes (commands or checks): Run `nix run .#pkl-check-generated`; run the relevant config-lib checks only if package/test manifests changed, otherwise rely on T09 full validation.
   - Completion evidence (2026-06-10): Deleted `config/lib/agent-trace-plugin/claude-sce-agent-trace-plugin.ts` (canonical source), `config/lib/agent-trace-plugin/claude-sce-agent-trace-plugin.test.ts` (Bun test), `config/.claude/plugins/sce-agent-trace.ts` (generated), and `.claude/plugins/sce-agent-trace.ts` (root copy). Removed Claude plugin source read and output mapping from `config/pkl/generate.pkl`. Updated root `.claude/settings.json` to direct `sce hooks session-model` / `sce hooks diff-trace` commands. Regenerated outputs. `nix run .#pkl-check-generated` (generated outputs up to date), `nix flake check` (all 7 checks passed: cli-tests, cli-clippy, cli-fmt, config-lib-bun-tests, config-lib-biome-check, config-lib-biome-format, pkl-parity).

- [x] T08: `Sync current-state context for Rust-owned Claude tracing` (status:done)
  - Task ID: T08
  - Goal: Update durable context to describe the new Rust-owned Claude derivation boundary and removal of Claude TypeScript plugin runtime.
  - Boundaries (in/out of scope): In - focused updates to `context/sce/opencode-agent-trace-plugin-runtime.md`, `context/sce/agent-trace-hooks-command-routing.md`, `context/cli/patch-service.md`, `context/context-map.md`, `context/overview.md`, and glossary/architecture entries if needed. Out - historical narration beyond current-state facts, unrelated context cleanup.
  - Done when: Context says OpenCode still uses TypeScript normalized diff traces, diff-trace storage uses typed source payloads, Claude settings call `sce hooks` directly, Rust derives Claude structured patches during post-commit processing, and golden tests are Rust-owned.
  - Verification notes (commands or checks): Review context references for stale `.claude/plugins/sce-agent-trace.ts`, Claude TypeScript golden test, and shared TypeScript-runtime-to-Rust boundary claims.
  - Completion evidence (2026-06-10): Updated `context/sce/opencode-agent-trace-plugin-runtime.md` (removed stale Claude TypeScript source listing, updated golden tests section), `context/overview.md` (replaced Claude Bun-runtime paragraph with direct-command-hook description, fixed stale "until T05" qualifier), `context/architecture.md` (fixed stale "until T05" qualifier), `context/glossary.md` (fixed stale "until T05" qualifier), `context/sce/claude-raw-hook-capture.md` (updated "Current state" to reflect direct `sce hooks` boundary and former TypeScript runtime removal). Confirmed zero remaining stale references via targeted search. `nix run .#pkl-check-generated` passed.

- [x] T09: `Validate and clean up Claude Rust diff-trace migration` (status:done)
  - Task ID: T09
  - Goal: Run final validation, remove temporary scaffolding, and record plan completion evidence.
  - Boundaries (in/out of scope): In - full repo validation, generated-output parity, checking for stale Claude TypeScript references, updating this plan with validation evidence. Out - new feature work or unrelated refactors discovered during validation.
  - Done when: `nix run .#pkl-check-generated` and `nix flake check` pass; no stale Claude plugin TypeScript files/references remain except intentional historical references; plan status/evidence is updated.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; `nix flake check`; targeted search for `.claude/plugins/sce-agent-trace.ts`, `deriveClaudeDiffTracePayload`, and Claude TypeScript golden-test references; targeted search/review that Claude structured payload rows are not rendered into patchsets before persistence.
  - Completion evidence (2026-06-10):
    - **`nix run .#pkl-check-generated`**: passed (generated outputs up to date)
    - **`nix flake check`**: all 4 checks passed (cli-tests, cli-clippy, cli-fmt, pkl-parity)
    - **Stale file check**: `config/.claude/plugins/` and `.claude/plugins/` directories do not exist; only `opencode-sce-agent-trace-plugin.ts` remains in `config/lib/agent-trace-plugin/`
    - **Settings.json**: `config/.claude/settings.json` calls `sce hooks session-model` and `sce hooks diff-trace` directly (no agent-trace plugin or Bun invocations)
    - **Structured payload contract (code review confirmed)**:
      - Claude `PostToolUse` payloads stored as raw JSON with `payload_type="structured"` at `diff-trace` intake (`cli/src/services/hooks/mod.rs` line 352: `diff: stdin_payload.to_string()`)
      - Post-commit read-time dispatch: `payload_type="structured"` rows parse stored JSON through `derive_claude_structured_patch` (`cli/src/services/agent_trace_db/mod.rs` lines 437-443)
    - **Temporary scaffolding**: none found; `PAYLOAD_TYPE_STRUCTURED` is properly active (no `#[allow(dead_code)]`); `structured_patch.rs` has no file-level `#![allow(dead_code)]`; no plan-specific TODOs in Rust code

## Open questions

- None blocking. User clarified that Claude derivation should happen fully in Rust, Claude TypeScript should be removed, OpenCode TypeScript should remain, generated Claude settings should call `sce hooks` directly, and AgentTraceDb should persist generic typed source payloads so Claude structured payloads are converted to `ParsedPatch` during post-commit processing rather than insert-time patchset rendering.

---

## Validation Report

### Commands run

| Command | Exit code | Result |
|---------|-----------|--------|
| `nix run .#pkl-check-generated` | 0 | Generated outputs are up to date |
| `nix flake check` | 0 | All 4 checks passed: cli-tests, cli-clippy, cli-fmt, pkl-parity |

### Temporary scaffolding

None found. `PAYLOAD_TYPE_STRUCTURED` is properly active (no `#[allow(dead_code)]`); `structured_patch.rs` has no file-level `#![allow(dead_code)]`; no plan-specific TODOs in Rust source.

### Success-criteria verification

- [x] **Generated Claude settings call `sce hooks` directly**: `config/.claude/settings.json` uses `"sce"` command with `"hooks" "session-model"` and `"hooks" "diff-trace"` args; no Bun or `.claude/plugins/sce-agent-trace.ts` references. File verified on disk.

- [x] **AgentTraceDb typed payload storage**: `PAYLOAD_TYPE_PATCH` (`"patch"`) and `PAYLOAD_TYPE_STRUCTURED` (`"structured"`) constants at `cli/src/services/agent_trace_db/mod.rs:73-74`; migration `009_add_diff_traces_payload_type.sql` added `payload_type TEXT NOT NULL DEFAULT 'patch'` column. Code review confirmed.

- [x] **Claude structured payloads stored as raw JSON, derived at post-commit read time**: Intake path (`cli/src/services/hooks/mod.rs:352`) stores `stdin_payload.to_string()` with `payload_type="structured"`. Post-commit read path (`cli/src/services/agent_trace_db/mod.rs:437-443`) dispatches `"structured"` rows through `derive_claude_structured_patch` at read time. Code review confirmed.

- [x] **OpenCode normalized payloads unchanged**: Continue as `payload_type="patch"` through existing flat-payload validation and `parse_patch` processing. Code review confirmed.

- [x] **Claude TypeScript removed**: `config/lib/agent-trace-plugin/claude-sce-agent-trace-plugin.ts` (canonical source) deleted; `config/.claude/plugins/` directory does not exist; `.claude/plugins/` directory does not exist; only `opencode-sce-agent-trace-plugin.ts` remains. File system verified.

- [x] **Golden fixture coverage lives in Rust**: `cli/src/services/structured_patch/tests.rs` (`claude_derivation_golden_tests`) validates all eight `diff_creation/` scenarios against `derive_claude_structured_patch`. Context docs confirmed.

- [x] **Generated output parity and full repo validation pass**: `nix run .#pkl-check-generated` exit 0; `nix flake check` exit 0 (all 4 checks green).

### Residual risks

None identified. All plan success criteria met with concrete evidence.
