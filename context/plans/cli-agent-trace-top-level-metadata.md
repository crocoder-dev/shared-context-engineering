# Plan: CLI agent-trace top-level metadata fields

## Change summary

Extend the current minimal agent-trace payload in `cli/src/services/agent_trace.rs` to include the top-level fields `version`, `id`, and `timestamp` while preserving the existing file/conversation/range shape and contributor taxonomy (`ai`, `mixed`, `unknown`).

User-confirmed decisions:

- `version` is fixed to `"v0.1.0"` for now.
- `id` must be a UUID string and is generated internally.
- `timestamp` must be an RFC 3339 date-time string and is generated internally.

## Success criteria

1. `AgentTrace` serializes top-level `version`, `id`, `timestamp`, and `files`.
2. `version` is always `"v0.1.0"`.
3. `id` is generated per `build_agent_trace(...)` call as a valid UUID string.
4. `timestamp` is generated per `build_agent_trace(...)` call as a valid RFC 3339 date-time string.
5. Existing file-level payload semantics remain unchanged (`files[].path`, `conversations[]`, nested `contributor.type`, `ranges[].start_line/end_line`).
6. Tests cover presence/format of new top-level fields and guard that existing nested payload structure still matches current behavior.
7. Final validation and cleanup are completed, and any required context updates are recorded.

## Constraints and non-goals

- In scope: Rust agent-trace domain model update, metadata generation in `build_agent_trace`, targeted tests and fixture/golden adjustments where needed.
- In scope: preserving existing contributor enum values (`ai`, `mixed`, `unknown`) without adding `human`.
- Out of scope: adding optional schema sections (`vcs`, `tool`, `metadata`, conversation URLs/related links, range content hashes).
- Out of scope: changing hook/runtime integration or adding CLI command surface.
- Out of scope: redesigning hunk-classification logic or hunk-to-conversation mapping.

## Task stack

- [x] T01: `Add top-level metadata fields to AgentTrace model` (status:done)
  - Task ID: T01
  - Goal: Update the agent-trace domain types to include top-level `version`, `id`, and `timestamp` with stable serialization names.
  - Boundaries (in/out of scope): In - `AgentTrace` struct shape and serialization contract, any supporting metadata type aliases/helpers needed for clarity. Out - changing nested `TraceFile` / `Conversation` / `LineRange` schema beyond compatibility-preserving adjustments.
  - Done when: serialized `AgentTrace` includes `version`, `id`, `timestamp`, and `files`, with `version` fixed at `"v0.1.0"`.
  - Verification notes (commands or checks): targeted serialization assertions demonstrating top-level key presence and fixed version value.
  - Completed: 2026-04-23
  - Files changed: `cli/src/services/agent_trace.rs`, `cli/src/services/agent_trace/tests.rs`
  - Evidence: `nix run .#pkl-check-generated` (pass); `nix build .#checks.x86_64-linux.cli-tests` (pass); `nix build .#default` (pass); `nix flake check` (fails on pre-existing `cli/src/services/patch/tests.rs` fmt drift)
  - Notes: Added top-level `version`/`id`/`timestamp` fields to `AgentTrace` with `version` fixed to `v0.1.0`; preserved nested trace payload structure and added focused serialization coverage for top-level fields.

- [x] T02: `Generate id and timestamp inside build_agent_trace` (status:done)
  - Task ID: T02
  - Goal: Implement internal generation of UUID `id` and RFC 3339 `timestamp` at trace build time.
  - Boundaries (in/out of scope): In - deterministic generation flow per invocation, validation-friendly string formats, and constructor/orchestration updates in `build_agent_trace`. Out - introducing caller-supplied metadata injection path.
  - Done when: each `build_agent_trace(...)` result carries a non-empty UUID-formatted `id` and RFC 3339 `timestamp` string, while file/conversation/range output remains unchanged.
  - Verification notes (commands or checks): focused tests that parse `id` as UUID and `timestamp` as RFC 3339 date-time; assertions that existing nested payload fields still serialize as before.
  - Completed: 2026-04-23
  - Files changed: `cli/Cargo.toml`, `cli/src/services/agent_trace.rs`, `cli/src/services/agent_trace/tests.rs`
  - Evidence: `nix run .#pkl-check-generated` (pass); `nix build .#checks.x86_64-linux.cli-fmt` (pass); `nix build .#checks.x86_64-linux.cli-clippy` (pass); `nix build .#checks.x86_64-linux.cli-tests` (pass); `nix build .#default` (pass)
  - Notes: `build_agent_trace` now generates UUIDv4 `id` and RFC 3339 `timestamp` per invocation; tests validate metadata formats while continuing to assert unchanged nested file/conversation/range payload semantics.

