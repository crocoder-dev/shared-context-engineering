# Plan: restrict-conversation-trace-tool-name

## Change summary

`sce hooks conversation-trace` currently accepts any non-empty `tool_name` for
normalized (non-Claude-raw) envelopes. The shared `prefixed_session_id()`
helper in `cli/src/services/hooks/mod.rs` silently falls through to the raw,
unprefixed session ID for any `tool_name` it does not recognize, so an unknown
producer (for example `tool_name: "cursor"`) is persisted with an unprefixed
session ID instead of being rejected. This extends existing behavior: it
tightens validation for the normalized conversation-trace entry point
(`parse_conversation_trace_payload`, around `cli/src/services/hooks/mod.rs:512`)
to require `tool_name` to be one of the currently supported normalized
producers (`opencode`, `pi`), erroring with the supported values named when it
is not. Raw Claude hook events (routed via `hook_event_name`) are untouched:
they keep deriving `claude` identity internally and are not gated by the new
allow-list, since that identity never comes from untrusted normalized input.
`diff-trace` intake, which independently accepts an unrestricted `tool_name`
by design (documented in `context/sce/agent-trace-hooks-command-routing.md`),
is out of scope and is not modified.

## Acceptance criteria

- [x] AC1: A normalized conversation-trace envelope with `tool_name: "opencode"` persists message/part rows with `oc_`-prefixed session IDs.
  - Validate: `cargo test --manifest-path cli/Cargo.toml services::hooks -- conversation_trace` (run via `nix flake check`, `checks.cli-tests`, passing)
- [x] AC2: A normalized conversation-trace envelope with `tool_name: "pi"` persists message/part rows with `pi_`-prefixed session IDs.
  - Validate: `cargo test --manifest-path cli/Cargo.toml services::hooks -- conversation_trace` (run via `nix flake check`, `checks.cli-tests`, passing)
- [x] AC3: A normalized conversation-trace envelope with an unsupported `tool_name` (for example `"cursor"`) is rejected with a conversation-trace validation error naming the supported producer set, and no row is persisted with an unprefixed session ID.
  - Validate: `cargo test --manifest-path cli/Cargo.toml services::hooks -- conversation_trace` (run via `nix flake check`, `checks.cli-tests`, passing)
- [x] AC4: A normalized conversation-trace envelope with an empty or missing `tool_name` is still rejected (no backward-compatibility fallback).
  - Validate: `cargo test --manifest-path cli/Cargo.toml services::hooks -- conversation_trace` (run via `nix flake check`, `checks.cli-tests`, passing)
- [x] AC5: A raw Claude conversation event (`hook_event_name` present) still derives `claude` identity internally, unaffected by the new allow-list, and persists with `cc_`-prefixed session IDs.
  - Validate: `cargo test --manifest-path cli/Cargo.toml services::hooks -- conversation_trace` (run via `nix flake check`, `checks.cli-tests`, passing)
- [x] AC6: An already-prefixed session ID (`oc_`, `pi_`, or `cc_`) for a valid producer is left unchanged (idempotent), not double-prefixed.
  - Validate: `cargo test --manifest-path cli/Cargo.toml services::hooks -- conversation_trace` (run via `nix flake check`, `checks.cli-tests`, passing)

### Full validation

- `nix flake check`
- `nix run .#pkl-check-generated`

### Context sync

- `context/sce/agent-trace-hooks-command-routing.md`: replace the current "Normalized envelopes require a non-empty `tool_name`" statement with the restricted supported-producer-set contract for conversation-trace, while leaving the diff-trace `tool_name` description (which stays unrestricted) unchanged.

## Constraints and non-goals

- **In scope:** `cli/src/services/hooks/mod.rs` normalized conversation-trace entry validation (`parse_conversation_trace_payload` and its normalized branch), its unit tests, and `context/sce/agent-trace-hooks-command-routing.md`.
- **Out of scope:** `diff-trace` intake and its `tool_name` handling; the shared `prefixed_session_id()` helper's fallback arm as used by `diff-trace` (must keep working for arbitrary producer names there); adding `claude` as an accepted normalized producer value; any generic/pluggable producer registry.
- **Constraints:** Preserve the existing idempotent `oc_`/`pi_`/`cc_` prefixing behavior for valid producers; keep the raw-Claude-event path (`hook_event_name`-driven) deriving `claude` identity internally without going through the new allow-list.
- **Non-goal:** Introducing a generic/extensible producer namespace, or refactoring `diff-trace`'s independent `tool_name` contract.

## Assumptions

- The allow-list check is added at the normalized conversation-trace entry point (before `parse_conversation_trace_payloads` is called with an externally supplied `tool_name`), rather than inside the shared `prefixed_session_id()` helper, because that helper's permissive fallback is still required by `diff-trace`'s separate, intentionally unrestricted `tool_name` contract, and the change request scopes the fix to "normalized conversation-trace payloads" only.
- The internal raw-Claude call site (`parse_conversation_trace_payloads(&items, CLAUDE_TOOL_NAME)`) is not routed through the new allow-list check, since `CLAUDE_TOOL_NAME` is an internal constant, not attacker/producer-controlled input, matching the request's instruction not to add `claude` as a normalized producer.

## Task stack

