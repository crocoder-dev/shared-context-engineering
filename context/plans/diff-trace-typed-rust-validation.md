# diff-trace typed Rust validation

## Change summary

Refactor the Rust-only `sce hooks diff-trace` intake path in `cli/src/services/hooks.rs` from loose `serde_json::Value` payload handling to an explicit typed payload with standardized validation. The change should preserve the existing command surface and artifact ownership while tightening `time` validation to `u64` Unix epoch milliseconds, improving validation diagnostics, and making trace artifact filenames collision-safe across separate short-lived `sce hooks diff-trace` processes. TypeScript plugin behavior, generated OpenCode assets, and context docs are out of scope unless needed by final validation notes.

## Success criteria

- `sce hooks diff-trace` parses STDIN into a dedicated Rust payload type instead of keeping the parsed payload as generic `serde_json::Value`.
- `sessionID` and `diff` remain required non-empty strings after trimming checks.
- `time` is required to be a `u64` Unix epoch millisecond value; negative, fractional, string, missing, or null values are rejected with deterministic diagnostics.
- Persisted diff-trace artifacts keep the same JSON field names: `sessionID`, `diff`, and `time`.
- Trace artifact filenames are collision-safe across separate `sce hooks diff-trace` process invocations, including multiple invocations within the same millisecond.
- Trace artifact filenames use a zero-padded attempt suffix before the trace name, for example `2026-04-27T12-38-03-001Z-000000-diff-trace.json` then `2026-04-27T12-38-03-001Z-000001-diff-trace.json`.
- Validation/error handling is clearer and standardized for this path.
- No new tests are created as part of this plan, per user constraint.

## Constraints and non-goals

- Rust-only scope: modify `cli/` Rust code only.
- Do not modify TypeScript OpenCode plugin source or generated plugin copies.
- Do not create new tests.
- Do not change the `sce hooks diff-trace` CLI invocation shape.
- Do not introduce new dependencies.
- Do not wire diff-trace into local DB, retry queue, git notes, or broader Agent Trace generation.
- Do not rely on a process-local-only sequence counter as the sole filename collision guard for hook trace artifacts.
- Keep each implementation task as one atomic commit unit.

## Task stack

- [x] T01: `Introduce typed diff-trace payload parsing` (status:done)
  - Task ID: T01
  - Goal: Replace generic `serde_json::Value` parsing in `cli/src/services/hooks.rs` with a dedicated Rust `DiffTracePayload` type that serializes back to the existing JSON field names.
  - Boundaries (in/out of scope): In - Rust payload struct, serde derive/field naming, parser return type changes, persistence function signatures needed to accept the typed payload. Out - TypeScript plugin changes, generated assets, tests, docs.
  - Done when: The diff-trace path compiles conceptually around a typed payload; persisted JSON still contains `sessionID`, `diff`, and `time`; no generic `Value` is needed for the accepted diff-trace payload after parsing.
  - Verification notes (commands or checks): Inspect `cli/src/services/hooks.rs` for a dedicated payload type and unchanged persisted field names; run targeted compile/lint checks if available in the implementation session, preferring `nix flake check` when practical.
  - Completion evidence: Added `DiffTracePayload` with serde `sessionID` field rename; `parse_diff_trace_payload` now returns the typed accepted payload; persistence accepts the typed payload and serializes the same artifact field names. Ran `nix develop -c sh -c 'cd cli && cargo fmt'`, `nix develop -c sh -c 'cd cli && cargo check'`, `nix develop -c sh -c 'cd cli && cargo clippy'`, `nix run .#pkl-check-generated`, and `nix flake check` successfully on 2026-04-27. No tests were added.

- [x] T02: `Standardize diff-trace payload validation diagnostics` (status:done)
  - Task ID: T02
  - Goal: Centralize and clarify validation rules for `sessionID`, `diff`, and `time`, with `time` accepted only as `u64` Unix epoch milliseconds.
  - Boundaries (in/out of scope): In - validation helper(s), deterministic error wording for missing/wrong/empty fields, `u64` enforcement for `time`. Out - new tests, TypeScript plugin changes, command-surface changes.
  - Done when: Missing, wrong-type, empty-string, negative, and fractional `time` cases have deterministic validation messages; the accepted payload stores `time` as `u64`; command success behavior remains unchanged.
  - Verification notes (commands or checks): Review validation branches for all required field classes; optionally run manual `sce hooks diff-trace` examples in the implementation session if the binary is available; prefer repo validation via `nix flake check` when practical.
  - Completion evidence: `DiffTracePayload.time` now stores a `u64`; diff-trace validation now uses centralized helpers for required fields, non-empty strings, and u64 Unix epoch millisecond validation. Manual validation examples confirmed deterministic diagnostics for negative, fractional, string, missing `time`, and empty `sessionID`. Ran `nix develop -c sh -c 'cd cli && cargo fmt'`, `nix develop -c sh -c 'cd cli && cargo check'`, and `nix develop -c sh -c 'cd cli && cargo clippy'` successfully on 2026-04-27. No tests were added.

