# Plan: doctor-verify-workos-session

## Change summary

Under `auth.mode: mandatory`, `sce doctor` inspects the saved WorkOS session
offline (`access::inspect_login` over `token_storage::load_current_tokens()`):
any non-empty refresh token reports `logged_in`. WorkOS ends sessions after
inactivity, and a refresh token WorkOS has already invalidated is still
non-empty, so doctor and the Claude Code status line
(`config/lib/sce-info/hooks/register.ts`, which renders `sce_login.state`) show
"logged in" while `sce auth whoami` and `sce sync` fail with `Stored WorkOS
refresh token is no longer valid (Session ended due to inactivity.)`.

This plan changes the doctor login row from an offline storage check to a
verified session check, and only when login is mandatory for the inspected
repository. The user decided that doctor may make a network request in that
case. This reverses the current contract statement that doctor never makes a
network request to inspect the login. The stored access-token expiry
(`stored_at_unix_seconds + expires_in`, about 1h) acts as the cache: while it is
valid, no network call is made. Once it has expired, doctor refreshes through
the existing `auth::ensure_valid_token_returning_token` under
`CREDENTIAL_LOOKUP_DEADLINE` and saves the refreshed tokens, which also counts
as WorkOS session activity. A refresh WorkOS rejects reports `not_logged_in`
with the WorkOS reason. A network failure keeps `logged_in` while the last
successful login or refresh is under 24h old, and otherwise reports the existing
`unavailable` state with a new reason, so the status line does not flip from
green to yellow on transient failures. Non-mandatory modes keep skipping every
lookup. The existing `unavailable` reasons, status-line labels, and the
separation of login status from telemetry credentials stay unchanged.

## Acceptance criteria

- [ ] AC1: Under `mandatory`, with an unexpired stored access token, doctor reports `logged_in` without any network call.
  - Validate: `cargo test -p sce doctor::access` covers the fresh-token case with a refresh seam that fails the test if called.
- [ ] AC2: Under `mandatory`, with an expired access token and a refresh WorkOS accepts, doctor reports `logged_in` and persists the refreshed tokens.
  - Validate: `cargo test -p sce doctor::access` asserts `logged_in` and that the save seam received the refreshed token.
- [ ] AC3: When WorkOS rejects the refresh (`AuthError::Unauthorized`, e.g. inactivity), doctor reports a blocking `[FAIL] SCE login` whose problem summary includes the WorkOS reason, and JSON `sce_login.state` is `not_logged_in`, so the Claude status line shows red and the session-start login notice fires.
  - Validate: `cargo test -p sce doctor` asserts the state, summary text, and rendered JSON.
- [ ] AC4: When the refresh fails on transport or deadline, doctor reports `logged_in` if `now - stored_at_unix_seconds < 24h`, otherwise `[WARN] SCE login status unavailable` with JSON `sce_login.state: "unavailable"` and `reason: "session_unreachable"`.
  - Validate: `cargo test -p sce doctor::access` covers both sides of the 24h boundary; `cargo test -p sce doctor` covers rendering.
- [ ] AC5: A rejected refresh that races with a newer stored session (another SCE process rotated the refresh token) reports `logged_in`, not `not_logged_in`.
  - Validate: `cargo test -p sce doctor::access` covers the reload-after-rejection case.
- [ ] AC6: Non-mandatory modes, missing tokens, and the existing `unavailable` storage reasons keep their current results.
  - Validate: existing `cargo test -p sce doctor` cases pass unchanged.

### Full validation

- `nix flake check`
- `cargo clippy --all-targets -- -D warnings` (from `cli/`)

### Context sync

- `context/sce/doctor-human-text-contract.md`: SCE login row becomes a verified session check under `mandatory`, with the refresh, save, grace, and `session_unreachable` behavior and the WorkOS reason in the summary. Remove "offline storage check … never refreshes credentials" and "never … makes a network request to inspect the login".
- `context/cli/access-policy.md`: the `sce doctor` SCE login row bullet.
- `context/sce/auth-db.md`: doctor becomes a non-interactive writer of refreshed `auth_credentials`.

## Task context synchronization lifecycle

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** `cli/src/services/doctor/access.rs`, `cli/src/services/doctor/access/tests.rs`, `cli/src/services/doctor/mod.rs`, `cli/src/services/doctor/render.rs`, and `cli/src/services/token_storage.rs` only if a non-interactive save into an existing current DB needs a helper.
- **Out of scope:** telemetry credential checks in login status; status-line or hook changes (the existing `unavailable` and `not_logged_in` labels already cover the new outcomes); typed `SessionExpired` error for `whoami`; any verification cache table; forced verification on Claude session start; WorkOS dashboard inactivity settings.
- **Constraints:**
  - Network access happens only under effective `auth.mode: mandatory`, and only when the stored access token has expired.
  - Doctor stays non-interactive: it never creates `auth.db` or a credential-store key and never migrates the schema; refreshed tokens are saved with `CredentialAccess::NonInteractive` into the existing current DB.
  - The refresh is bounded by `CREDENTIAL_LOOKUP_DEADLINE`; a timeout is a transport failure.
  - Doctor output never includes token values; the WorkOS reason is the `error_description` already carried in `AuthError::Unauthorized`.
  - Typed errors and static dispatch per `AGENTS.md`; extend the existing `inspect_login_with` seam pattern for tests.
