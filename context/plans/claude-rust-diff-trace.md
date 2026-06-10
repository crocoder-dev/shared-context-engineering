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
- `sce hooks diff-trace` can accept Claude structured `PostToolUse` payloads and derive the same patch output currently covered by the `diff_creation` golden fixtures.
- Existing OpenCode normalized `diff-trace` payloads remain accepted and behaviorally unchanged.
- Claude TypeScript plugin source, generated plugin output, and Claude TypeScript golden tests are removed from the repo-owned Claude path.
- Golden fixture coverage for Claude diff derivation lives in Rust and validates the checked-in `cli/src/services/patch/fixtures/diff_creation/` scenarios.
- Generated output parity and full repo validation pass.

## Constraints and non-goals

- In scope: Rust hook intake/derivation, Rust tests, Pkl-generated Claude settings, generated output updates, context sync.
- In scope: removing Claude-specific TypeScript plugin source/tests and generated `.claude/plugins` output.
- Out of scope: changing OpenCode TypeScript plugin behavior or removing OpenCode TypeScript runtime code.
- Out of scope: changing post-commit Agent Trace payload semantics, AgentTraceDb schema, or OpenCode plugin registration.
- Out of scope: adding a new external dependency unless implementation proves the existing Rust stack cannot parse the structured Claude payload safely.
- Preserve one-task/one-atomic-commit slicing; each executable task below should land independently.

## Task stack

- [x] T01: `Add Rust Claude hook payload derivation model` (status:done)
  - Task ID: T01
  - Goal: Add Rust data models and pure derivation helpers that convert supported Claude `PostToolUse` structured payloads into the existing normalized diff-trace shape.
  - Boundaries (in/out of scope): In - Rust-only parsing/normalization helpers for Claude `Write` create and `Edit` structured-patch payloads, status/skip reasons matching the current supported cases, no CLI routing changes. Out - generated settings changes, TypeScript deletion, DB persistence changes, OpenCode flow changes.
  - Done when: Rust exposes a testable pure function that accepts event name + JSON payload + fixed time/tool version inputs and returns derived `{ sessionID, diff, time, tool_name="claude", tool_version }` or deterministic skip/error results for unsupported payloads.
  - Verification notes (commands or checks): Run the narrow Rust tests added for the derivation helper via `nix develop -c sh -c 'cd cli && cargo test claude'` if a narrow test target exists; otherwise use the narrowest relevant `cargo test` selector. Final full validation remains T07.
  - Completion evidence (2026-06-10): Added synchronous Rust `structured_patch` service module; Claude `PostToolUse` `Write` and `Edit` structured payload derivation returns `ParsedPatch`-backed `ClaudeStructuredPatch` results with deterministic skip reasons and fixed time/tool-version inputs. Generated helper tests were removed after review; golden fixture coverage remains deferred to T02. `nix flake check` passed. `nix run .#pkl-check-generated` passed. Direct narrow `cargo test claude` was not run because the repo bash policy blocks direct Cargo test commands in favor of `nix flake check`.

- [ ] T02: `Move Claude diff-creation golden tests to Rust` (status:todo)
  - Task ID: T02
  - Goal: Recreate the current Claude derivation golden coverage in Rust against `cli/src/services/patch/fixtures/diff_creation/`.
  - Boundaries (in/out of scope): In - Rust tests that discover/validate the expected eight fixture scenarios and assert derived patch equality using `claude-post-tool-use.json` plus `expected.patch`. Out - deleting TypeScript tests/source, modifying fixture contents except for necessary fixture-contract corrections, changing OpenCode tests.
  - Done when: Rust tests fail on missing/extra scenarios, use fixed time/tool-version inputs, assert `sessionID`, `tool_name="claude"`, nullable/omitted `model_id` behavior as appropriate, and exact diff output for each golden fixture.
  - Verification notes (commands or checks): Run the narrow Rust golden test selector through Nix, for example `nix develop -c sh -c 'cd cli && cargo test claude_derivation'` once test names are known.

- [ ] T03: `Teach sce hooks diff-trace to accept Claude structured payloads` (status:todo)
  - Task ID: T03
  - Goal: Extend the Rust `sce hooks diff-trace` STDIN intake so Claude structured `PostToolUse` payloads are derived in Rust and then pass through the existing diff-trace persistence path.
  - Boundaries (in/out of scope): In - payload classification, validation errors/skips, derivation-to-existing `DiffTracePayload` adapter, tests for Claude structured payload runtime path and existing normalized payload compatibility. Out - new DB schema, post-commit flow changes, OpenCode plugin changes, generated settings changes.
  - Done when: `diff-trace` accepts both the existing normalized payload and the new Claude structured payload; Claude unsupported/no-diff cases produce deterministic success/no-op or validation behavior consistent with current hook semantics; OpenCode normalized payload tests still pass unchanged.
  - Verification notes (commands or checks): Run focused hooks tests through Nix, for example `nix develop -c sh -c 'cd cli && cargo test hooks'`; include a targeted exact test when available.

