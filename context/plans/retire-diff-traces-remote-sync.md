# Plan: retire-diff-traces-remote-sync

## Change summary

Phase 2 of retiring `diff_traces` from the Control Plane ingestion contract.
Today `cli/src/services/sync/sync.rs` starts four concurrent remote stream
state machines after the single `/state` call (`messages`, `parts`,
`diff_traces`, `agent_traces`) through `try_join_four`. This plan changes the
active sync orchestration so `sce sync` starts exactly three: `messages`,
`parts`, and `agent_traces`. The `diff_traces` stream is no longer read through
`AgentTraceExportReader::read_diff_traces_after`, no longer uploaded through
`AuthenticatedControlPlaneClient::ingest_diff_traces`, and no longer produces
progress events or a progress row.

This replaces behavior in the sync orchestration and its presentation only. It
preserves everything else: the local `diff_traces` evidence pipeline (table,
migrations, hook ingress, structured-patch reconstruction, post-commit
intersection, staged-diff AI-overlap, attribution) is untouched, and the
low-level protocol compatibility surfaces (`IngestionStream::DiffTraces`,
`read_diff_traces_after`, `AgentTraceDiffTraceExportRow`, `ingest_diff_traces`,
the batch request DTO, `state.cursors.diff_traces` decoding) stay in place as
deferred-cleanup surfaces. The `streams.diffTraces` entry in
`sce sync --format json` is retained as a zero-upload compatibility report that
echoes the server's unchanged `diffTraces` cursor. The governing distinction
after this plan is: four local Agent Trace capture streams, three remotely
synchronized streams.

Phase 2 stops using the stream; it does not redesign the protocol. The
orchestration change and its user-facing presentation (progress rows, text
heading, JSON compatibility report) land in one atomic task, so there is no
completed task state in which `diff_traces` is still remotely synchronized but
its progress reporting has disappeared.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the
check that proves it. `/validate` runs these checks; no task in the stack
performs final validation.

- [x] AC1: `sce sync` starts exactly three remote stream state machines — `messages`, `parts`, `agent_traces` — and the active sync path contains no `diff_traces` stream future.
  - Validate: `nix shell nixpkgs#ripgrep -c rg -n "read_diff_traces_after|ingest_diff_traces|IngestionStream::DiffTraces =>|try_join_four" cli/src/services/sync/sync.rs` shows no call to `read_diff_traces_after`, `ingest_diff_traces`, or `try_join_four`, and the only `DiffTraces` reference outside `#[cfg(test)]` is the `cursor_for_stream` match arm; the concurrency integration test in `cli/src/services/sync/sync.rs` asserts `max_in_flight() == 3` and exactly four captured requests (one `/state`, three `/batch`).
- [x] AC2: With rows present in all four local tables, a normal sync uploads `messages`, `parts`, and `agent_traces`, and sends no ingestion batch whose body has `"stream": "diff_traces"` — on the first run and on a second incremental run.
  - Validate: the full-sync integration test in `cli/src/services/sync/sync.rs` parses every captured `/agent-trace/ingestion/batch` request body and asserts the set of `stream` values is exactly `{messages, parts, agent_traces}` and that none equals `diff_traces`, after both runs; run with `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::sync`.
- [x] AC3: Sync neither reads nor changes local `diff_traces` rows: the seeded row is byte-identical after sync, and sync succeeds even when the local `diff_traces` table holds a row the export reader would reject.
  - Validate: the same integration test asserts the `diff_traces` row count and column values are unchanged after both runs; a sync test seeds a `diff_traces` row with an exportable numeric field above `9_007_199_254_740_991` (which makes `read_diff_traces_after` return an error) and asserts `run_sync_against` still returns `Ok`. This behavioral proof is required in addition to the AC1 source inspection; neither replaces the other.
- [x] AC4: A `/state` response carrying `cursors.diffTraces` is still required and decoded — `AgentTraceCursors.diff_traces` stays a mandatory, non-optional field — and the initial `POST /agent-trace/ingestion/state` request is unchanged.
  - Validate: existing `control_plane.rs` cursor decode/validation tests pass unmodified; the full-sync integration test asserts the first captured request is `POST /agent-trace/ingestion/state` with the unchanged `repositoryId`/`sourceInstanceId` body, and `git diff main -- cli/src/services/agent_trace_sync/control_plane.rs` shows no change to request/response DTOs (including the type of `AgentTraceCursors.diff_traces`) or cursor validation.