- **Non-goal:** detecting server-side revocation inside the access-token lifetime; it surfaces at the next refresh (≤ ~1h).

## Assumptions

- The 24h grace window and the 2s `CREDENTIAL_LOOKUP_DEADLINE` are the defaults agreed with the user.
- The WorkOS client id comes from `config::resolve_auth_runtime_config` for the inspected repository, so `inspect_login` takes the repository root; a missing client id maps to `unavailable` with a dedicated reason rather than `not_logged_in`.
- Carrying the WorkOS reason may require `LoginState` and `DoctorAccess` to become `Clone` instead of `Copy`.

## Task stack

- [x] T01: `Verify the saved WorkOS session by refreshing an expired access token` (status:done)
  - Task ID: T01
  - Scope: In — extend `inspect_login_with` so a found session with an unexpired access token stays `LoggedIn` with no network; an expired access token is refreshed through `auth::ensure_valid_token_returning_token` under the deadline and saved non-interactively; rejection → `NotLoggedIn` (with the concurrent-rotation reload); transport/deadline → `LoggedIn` within 24h of `stored_at_unix_seconds`, else `Unavailable(SessionUnreachable)`; missing client id → `Unavailable` with its own reason; wire the repository root from `execute_doctor_with_context`; unit tests. Out — carrying the WorkOS reason into output (T02).
  - Dependencies: none
  - Done when: AC1, AC2, AC4, AC5, and AC6 hold.
  - Verify: `cargo test -p sce doctor::access` — passed via `nix build .#checks.x86_64-linux.cli-tests` (repository bash policy blocks direct `cargo test`); all 12 `services::doctor::access::tests` pass; suite result `850 passed; 0 failed`. `nix build .#checks.x86_64-linux.cli-fmt .#checks.x86_64-linux.cli-clippy` — passed.
  - Completed: 2026-11-17
  - Files changed: `cli/src/services/doctor/access.rs`, `cli/src/services/doctor/access/tests.rs`, `cli/src/services/doctor/mod.rs`
  - Result: `inspect_login(repository_root)` resolves the WorkOS client id and passes `inspect_login_with(deadline, lookup, renew)` two seams. An unexpired access token reports `LoggedIn` with no network. An expired one runs refresh and `save_tokens(NonInteractive)` as one deadline-bounded unit. `Unauthorized` reports `NotLoggedIn` unless a reload shows a rotated refresh token (`LoggedIn`). `MissingClientId` reports `Unavailable(AuthConfigUnavailable)`. A save failure reports the matching storage reason. Other auth errors or a missed deadline report `LoggedIn` within 24h of `stored_at_unix_seconds`, otherwise `Unavailable(SessionUnreachable)`. New JSON reasons: `session_unreachable`, `auth_config_unavailable`. Deviation: uses `auth::renew_stored_token_from_refresh_token` instead of `ensure_valid_token_returning_token`, because doctor already performs the expiry check. Same behavior.
  - Context impact: doctor login row contract (`context/sce/doctor-human-text-contract.md`), access-policy doctor login bullet (`context/cli/access-policy.md`), and auth DB writers (`context/sce/auth-db.md`) now differ from code: doctor makes a network refresh under `mandatory` and saves refreshed tokens, and adds two `unavailable` reasons.
  - Context synchronization: synced

- [ ] T02: `Report the WorkOS rejection reason in the doctor login problem` (status:todo)
  - Task ID: T02
  - Scope: In — carry the `AuthError::Unauthorized` message from a rejected refresh into the `sce_login_required` problem summary in text and JSON `problems`; keep `sce_login.state: "not_logged_in"` and the existing remediation; render tests. Out — new login states or status-line labels.
  - Dependencies: T01
  - Done when: AC3 holds and existing doctor render tests pass.
  - Verify: `cargo test -p sce doctor`
  - Context synchronization: pending

## Open questions

- The keep-alive effect assumes WorkOS counts a refresh-token exchange as session activity for its inactivity timeout. That doesn't affect this plan's correctness, because a rejected refresh is still reported accurately. It's worth confirming in the WorkOS environment's session settings before relying on it to prevent expiry.
