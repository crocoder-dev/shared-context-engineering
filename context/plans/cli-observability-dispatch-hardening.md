# Plan: cli-observability-dispatch-hardening

## Change summary

Harden two CLI runtime hot-path issues in `cli/src/app.rs` and `cli/src/services/observability.rs`:

- Remove the `Option<Vec<String>>::take().expect(...)` panic hazard from command lifecycle dispatch when a telemetry subscriber implementation invokes the supplied action zero or multiple times.
- Avoid constructing a `serde_json::Map` and serialized JSON string for tracing events when the corresponding tracing level/target is disabled or filtered out.

This is a reliability and performance hardening change. It should preserve the existing CLI command behavior, stdout/stderr contracts, log rendering format, error taxonomy, and public command surface.

## Success criteria

- Command dispatch no longer relies on `Option::take().expect(...)` to guard single execution of the telemetry subscriber action.
- If telemetry invokes the action more than once, dispatch returns a classified runtime error instead of panicking.
- If telemetry invokes the action zero times, dispatch behavior remains determined by the `Telemetry` trait implementation result without hidden app-level state assumptions or panics.
- Observability tracing event construction avoids allocating/serializing `fields_json` when the event is disabled by tracing level/target filtering.
- Existing text and JSON log line rendering remains unchanged for enabled logger emissions.
- Code-structure review and repository validation cover the dispatch re-entry hazard and the tracing-event filtering guard without adding focused regression tests.
- Repository validation passes through the standard root checks.

## Constraints and non-goals

- Do not change CLI command names, help text, stdout payloads, stderr diagnostic format, exit-code classes, or log event IDs.
- Do not redesign the `Telemetry` trait beyond the minimum needed to remove the panic hazard.
- Do not introduce new dependencies or async runtime behavior.
- Do not alter file-sink behavior, redaction policy, config precedence, or logger threshold semantics.
- Prefer small, local helper functions over broad lifecycle refactors.

## Assumptions

- `Telemetry::with_default_subscriber` remains synchronous and returns `Result<String, ClassifiedError>`.
- A telemetry implementation calling the action multiple times is invalid for command execution, but should be handled as a runtime failure rather than a panic.
- The tracing optimization can use `tracing::enabled!` / level metadata checks or an equivalent built-in tracing guard before constructing structured field JSON.

## Task stack

- [x] T01: Replace dispatch argument `expect` with panic-free single-use handling (status:done)
  - Task ID: T01
  - Goal: Make `run_command_lifecycle` robust when `Telemetry::with_default_subscriber` invokes its action more than once by returning a classified runtime error instead of panicking.
  - Boundaries (in/out of scope): In - `cli/src/app.rs` command-lifecycle closure state handling. Out - changing parser behavior, command registry behavior, command execution behavior, adding test-only telemetry stubs, or changing the `Telemetry` trait signature unless strictly necessary.
  - Done when: The closure no longer contains `expect("command lifecycle should execute exactly once")`; repeated action invocation returns `ClassifiedError::runtime(...)` or equivalent runtime-class failure; normal command dispatch still succeeds.
  - Verification notes (commands or checks): Prefer `nix develop -c sh -c 'cd cli && cargo test run_command_lifecycle'` for narrow feedback if available; otherwise rely on `nix flake check` in final validation.
  - Completed: 2026-06-07
  - Files changed: `cli/src/app.rs`
  - Evidence: `nix develop -c sh -c 'cd cli && cargo test run_command_lifecycle'` was blocked by the repo bash policy preferring `nix flake check`; `nix develop -c sh -c 'cd cli && cargo fmt'` completed; `nix flake check` passed; `nix run .#pkl-check-generated` passed.
  - Notes: Replaced the single-use dispatch `expect(...)` with a runtime-classified repeated-dispatch error. The initially added focused test telemetry stub was removed after feedback. Context sync classified this as an app-lifecycle/observability contract change and updated the relevant current-state context.

- [x] T02: Gate tracing field JSON construction behind tracing enablement (status:done)
  - Task ID: T02
  - Goal: Avoid building `serde_json::Map` and calling `.to_string()` in `emit_tracing_event` when the `sce` tracing event at the requested level is disabled or filtered.
  - Boundaries (in/out of scope): In - `cli/src/services/observability.rs` tracing-emission helper and directly related unit tests/helpers. Out - changing rendered logger output, file sink writes, redaction, log-level filtering, or event IDs/field names.
  - Done when: `emit_tracing_event` performs a cheap tracing-enabled check before allocating field JSON; enabled tracing events still emit the same `event_id`, `event_message`, and `fields` payload; focused tests or code-structure assertions demonstrate disabled events do not require JSON construction where practical.
  - Verification notes (commands or checks): Prefer a narrow observability test such as `nix develop -c sh -c 'cd cli && cargo test observability'` if available; otherwise rely on `nix flake check` in final validation.
  - Completed: 2026-06-07
  - Files changed: `cli/src/services/observability.rs`; `cli/src/services/db/encryption_key.rs` (approved rustfmt-only doc-comment indentation fix); `context/sce/cli-observability-contract.md`
  - Evidence: `nix develop -c sh -c 'cd cli && cargo test observability'` was blocked by the repo bash policy preferring `nix flake check`; initial `nix flake check` failed on `clippy::items_after_test_module`, then the test module placement was fixed; validation later exposed one unrelated rustfmt-required doc-comment indentation fix in `cli/src/services/db/encryption_key.rs`, which was explicitly approved as a scope expansion; final `nix flake check` passed; after feedback removed the generated observability tests, `nix build .#checks.x86_64-linux.cli-clippy`, `nix build .#checks.x86_64-linux.cli-fmt`, and `nix build .#checks.x86_64-linux.cli-tests` completed; final `nix run .#pkl-check-generated` reported "Generated outputs are up to date."
  - Notes: Added a tracing target/level enablement guard before field JSON construction and isolated field JSON serialization behind a lazy helper closure. The initially added observability unit tests were removed after feedback, leaving the code-structure seam as the practical assertion that disabled events return before JSON construction. Context sync classification: localized observability implementation hardening; root context verified unchanged, with the observability domain contract updated to note lazy tracing field construction.