- [x] AC5: `sce sync --format json` retains `streams.diffTraces` with `uploaded == 0`, `batches == 0`, and `initialCursor == finalCursor == state.cursors.diffTraces`, including when local `diff_traces` rows exist beyond that cursor.
  - Validate: an integration test in `cli/src/services/sync/sync.rs` answers `/state` with a non-zero `diffTraces` cursor (for example `123`) while the local table holds a newer row, and asserts `report.streams.diff_traces == StreamSyncReport { uploaded: 0, initial_cursor: 123, final_cursor: 123, batches: 0 }`; `json_shape_matches_contract` in `cli/src/services/sync/render_sync.rs` still asserts the `diffTraces` object shape.
- [x] AC6: Text-mode progress shows exactly three rows — `messages`, `parts`, `agent_traces` — with no `diff_traces` spinner or progress row, and sync emits no `BatchAccepted` or `StreamCompleted` event for `diff_traces`.
  - Validate: the progress reporter test in `cli/src/services/sync/progress.rs` asserts the three labels are rendered and that the output does not contain `diff_traces`; the progress-events test in `cli/src/services/sync/sync.rs` asserts the exact event list contains events only for the three active streams.
- [x] AC7: The final text heading is `Agent Trace already synced.` exactly when the three active streams uploaded zero rows, independent of the `diffTraces` compatibility entry.
  - Validate: a `render_sync.rs` test renders a report whose three active streams have `uploaded == 0` and asserts the already-synced heading, and a report with one active stream uploading asserts `Agent Trace sync complete.`.
- [x] AC8: The three active streams keep their existing behavior for concurrency across streams, sequential batches within a stream, cursor validation, `409` reconciliation, ambiguous `5xx`/transport/invalid-response reconciliation, terminal `400`/`403`/protocol failures, WorkOS authentication and refresh single-flight, batching, and no local sync cursor state.
  - Validate: the existing tests `concurrent_sync_keeps_batches_sequential_within_one_stream`, `invalid_state_cursor_fails_before_any_batch_request`, `terminal_batch_status_fails_without_state_reconciliation`, `malformed_2xx_batch_response_still_reconciles_via_state`, `forbidden_state_response_fails_without_mutating_local_metadata`, the no-local-cursor-file assertion in the full-sync test, and all `agent_trace_sync` engine and `control_plane` tests pass with assertions changed only where they counted the fourth stream.
- [x] AC9: The local `diff_traces` pipeline and every protocol compatibility surface are unchanged, and no Control Plane code or contract change is required.
  - Validate: `git diff main --stat` shows no change under `cli/src/services/hooks/`, `cli/src/services/agent_trace_db/`, `cli/src/services/patch.rs`, `cli/src/services/structured_patch.rs`, `cli/migrations/`, or `config/`; `nix shell nixpkgs#ripgrep -c rg -n "DiffTraces|fn read_diff_traces_after|struct AgentTraceDiffTraceExportRow|fn ingest_diff_traces|pub diff_traces" cli/src/services/agent_trace_sync cli/src/services/agent_trace_export` still finds each surface; existing diff-trace hook, intersection, and export-reader tests pass unmodified.
- [x] AC10: Durable context consistently distinguishes four local capture streams from three remotely synchronized streams, states that `AgentTraceExportReader` still exposes four readers of which `sce sync` consumes three, documents `streams.diffTraces` as a temporary compatibility no-op entry, and records the release gate with its compatibility boundary: Phase 3 stops the Control Plane accepting and storing new `diff_traces` batches while `/state` keeps returning `cursors.diffTraces`, and removing that cursor and the retained client/server compatibility surfaces belongs to a later compatibility-removal phase. This is a final consistency check across current-state context; the context itself is written by T01's task context synchronization, not by a separate task.
  - Validate: `nix shell nixpkgs#ripgrep -c rg -n -i "four[- ](concurrent |remote )?(agent trace )?stream|four-stream|four progress|all four streams" context --glob '!context/plans/**' --glob '!context/handovers/**' --glob '!context/decisions/**'` returns no statement describing four *remote/synchronized* streams; inspect `context/cli/sync-command.md` and `context/cli/agent-trace-sync-command.md` for the local-vs-remote lists, the `diffTraces` compatibility note, and the release gate, and confirm no current-state context file says or implies that Phase 3 removes `/state.cursors.diffTraces` or the whole `diff_traces` wire contract.