- [x] T03: `Refresh goldens/tests for top-level metadata contract` (status:done)
  - Task ID: T03
  - Goal: Update affected tests/fixtures to match the enriched top-level shape without regressing current per-file/per-hunk semantics.
  - Boundaries (in/out of scope): In - test/golden updates under `cli/src/services/agent_trace/` and helper assertions that tolerate generated dynamic values safely. Out - unrelated fixture scenario expansion.
  - Done when: all relevant agent-trace tests validate new metadata fields and continue validating existing nested payload semantics.
  - Verification notes (commands or checks): run the narrowest agent-trace-focused tests first, then repo-preferred checks used by this project.
  - Completed: 2026-04-23
  - Files changed: `cli/src/services/agent_trace/tests.rs`, `cli/src/services/agent_trace/fixtures/average_age_reconstruction/golden.json`, `cli/src/services/agent_trace/fixtures/hello_world_reconstruction/golden.json`, `cli/src/services/agent_trace/fixtures/mixed_change_reconstruction/golden.json`, `cli/src/services/agent_trace/fixtures/poem_edit_reconstruction/golden.json`, `cli/src/services/agent_trace/fixtures/poem_write_reconstruction/golden.json`, `cli/src/services/agent_trace/fixtures/text_file_lifecycle_reconstruction/golden.json`
  - Evidence: `nix run .#pkl-check-generated` (pass); `nix build .#checks.x86_64-linux.cli-tests` (pass); `nix build .#checks.x86_64-linux.cli-fmt` (pass); `nix build .#checks.x86_64-linux.cli-clippy` (pass); `nix build .#default` (pass); `nix flake check` (pass)
  - Notes: Refreshed all agent-trace goldens to include top-level metadata keys (`version`, `id`, `timestamp`) with stable placeholders for generated values; updated test helper to assert UUID/RFC3339 formats and compare full serialized payloads after normalizing dynamic metadata.

- [x] T04: `Validation and cleanup` (status:done)
  - Task ID: T04
  - Goal: Complete final verification, remove temporary scaffolding, and sync context artifacts if durable contracts changed.
  - Boundaries (in/out of scope): In - final validation pass, plan status/evidence updates, and focused context sync (e.g., `context/sce/agent-trace-minimal-generator.md`, `context/glossary.md`, `context/context-map.md`) only where code truth changed. Out - any new feature scope beyond metadata fields.
  - Done when: validation evidence is recorded, no temporary scaffolding remains, and context reflects the current top-level metadata contract.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; `nix flake check`; verify targeted context files for drift.
  - Completed: 2026-04-23
  - Files changed: `context/plans/cli-agent-trace-top-level-metadata.md`
  - Evidence: `nix run .#pkl-check-generated` (pass); `nix flake check` (pass); verify-only context sync pass completed for `context/overview.md`, `context/architecture.md`, `context/glossary.md`, `context/patterns.md`, and `context/context-map.md` with no additional root edits required.
  - Notes: No task-scoped temporary scaffolding was introduced by this task; existing agent-trace context coverage already reflects top-level metadata (`version`, `id`, `timestamp`) and remains linked from `context/context-map.md`.

## Open questions

- None.

## Validation Report

### Commands run

- `nix run .#pkl-check-generated` -> exit 0 (`Generated outputs are up to date.`)
- `nix flake check` -> exit 0 (`all checks passed!`), covering repo check derivations including CLI tests, clippy, and fmt checks.

### Temporary scaffolding cleanup

- No temporary scaffolding was introduced by this task.
- No cleanup deletions were required for T04 scope.

### Success-criteria verification

- [x] 1. `AgentTrace` serializes top-level `version`, `id`, `timestamp`, and `files`.
  - Evidence: T01/T03 test updates in `cli/src/services/agent_trace/tests.rs` and refreshed fixtures in `cli/src/services/agent_trace/fixtures/**/golden.json`.
- [x] 2. `version` is always `"v0.1.0"`.
  - Evidence: T01 model + serialization assertions (`cli/src/services/agent_trace.rs`, `cli/src/services/agent_trace/tests.rs`).
- [x] 3. `id` is generated per `build_agent_trace(...)` call as a valid UUID string.
  - Evidence: T02 generator changes and UUID-format assertions in `cli/src/services/agent_trace/tests.rs`.
- [x] 4. `timestamp` is generated per `build_agent_trace(...)` call as a valid RFC 3339 date-time string.
  - Evidence: T02 generator changes and RFC 3339 parsing assertions in `cli/src/services/agent_trace/tests.rs`.
- [x] 5. Existing file-level payload semantics remain unchanged.
  - Evidence: T03 normalized-golden comparisons preserve `files[].path`, `conversations[].contributor.type`, and `ranges[].start_line/end_line` semantics.
- [x] 6. Tests cover presence/format of new top-level fields and guard nested payload shape.
  - Evidence: T03 test helper + fixture refresh; `nix flake check` passed.
- [x] 7. Final validation and cleanup are completed, and required context updates are recorded.
  - Evidence: T04 completion entry, successful final checks, and verify-only context sync confirmation across root files with domain coverage present in `context/sce/agent-trace-minimal-generator.md` and discoverable via `context/context-map.md`.

### Failed checks and follow-ups

- None.

### Residual risks

- None identified for this scoped change.