- [x] T03: `Make hook trace filenames collision-safe across processes` (status:done)
  - Task ID: T03
  - Goal: Replace or augment the current process-local sequence-based trace filename suffix so repeated short-lived `sce hooks diff-trace` invocations cannot overwrite each other when they occur in the same millisecond.
  - Boundaries (in/out of scope): In - Rust filename generation/persistence path in `cli/src/services/hooks.rs`, `OpenOptions::create_new(true)` atomic create/retry behavior, zero-padded attempt suffixes, preserving readable sanitized trace-name suffixes. Out - TypeScript plugin changes, generated assets, local DB persistence, new dependencies, new tests.
  - Done when: Filename construction no longer depends on `AtomicU64` process-local state; persistence attempts `000000`, then `000001`, etc. for the same timestamp/trace-name base; file creation uses atomic create-new semantics so concurrent processes cannot overwrite each other; existing hook trace persistence continues to use sanitized trace-name suffixes.
  - Verification notes (commands or checks): Inspect `build_trace_file_name`/persistence flow for `YYYY-MM-DDTHH-MM-SS-mmmZ-000000-trace-name.json` formatting and `create_new(true)` collision safety; if running manually, invoke `sce hooks diff-trace` multiple times quickly and confirm distinct artifacts; prefer `nix flake check` when practical.
  - Completion evidence: Removed the process-local `AtomicU64` trace filename sequence; trace persistence now builds filenames with a shared timestamp plus zero-padded attempt suffixes starting at `000000`, writes with `OpenOptions::create_new(true)`, and retries on `AlreadyExists` without overwriting existing artifacts. The shared persistence helper preserves sanitized trace-name suffixes for diff-trace and existing hook trace artifacts. Ran `nix develop -c sh -c 'cd cli && cargo fmt'`, `nix develop -c sh -c 'cd cli && cargo check'`, `nix develop -c sh -c 'cd cli && cargo clippy'`, `nix run .#pkl-check-generated`, and `nix flake check` successfully on 2026-04-27. No tests were added.

- [x] T04: `Validation and cleanup` (status:done)
  - Task ID: T04
  - Goal: Verify the Rust-only refactor is coherent, formatted, and does not require context/docs updates beyond this active plan.
  - Boundaries (in/out of scope): In - format/lint/build validation, generated-output parity check if touched files could affect generated outputs, removal of incidental scaffolding. Out - adding tests, broad docs rewrites, TypeScript/generated plugin changes.
  - Done when: Required verification commands either pass or have captured follow-up notes; no temporary files or accidental generated output edits remain; context sync is verify-only unless implementation discovers a durable behavior/contract change beyond typed validation.
  - Verification notes (commands or checks): Preferred full validation: `nix flake check`. If generated outputs were not touched, `nix run .#pkl-check-generated` can be used as a parity confidence check but should remain unchanged. Confirm no tests were added.
  - Completion evidence: Ran `nix run .#pkl-check-generated` successfully; generated outputs are up to date. Ran `nix flake check` successfully; all checks passed. `git diff --name-only` reported no unstaged changes before validation, and staged file inventory contained no test-file additions. No tests were added. Context sync classification: verify-only for T04; existing context already reflects final diff-trace behavior and no additional context edits were required.

## Validation Report

### Commands run

- `git status --short` -> exit 0; showed staged implementation/context/plan changes and no untracked test files.
- `git diff --name-only` -> exit 0; no unstaged changes before validation.
- `git diff --cached --name-only` -> exit 0; staged inventory contained `cli/src/services/hooks.rs`, context files, and this plan file; no test-file additions.
- `nix run .#pkl-check-generated` -> exit 0; generated outputs are up to date.
- `nix flake check` -> exit 0; all checks passed.
- `git status --short` after validation -> exit 0; no generated-output drift or new test files reported. This plan file has an unstaged validation-report update from the final validation step.

### Success-criteria verification

- [x] `sce hooks diff-trace` parses STDIN through a dedicated Rust payload type: confirmed in `cli/src/services/hooks.rs` with `DiffTracePayload` returned by `parse_diff_trace_payload`.
- [x] `sessionID` and `diff` remain required non-empty strings: confirmed by `required_non_empty_string_field` validation.
- [x] `time` is required as `u64` Unix epoch milliseconds: confirmed by `DiffTracePayload.time: u64` and `required_u64_millisecond_field` rejection branches for negative/fractional/wrong-type values.
- [x] Persisted artifact field names remain `sessionID`, `diff`, and `time`: confirmed by serde rename on `session_id` and typed payload serialization.
- [x] Trace artifact filenames are collision-safe across processes: confirmed by `OpenOptions::create_new(true)` retry loop in `persist_serialized_trace_payload`.
- [x] Filenames use zero-padded attempt suffixes before trace name: confirmed by `build_trace_file_name` formatting `{}-{:06}-{}.json`.
- [x] Validation/error handling is clearer and standardized: confirmed by centralized `diff_trace_validation_error` and field helper functions.
- [x] No new tests were created: confirmed by staged file inventory and final `git status --short`.

### Failed checks and follow-ups

- None.

### Residual risks

- Existing ignored artifacts under `context/tmp/` were present before final validation and were not removed because they were not introduced by this validation step.

## Open questions

- None. User resolved scope as Rust-only, `time` as `u64`, standardized diagnostics allowed, no new tests, and cross-process trace filename collision safety via atomic create/retry with zero-padded attempt suffixes.
