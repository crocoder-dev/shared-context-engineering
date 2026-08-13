# Plan: trace-sync-progress

## Change summary

Improve `sce trace sync` human-facing feedback by emitting deterministic progress to `stderr` while a text-mode sync is running. The current command waits until all four streams finish before returning its concise report, leaving users with no indication that a slow or large sync is still active. JSON mode remains machine-readable and unchanged: it emits no progress messages and continues to return only the existing JSON payload on `stdout`.

The recommended end-user experience is a short start message followed by one progress line for each accepted batch, in the existing fixed stream order. Each line names the stream, cumulative rows uploaded, and the current server cursor; empty streams still receive a concise no-new-rows status. The existing final text summary remains the command payload on `stdout`, while live progress goes to `stderr` so redirects and pipes remain safe.

This extends the progress lifecycle with explicit start and end timestamps for one text-mode `sce trace sync` invocation. The start timestamp is emitted before the first control-plane request and the end timestamp is emitted after the invocation reaches a terminal success or failure; JSON output and the final report schema remain unchanged.

## Acceptance criteria

- [x] AC1: Text-mode `sce trace sync` emits live, human-readable progress to `stderr` while synchronization is underway, including the active stream and cumulative upload/cursor information for accepted batches, without waiting for the final report.
  - Validate: `cli/src/services/trace` progress tests capture the injected progress sink and assert event order and batch values for a multi-batch sync.
- [x] AC2: Text-mode progress is deterministic and complete for all four streams: streams are reported in `messages`, `parts`, `diff_traces`, `agent_traces` order, and an empty stream reports that no rows were uploaded rather than disappearing.
  - Validate: targeted sync progress test with populated and empty streams asserts the exact rendered progress lines/events.
- [x] AC3: JSON-mode `sce trace sync --format json` emits no human progress and preserves the existing JSON-only `stdout` contract and schema.
  - Validate: command/render test runs the JSON path with a recording progress sink and asserts it remains empty while the parsed stdout payload matches the existing `render_sync` shape.
- [x] AC4: Progress reporting does not change sync correctness or persistence invariants: cursor advancement, reconciliation behavior, final text/JSON reports, and the absence of local sync state remain unchanged.
  - Validate: existing `trace::sync` reconciliation/incremental tests and the focused progress tests pass.
- [x] AC5: Text-mode `sce trace sync` emits a start timestamp before its first control-plane request and an end timestamp after its terminal success or failure, with deterministic ordering and an unambiguous UTC representation.
  - Validate: focused command tests with an injected clock and recording stderr sink assert start-before-request, end-after-terminal-result, and success/failure event ordering.
- [x] AC6: JSON-mode `sce trace sync --format json` emits neither lifecycle timestamp nor other human progress text and preserves the existing stdout JSON payload exactly.
  - Validate: the JSON command test captures stderr and parses stdout, asserting an empty progress sink and the existing `render_sync` shape.

### Full validation

- `nix flake check`
- `nix run .#pkl-check-generated`

### Context sync

- `context/cli/trace-command.md` — document text-mode live progress, its `stderr` destination, lifecycle timestamps, and the unchanged JSON contract.
- `context/cli/agent-trace-sync-command.md` — document progress and request lifecycle timestamps as presentation behavior without changing the local-to-control-plane sync architecture.
- `context/sce/cli-stdout-stderr-contract.md` — record that live human-readable progress and lifecycle timestamps are emitted on `stderr` while command payloads remain on `stdout`.

## Constraints and non-goals

- **In scope:** `cli/src/services/trace/sync.rs`, `cli/src/services/trace/command.rs`, the text-mode progress sink/event seam and focused tests, plus the three context documents listed under Context sync.
- **Out of scope:** JSON schema changes, per-row payload dumps, progress bars or terminal cursor control, background/parallel stream synchronization, local cursors or any other persisted progress state, and changes to the control-plane protocol or reconciliation algorithm.
- **Constraints:** preserve the fixed stream order and existing final renderers; use an injectable in-memory sink in tests; send live progress only to `stderr`; do not leak credentials, raw server responses, or local database rows in progress output.
- **Non-goal:** redesigning the final concise sync report or adding progress fields to machine-readable JSON.

## Assumptions