- [x] T01: `Reject unsupported normalized conversation-trace tool_name values` (status:done)
  - Task ID: T01
  - Goal: `sce hooks conversation-trace` rejects normalized envelopes whose `tool_name` is not `opencode` or `pi`, with a clear error naming the supported values, while `opencode`/`pi`/raw-Claude paths keep working exactly as before.
  - Boundaries (in/out of scope): In — the normalized-branch validation in `parse_conversation_trace_payload` (`cli/src/services/hooks/mod.rs`), its unit tests, and the `context/sce/agent-trace-hooks-command-routing.md` doc update. Out — `diff-trace` code/tests/docs, the shared `prefixed_session_id()` fallback arm, any new producer-registry abstraction.
  - Dependencies: none
  - Done when: the normalized conversation-trace entry validates `tool_name` against `{"opencode", "pi"}` and returns a `conversation_trace_validation_error` naming the supported values for any other non-empty value; empty/missing `tool_name` remains rejected by the existing `required_non_empty_string_field` check; tests for AC1-AC6 pass; `context/sce/agent-trace-hooks-command-routing.md` reflects the restricted contract.
  - Verification notes (commands or checks): `cargo test --manifest-path cli/Cargo.toml services::hooks`; `cargo clippy --manifest-path cli/Cargo.toml -- -D warnings`.
  - Implementation evidence: Added `NORMALIZED_CONVERSATION_TRACE_TOOL_NAMES = ["opencode", "pi"]` and an allow-list check in `parse_conversation_trace_payload` (`cli/src/services/hooks/mod.rs`) right after the existing `tool_name` non-empty check, bailing with `conversation_trace_validation_error` naming the supported producers for any other value. The raw-Claude branch (`hook_event_name` present) is unchanged and still calls `parse_conversation_trace_payloads(&items, CLAUDE_TOOL_NAME)` directly, bypassing the new allow-list. Added six unit tests covering AC2-AC6 (`conversation_trace_normalized_payload_accepts_pi_tool_name_with_prefixed_session_id`, `conversation_trace_normalized_payload_rejects_unsupported_tool_name`, `conversation_trace_normalized_payload_rejects_empty_tool_name`, `conversation_trace_normalized_payload_rejects_missing_tool_name`, `conversation_trace_normalized_payload_keeps_already_prefixed_session_id`, `conversation_trace_raw_claude_event_uses_claude_identity_with_cc_prefixed_session_id`); AC1 is already covered by the existing `conversation_trace_mixed_payload_maps_to_message_and_part_insert_inputs` test. Updated `context/sce/agent-trace-hooks-command-routing.md:78` to describe the restricted `{opencode, pi}` producer set for conversation-trace while leaving the `diff-trace` `tool_name` description unchanged.
  - Verification outcome: `nix flake check` — all checks passed (includes `checks.cli-tests` running the full cargo test suite, and `checks.cli-clippy` running `cargo clippy -- -D warnings`). Direct `cargo test`/`cargo clippy` invocations are blocked by this repository's bash-tool policy (`use-nix-flake-check-over-cargo-test`), so `nix flake check` was run in place of the plan's literal `cargo test --manifest-path ...` / `cargo clippy --manifest-path ...` commands; it exercises the same test and clippy targets.
  - Deviations/assumptions: None beyond the plan's stated assumptions.

## Open questions

None. The change request's six named test cases plus its explicit non-goals (no `claude` normalized producer, no generic producer namespace, `diff-trace` untouched) fully pin down where the new validation belongs and what it must and must not affect.

## Validation Report

**Status:** validated  
**Date:** 2026-08-15

### Commands run

- `nix flake check` -> exit 0 (all checks passed, including `checks.cli-tests`: 347 passed, 0 failed)
- `nix run .#pkl-check-generated` -> exit 0 (Ephemeral Pkl generation passed: 101 files)

### Scaffolding removed

- None.

### Success-criteria verification

- [x] AC1: Normalized `tool_name: "opencode"` persists `oc_`-prefixed session IDs -> `conversation_trace_mixed_payload_maps_to_message_and_part_insert_inputs` passed.
- [x] AC2: Normalized `tool_name: "pi"` persists `pi_`-prefixed session IDs -> `conversation_trace_normalized_payload_accepts_pi_tool_name_with_prefixed_session_id` passed.
- [x] AC3: Unsupported `tool_name` (e.g. `"cursor"`) is rejected naming the supported producer set, no unprefixed row persisted -> `conversation_trace_normalized_payload_rejects_unsupported_tool_name` passed; diff confirms the allow-list check runs before any payload parsing.
- [x] AC4: Empty or missing `tool_name` is rejected -> `conversation_trace_normalized_payload_rejects_empty_tool_name` and `conversation_trace_normalized_payload_rejects_missing_tool_name` passed.
- [x] AC5: Raw Claude event derives `claude` identity internally, unaffected by the allow-list, persists `cc_`-prefixed session ID -> `conversation_trace_raw_claude_event_uses_claude_identity_with_cc_prefixed_session_id` passed; diff confirms the raw-Claude branch calls `parse_conversation_trace_payloads(&items, CLAUDE_TOOL_NAME)` directly, bypassing the new allow-list.
- [x] AC6: Already-prefixed session ID for a valid producer is left unchanged -> `conversation_trace_normalized_payload_keeps_already_prefixed_session_id` passed.

### Failed checks and follow-ups

- None.

### Residual risks

- None identified.