- [x] T03: Validate, clean up, and sync context (status:done)
  - Task ID: T03
  - Goal: Run full repository validation and update durable context only if the implemented hardening changes current-state observability or app-lifecycle contracts.
  - Boundaries (in/out of scope): In - `nix flake check`, `nix run .#pkl-check-generated`, cleanup of temporary scaffolding, plan evidence capture, and focused updates to `context/sce/cli-observability-contract.md`, `context/cli/cli-command-surface.md`, `context/overview.md`, or `context/glossary.md` only if needed. Out - broad documentation rewrites or durable history summaries in core context files.
  - Done when: Root validation passes; generated outputs are current; no temporary test/debug artifacts remain; the plan records validation evidence; durable context either reflects the new current state or is verified unchanged because the hardening is implementation-local.
  - Verification notes (commands or checks): `nix flake check`; `nix run .#pkl-check-generated`.
  - Completed: 2026-06-07
  - Files changed: `cli/src/services/db/encryption_key.rs` (rustfmt-only doc-comment indentation cleanup); `context/plans/cli-observability-dispatch-hardening.md`; `context/sce/cli-observability-contract.md`
  - Evidence: Initial `nix run .#pkl-check-generated` passed; initial `nix flake check` failed only on `cli-fmt` for a rustfmt-required doc-comment indentation issue in `cli/src/services/db/encryption_key.rs`; the formatting-only cleanup was applied; rerun `nix flake check` passed; rerun `nix run .#pkl-check-generated` reported "Generated outputs are up to date." Worktree inspection found no untracked temporary/debug artifacts requiring cleanup.
  - Notes: Context sync classification: validation/finalization task with no new current-state CLI behavior beyond the T01/T02 hardening already reflected in domain context; root context verified unchanged. Domain context received a small current-state correction to remove duplicate tracing-emission wording and align verification wording with the current app-lifecycle/flakes coverage.

## Validation Report

### Commands run

- `nix run .#pkl-check-generated` -> exit 0; output included `Generated outputs are up to date.`
- `nix flake check` -> initial exit 1; failed only in `checks.x86_64-linux.cli-fmt` on rustfmt-required doc-comment indentation in `cli/src/services/db/encryption_key.rs`.
- `nix flake check` -> exit 0 after formatting cleanup; output ended with `all checks passed!`.
- `nix run .#pkl-check-generated` -> exit 0 after formatting cleanup; output included `Generated outputs are up to date.`
- `git diff --check` -> exit 0; no whitespace errors reported.

### Success-criteria verification

- [x] Command dispatch no longer relies on `Option::take().expect(...)` to guard single execution of the telemetry subscriber action -> verified by T01 code/context evidence in `cli/src/app.rs` and passing `nix flake check`.
- [x] If telemetry invokes the action more than once, dispatch returns a classified runtime error instead of panicking -> verified by code structure in `cli/src/app.rs` and passing `nix flake check`.
- [x] If telemetry invokes the action zero times, dispatch behavior remains determined by the `Telemetry` trait implementation result without hidden app-level state assumptions or panics -> verified by the panic-free command lifecycle implementation and passing `nix flake check`.
- [x] Observability tracing event construction avoids allocating/serializing `fields_json` when the event is disabled by tracing level/target filtering -> verified by T02 code structure in `cli/src/services/observability.rs` and passing `nix flake check`.
- [x] Existing text and JSON log line rendering remains unchanged for enabled logger emissions -> no logger-rendering contract changes were made; validated by existing flake checks.
- [x] Code-structure review and repository validation cover the dispatch re-entry hazard and the tracing-event filtering guard without adding focused regression tests -> dispatch re-entry and tracing guard are covered by code-structure review after feedback removed focused tests.
- [x] Repository validation passes through the standard root checks -> `nix flake check` passed and `nix run .#pkl-check-generated` reported generated outputs current.

### Failed checks and follow-ups

- Initial `nix flake check` exposed a rustfmt-only doc-comment indentation issue in `cli/src/services/db/encryption_key.rs`; the formatting cleanup was applied and the rerun passed.

### Residual risks

- None identified.

## Open questions

None.