### Full validation

Repository-wide checks `/validate` runs after the last task, regardless of
which criterion they map to.

- `nix flake check`
- `nix run .#pkl-check-generated`

### Context sync

T01's task context synchronization updates every file below before T01 becomes
`synced`; there is no separate context task.

- `context/cli/sync-command.md` — three concurrent remote stream state machines, three progress rows, three-stream "already synced" rule, and `streams.diffTraces` documented as a retained compatibility no-op (`uploaded: 0`, `batches: 0`, `initialCursor == finalCursor == state.cursors.diffTraces`) that does not mean the stream is synchronized.
- `context/cli/agent-trace-sync-command.md` — local capture streams (`messages`, `parts`, `diff_traces`, `agent_traces`) versus remote `sce sync` streams (`messages`, `parts`, `agent_traces`); `diff_traces` stays local with no remote upload; the release gate, including that Phase 3 only stops new `diff_traces` batch ingestion and persistence, that `/state.cursors.diffTraces` remains required through the compatibility window, and that a later compatibility-removal phase removes the retained surfaces.
- `context/sce/agent-trace-export-readers.md` — the reader still exposes four `read_*_after` methods during the compatibility window; `sce sync` consumes three; `read_diff_traces_after` and `AgentTraceDiffTraceExportRow` are deferred-cleanup surfaces with no active caller.
- `context/sce/agent-trace-db.md` — the export-reader paragraph that currently calls all four "exported streams".
- `context/overview.md`, `context/context-map.md`, `context/glossary.md`, `context/architecture.md` — only the entries that state four-stream remote synchronization (context-map `sync-command.md` annotation, glossary `progress reporter contract`, architecture `SyncProgressEvent`/`sce sync` lines); `overview.md` is verify-only unless a stale four-stream statement is found.

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.
- This plan has one implementation task, so durable context made stale by the
  implementation is never deferred to a later task. T01 completes as:
  implement → run targeted verification → synchronize the durable context
  listed under Context sync → record `Context synchronization: synced` → task
  complete. `/validate` then runs the acceptance criteria.

## Constraints and non-goals

- **In scope:** `cli/src/services/sync/sync.rs`, `cli/src/services/sync/progress.rs`, `cli/src/services/sync/render_sync.rs` (code and inline tests); lint allowances only on the retained compatibility surfaces in `cli/src/services/agent_trace_sync/control_plane.rs` and `cli/src/services/agent_trace_export/mod.rs`; the durable context files listed under Context sync.
- **Out of scope:** `crocoder-dev/control-plane`; ClickHouse or DWH behavior; backfilling or deleting historical data; the local `diff_traces` table, migrations, `RepositoryAgentTraceDb::insert_diff_trace`, `RepositoryAgentTraceDb::recent_diff_trace_patches`, `sce hooks diff-trace`, OpenCode/Claude/Pi/Codex diff-trace capture, structured-patch reconstruction, post-commit patch intersection, staged-diff AI-overlap, and Agent Trace attribution.
- **Constraints:** the initial `POST /agent-trace/ingestion/state` request and `/state` response decoding are unchanged — `cursors.diffTraces` stays required and `AgentTraceCursors.diff_traces` stays a mandatory field; the `sce sync --format json` field `streams.diffTraces` is retained; no local `diff_traces` row may be read to build that entry; the orchestration change and its progress/heading presentation change land together in one task; the three remaining streams keep identical concurrency, cursor, reconciliation, batching, and authentication semantics; comments and doc comments touched by the implementation must accurately describe three active remote streams versus four local capture streams; `cli/Cargo.toml` denies warnings, so retained-but-unused compatibility surfaces need a `#[allow(dead_code)]` allowance rather than deletion; no new dependency; no host `cargo` — use `nix develop -c ./scripts/run-cli-cargo.sh ...` and `nix flake check`; do not rewrite other stable output strings.
- **Non-goal:** removing `IngestionStream::DiffTraces`, `read_diff_traces_after`, `AgentTraceDiffTraceExportRow`, `ingest_diff_traces`, the `diff_traces` batch DTO serialization, or `AgentTraceCursors.diff_traces`; making `AgentTraceCursors.diff_traces` optional or otherwise altering the `/state` DTO; renaming `diff_traces`; introducing a feature flag; introducing a generic N-way concurrency framework. These removals belong to the later compatibility-removal phase described under Release gate.

