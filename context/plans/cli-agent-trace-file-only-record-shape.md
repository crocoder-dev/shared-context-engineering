# Plan: CLI agent-trace file-only record shape

## Change summary

Update the existing Rust agent-trace payload contract so the implemented output is file-only for now and matches the requested nested file/conversation/range shape. This follow-up changes the current `AgentTrace` model away from per-conversation `new_start`/`new_count` fields toward `contributor: { type: ... }` plus `ranges: [{ start_line, end_line }]`, while explicitly leaving top-level `version`, `id`, and `timestamp` metadata out of scope.

## Success criteria

1. The implemented agent-trace payload is file-only for this change and does not add top-level `version`, `id`, or `timestamp` fields.
2. Each trace file entry continues to expose `path` plus `conversations`, but each conversation now serializes as a nested contributor object and a `ranges` array.
3. Current hunk granularity is preserved: one post-commit hunk maps to one conversation with exactly one range object derived from that hunk.
4. Range output uses the post-commit hunk's new-file line span and serializes as `start_line` and `end_line`.
5. Contributor classification remains in scope as `ai`, `mixed`, and `unknown`, now represented as `{ "type": "..." }`.
6. Tests and any checked-in golden artifacts are updated so the new payload shape is covered deterministically.
7. Final validation and cleanup confirm the new payload contract is reflected in plan progress and any required context updates.

## Constraints and non-goals

- In scope: the Rust agent-trace domain model, serialization shape, range mapping from current hunk metadata, focused test/golden updates, and required context/plan updates.
- In scope: preserving current file ordering, conversation ordering, and contributor classification semantics unless the payload shape change requires a tightly scoped test adjustment.
- Out of scope: adding top-level trace metadata (`version`, `id`, `timestamp`).
- Out of scope: changing the generator from file-only to a broader runtime/persistence/CLI surface.
- Out of scope: changing contributor taxonomy beyond `ai`, `mixed`, and `unknown`.
- Out of scope: merging multiple hunks into one conversation; this plan keeps one conversation and one range per post-commit hunk.
- Non-goal: unrelated refactors in patch parsing, hook runtime, or non-agent-trace payload code.
- Assumption: the existing agent-trace generator/tests are the source of truth for current behavior, and this plan only reshapes the serialized contract within that existing implementation seam.

## Task stack

- [x] T01: `Reshape agent-trace domain types to file-only nested range payload` (status:done)
  - Task ID: T01
  - Goal: Update the Rust agent-trace domain model and serializer-facing structures so file entries emit conversations as `contributor: { type: ... }` plus `ranges: [{ start_line, end_line }]`, without introducing top-level trace metadata.
  - Boundaries (in/out of scope): In - `AgentTrace`/`TraceFile`/`Conversation`-adjacent types, any new small support structs needed for nested contributor/range output, and preserving current file-only scope. Out - runtime integration changes, metadata generation, or broader agent-trace feature expansion.
  - Done when: the codebase has a deterministic file-only agent-trace model that serializes to the requested nested conversation/range shape and still supports `ai` / `mixed` / `unknown` classification.
  - Verification notes (commands or checks): targeted assertions should prove the serialized payload no longer uses `new_start`/`new_count` and now emits nested `contributor.type` plus `ranges.start_line`/`end_line`.
  - Completed: 2026-04-22
  - Files changed: `cli/src/services/agent_trace.rs`, `cli/src/services/agent_trace/tests.rs`, `cli/src/services/agent_trace/fixtures/*/golden.json`
  - Evidence: `nix flake check`; `nix build .#default`
  - Notes: Added nested `Contributor` and `LineRange` serializer-facing types; each conversation now serializes one derived range while preserving file-only scope and existing `ai`/`mixed`/`unknown` classification.