- Progress is most useful at accepted-batch granularity: it gives feedback during large uploads without dumping individual rows or requiring terminal-specific redraw behavior.
- The production text reporter writes plain deterministic lines to `stderr`; it does not use carriage returns, spinners, or a TTY-only progress bar, so redirected human-readable output remains understandable.
- JSON mode has no progress side channel. This keeps `--format json` suitable for callers that expect no human text and avoids changing its established payload contract.
- “Request” means one `sce trace sync` invocation, not each individual HTTP batch request; the timestamps use UTC RFC3339 text and are supplied through an injectable clock so tests do not depend on wall-clock timing.
- A terminal end timestamp is reported for both successful and failed text-mode invocations, while preserving the existing classified error returned to the app.

## Task stack

- [x] T01: `Add injectable per-batch sync progress events` (status:complete)
  - Task ID: T01
  - Goal: Extend the existing trace-sync orchestration with a typed, testable progress callback/sink that reports sync start, accepted-batch progress, and stream completion without changing cursor, reconciliation, or report behavior.
  - Boundaries (in/out of scope): In — `cli/src/services/trace/sync.rs` progress event/sink types, callbacks at the existing four-stream and accepted-batch boundaries, and focused in-memory tests covering multi-batch, empty-stream, and fixed-order events. Out — production `stderr` formatting, CLI format selection, JSON behavior, and durable context edits.
  - Dependencies: none
  - Done when: callers can observe deterministic progress events as each batch is accepted, empty streams produce an explicit completion event, the four streams retain their existing order, and all existing sync/reconciliation tests still pass.
  - Implementation evidence: Added typed `SyncProgressEvent` and injectable `SyncProgressSink` APIs with no-op compatibility wrappers; emitted start, validated accepted-batch, and stream-completion events through the existing fixed-order orchestration; added an in-memory sink test covering 1,001 rows across three batches and three empty streams.
  - Verification notes (commands or checks): `nix build .#checks.x86_64-linux.cli-tests` passed; `nix build .#checks.x86_64-linux.cli-clippy` passed; `nix build .#checks.x86_64-linux.cli-fmt` passed; focused `services::trace::sync::tests` passed (6 tests).

- [x] T02: `Wire text-only stderr progress into trace sync` (status:complete)
  - Task ID: T02
  - Goal: Connect the progress seam to `TraceCommand` so text mode emits concise human-readable start/batch/completion lines on `stderr`, while JSON mode uses a no-op sink and preserves the existing final payload on `stdout`; document the resulting stream and sync presentation contract.
  - Boundaries (in/out of scope): In — `cli/src/services/trace/command.rs` production reporter and format gating, exact progress-line rendering/tests, and updates to `context/cli/trace-command.md`, `context/cli/agent-trace-sync-command.md`, and `context/sce/cli-stdout-stderr-contract.md`. Out — changes to JSON fields, final report layout, sync protocol/reconciliation, progress bars, and local persistence.
  - Dependencies: T01
  - Done when: text-mode execution emits the recommended deterministic progress lines incrementally to `stderr`; JSON-mode execution emits none and retains the current JSON shape; focused command/render tests and documentation reflect the behavior.
  - Implementation evidence: Added an app-level stderr writer handoff for trace commands, a deterministic flushed text progress reporter for start, accepted-batch, and stream-completion events, and format gating that supplies a no-op sink for JSON. Documented the stderr progress and unchanged stdout/JSON contracts in the three requested context files.
  - Verification notes (commands or checks): focused `services::trace::command` test passed (1 test); focused `services::trace::sync` tests passed (6 tests); `nix build .#checks.x86_64-linux.cli-tests` passed; `nix build .#checks.x86_64-linux.cli-clippy` passed; `nix build .#checks.x86_64-linux.cli-fmt` passed