## Assumptions

- The three-way join is implemented by converting the existing private `try_join_four` helper in `sync.rs` to a three-way equivalent. `tokio` is built without the `macros` feature and the crate has no `futures` dependency, so no existing primitive fits without a dependency change.
- The internal `StreamSyncReports.diff_traces` field is kept so `render_sync.rs` keeps emitting `streams.diffTraces`; it is constructed directly from `state.cursors.diff_traces` after the `/state` call, with no stream future.
- "`read_diff_traces_after` is not needed by the active sync path" is proven behaviorally by seeding a local `diff_traces` row that the reader rejects (numeric field above the JS safe-integer bound) and asserting sync still succeeds, plus the source inspection in AC1.
- Existing sync integration tests are strengthened and renamed where they say "four" rather than duplicated; `seed_one_row_per_stream` keeps seeding all four local tables.
- Releasing and adopting the new SCE version, Phase 3, and the later compatibility-removal phase (the release gate below) are follow-up work, not tasks in this plan.

## Task stack

- [x] T01: `Retire diff_traces from active sce sync while preserving protocol compatibility` (status:done)
  - Task ID: T01
  - Scope: In, as one atomic change —
    - **Sync orchestration** (`cli/src/services/sync/sync.rs`): remove the active `diff_traces` `sync_one_stream` future; convert the four-way join (`try_join_four`) to a three-way join so the active streams are exactly `messages`, `parts`, `agent_traces`; build `StreamSyncReports.diff_traces` directly from `state.cursors.diff_traces` as `{ uploaded: 0, batches: 0, initial_cursor: state.cursors.diff_traces, final_cursor: state.cursors.diff_traces }` without reading any local `diff_traces` row; add `#[allow(dead_code)]` only where the retained compatibility surfaces (`ingest_diff_traces`, `read_diff_traces_after`, and anything reachable only through them) would otherwise fail the deny-warnings build; make touched comments and doc comments describe three active remote streams versus four local capture streams.
    - **Progress** (`cli/src/services/sync/progress.rs`): `STREAM_LABELS` becomes `["messages", "parts", "agent_traces"]` and every array sized from it follows; no `diff_traces` spinner or progress row.
    - **Text completion heading** (`cli/src/services/sync/render_sync.rs`): `render_text` decides `Agent Trace already synced.` from `messages`, `parts`, and `agent_traces` only, ignoring the compatibility `diffTraces` report; `render_json` and the `streams.diffTraces` field are unchanged.
    - **Tests** (inline, same task): strengthen the full-sync test (all four local tables seeded; one `/state` request; exactly three first-run `/batch` requests whose parsed `stream` values are exactly `messages`, `parts`, `agent_traces`; no request with `stream = "diff_traces"`; second incremental run sends no `diff_traces`; local `diff_traces` row present and unchanged; unchanged `/state` request body; no local cursor file); add the reader-rejecting-row proof (a local `diff_traces` row that `read_diff_traces_after` would reject does not make sync fail); add the non-zero `diffTraces` cursor compatibility assertion (`streams.diffTraces` echoes the unchanged server cursor with zero uploads and batches); adjust the concurrency test to three-way (`max_in_flight() == 3`), the progress-events test to events for the three active streams only, and the terminal-failure tests where they counted the fourth stream; update the progress reporter test to assert three rows and the absence of `diff_traces`; add `render_sync.rs` heading tests for both outcomes; keep `json_shape_matches_contract` passing unmodified.
    - **Task context synchronization** (before the task is `synced`): update every file listed under Context sync so current-state context matches the implemented behavior — the local-capture list (`messages`, `parts`, `diff_traces`, `agent_traces`) versus the remote `sce sync` list (`messages`, `parts`, `agent_traces`); `AgentTraceExportReader` exposes four readers during the compatibility window while `sce sync` consumes three; the `streams.diffTraces` compatibility no-op contract and that it does not mean the stream is synchronized; the list of deferred-cleanup compatibility surfaces; and the release gate with its Phase 3 / later compatibility-removal boundary.
  - Scope: Out — deleting or changing any compatibility surface's signature or serialization; `control_plane.rs` DTOs and cursor validation (including making `AgentTraceCursors.diff_traces` optional); the `agent_trace_sync` engine; hooks; the Agent Trace DB; any change to JSON field names or other output strings; a feature flag; a generic N-way concurrency abstraction; edits to immutable decision records; rewriting context not made stale by this change; plan files other than this one.
  - Dependencies: none
  - Done when: a sync against a database with rows in all four local tables sends one `/state` request and exactly three `/batch` requests, none with `"stream": "diff_traces"`; a second run sends only `/state`; local `diff_traces` rows are untouched and a reader-rejected row does not fail sync; `report.streams.diff_traces` echoes the server cursor with zero uploads and zero batches even when local rows exist beyond it; progress events and text-mode progress rows cover only `messages`, `parts`, `agent_traces` in that fixed order; the heading is computed from the three active streams; cross-stream concurrency is three-way and all per-stream cursor, reconciliation, and authentication tests pass with assertions changed only where they counted the fourth stream; the build is warning-free; no current-state context file describes four remotely synchronized streams, and `sync-command.md`, `agent-trace-sync-command.md`, and `agent-trace-export-readers.md` carry the stream lists, compatibility note, four-readers/three-consumed boundary, and release gate; `Context synchronization` is recorded as `synced`.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::sync`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::agent_trace_sync`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::agent_trace_export`; `nix develop -c ./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets --all-features`; after context synchronization, `nix shell nixpkgs#ripgrep -c rg -n -i "four[- ](concurrent |remote )?(agent trace )?stream|four-stream|four progress|all four streams" context --glob '!context/plans/**' --glob '!context/handovers/**' --glob '!context/decisions/**'`.
  - Completed: 2026-10-01
  - Files changed: `cli/src/services/sync/sync.rs`, `cli/src/services/sync/progress.rs`, `cli/src/services/sync/render_sync.rs`, `context/plans/retire-diff-traces-remote-sync.md`
  - Result: `run_sync_async` now joins three `sync_one_stream` futures (`messages`, `parts`, `agent_traces`) through `try_join_three`; `StreamSyncReports.diff_traces` is built directly from `state.cursors.diff_traces` with zero uploads and batches and no local read. `STREAM_LABELS` has three entries. `render_text` decides the heading from the three active streams. Tests: the full-sync test was renamed to `full_sync_uploads_three_remote_streams_and_second_run_is_naturally_incremental` and strengthened (parsed batch `stream` values, unchanged `/state` body, unchanged local `diff_traces` rows across both runs); the concurrency test was renamed to `concurrent_sync_overlaps_all_three_stream_batches_after_one_state_request` and asserts `max_in_flight() == 3` with four captured requests; added `sync_succeeds_when_local_diff_trace_row_is_rejected_by_export_reader`, `diff_traces_report_echoes_server_cursor_without_uploading_newer_local_rows`, `text_heading_is_already_synced_when_three_active_streams_uploaded_nothing`, and `text_heading_is_complete_when_any_active_stream_uploaded_rows`; the progress-events and progress-reporter tests cover three streams and assert the absence of `diff_traces`.
  - Verify:
    - `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::sync` — passed (32 passed, 0 failed).
    - `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::agent_trace_sync` — passed (38 passed, 0 failed).
    - `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::agent_trace_export` — passed (48 passed, 0 failed).
    - `nix develop -c ./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets --all-features` — passed, no warnings.
    - Context `rg` four-stream sweep — passed after context synchronization (no matches).
  - Deviations:
    - No `#[allow(dead_code)]` was added. `cli/src/services/mod.rs` already declares `agent_trace_export` and `agent_trace_sync` under module-level `#[allow(dead_code)]`, so the retained compatibility surfaces build warning-free and `control_plane.rs` and `agent_trace_export/mod.rs` are untouched.
    - The reader-rejecting-row test does not call `read_diff_traces_after` itself, so AC1's source inspection of `sync.rs` stays clean; the rejection premise is proven by the unmodified export test `read_diff_traces_after_rejects_row_above_safe_integer_bound`, which seeds the same shape of row.
    - Doc comments in `sync.rs` that described four synchronized streams were removed rather than reworded, per the user's instruction to keep generated comments out of code.
  - Context impact: required — current-state context describes four concurrent remote streams, four progress rows, and four exported streams; the files listed under Context sync must be updated.
  - Context synchronization: synced

