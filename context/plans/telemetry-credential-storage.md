# Plan: telemetry-credential-storage

## Change summary

Adds the CLI-side foundation for a telemetry-only OpenBao credential: a typed client for the already-implemented `POST /auth/telemetry/credential`, and a separate encrypted `auth.db` record holding the issued token. The backend owns identity and workspace enforcement, so the CLI does not read `user.id` / `workspace.id` from `GET /me` and does not bind or compare identity on lookup. `sce auth logout` also removes the cached credential.

This extends the existing `AuthenticatedControlPlaneClient` and the encrypted `AuthDb` without altering the WorkOS `auth_credentials` row. There is no user-facing provisioning command, exporter, refresh loop, or doctor check. The change is independent of PR #306.

## Acceptance criteria

- [ ] AC1: The client posts to `/auth/telemetry/credential` with the WorkOS bearer and no body, validates `{ token, accessor, expiresAt }`, and maps 401/403/503/500/malformed/transport failures to sanitized errors; neither errors nor `Debug` output contain token, accessor, Authorization header, or raw body.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml telemetry_credential_client`
- [ ] AC2: A telemetry credential persists in a dedicated encrypted `auth.db` table and survives reopening the database; expired or corrupt records are not returned, and lookup never mints.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml telemetry_credential_storage`
- [ ] AC3: Concurrent saves leave exactly one complete, valid record.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml telemetry_credential_concurrent`
- [ ] AC4: `sce auth logout` deletes the telemetry credential; WorkOS credentials are unaffected by telemetry save/delete.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml telemetry_credential_logout` and `... auth_command`
- [ ] AC5: Context documents the schema, lifecycle, and security boundaries, including that local deletion does not revoke the OpenBao token and that identity enforcement is the backend's.
  - Validate: inspect `context/sce/auth-db.md` and `context/sce/telemetry-credential.md`.

### Full validation

- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml telemetry_credential`
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml auth`
- `nix flake check`

### Context sync

- `context/sce/auth-db.md` (new migration and table, logout behavior)
- `context/sce/telemetry-credential.md` (new: lifecycle, security boundaries, non-revocation)
- `context/context-map.md` (link the new file)

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** `cli/src/services/agent_trace_sync/control_plane.rs`, `cli/src/services/auth_db/`, a new telemetry credential storage module beside `token_storage.rs`, `cli/src/services/auth_command/mod.rs` (logout), `cli/migrations/auth/`, `context/sce/`.
- **Out of scope:** `/me` identity decoding, identity/workspace binding or comparison, `sce auth telemetry` or any provisioning command, OTLP exporter, collector config, telemetry mode, `sce doctor` checks, Claude hooks, token refresh, server-side revocation, Control Plane changes, PR #306.
- **Constraints:** concrete types and static dispatch (the existing `Box<dyn CredentialStore>` is the only permitted `dyn`); no detached tasks or background loops; no new env vars; no secrets in logs, spans, output, errors, or `Debug`; reuse `AuthDb` and its encryption; no production-network tests; no code comments.
- **Non-goal:** generalizing token storage into a multi-credential framework.

## Assumptions

- The backend enforces user and workspace; the CLI stores one credential row and returns it if unexpired (decision by the user, replacing the earlier identity-binding design).
- Telemetry errors use fixed messages and do not reuse `safe_error_message`.
- Concurrent writers are safe via single-statement atomic replacement of one fixed row (last complete writer wins).
- Migration is `003_create_telemetry_credentials.sql` with an `updated_at` trigger mirroring 002.

## Task stack

- [x] T01: `Add encrypted telemetry credential table and storage operations` (status:done)
  - Task ID: T01
  - Scope: In — migration `003`, storage module with `save` (single-statement atomic replace), `load` (returns only unexpired, valid rows), `delete`, redacted `Debug`; tests for reopen-after-restart, expiry, corrupt row, WorkOS row preservation, concurrent writers. Out — HTTP client, logout wiring, identity checks.
  - Dependencies: none
  - Done when: a saved credential round-trips while unexpired; expired or invalid rows are not returned; the WorkOS `auth_credentials` row is untouched; concurrent saves leave one complete row.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml telemetry_credential_storage` and `telemetry_credential_concurrent`; migration-count test in `auth_db`.
  - Completed: 2026-10-14
  - Files changed: `cli/migrations/auth/003_create_telemetry_credentials.sql`, `cli/src/services/telemetry_credential_storage.rs`, `cli/src/services/mod.rs`, `cli/src/services/token_storage.rs`, `cli/src/services/auth_db/mod.rs`
  - Result: Added migration 003 (single fixed-row `telemetry_credentials` table with `updated_at` trigger) and a storage module with `save` (single-statement `INSERT OR REPLACE`), `load` (unexpired, non-empty, non-negative-expiry rows only), `delete`, and redacted `Debug`; `get_auth_db` made `pub(crate)` for reuse. Fixed DB error messages carry no secrets.
  - Verify outcome: `telemetry_credential` tests passed (6: round-trip across reopen, expired, corrupt, WorkOS row preserved, redacted Debug, concurrent saves); `auth_db` tests passed including the extended migration test for the new table and trigger. `nix flake check` not run for this task.
  - Context impact: minor — new auth.db table and storage module; documented in T03.
  - Context synchronization: synced

- [x] T02: `Add typed telemetry credential client and save-on-issue` (status:done)
  - Task ID: T02
  - Scope: In — `issue_telemetry_credential` on `AuthenticatedControlPlaneClient` via `execute_authenticated`, response type with redacted `Debug`, validation (non-empty token/accessor, parseable `expiresAt`), sanitized error classification, one function that issues and saves; tests against `TestHttpServer`. Out — CLI command surface, refresh.
  - Dependencies: T01
  - Done when: request has bearer, POST, no body; success persists the credential; each failure class maps to a fixed message and leaves prior stored state intact; no secret appears in error `Display` or `Debug`.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml telemetry_credential_client`
  - Completed: 2026-10-14
  - Files changed: `cli/src/services/agent_trace_sync/control_plane.rs`
  - Result: Added `issue_telemetry_credential` (POST `auth/telemetry/credential`, WorkOS bearer, no body) with a redacted-Debug response type, validation of non-empty token/accessor and future RFC3339 `expiresAt`, fixed-message error classification, and `issue_and_save_telemetry_credential` which persists only after successful issuance. Comments stripped from the file per user request.
  - Verify outcome: `telemetry_credential` tests passed (9: 3 new client tests covering request shape, 403/503/500/401/malformed, transport sanitization; 6 storage). The save-on-issue path against the real auth DB is not exercised by a test; failure leaves state intact because issuance errors return before save. `nix flake check` not run.
  - Context impact: minor — new client method; documented in T03.
  - Context synchronization: synced

- [ ] T03: `Delete telemetry credential on logout and document lifecycle` (status:todo)
  - Task ID: T03
  - Scope: In — `run_logout` also deletes the telemetry credential (existing output contract preserved), logout test, `context/sce/auth-db.md` update, new `context/sce/telemetry-credential.md`, context-map link. Out — new CLI output fields.
  - Dependencies: T01
  - Done when: logout removes both records and a failure in either surfaces a storage error; docs state schema, lifecycle, backend-owned identity, and that local deletion does not revoke the OpenBao token.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml telemetry_credential_logout`; `... auth_command`.
  - Context synchronization: pending

## Open questions

- None. Identity binding was removed at the user's direction; the backend is the authority on user and workspace.