- [x] T02: `Map post-commit hunks to single-range conversations and update tests` (status:done)
  - Task ID: T02
  - Goal: Update `build_agent_trace` behavior and focused tests/golden artifacts so each post-commit hunk produces one conversation with exactly one derived range entry using the new payload shape.
  - Boundaries (in/out of scope): In - range derivation from post-commit hunk metadata, test updates, fixture/golden updates, and deterministic ordering assertions. Out - changing hunk classification rules, merging hunks, or adding unrelated fixture scenarios.
  - Done when: `build_agent_trace` emits one-range-per-hunk conversations, contributor output is wrapped as `{ type: ... }`, and focused tests/golden coverage passes against the new schema.
  - Verification notes (commands or checks): run the narrowest available validation covering agent-trace tests/golden assertions; verify at least one fixture-backed expected payload matches the new file-only shape exactly.
  - Completed: 2026-04-22
  - Files changed: `cli/src/services/agent_trace.rs`, `cli/src/services/agent_trace/tests.rs`, `cli/src/services/agent_trace/fixtures/*/golden.json`
  - Evidence: `nix run .#pkl-check-generated`; `nix flake check`; `nix build .#default`
  - Notes: `build_agent_trace` now wraps contributor classification as `{ type: ... }`, derives exactly one `{ start_line, end_line }` range per post-commit hunk, and includes focused multi-hunk test coverage in addition to fixture-backed goldens.

- [x] T03: `Validation and cleanup` (status:done)
  - Task ID: T03
  - Goal: Run final validation, remove any temporary scaffolding, and update plan/context artifacts required to reflect the reshaped agent-trace payload contract.
  - Boundaries (in/out of scope): In - final verification, plan evidence/status updates, and focused context sync only if current durable context describes the old payload shape. Out - new behavior beyond the approved payload reshaping.
  - Done when: validation passes, no temporary scaffolding remains, and plan/context artifacts accurately describe the file-only nested contributor/range payload.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; `nix flake check`; verify whether `context/sce/agent-trace-minimal-generator.md`, `context/overview.md`, or `context/glossary.md` require payload-shape updates.
  - Completed: 2026-04-22
  - Files changed: `context/glossary.md`, `context/context-map.md`, `context/plans/cli-agent-trace-file-only-record-shape.md`
  - Evidence: `nix run .#pkl-check-generated`; `nix flake check`; `nix build .#default`
  - Notes: Final validation passed, no temporary scaffolding was needed, and durable context drift was repaired by updating the glossary and context-map descriptions to the nested contributor/range payload shape.

## Open questions

- None.

## Validation Report

### Commands run

- `nix run .#pkl-check-generated` -> exit 0 (`Generated outputs are up to date.`)
- `nix flake check` -> exit 0 (`all checks passed!`)
- `nix build .#default` -> exit 0

### Cleanup

- Temporary scaffolding removed: none needed
- Durable context repaired: `context/glossary.md`, `context/context-map.md`

### Success-criteria verification

- [x] File-only payload remains in scope with no top-level `version`, `id`, or `timestamp` fields -> confirmed in `cli/src/services/agent_trace.rs` `AgentTrace` shape and `context/sce/agent-trace-minimal-generator.md`
- [x] Each trace file entry exposes `path` plus `conversations`, with conversations serialized as nested contributor object plus `ranges` array -> confirmed in `cli/src/services/agent_trace.rs` (`TraceFile`, `Conversation`, `Contributor`, `LineRange`) and fixture goldens under `cli/src/services/agent_trace/fixtures/*/golden.json`
- [x] One post-commit hunk maps to one conversation with exactly one range object -> confirmed by `build_agent_trace` in `cli/src/services/agent_trace.rs` and focused test `poem_edit_reconstruction_maps_each_hunk_to_one_range` in `cli/src/services/agent_trace/tests.rs`
- [x] Range output uses post-commit hunk new-file line span as `start_line` / `end_line` -> confirmed by `line_range_from_hunk` in `cli/src/services/agent_trace.rs`
- [x] Contributor classification remains `ai`, `mixed`, and `unknown` under `{ "type": "..." }` -> confirmed by `HunkContributor`, `Contributor`, and test `conversation_serializes_nested_contributor_and_ranges_shape`
- [x] Tests and checked-in goldens cover the new payload shape deterministically -> confirmed by `nix flake check` passing and updated fixture goldens in `cli/src/services/agent_trace/fixtures/*/golden.json`
- [x] Final validation and cleanup confirm plan/context accuracy -> confirmed by this report plus updated `context/sce/agent-trace-minimal-generator.md`, `context/glossary.md`, and `context/context-map.md`

### Failed checks and follow-ups

- None.

### Residual risks

- None identified within the approved scope.