After T01 is `done` and `synced`, the plan proceeds directly to `/validate`.

## Release gate

This plan ends at the following compatibility gate. Each step must not start
before every step above it is complete.

```
Phase 2 (this plan)
SCE stops POSTing diff_traces batches
but still decodes /state.cursors.diffTraces
       ↓
merge + release a new SCE version
       ↓
verify/adopt that version so supported SCE clients no longer upload diff_traces
       ↓
Phase 3
Control Plane removes/rejects stream = "diff_traces" from active batch ingestion
Control Plane stops storing new diff_traces rows
BUT /state.cursors.diffTraces remains,
for compatibility with the Phase 2 SCE client
       ↓
later compatibility-removal phase
remove:
  /state.cursors.diffTraces
  IngestionStream::DiffTraces
  ingest_diff_traces
  read_diff_traces_after export compatibility
  remaining server/client protocol compatibility
```

Phase 3 removes `diff_traces` from active batch ingestion and prevents new
server-side `diff_traces` persistence. `POST /agent-trace/ingestion/state`
continues returning `cursors.diffTraces` during the compatibility window because
the Phase 2 SCE release still decodes that field as mandatory: a Control Plane
that dropped it would break every Phase 2 client. Removing the state cursor and
the retained client/server compatibility surfaces belongs to a later
compatibility-removal phase, which removes the field coherently from both sides.