- [x] T03: `Restore the recorded text-progress implementation against current code` (status:complete)
  - Task ID: T03
  - Goal: Bring the current trace-sync implementation back to the behavior recorded as complete in T01/T02: injectable typed progress events, deterministic text-only stderr reporting, fixed stream order, explicit empty-stream completion, and a no-op JSON path.
  - Boundaries (in/out of scope): In — `cli/src/services/trace/sync.rs`, `cli/src/services/trace/command.rs`, focused progress tests, and the three context documents listed under Context sync. Out — lifecycle timestamps, final report fields, control-plane protocol/reconciliation, and local persistence.
  - Dependencies: T02
  - Done when: the current code exposes and exercises the recorded progress seam and text/JSON format gating, with tests proving multi-batch, empty-stream, fixed-order, and JSON-no-progress behavior; the context documents agree with the implementation.
  - Implementation evidence: Restored typed `SyncProgressEvent`/`SyncProgressSink` APIs with no-op and callback-compatible sinks, emitted validated accepted-batch and fixed-order stream-completion events, wired flushed deterministic text progress to the app-owned `stderr`, and kept JSON on the no-op path. Added focused multi-batch/empty-stream event coverage and deterministic reporter assertions; synchronized the three requested context documents.
  - Verification notes (commands or checks): `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::trace::` passed (31 tests); `nix develop -c ./scripts/run-cli-cargo.sh check --manifest-path cli/Cargo.toml` passed; `nix develop -c sh -c 'cd cli && cargo fmt'` passed.

- [x] T04: `Add text-mode request lifecycle timestamps` (status:complete)
  - Task ID: T04
  - Goal: Emit an injected-clock UTC RFC3339 start timestamp before the first sync request and an end timestamp after successful or failed terminal completion, through the existing text progress reporter only.
  - Boundaries (in/out of scope): In — lifecycle progress events/reporter wiring in `cli/src/services/trace/sync.rs` and `cli/src/services/trace/command.rs`, deterministic success/failure tests, and timestamp wording in the three context documents listed under Context sync. Out — JSON fields or human progress in JSON mode, final report layout, batch protocol/reconciliation behavior, terminal UI controls, and durable sync state.
  - Dependencies: T03
  - Done when: text-mode output records start-before-request and end-after-terminal-result with deterministic event ordering and parseable UTC timestamps; JSON emits no timestamp/progress text and retains the existing payload; failure classification and sync persistence invariants are unchanged.
  - Implementation evidence: Added injectable UTC progress clocks and started/finished lifecycle events around the full sync result, rendered lifecycle timestamps only through the text stderr reporter, preserved the no-op JSON sink, and added deterministic success/failure event coverage plus timestamp rendering assertions. Documented lifecycle timestamp behavior in the three requested context files.
  - Verification notes (commands or checks): `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::trace::` passed (32 tests); `nix develop -c ./scripts/run-cli-cargo.sh check --manifest-path cli/Cargo.toml` passed; `nix build .#checks.x86_64-linux.cli-tests` passed; `nix build .#checks.x86_64-linux.cli-clippy` passed; `nix build .#checks.x86_64-linux.cli-fmt` passed; `git diff --check` passed.

## Open questions

None. The user constrained the change to human-readable text mode; per-accepted-batch, deterministic `stderr` lines are the recommended end-user behavior and are recorded as assumptions rather than additional scope questions.

## Validation Report

**Status:** validated
**Date:** 2026-08-13

### Commands run

- `nix flake check` -> exit 0 (flake evaluation and available checks passed)
- `nix run .#pkl-check-generated` -> exit 0 (ephemeral generated-output parity passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::trace::` -> exit 0 (32 focused trace tests passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::trace::command::tests::progress_reporter_writes_deterministic_text_lines_and_flushes_each_event -- --exact` -> exit 0 (focused reporter test passed)
- `git diff --check` -> exit 0 (no whitespace errors)

### Scaffolding removed

- None.

### Success-criteria verification

- [x] AC1: Text-mode progress is emitted through the injected sink at accepted-batch boundaries; the multi-batch progress test passed.
- [x] AC2: The focused progress test passed with cumulative batch values, fixed four-stream order, and explicit empty-stream completion events.
- [x] AC3: The JSON renderer shape test passed; the command's JSON branch uses `NoopSyncProgressSink`, preserving the JSON-only payload path.
- [x] AC4: Focused trace tests passed, including incremental sync, reconciliation, final render shape, and the no-local-sync-state assertion.
- [x] AC5: Focused trace tests passed for injected UTC timestamps, start/end event order, and terminal failure completion; the reporter rendering test passed.
- [x] AC6: The JSON renderer shape test passed and the command's JSON branch uses the no-op sink, so lifecycle/progress text is not emitted.

### Failed checks and follow-ups

- None.

### Residual risks

- None identified.
