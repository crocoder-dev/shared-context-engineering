# Plan: agent-trace-sync-hardening

## Change summary

`sce trace sync` (PR #200, `context/plans/agent-trace-sync.md`, now complete) is functionally
wired but has three correctness/hardening gaps ahead of merge, all inside
`cli/src/services/agent_trace_sync/` and `cli/src/services/trace/sync.rs`:

1. `AgentTraceIngestionStateResponse.cursors` (`AgentTraceCursors { messages, parts, diff_traces,
   agent_traces }` in `control_plane.rs`) deserializes any syntactically valid `i64` from
   `/state`, including negative values or values above `Number.MAX_SAFE_INTEGER`. Nothing rejects
   an out-of-range cursor before it reaches `AgentTraceExportReader::read_*_after(...)` or
   `sync_stream`.
2. `AuthenticatedControlPlaneClient::resolve_access_token` calls `auth::is_stored_token_expired`
   and then `auth::ensure_valid_token_returning_token` as two separate expiry checks. If time
   advances between them at the expiry boundary, the second check refreshes a token the first
   check considered valid, and the `was_expired` flag (computed from the first check) suppresses
   the save — a refreshed token can be used without ever being persisted.
3. `classify_response` in `control_plane.rs` puts the raw HTTP response body directly into
   `ControlPlaneError::{BadRequest,Forbidden,Conflict,ServerError,InvalidResponse}`, so a server
   implementation detail (SQL error, stack trace, HTML) can reach the CLI's user-visible error
   text verbatim. Separately, unexpected statuses such as `404`/`405`/`415`/`422` are currently
   folded into `InvalidResponse`, which `trace/sync.rs::is_stream_terminal` does not treat as
   terminal — so a clear protocol/API mismatch during `/batch` incorrectly enters the ambiguous
   `/state`-reconciliation loop instead of failing immediately.

This plan closes all three gaps without touching the sync architecture, the command surface, the
no-local-state invariant, or PR #197/#198 source-identity/export semantics. It only tightens
validation and error classification inside the already-shipped client/engine/orchestration.

## Acceptance criteria

- [x] AC1: Every `/state` response is validated so all four cursor fields satisfy
  `0 <= cursor <= 9_007_199_254_740_991` before the response is returned to any caller; an
  out-of-range value in any field yields `ControlPlaneError::InvalidResponse` and never reaches
  `AgentTraceExportReader::read_*_after` or `sync_stream`, whether the response came from the
  initial `/state` call or from `409`/ambiguous-result reconciliation.
  - Validate: `cargo test --manifest-path cli/Cargo.toml agent_trace_cursors`
  - Validate: `cargo test --manifest-path cli/Cargo.toml invalid_state_cursor_fails_before_any_batch_request`
- [x] AC2: Access-token resolution makes exactly one expiry decision. A valid stored token is
  reused with zero refresh-endpoint calls and zero `CredentialStore::save` calls; an expired
  stored token triggers exactly one refresh-endpoint call and exactly one `save` call, and the
  control-plane request uses the refreshed token. No code path can use a refreshed token without
  persisting it.
  - Validate: `cargo test --manifest-path cli/Cargo.toml valid_token_is_reused_without_resave valid_token_is_not_resaved expired_token_is_refreshed_and_saved`
- [x] AC3: The existing unexpected-`401` behavior is unchanged: one forced refresh, one save, one
  retry; a second `401` fails with authentication guidance and no further retry.
  - Validate: `cargo test --manifest-path cli/Cargo.toml unexpected_401_refreshes_once_and_retries_once_on_success unexpected_401_twice_fails_without_a_third_attempt`
- [x] AC4: No control-plane error surfaced to the CLI contains an arbitrary raw server response
  body. Only a narrow, length-bounded `message`/`error` string field is ever extracted from a
  JSON body; malformed, HTML, oversized, empty, or non-string-field bodies fall back to a generic
  per-status message.
  - Validate: `cargo test --manifest-path cli/Cargo.toml server_error_body_is_not_leaked known_safe_error_payload_is_surfaced html_error_body_falls_back_to_generic_message`
- [x] AC5: During `/batch`, a clearly terminal protocol/API mismatch (`404`, `405`, `415`, `422`,
  and other terminal `4xx`, plus a post-refresh `401`) fails the stream immediately without
  entering `/state` reconciliation, while a `409`, a transport failure, a `5xx`, and a
  syntactically successful (`2xx`) but undecodable batch body all still reconcile via `/state`
  exactly as before.
  - Validate: `cargo test --manifest-path cli/Cargo.toml terminal_batch_status_fails_without_state_reconciliation malformed_2xx_batch_response_still_reconciles_via_state`

### Full validation

- `cargo test --manifest-path cli/Cargo.toml`
- `cargo clippy --manifest-path cli/Cargo.toml -- -D warnings`
- `nix run .#pkl-check-generated`
- `nix flake check`

### Context sync

- `context/cli/agent-trace-sync-command.md` — its "Recovery semantics" section must describe the
  terminal-vs-ambiguous split for `4xx` batch responses and the sanitized-error-body contract
  once implemented.

## Constraints and non-goals

- **In scope:** `cli/src/services/agent_trace_sync/control_plane.rs`,
  `cli/src/services/agent_trace_sync/mod.rs`, `cli/src/services/trace/sync.rs`, and their test
  modules; `context/cli/agent-trace-sync-command.md`.
- **Out of scope:** the sync architecture, the `sce trace sync` command surface, source
  identity/export semantics (PR #197/#198), the control-plane API, background sync, workspace
  selection, local synchronization state of any kind.
- **Constraints:** reuse `agent_trace_export::JS_MAX_SAFE_INTEGER` /
  `validate_js_safe_integer` rather than duplicating the numeric bound; reuse
  `auth::renew_stored_token_from_refresh_token` rather than duplicating WorkOS refresh HTTP
  logic; do not call the device-authorization flow from the sync client.
- **Non-goal:** redesigning `ControlPlaneError` beyond what's needed to distinguish terminal
  protocol/API mismatches from ambiguous batch outcomes and to carry a sanitized message instead
  of a raw body.

## Task stack

- [x] T01: `Validate /state cursors against the JS-safe-integer range` (status:done)
  - Task ID: T01
  - Goal: `AgentTraceIngestionStateResponse` cursors are validated immediately after JSON decoding
    in `AuthenticatedControlPlaneClient::send_state_request`, before the response reaches any
    caller (the initial `/state` call in `trace/sync.rs::run_sync_against` and the
    `/state` refetch inside `sync_one_stream`'s `refresh_cursor` closure use the same client
    method, so one change point covers both).
  - Boundaries (in/out of scope): In — add `impl AgentTraceCursors { pub fn validate(&self) ->
    Result<(), ControlPlaneError> }` in `control_plane.rs` calling
    `agent_trace_export::validate_js_safe_integer` on all four fields and mapping a failure to
    `ControlPlaneError::InvalidResponse`; call it from `send_state_request` before returning.
    Out — changing `AgentTraceCursors`'s field types, changing `/state`'s request shape, changing
    `AgentTraceExportReader`.
  - Dependencies: none
  - Done when: a `/state` response with any cursor field `< 0` or `> 9_007_199_254_740_991`
    yields `ControlPlaneError::InvalidResponse` from `ingestion_state`, for each of the four
    fields independently; boundary values `0`, `1`, and `9_007_199_254_740_991` are accepted; an
    orchestration-level test (mocked control plane returning an invalid `/state` cursor) proves
    `run_sync_against` fails before any `/batch` request is sent.
  - Verification notes (commands or checks): `cargo test --manifest-path cli/Cargo.toml agent_trace_cursors`; `cargo test --manifest-path cli/Cargo.toml invalid_state_cursor_fails_before_any_batch_request`
  - Completed: 2026-08-11
  - Files changed: `cli/src/services/agent_trace_sync/control_plane.rs`, `cli/src/services/trace/sync.rs`
  - Evidence: Added `AgentTraceCursors::validate`, called from `send_state_request` after JSON
    decoding, so both the initial `/state` call and the `/state` refetch used by reconciliation
    are covered by one change point. Added `agent_trace_cursors_accept_boundary_values` and
    `agent_trace_cursors_reject_out_of_range_value_in_each_field` (table-driven across all four
    fields, boundaries `0`/`1`/`9_007_199_254_740_991` valid, `-1`/`9_007_199_254_740_992`
    invalid) in `control_plane.rs`, and `invalid_state_cursor_fails_before_any_batch_request` in
    `trace/sync.rs` (mocked `/state` returning an out-of-range `agentTraces` cursor; asserts
    `TraceSyncError::ControlPlane(ControlPlaneError::InvalidResponse(_))` and
    `server.call_count() == 1`, proving no `/batch` request was sent). `cargo fmt --manifest-path
    cli/Cargo.toml` applied to satisfy formatting. Ran `nix flake check`: all checks passed
    (`cli-tests`, `cli-clippy`, `cli-fmt`, plus the repo's other checks).
  - Notes: none.

- [x] T02: `Make access-token expiry a single decision` (status:done)
  - Task ID: T02
  - Goal: Rewrite `AuthenticatedControlPlaneClient::resolve_access_token` to call
    `auth::is_stored_token_expired` exactly once and branch on that single result — refresh via
    `auth::renew_stored_token_from_refresh_token` and save only in the expired branch, otherwise
    return the stored access token unchanged with no HTTP call and no save.
  - Boundaries (in/out of scope): In — `resolve_access_token` body only. Out —
    `force_refresh_access_token` (the unexpected-`401` path), `execute_authenticated`,
    `auth::ensure_valid_token_returning_token` itself (still used by `auth_command`), the device
    authorization flow.
  - Done when: a valid stored token path makes zero refresh-endpoint HTTP calls and zero
    `CredentialStore::save` calls; an expired stored token path makes exactly one refresh-endpoint
    call, exactly one `save` call, and the control-plane request carries the refreshed token; the
    existing `valid_token_is_reused_without_resave`, `valid_token_is_not_resaved`, and
    `expired_token_is_refreshed_and_saved` tests pass against the rewritten implementation
    without depending on wall-clock timing.
  - Dependencies: none
  - Verification notes (commands or checks): `cargo test --manifest-path cli/Cargo.toml control_plane::tests`
  - Completed: 2026-08-11
  - Files changed: `cli/src/services/agent_trace_sync/control_plane.rs`
  - Evidence: Rewrote `resolve_access_token` to call `auth::is_stored_token_expired` exactly
    once and branch on that single result: a non-expired stored token is returned immediately
    with no HTTP call and no save, while an expired one is refreshed via
    `auth::renew_stored_token_from_refresh_token` and saved before its access token is returned.
    Removed the prior two-call sequence through `auth::ensure_valid_token_returning_token`
    (still used elsewhere by `auth_command`, left unchanged) that let the two expiry checks
    disagree at the boundary. `valid_token_is_reused_without_resave`,
    `valid_token_is_not_resaved`, and `expired_token_is_refreshed_and_saved` pass unmodified
    against the rewritten implementation, as do the unexpected-`401` tests
    (`force_refresh_access_token`/`execute_authenticated` untouched). Ran
    `nix build .#checks.x86_64-linux.cli-tests`: all 280 tests pass. Ran
    `nix build .#checks.x86_64-linux.cli-clippy`: clean.
  - Notes: none.

- [x] T03: `Add safe HTTP error-body parsing and a terminal-protocol error variant` (status:done)
  - Task ID: T03
  - Goal: Replace raw-body exposure in `classify_response` with a narrow safe parser, and add a
    `ControlPlaneError` variant for terminal-but-otherwise-uncategorized `4xx` statuses
    (`404`/`405`/`415`/`422` and similar) distinct from `InvalidResponse`, which becomes reserved
    for a syntactically successful (`2xx`) but undecodable body.
  - Boundaries (in/out of scope): In — a private `fn extract_safe_error_message(body: &str) ->
    Option<String>` that accepts only a top-level JSON object with a `message` or `error` string
    field under a fixed max length, returning `None` for malformed JSON, HTML, non-object/array
    top-level values, missing/non-string fields, or oversized bodies; `classify_response` uses it
    to build sanitized `BadRequest`/`Forbidden`/`Conflict`/`ServerError` messages with a
    status-specific generic fallback (e.g. `400` -> "control plane rejected the Agent Trace
    request", `403` -> "Agent Trace source cannot be synchronized by the current authenticated
    user", `409` -> "Agent Trace cursor conflict", `500` -> "control plane encountered an internal
    error", `503` -> "control-plane Agent Trace storage is unavailable"); a new
    `ControlPlaneError::Protocol { status: reqwest::StatusCode, message: String }` variant (with a
    `Display` impl) for other terminal `4xx` responses, built the same sanitized way. Out —
    changing which statuses map to `BadRequest`/`Forbidden`/`Conflict`/`ServerError` (unchanged);
    `trace/sync.rs::is_stream_terminal` (T04).
  - Dependencies: none
  - Done when: a `500` response whose body contains `SQLITE_ERROR`, a table name, or a filesystem
    path never appears in the resulting error's `Display` output; a `{"message": "..."}` body
    surfaces exactly that string and no other field; an HTML `500` body produces the generic
    server-error message with no HTML in it; a `404`/`405`/`415`/`422` response yields
    `ControlPlaneError::Protocol` rather than `InvalidResponse`.
  - Verification notes (commands or checks): `cargo test --manifest-path cli/Cargo.toml server_error_body_is_not_leaked known_safe_error_payload_is_surfaced html_error_body_falls_back_to_generic_message batch_classifies_404_as_protocol_error`
  - Completed: 2026-08-11
  - Files changed: `cli/src/services/agent_trace_sync/control_plane.rs`, `cli/src/services/agent_trace_sync/test_http_server.rs`
  - Evidence: Added `extract_safe_error_message` (top-level JSON object only, `message` then `error` string field, max 500 bytes, rejects malformed/HTML/non-object/oversized bodies) and `safe_error_message` helper; rewrote `classify_response`'s non-2xx branch to use them with status-specific generic fallbacks for `400`/`403`/`409`/`5xx`(including a dedicated `503` fallback), and to return the new `ControlPlaneError::Protocol { status, message }` variant (with `Display` impl) for any other client-error (`4xx`) status instead of folding it into `InvalidResponse`; `InvalidResponse` for non-4xx/non-5xx unexpected statuses no longer includes the raw body. Added a `CannedResponse::text` constructor to the in-repo test HTTP server (needed to send a non-JSON body) and four tests: `server_error_body_is_not_leaked` (SQL-error-shaped `500` body never appears in `Display` output), `known_safe_error_payload_is_surfaced` (`{"message": ...}` on `400` surfaces exactly that string), `html_error_body_falls_back_to_generic_message` (HTML `500` body produces the generic message with no HTML), and `batch_classifies_404_as_protocol_error` (`404` yields `ControlPlaneError::Protocol` carrying the extracted message). Ran `nix build .#checks.x86_64-linux.cli-tests`: 284 tests pass (up from 280 in T02). Ran `nix build .#checks.x86_64-linux.cli-clippy`: clean. Ran `nix build .#checks.x86_64-linux.cli-fmt`: clean.
  - Notes: `cargo test`/`cargo clippy` direct invocation is blocked by this repo's bash-tool policy (`use-nix-flake-check-over-cargo-test`, not satisfiable by any wrapper); ran the equivalent `nix build .#checks.x86_64-linux.{cli-tests,cli-clippy,cli-fmt}` derivations instead, which cover the same test filters plus the full suite.

- [x] T04: `Keep terminal 4xx statuses out of /batch ambiguous reconciliation` (status:done)
  - Task ID: T04
  - Goal: `trace/sync.rs::is_stream_terminal` treats `ControlPlaneError::Protocol(_)` as terminal
    alongside the existing `MissingCredentials`/`AuthenticationFailed`/`BadRequest`/`Forbidden`/
    `Storage` cases, so a `404`/`405`/`415`/`422` (and post-refresh `401`, already covered by
    `AuthenticationFailed`) during `/batch` fails the stream immediately with no `/state`
    refetch, while `409`, transport failures, `5xx`, and a malformed `2xx` batch body (still
    `InvalidResponse`, not in `is_stream_terminal`) continue to reconcile via `/state` exactly as
    before.
  - Boundaries (in/out of scope): In — `is_stream_terminal` in `trace/sync.rs`; new orchestration
    tests proving the terminal-vs-ambiguous split for `/batch`; the `context/cli/agent-trace-
    sync-command.md` "Recovery semantics" update. Out — `sync_stream`'s reconciliation loop logic
    itself (unchanged), the `/state` path (already terminal on invalid response per T01).
  - Dependencies: T03
  - Done when: an orchestration test with a mocked `404` `/batch` response proves the sync fails
    with no follow-up `/state` call and no batch resend; a second orchestration test with a
    mocked `2xx` `/batch` response carrying an undecodable body proves sync still reconciles via
    `/state` (regression coverage for the still-ambiguous case); `context/cli/agent-trace-sync-
    command.md`'s "Recovery semantics" section names the terminal-`4xx`/sanitized-error behavior.
  - Verification notes (commands or checks): `cargo test --manifest-path cli/Cargo.toml terminal_batch_status_fails_without_state_reconciliation malformed_2xx_batch_response_still_reconciles_via_state`
  - Completed: 2026-08-11
  - Files changed: `cli/src/services/trace/sync.rs`, `context/cli/agent-trace-sync-command.md`
  - Evidence: Added `ControlPlaneError::Protocol { .. }` to `is_stream_terminal`'s match arm in
    `trace/sync.rs`, alongside the existing `MissingCredentials`/`AuthenticationFailed`/
    `BadRequest`/`Forbidden`/`Storage` cases, and updated its doc comment. Added
    `terminal_batch_status_fails_without_state_reconciliation` (mocked `404` `/batch` response;
    asserts `TraceSyncError::Stream { stream: "messages", .. }` and `server.call_count() == 2`,
    proving no `/state` refetch and no batch resend followed the terminal status) and
    `malformed_2xx_batch_response_still_reconciles_via_state` (mocked `2xx` `/batch` response with
    an undecodable body; asserts the sync still succeeds via `/state` reconciliation and resend,
    with `server.call_count() == 7` covering the initial `/state`, the malformed batch, the
    refetch, the resent + successful messages batch, and the three remaining streams' batches).
    Updated `context/cli/agent-trace-sync-command.md`'s "Recovery semantics" section: reworded the
    ambiguous-batch-failure bullet to name an undecodable `2xx` body instead of "missing/invalid
    response", clarified the "Invalid response" bullet to state it reconciles via `/state` rather
    than failing outright, and added a new terminal-protocol-mismatch bullet
    (`404`/`405`/`415`/`422`/other unrecognized `4xx` -> `ControlPlaneError::Protocol`, terminal
    like `400`/`403`) and a sanitized-error-message bullet (T03's narrow `message`/`error`
    extraction, no raw body ever surfaced). Ran `nix build .#checks.x86_64-linux.cli-tests`: 286
    tests pass (up from 284 in T03; an initial run showed 4 unrelated failures in
    `agent_trace_db`/`agent_trace_export` from `UNIQUE constraint failed`/row-count-mismatch
    errors — reran with no changes and got 286/286 passing, confirming pre-existing test-isolation
    flakiness unrelated to this task, not a regression from this change). Ran `nix build
    .#checks.x86_64-linux.cli-clippy`: clean. Ran `nix build .#checks.x86_64-linux.cli-fmt`: clean
    after reformatting one `assert!(matches!(...))` in the new terminal-status test.
  - Notes: `cargo test`/`cargo clippy` direct invocation is blocked by this repo's bash-tool policy
    (`use-nix-flake-check-over-cargo-test`); ran the equivalent `nix build
    .#checks.x86_64-linux.{cli-tests,cli-clippy,cli-fmt}` derivations instead, which cover the
    same test filters plus the full suite. `nix run .#pkl-check-generated` and `nix flake check`
    (the plan's "Full validation" commands) were not run for this single-task verification per the
    workflow's narrowest-authoritative-check guidance; no `.pkl` schema or flake-wide surface was
    touched by this task.

## Open questions

None. The three fixes, their required behavior, and their test coverage are fully specified by
the change request against code already read in `control_plane.rs`, `agent_trace_sync/mod.rs`,
and `trace/sync.rs`.

## Validation Report

**Status:** validated  
**Date:** 2026-08-11

### Commands run

- `cargo test --manifest-path cli/Cargo.toml` (via `nix build .#checks.x86_64-linux.cli-tests --rebuild -L`) -> exit 0 (286 passed; 0 failed; 0 ignored)
- `cargo clippy --manifest-path cli/Cargo.toml -- -D warnings` (via `nix build .#checks.x86_64-linux.cli-clippy -L`) -> exit 0 (clean)
- `nix run .#pkl-check-generated` -> exit 0 (101 files, ephemeral generation matched committed output)
- `nix flake check` -> exit 0 (all checks passed)
- `nix build .#checks.x86_64-linux.cli-fmt -L` -> exit 0 (clean; ran in addition to `nix flake check` to directly confirm formatting)

### Scaffolding removed

- None.

### Success-criteria verification

- [x] AC1: `/state` cursor validation -> `agent_trace_cursors_accept_boundary_values`, `agent_trace_cursors_reject_out_of_range_value_in_each_field`, and `invalid_state_cursor_fails_before_any_batch_request` all pass.
- [x] AC2: Single expiry decision for access-token resolution -> `valid_token_is_reused_without_resave`, `valid_token_is_not_resaved`, and `expired_token_is_refreshed_and_saved` all pass.
- [x] AC3: Unexpected-401 behavior unchanged -> `unexpected_401_refreshes_once_and_retries_once_on_success` and `unexpected_401_twice_fails_without_a_third_attempt` both pass.
- [x] AC4: No raw server body leaks into control-plane errors -> `server_error_body_is_not_leaked`, `known_safe_error_payload_is_surfaced`, and `html_error_body_falls_back_to_generic_message` all pass.
- [x] AC5: Terminal `4xx` batch statuses skip `/state` reconciliation; ambiguous cases still reconcile -> `terminal_batch_status_fails_without_state_reconciliation` and `malformed_2xx_batch_response_still_reconciles_via_state` both pass.

### Failed checks and follow-ups

- None.

### Residual risks

- None identified.