The two follow-up phases are therefore distinct:

- **Phase 3:** stop accepting and storing *new* `diff_traces`.
- **Later compatibility-removal phase:** remove the retained `diff_traces`
  protocol compatibility.

## Open questions

None. The change request fixes scope, compatibility surfaces, output contract,
test obligations, non-goals, and the release gate; the remaining choices are
local and recorded under Assumptions.

## Validation Report

**Status:** validated  
**Date:** 2026-10-01

### Commands run

- `nix flake check` -> exit 0 (all 12 flake checks passed, including `cli-tests`, `cli-clippy`, `cli-fmt`, Bun tests, and `pkl-generated-check`)
- `nix run .#pkl-check-generated` -> exit 0 (ephemeral Pkl generation passed: 142 files)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::sync` -> exit 0 (32 passed, 0 failed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::agent_trace_sync` -> exit 0 (38 passed, 0 failed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::agent_trace_export` -> exit 0 (48 passed, 0 failed)
- `nix shell nixpkgs#ripgrep -c rg -n "read_diff_traces_after|ingest_diff_traces|IngestionStream::DiffTraces =>|try_join_four" cli/src/services/sync/sync.rs` -> exit 0 (one match: the `cursor_for_stream` arm at line 565)
- `git diff main --stat` -> exit 0 (only `cli/src/services/sync/{sync,progress,render_sync}.rs` and `context/` files changed)
- `git diff main --stat -- cli/src/services/hooks/ cli/src/services/agent_trace_db/ cli/src/services/patch.rs cli/src/services/structured_patch.rs cli/migrations/ config/ cli/src/services/agent_trace_sync cli/src/services/agent_trace_export` -> exit 0 (no changes)
- `nix shell nixpkgs#ripgrep -c rg -n "DiffTraces|fn read_diff_traces_after|struct AgentTraceDiffTraceExportRow|fn ingest_diff_traces|pub diff_traces" cli/src/services/agent_trace_sync cli/src/services/agent_trace_export` -> exit 0 (every retained surface found)
- `nix shell nixpkgs#ripgrep -c rg -n -i "four[- ](concurrent |remote )?(agent trace )?stream|four-stream|four progress|all four streams" context --glob '!context/plans/**' --glob '!context/handovers/**' --glob '!context/decisions/**'` -> exit 1 (no matches, as required)

### Success-criteria verification

- [x] AC1: `sce sync` starts exactly three remote stream state machines and the active sync path contains no `diff_traces` stream future -> the `rg` inspection finds no `read_diff_traces_after`, `ingest_diff_traces`, or `try_join_four` in `sync.rs`; the only non-test `DiffTraces` reference is the `cursor_for_stream` arm (line 565, test module starts at 585); `concurrent_sync_overlaps_all_three_stream_batches_after_one_state_request` asserts `max_in_flight() == 3` and four captured requests, and passed.
- [x] AC2: A normal sync uploads `messages`, `parts`, `agent_traces` and sends no `diff_traces` batch on the first or second run -> `full_sync_uploads_three_remote_streams_and_second_run_is_naturally_incremental` parses every captured batch body, asserts the stream set is exactly `agent_traces`, `messages`, `parts` after both runs and that none is `diff_traces`; passed.
- [x] AC3: Sync neither reads nor changes local `diff_traces` rows, including a reader-rejected row -> the full-sync test compares a full column snapshot of `diff_traces` before and after both runs; `sync_succeeds_when_local_diff_trace_row_is_rejected_by_export_reader` seeds `time_ms = JS_MAX_SAFE_INTEGER + 1` and asserts `run_sync_against` returns `Ok`; the rejection premise is covered by the unmodified `read_diff_traces_after_rejects_row_above_safe_integer_bound`; all passed.
- [x] AC4: `/state` `cursors.diffTraces` stays required and decoded and the initial state request is unchanged -> `git diff main` shows no change under `cli/src/services/agent_trace_sync`; the `control_plane` cursor decode/validation tests passed unmodified; the full-sync test asserts the first request is `POST /agent-trace/ingestion/state` with exactly `repositoryId`/`sourceInstanceId`.
- [x] AC5: `streams.diffTraces` is retained with zero uploads and batches and echoes the server cursor -> `diff_traces_report_echoes_server_cursor_without_uploading_newer_local_rows` answers `/state` with `diffTraces: 123` while a local row `124` exists and asserts `StreamSyncReport { uploaded: 0, initial_cursor: 123, final_cursor: 123, batches: 0 }`; `json_shape_matches_contract` still asserts the `diffTraces` object shape; both passed.
- [x] AC6: Text-mode progress shows exactly three rows and no `diff_traces` events -> `progress_reporter_creates_aligned_rows_and_updates_only_the_matching_stream` asserts the three labels in order and that output does not contain `diff_traces`; `progress_events_cover_batches_empty_streams_and_fixed_order` asserts an exact event list covering only the three active streams; both passed.
- [x] AC7: The heading is `Agent Trace already synced.` exactly when the three active streams uploaded zero rows -> `text_heading_is_already_synced_when_three_active_streams_uploaded_nothing` (including a non-zero `diffTraces` upload count) and `text_heading_is_complete_when_any_active_stream_uploaded_rows` passed.
- [x] AC8: The three active streams keep their existing concurrency, cursor, reconciliation, authentication, and batching behavior -> all named tests and all `agent_trace_sync` engine and `control_plane` tests passed; the `sync.rs` test diff against `main` changes existing assertions only where they counted the fourth stream.
- [x] AC9: The local `diff_traces` pipeline and every protocol compatibility surface are unchanged -> the scoped `git diff main --stat` is empty for hooks, Agent Trace DB, patch, structured patch, migrations, `config/`, `agent_trace_sync`, and `agent_trace_export`; the `rg` inspection finds each retained surface; `nix flake check` ran the full CLI test suite unmodified.
- [x] AC10: Durable context distinguishes four local capture streams from three remote streams and records the release gate -> the four-stream `rg` sweep returns no matches; inspection of `context/cli/sync-command.md`, `context/cli/agent-trace-sync-command.md`, and `context/sce/agent-trace-export-readers.md` confirms the local-vs-remote lists, the four-readers/three-consumed boundary, the `streams.diffTraces` compatibility no-op note, and the release gate stating Phase 3 keeps `/state.cursors.diffTraces`; no current-state context file says or implies Phase 3 removes that cursor or the whole wire contract.

### Failed checks and follow-ups

- None.

### Residual risks

- The release gate is follow-up work outside this plan: Phase 3 in the Control Plane must not start before the new SCE version is released and adopted, or older clients fail their `diff_traces` batch requests.
- The retained compatibility surfaces (`read_diff_traces_after`, `ingest_diff_traces`, `IngestionStream::DiffTraces`) have no active caller and are kept warning-free only by the existing module-level `#[allow(dead_code)]` in `cli/src/services/mod.rs`.
