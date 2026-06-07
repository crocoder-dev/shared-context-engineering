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
- Focused regression tests cover the dispatch re-entry hazard and the tracing-event filtering guard where feasible.
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
  - Boundaries (in/out of scope): In - `cli/src/app.rs` command-lifecycle closure state handling, minimal test-only telemetry stub(s), regression coverage for repeated action invocation. Out - changing parser behavior, command registry behavior, command execution behavior, or the `Telemetry` trait signature unless strictly necessary.
  - Done when: The closure no longer contains `expect("command lifecycle should execute exactly once")`; repeated action invocation returns `ClassifiedError::runtime(...)` or equivalent runtime-class failure; normal command dispatch still succeeds; focused tests cover the repeated-invocation path.
  - Verification notes (commands or checks): Prefer `nix develop -c sh -c 'cd cli && cargo test run_command_lifecycle'` for narrow feedback if available; otherwise rely on `nix flake check` in final validation.
  - Completed: 2026-06-07
  - Files changed: `cli/src/app.rs`
  - Evidence: `nix develop -c sh -c 'cd cli && cargo test run_command_lifecycle'` was blocked by the repo bash policy preferring `nix flake check`; `nix develop -c sh -c 'cd cli && cargo fmt'` completed; `nix flake check` passed; `nix run .#pkl-check-generated` passed.
  - Notes: Replaced the single-use dispatch `expect(...)` with a runtime-classified repeated-dispatch error and added a focused test telemetry stub that invokes the command action twice. Context sync classified this as an app-lifecycle/observability contract change and updated the relevant current-state context.

- [ ] T02: Gate tracing field JSON construction behind tracing enablement (status:todo)
  - Task ID: T02
  - Goal: Avoid building `serde_json::Map` and calling `.to_string()` in `emit_tracing_event` when the `sce` tracing event at the requested level is disabled or filtered.
  - Boundaries (in/out of scope): In - `cli/src/services/observability.rs` tracing-emission helper and directly related unit tests/helpers. Out - changing rendered logger output, file sink writes, redaction, log-level filtering, or event IDs/field names.
  - Done when: `emit_tracing_event` performs a cheap tracing-enabled check before allocating field JSON; enabled tracing events still emit the same `event_id`, `event_message`, and `fields` payload; focused tests or code-structure assertions demonstrate disabled events do not require JSON construction where practical.
  - Verification notes (commands or checks): Prefer a narrow observability test such as `nix develop -c sh -c 'cd cli && cargo test observability'` if available; otherwise rely on `nix flake check` in final validation.

- [ ] T03: Validate, clean up, and sync context (status:todo)
  - Task ID: T03
  - Goal: Run full repository validation and update durable context only if the implemented hardening changes current-state observability or app-lifecycle contracts.
  - Boundaries (in/out of scope): In - `nix flake check`, `nix run .#pkl-check-generated`, cleanup of temporary scaffolding, plan evidence capture, and focused updates to `context/sce/cli-observability-contract.md`, `context/cli/cli-command-surface.md`, `context/overview.md`, or `context/glossary.md` only if needed. Out - broad documentation rewrites or durable history summaries in core context files.
  - Done when: Root validation passes; generated outputs are current; no temporary test/debug artifacts remain; the plan records validation evidence; durable context either reflects the new current state or is verified unchanged because the hardening is implementation-local.
  - Verification notes (commands or checks): `nix flake check`; `nix run .#pkl-check-generated`.

## Open questions

None.