- [ ] T04: `Render Claude settings with direct sce hook commands` (status:todo)
  - Task ID: T04
  - Goal: Update canonical Pkl-generated Claude settings so Claude invokes `sce hooks session-model` and `sce hooks diff-trace` directly instead of running Bun against `.claude/plugins/sce-agent-trace.ts`.
  - Boundaries (in/out of scope): In - `config/pkl/renderers/claude-content.pkl` settings command definitions and regenerated `config/.claude/settings.json` / repo-root `.claude/settings.json` outputs. Out - OpenCode renderer/plugin registration, agent/skill content changes unrelated to settings, manual edits to generated outputs without source updates.
  - Done when: Generated Claude settings contain no `.claude/plugins/sce-agent-trace.ts` or `bun` hook invocation for agent tracing, and route `SessionStart` to `sce hooks session-model` while routing matched `PostToolUse` to `sce hooks diff-trace` with Claude hook payload on STDIN according to Claude hook command behavior.
  - Verification notes (commands or checks): Run `nix develop -c pkl eval -m . config/pkl/generate.pkl` after source edits, then `nix run .#pkl-check-generated`.

- [ ] T05: `Remove Claude TypeScript plugin source and generated outputs` (status:todo)
  - Task ID: T05
  - Goal: Delete the now-obsolete Claude TypeScript agent-trace runtime and its TypeScript golden tests while preserving OpenCode TypeScript plugin/runtime code.
  - Boundaries (in/out of scope): In - remove `config/lib/agent-trace-plugin/claude-sce-agent-trace-plugin.ts`, its Bun test, generated `config/.claude/plugins/sce-agent-trace.ts`, generated root `.claude/plugins/sce-agent-trace.ts`, and references that assume a Claude plugin path exists. Out - `config/lib/agent-trace-plugin/opencode-sce-agent-trace-plugin.ts`, OpenCode generated plugins, bash-policy code.
  - Done when: No repo-owned Claude `.claude/plugins` agent-trace TypeScript remains; config-lib package/test configuration no longer expects the deleted Claude test; generated output parity is clean after regeneration.
  - Verification notes (commands or checks): Run `nix run .#pkl-check-generated`; run the relevant config-lib checks only if package/test manifests changed, otherwise rely on T07 full validation.

- [ ] T06: `Sync current-state context for Rust-owned Claude tracing` (status:todo)
  - Task ID: T06
  - Goal: Update durable context to describe the new Rust-owned Claude derivation boundary and removal of Claude TypeScript plugin runtime.
  - Boundaries (in/out of scope): In - focused updates to `context/sce/opencode-agent-trace-plugin-runtime.md`, `context/sce/agent-trace-hooks-command-routing.md`, `context/cli/patch-service.md`, `context/context-map.md`, `context/overview.md`, and glossary/architecture entries if needed. Out - historical narration beyond current-state facts, unrelated context cleanup.
  - Done when: Context says OpenCode still uses TypeScript normalized diff traces, Claude settings call `sce hooks` directly, Rust derives Claude structured patches, and golden tests are Rust-owned.
  - Verification notes (commands or checks): Review context references for stale `.claude/plugins/sce-agent-trace.ts`, Claude TypeScript golden test, and shared TypeScript-runtime-to-Rust boundary claims.

- [ ] T07: `Validate and clean up Claude Rust diff-trace migration` (status:todo)
  - Task ID: T07
  - Goal: Run final validation, remove temporary scaffolding, and record plan completion evidence.
  - Boundaries (in/out of scope): In - full repo validation, generated-output parity, checking for stale Claude TypeScript references, updating this plan with validation evidence. Out - new feature work or unrelated refactors discovered during validation.
  - Done when: `nix run .#pkl-check-generated` and `nix flake check` pass; no stale Claude plugin TypeScript files/references remain except intentional historical references; plan status/evidence is updated.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; `nix flake check`; targeted search for `.claude/plugins/sce-agent-trace.ts`, `deriveClaudeDiffTracePayload`, and Claude TypeScript golden-test references.

## Open questions

- None blocking. User clarified that Claude derivation should happen fully in Rust, Claude TypeScript should be removed, OpenCode TypeScript should remain, and generated Claude settings should call `sce hooks` directly.
