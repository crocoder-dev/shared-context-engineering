# Plan: remove-telemetry-readiness-marker

## Change summary

Remove the non-secret `<state_root>/sce/telemetry-ready.json` readiness hint and make the encrypted `auth.db` the only source of truth for whether a usable telemetry credential exists. The effective access policy (global, Git-root `.sce/config.json`, and explicit config layers) still decides whether telemetry is permitted. The OS credential store still guards the encryption key. Nothing replaces the marker: no cache file, no new database, no new persisted state.

This replaces existing behavior. Today every telemetry consumer must see both the marker and a valid `auth.db` row. The marker can go stale, it adds write and clear steps to login, logout, and session replacement, and it duplicates what `auth.db` already records. Its one real job is to keep hooks away from `auth.db` and the Keychain when telemetry was never provisioned. This plan keeps that protection with an existence check on `auth.db` plus a typed lookup. It also closes a hazard that removing the marker would otherwise widen: today the read-only lookup path silently migrates an outdated `auth.db`.

### Current implementation findings

Marker module and path:

- `cli/src/services/telemetry_readiness.rs`: `ReadinessState { NotConfigured, Expired, Ready }`, `mark_ready`, `clear`, `is_ready`, `state`, `mark_ready_at`, `clear_at`, `state_at`, `is_ready_at` (test-only), `temp_path_for`. Its three tests: `readiness_state_requires_valid_unexpired_marker_and_auth_db`, `marked_readiness_is_ready_until_expiry_and_holds_no_secret`, `clearing_readiness_disables_it_and_is_idempotent`.
- `cli/src/services/mod.rs:40`: `pub mod telemetry_readiness;`.
- `cli/src/services/default_paths.rs:257`: `telemetry_readiness_path()`.

Code that reads the marker:

- `cli/src/services/managed_telemetry.rs`:
  - `RuntimeTelemetry::resolve` passes `|| telemetry_readiness::is_ready(now)` into `resolve_when_ready`. When that returns false, the result is Noop `NotConfigured` and `auth.db` is never opened.
  - `inspect_health` passes `telemetry_readiness::state(now)` and `readiness_hint_present()` into `inspect_health_with`. `NotConfigured` and `Expired` hints return without a lookup.
  - `ManagedTelemetryHealth::DisabledByConfig { readiness_hint_present }`.
- `cli/src/services/auth_command/mod.rs`: `run_telemetry_headers` passes `|| telemetry_readiness::is_ready(..)` as `ready_probe` to `telemetry_headers_with`. When it fails, the helper fails closed before repository resolution, which spawns `git config`.
- `cli/src/services/doctor/render.rs:318-330`: `readiness_hint_present` decides whether the `OPT_IN_NOTE` line is shown under `[PASS] Managed telemetry (disabled by config)`.

Code that writes or clears the marker (`cli/src/services/auth_command/mod.rs`):

- `provision_telemetry_credential` calls `record_telemetry_readiness(.., mark_ready, clear)`. The result is `Issued` only when the credential was issued, persisted, and the marker was written. Otherwise the marker is cleared and the result is `Unavailable`.
- `run_logout` passes `telemetry_readiness::clear` into `run_logout_with`. The clear runs first, and a failure aborts logout.
- `save_new_session_tokens` passes `telemetry_readiness::clear` into `replace_session_with`, before the old credential is invalidated and the new session is saved.

Tests that depend on the marker:

- `auth_command/mod.rs`:
  - `telemetry_readiness_is_recorded_only_after_credential_persistence`
  - `new_session_invalidates_previous_telemetry_credential_before_saving` (expects `["clear", "invalidate", "save"]`)
  - the logout tests that expect `["clear-readiness", "delete-workos", "delete-telemetry"]` (around lines 972, 1293, and 1668)
  - the `telemetry_headers_with` table test, which has a `ready` column and a `probed` flag
- `managed_telemetry.rs`:
  - `claude_hook_without_readiness_never_looks_up_credentials`
  - `telemetry_disabled_by_config_never_probes_readiness_or_looks_up_credentials`
  - `health_disabled_by_config_never_probes_readiness_or_looks_up_credentials`
  - `health_without_readiness_or_with_expired_readiness_never_looks_up_credentials`
  - `health_with_readiness_classifies_lookup_outcomes`
  - `health_lookup_is_bounded_by_the_credential_lookup_deadline`
  - `claude_hook_with_inaccessible_credential_store_skips_quietly`
- `auth_db/multiprocess_tests.rs`:
  - `seed()` calls `telemetry_readiness::mark_ready(FAR_FUTURE)` (line 1211)
  - `doctor_telemetry_health_classifies_real_auth_db_states_without_mutating_them` writes an expired marker and removes it (lines 481-492)
  - `claude_hook_with_telemetry_disabled_by_config_never_resolves_the_credential` asserts the marker exists (line 872)
- `app/doctor_reconciliation_cli_tests.rs`: lines 647, 656, and 769 write `FAR_FUTURE_READINESS` or assert the marker is absent.

Credential lookup today:

- `telemetry_credential_storage::load_telemetry_credential` and `load_repository_telemetry_credential` call `token_storage::open_existing_auth_db(CredentialAccess::NonInteractive)`. That calls `EncryptedTursoDb::open_existing_at`, which returns `None` when `auth.db` is missing; `open_lock_file(.., false)` runs before any key resolution.
- `load_with` uses an inner `JOIN auth_credentials` and returns `Option`. Several different states all collapse to `None`: no row, no WorkOS session, expired, identity-less (pre-`004`), empty fields, and an ownership mismatch.
- Today the `credential_expired` diagnosis comes only from the marker (`ReadinessState::Expired`) or from `select_mode` on a returned row. A storage-level expiry is reported as `credential_unavailable`.
- Hazard: `EncryptedTursoDb::open_locked` always calls `TursoConnectionCore::ensure_migrations_current()`. On an outdated or schemaless `auth.db`, the non-interactive hook lookup therefore runs a `BEGIN IMMEDIATE` migration. Today the marker usually prevents this, because only a current-binary login writes it. Without the marker, every admitted Claude hook would migrate a legacy `auth.db` that has no credential, and `auth-db.md` documents an upstream Turso race when many processes migrate concurrently.

Other facts the plan relies on:

- `sce setup` no longer creates `auth.db`; `AuthDbLifecycle::setup` only creates the parent directory. So `auth.db` exists only after `sce auth login`, or on installs whose `auth.db` was created by an older `sce setup`.
  - Drift: `context/overview.md:71` still says the auth_db provider runs `AuthDb::new()` setup.
- `managed_span_kind` (`cli/src/app.rs:308`) admits Claude hooks and ordinary commands to the startup lookup. `hooks` that are not Claude hooks, `setup`, and `auth telemetry-headers` stay Noop without a lookup.
- `CREDENTIAL_LOOKUP_DEADLINE` (2 s) bounds both lookups through `run_with_deadline`.
- `context/decisions/2026-10-20-consent-gated-access-policy-precedence.md:53` already rejects inferring consent from `telemetry-ready.json` or `auth.db`. This plan keeps that: `auth.db` affects credential readiness only, never consent.

### Existing vs proposed readiness flow

Existing:

```text
policy disabled -> Noop(disabled_by_config)
policy enabled
  -> marker missing/corrupt or auth.db missing -> Noop(not_configured)   [no DB, no Keychain]
  -> marker expired                            -> Noop(credential_expired) [no DB, no Keychain]
  -> marker ready -> non-interactive lookup (may MIGRATE an outdated DB)
       valid -> Managed; None -> credential_unavailable; errors -> classified
```

Proposed:

```text
policy disabled -> Noop(disabled_by_config)            [no stat of credentials, no DB, no Keychain]
policy enabled
  -> auth.db missing (open without O_CREAT)  -> Noop(not_configured)    [no key resolution]
  -> auth.db exists -> resolve key with CredentialAccess::NonInteractive
       key inaccessible                       -> Noop(credential_inaccessible)
       open, schema not current               -> Noop(credential_unavailable) [never migrates, not cached]
       open, schema current -> typed lookup within CREDENTIAL_LOOKUP_DEADLINE
         Missing   (no telemetry row)        -> Noop(not_configured)
         Expired   (owned, complete, expired) -> Noop(credential_expired)
         Unusable  (no WorkOS session, identity mismatch, legacy/empty/negative row) -> Noop(credential_unavailable)
         Valid     -> Managed (bearer); exporter init failure -> exporter_init_failed
       DB/lock error -> database_error; path error -> path_resolution_error; deadline -> lookup_timeout
```

The header helper follows the same policy gate and destination tripwire. It then checks that `auth.db` exists (a stat, so a never-logged-in machine spawns no `git`), resolves the repository, and prints a bearer only for `Valid` with a matching repository binding. Every other outcome fails closed with `Telemetry credential is unavailable.`

### Credential lookup and fallback matrix

| State | Runtime fallback (log level) | Doctor text / JSON `state`/`reason` | Header helper |
| --- | --- | --- | --- |
| Policy not `Enabled` | `disabled_by_config` (debug) | `[PASS] (disabled by config)` / `disabled_by_config`; `OPT_IN_NOTE` only when `auth.db` exists | fail closed before tripwire |
| `auth.db` missing | `not_configured` (debug) | `[PASS] (not configured)` / `not_configured` | fail closed, no `git` spawn |
| `auth.db` exists, no telemetry row (never provisioned, post-logout, telemetry-disabled login) | `not_configured` (debug) | `[PASS] (not configured)` / `not_configured` | fail closed |
| Row present, no WorkOS session / ownership mismatch / legacy identity-less / malformed | `credential_unavailable` (debug) | `[WARN]` / `disabled`/`credential_unavailable`, `auth_login` | fail closed |
| Row owned and complete but expired | `credential_expired` (debug) | `[WARN]` / `disabled`/`credential_expired`, `auth_login` | fail closed |
| Schema not current (outdated/schemaless DB) | `credential_unavailable` (debug) | `[WARN]` / `disabled`/`credential_unavailable`, `auth_login` | fail closed |
| Key not silently accessible | `credential_inaccessible` (debug) | `[WARN]` / `disabled`/`credential_inaccessible`, `auth_login` | fail closed |
| Wrong key, lock conflict, I/O | `database_error` (warn) | `[WARN]` / `disabled`/`database_error`, `manual_steps` | fail closed |
| State dir unresolvable | `path_resolution_error` (warn) | `[WARN]` / `path_resolution_error`, `manual_steps` | fail closed |
| Deadline exceeded | `lookup_timeout` (warn) | `[WARN]` / `lookup_timeout`, `manual_steps` | fail closed |
| Valid, exporter fails | `exporter_init_failed` (warn) | n/a (doctor starts no exporter) | n/a |
| Valid, repository unbound or bound elsewhere | runtime: Managed | `[PASS]` | fail closed |
| Valid | Managed | `[PASS] Managed telemetry` / `healthy` | bearer printed |

`telemetry-ready.json`, whether present, missing, expired, or malformed, changes no row of this table.

### Lifecycle changes

- **Login:** `provision_telemetry_credential` returns `Issued` exactly when `issue_and_save_telemetry_credential` succeeds; only then does it enroll the working-directory repository (`EnrollIfUnbound`). Issuance or persistence failure returns `Unavailable` and writes nothing, so a credential stored earlier stays in place on the stored and renewed session paths, as the save-on-issue contract requires. On a fresh device login, `replace_session_with` still deletes the old telemetry credential before saving the new session, with the marker clear removed from the front of that sequence. A login with telemetry disabled stays `Disabled`: no issuance, no load, no enrollment.
- **Logout:** `run_logout_with` deletes the WorkOS row and the telemetry row with the existing APIs. It still attempts both and surfaces the first storage error, and it runs no policy check. Nothing comes before the deletes. A later lookup classifies `Missing`, so the result is `not_configured`.
- **Runtime:** `RuntimeTelemetry::resolve` checks policy and then runs the typed lookup. There is no marker probe.
- **Doctor:** `inspect_health` checks policy and then runs the same typed lookup under the deadline. `--fix` is unchanged: `manual_only`, no repair, and no storage initialization, key creation, or prompt.
- **Partial, stale, and concurrent operations:** saves stay single-statement `INSERT OR REPLACE` of row 1, and the last complete writer wins. Without a marker, a login that crashes after the save is simply ready, and a crash before the save is simply not ready. Previously the marker and the row could disagree in either direction. Concurrent readers share the multiprocess WAL, and readiness lookups never write.

### Keychain, concurrency, and performance

- **More reads, only after an `auth.db` exists.** When the policy is `Enabled` and `auth.db` exists but the marker would have been missing or expired, each admitted Claude hook and each admitted command now does one more non-interactive key read plus one encrypted open per process. Those cases are: after logout, a login with telemetry disabled, failed provisioning, a legacy setup-created `auth.db`, and expired credentials. The open costs about 3 ms of serialized cold open, a read-only `has_pending_migrations` check, and one `SELECT`, all under `CREDENTIAL_LOOKUP_DEADLINE`.
  - Nothing changes when the policy is not `Enabled` or `auth.db` is missing (fresh installs).
  - Previously ready installs do the same work as today.
  - Exposure is limited to users who opted in with `telemetry.enabled: true`.
- **Hook safety:** lookups keep `CredentialAccess::NonInteractive`. On macOS that is `SecKeychainSetUserInteractionAllowed(false)`, so an unauthorized binary or a locked keychain fails with `errSecInteractionNotAllowed` and no dialog appears. Lookups never create a key: `open_existing_at` resolves the key only after an existing-file open succeeds, and `NonInteractive` never generates one. After T02, readiness lookups never migrate. An outdated schema yields `credential_unavailable` without writes and without caching the handle, so a later `sce auth login` in the same process still migrates.
- **Diagnostic shift:** a user with a legacy setup-created `auth.db` whose Keychain item this binary may not silently read will now see `[WARN] credential_inaccessible` (next action `auth_login`) instead of `[PASS] not configured`. This is accurate, because a login is required, but it is a visible change.
- **Linux and Windows:** the adapter has no interaction switch there; `NonInteractive` only guarantees that no key is created. This limitation already exists, but more hook invocations now reach Secret Service when `auth.db` exists. See Open questions.
- **Unchanged:** destination-conflict checks, repository-binding isolation, the compatibility lock, the cold-open lock, the busy timeout, code signing, Keychain ACLs, and encryption.
- **Follow-up:** verifying in a fresh process that a Keychain item is silently accessible right after `sce auth login` is a separate change, not part of this plan.

## Acceptance criteria

- [ ] AC1: A valid, owned, unexpired telemetry credential in `auth.db` enables managed telemetry, doctor health, and the header helper when no `telemetry-ready.json` exists.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml valid_credential_without_readiness_marker` passes, covering runtime `Managed`, doctor `healthy`, and the helper bearer for the bound repository in a sandboxed state root.
- [ ] AC2: `telemetry-ready.json` is inert. A present-but-ready marker with no telemetry row reports `not_configured`. An expired or malformed marker next to a valid credential reports `healthy`. The file is never read, written, or removed.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml legacy_readiness_marker_is_ignored` passes, and asserts the marker bytes are unchanged afterwards.
- [ ] AC3: The readiness classification matches the fallback matrix for a missing `auth.db`, an existing DB with no row, a valid row, an expired row, a missing WorkOS session, an identity mismatch, and a legacy identity-less row. A missing `auth.db` creates no DB file, sidecar, or key.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml telemetry_credential_lookup_classifies` and `... doctor_telemetry_health` pass.
- [ ] AC4: Readiness lookups never migrate or initialize `auth.db`. Concurrent hook lookups against an outdated-schema `auth.db` leave its bytes and `__sce_migrations` unchanged, and each reports `credential_unavailable`.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml readiness_lookup_never_migrates` passes.
- [ ] AC5: Telemetry disabled by policy never reaches the credential lookup, even with a valid credential stored. Doctor shows `OPT_IN_NOTE` only when `auth.db` exists.
  - Validate: the policy-disabled tests in `managed_telemetry.rs`, `auth_command/mod.rs`, `doctor/render.rs`, and `claude_hook_with_telemetry_disabled_by_config_never_resolves_the_credential` pass.
- [ ] AC6: Login reports `issued` exactly when the credential is persisted, `unavailable` on issuance or persistence failure (keeping the prior row on the stored and renewed paths), and `disabled` with no issuance when telemetry is off. Logout deletes both rows, and a following lookup reports `not_configured`.
  - Validate: the `auth_command/mod.rs` login and logout tests pass, and `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml logout_then_lookup_reports_not_configured` passes.
- [ ] AC7: No production code, test, or current-state context references the marker.
  - Validate: `nix shell nixpkgs#ripgrep -c rg -n "telemetry-ready|telemetry_readiness|ReadinessState|mark_ready|readiness_hint" cli/ context/ --glob '!context/plans/**' --glob '!context/decisions/**' --glob '!context/handovers/**'` returns no matches.
- [ ] AC8: On macOS, an admitted Claude hook and `sce auth telemetry-headers` with an unauthorized or locked Keychain item show no dialog and resolve `credential_inaccessible` / fail closed. A previously authorized install keeps exporting after the marker file is deleted.
  - Validate: manual inspection on macOS: delete `~/Library/Application Support/sce/telemetry-ready.json` (or the platform state root), then run a Claude session and `sce doctor --format json` and check that `managed_telemetry.state` is `healthy` with no prompt. Then lock the login keychain and rerun: no dialog, and `credential_inaccessible`.

### Full validation

- `nix flake check`

### Context sync

- `context/sce/telemetry-credential.md`: remove the "Telemetry readiness hint" section; rewrite Runtime use, Doctor health check (the table row "Hint ready, no usable credential" and the `telemetry-ready.json` opt-in sentence), and Hidden header helper for the `auth.db`-only flow and the typed lookup.
- `context/cli/access-policy.md`: the Enforcement bullets for managed telemetry, doctor, the header helper, login, and logout.
- `context/sce/cli-observability-contract.md`: the managed telemetry startup paragraph.
- `context/sce/auth-db.md`: Concurrency and schema readiness. Telemetry lookups open without migrating.
- `context/sce/shared-turso-db.md` and `context/glossary.md`: the new non-migrating encrypted existing-DB open.
- `context/overview.md:71`: fix the drift that says the auth_db provider runs `AuthDb::new()` setup.

## Task context synchronization lifecycle

- **Task context synchronization:** every task carries `pending | synced | blocked`. A completed task must be `synced` before another task can start or the plan can finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition** beside the status. Never infer `synced` from conversation history; write every lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:**
  - `cli/src/services/telemetry_credential_storage.rs`
  - `cli/src/services/token_storage.rs`
  - `cli/src/services/db/mod.rs` (the encrypted existing-DB open only)
  - `cli/src/services/managed_telemetry.rs`
  - `cli/src/services/auth_command/mod.rs`
  - `cli/src/services/doctor/render.rs` and `cli/src/services/doctor/managed_telemetry.rs` (only if a field rename requires it)
  - `cli/src/services/telemetry_readiness.rs` (deleted)
  - `cli/src/services/mod.rs`
  - `cli/src/services/default_paths.rs`
  - `cli/src/services/auth_db/multiprocess_tests.rs`
  - `cli/src/app/doctor_reconciliation_cli_tests.rs`
  - the context files listed under Context sync
- **Out of scope:**
  - new interactive `sce setup` questions, or folding login into setup
  - Claude-native settings reconciliation
  - changes to telemetry consent or `auth.mode` precedence
  - new `auth.db` schema or migrations
  - replacing or reconfiguring the Keychain adapter, code signing, ACLs, or encryption
  - fresh-process Keychain access verification
  - deleting existing `telemetry-ready.json` files
  - changing `sce sync`'s `NonInteractive` `load_tokens`, which keeps migrating
  - Linux Secret Service prompt suppression
- **Constraints:**
  - Keep `CredentialAccess::NonInteractive` and `CREDENTIAL_LOOKUP_DEADLINE` on every readiness path.
  - Keep the exact `TelemetryFallbackReason` strings, doctor JSON fields, problem kinds, `next_action` values, and fixed text, including `OPT_IN_NOTE` and `Telemetry credential is unavailable.`.
  - Keep the multiprocess WAL, compatibility lock, and cold-open lock behavior.
  - Use static dispatch and typed enums, not boxed traits.
  - Follow the no-comments-in-code preference.
- **Non-goal:**
  - a replacement readiness cache, marker, or `auth.db` side table
  - a migration step that deletes the old marker
  - a generic "lookup policy" abstraction over every `open_existing_auth_db` caller

## Assumptions

- `OPT_IN_NOTE` under `disabled by config` keys off whether `auth.db` exists (a stat, with no credential access). That is the nearest marker-free signal for "an earlier login provisioned telemetry", and the note text stays unchanged.
- An `auth.db` with no telemetry row classifies as `not_configured` (`[PASS]`), matching the requested tree "Missing -> Not configured". A row that exists but is unusable stays `credential_unavailable`, whose doctor summary ("was configured, but its credential is no longer available") is still accurate.
- An outdated or schemaless `auth.db` on the readiness path maps to `credential_unavailable` with next action `auth_login`, because login migrates the DB through its interactive open. No new `TelemetryFallbackReason` is added.
- The header helper keeps a stat-only `auth.db` existence probe in place of `ready_probe`, so a never-logged-in machine spawns no `git config` process every refresh. A file-existence check is not persisted readiness state.
- Test filters named in acceptance criteria are the intended new or renamed test names; the implementing task creates them.

## Task stack

- [x] T01: `Return a typed telemetry credential lookup from auth.db` (status:done)
  - Task ID: T01
  - Scope: In — typed lookup classification in storage and its mapping into `TelemetryFallbackReason`. Out — marker removal, the migration behavior on open, and helper changes.
  - Files and functions:
    - `cli/src/services/telemetry_credential_storage.rs`: a new `TelemetryCredentialLookup { Missing, Expired, Unusable, Valid(TelemetryCredential) }` (derive `Clone, Debug, Eq, PartialEq`, redacted through `TelemetryCredential`'s `Debug`); `load_with`; `load_telemetry_credential`; `load_for_repository_with`.
    - `cli/src/services/managed_telemetry.rs`: `load_credential_within_deadline`, `inspect_health_with`, `RuntimeTelemetry::resolve_with`, and the lookup future bounds.
  - Steps:
    1. Change `load_with` to `LEFT JOIN auth_credentials` and classify:
       - no telemetry row: `Missing`
       - incomplete or legacy identity, empty token or accessor, or negative expiry: `Unusable`
       - no session row, or session claims that do not equal the identity: `Unusable`
       - expiry not after `now`: `Expired`
       - otherwise: `Valid`
    2. Change `load_telemetry_credential` to return `Result<TelemetryCredentialLookup, TokenStorageError>`, with a missing DB mapping to `Missing`.
    3. Keep `load_repository_telemetry_credential` returning `Option`: `Valid` plus a binding gives `Some`, everything else gives `None`.
    4. Map in `load_credential_within_deadline`: `Missing` to `NotConfigured`, `Expired` to `CredentialExpired`, `Unusable` to `CredentialUnavailable`, and `Valid` to the credential.
  - Dependencies: none
  - Done when: the lookup distinguishes missing, expired, unusable, and valid rows without opening anything new. All existing ownership, identity, and repository-binding checks still hold. The runtime and doctor map the new variants exactly as in the matrix. The marker still gates the lookup, because T03 removes that gate.
  - Verify:
    - A table-driven `telemetry_credential_lookup_classifies_auth_db_states` test in `telemetry_credential_storage.rs` covers no row, valid, expired, no WorkOS session, identity mismatch, and a legacy identity-less row. It replaces the separate expired and corrupt `None` assertions instead of duplicating them.
    - The existing ownership, identity-switch, and repository-binding tests pass unchanged.
    - `health_with_readiness_classifies_lookup_outcomes` is extended with `Missing`, `Expired`, and `Unusable` rows.
    - `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml telemetry_credential` passes.
  - Completed: 2026-10-21
  - Files changed:
    - `cli/src/services/telemetry_credential_storage.rs`
    - `cli/src/services/managed_telemetry.rs`
    - `cli/src/services/auth_db/multiprocess_tests.rs`
    - `cli/src/app.rs`
  - Result: Added `TelemetryCredentialLookup { Missing, Expired, Unusable, Valid }` with `into_valid()`. `load_with` now uses `LEFT JOIN auth_credentials` and classifies a private `StoredTelemetryCredential` row: a negative expiry, an empty token or accessor, an incomplete identity, a missing session, or a session identity mismatch gives `Unusable`; an expiry at or before `now` gives `Expired`. `load_telemetry_credential` returns the typed lookup, and a missing DB maps to `Missing`. `load_for_repository_with`, `bind_repository_with`, and `load_repository_telemetry_credential` keep `Option`/`bool` through `into_valid()`. `load_credential_within_deadline` maps `Missing` to `NotConfigured`, `Expired` to `CredentialExpired`, and `Unusable` to `CredentialUnavailable`. The marker gate is unchanged.
  - Verify:
    - `telemetry_credential_lookup_classifies_auth_db_states` covers no row, valid, expired, no WorkOS session, identity mismatch, legacy identity-less, empty token, and negative expiry. It replaces the removed `telemetry_credential_storage_does_not_return_expired_record`, `telemetry_credential_storage_does_not_return_corrupt_record`, and `telemetry_credential_is_rejected_without_owning_workos_session`: passed.
    - The ownership, identity-switch, legacy, and repository-binding tests pass. Their assertions changed only for the new return type (`into_valid()`, or `Unusable`/`Missing` in place of `None`).
    - `health_with_readiness_classifies_lookup_outcomes` is extended with `Missing`, `Expired`, and `Unusable` rows. `claude_hook_credential_failure_or_expiry_resolves_noop_with_reason` now covers all three non-valid variants: passed.
    - `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml telemetry_credential`: 27 passed.
    - The additional `managed_telemetry` (42), `stalled` (10), and `multiprocess` (20) filters passed, and `clippy --all-targets -D warnings` is clean.
  - Deviation: `cli/src/app.rs` test stubs (`stalled_keychain_lookup`, `stalled_blocking_database_lookup`, and the header-helper stall call) were retyped to the lookup enum. The task did not list this file, but the change in the lookup future type required it.
  - Context impact: localized. When the marker is ready, an expired stored row now reports `credential_expired`, and a missing telemetry row now reports `not_configured`; both previously reported `credential_unavailable`. All other reasons are unchanged. The marker-free current-state context is rewritten when T03 removes the gate.
  - Context synchronization: synced

- [x] T02: `Open auth.db for telemetry lookups without running migrations` (status:done)
  - Task ID: T02
  - Scope: In — a non-migrating encrypted existing-DB open used only by the two non-interactive telemetry lookups. Out — `load_tokens`, `delete_*`, `bind_repository_telemetry`, `sce sync`, and every interactive path, which all keep migrating.
  - Files and functions:
    - `cli/src/services/db/mod.rs`: a new `EncryptedTursoDb::open_existing_current_at(path, access)` next to `open_existing_at`. It reuses `open_lock_file(.., false)`, `resolve_encryption_key`, and the connect path, but replaces `ensure_migrations_current` with the read-only `has_pending_migrations`. It returns a typed result that distinguishes missing, schema-not-current, and open.
    - `cli/src/services/token_storage.rs`: a lookup-only opener next to `open_existing_auth_db`. It reuses the cached `AUTH_DB` when that is set, and caches only a current-schema handle.
    - `cli/src/services/telemetry_credential_storage.rs`: `load_telemetry_credential` and `load_repository_telemetry_credential` use that opener. Schema-not-current maps to `TelemetryCredentialLookup::Unusable` and `None` respectively.
  - Steps:
    1. Factor the shared connect step out of `open_locked` so the migrating and non-migrating opens differ only in the schema step.
    2. On schema-not-current, drop the connection and do not populate `AUTH_DB`.
    3. Keep error typing unchanged: `CredentialAccessUnavailable`, `DatabaseLocked`, `Database`, `PathResolution`.
  - Dependencies: T01
  - Done when:
    - A telemetry lookup never runs a migration or DDL statement. The `BEGIN IMMEDIATE`-holder reader test still passes.
    - An outdated or schemaless `auth.db` is left byte-identical after concurrent lookups.
    - A process that ran a lookup on an outdated DB can still migrate it through a later interactive open.
  - Verify:
    - A new `readiness_lookup_never_migrates_outdated_auth_db` multiprocess test in `auth_db/multiprocess_tests.rs`: seed a DB at an older migration, run 8 concurrent `load_telemetry_credential` children, then assert the file and `__sce_migrations` are unchanged and every child reports the unusable classification.
    - Extend the existing `current_schema_reader_succeeds_while_writer_holds_begin_immediate`-style test, or keep it unchanged if it already covers the current-schema path.
    - `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml multiprocess` passes.
  - Completed: 2026-10-21
  - Files changed:
    - `cli/src/services/db/mod.rs`
    - `cli/src/services/token_storage.rs`
    - `cli/src/services/telemetry_credential_storage.rs`
    - `cli/src/services/auth_db/multiprocess_tests.rs`
  - Result: Added `ExistingDbOpen<T> { Missing, SchemaNotCurrent, Open(T) }` and `EncryptedTursoDb::open_existing_current_at`. That open shares a new `connect_with_retry` step with `open_locked`. It keeps `open_lock_file(.., false)` before key resolution, but replaces `ensure_migrations_current` with the read-only `has_pending_migrations`. On pending migrations it drops the connection and returns `SchemaNotCurrent`. `token_storage::open_existing_current_auth_db` reuses a cached `AUTH_DB` and caches only an `Open` handle. Error typing is unchanged through `open_error`. `load_telemetry_credential` maps `SchemaNotCurrent` to `Unusable`, and `load_repository_telemetry_credential` maps it to `None`. `load_tokens`, `save_tokens`, `bind_repository_telemetry`, `delete_*`, and interactive opens still go through the migrating paths.
  - Verify:
    - New `readiness_lookup_never_migrates_outdated_auth_db`: a pre-binding (3-migration) `auth.db` and 8 concurrent `lookup-outdated` children, each asserting `Unusable` and a repository lookup of `None`. Afterwards the `auth.db` bytes are unchanged and `__sce_migrations` still has 3 rows. A `lookup-then-migrate` child gets `Unusable` and then migrates through `open_or_create_auth_db` to the full migration set in the same process: passed.
    - `current_schema_reader_performs_no_migration_write_under_held_write_lock` already covers the current-schema path under a held `BEGIN IMMEDIATE`; kept unchanged: passed.
    - `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml multiprocess`: 21 passed.
    - The additional `telemetry_credential` (27), `managed_telemetry` (42), and `stalled` (10) filters passed, and `clippy --all-targets -D warnings` is clean.
  - Deviation: the new child roles are dispatched through a `run_outdated_schema_child` helper, so `auth_db_child_process` stays within clippy's `too_many_lines` limit.
  - Context impact: localized. Telemetry readiness lookups no longer migrate an outdated or schemaless `auth.db`. They report `credential_unavailable` (`Unusable`) instead, and a later interactive open still migrates. This affects `context/sce/auth-db.md` (concurrency and schema readiness), and `shared-turso-db.md`/`glossary.md` (the new non-migrating encrypted existing-DB open).
  - Context synchronization: synced

- [ ] T03: `Gate runtime, doctor and header helper on auth.db instead of the marker` (status:todo)
  - Task ID: T03
  - Scope: In — remove every marker read. Out — marker writes and clears in login and logout, and deleting the module (both in T04).
  - Files and functions:
    - `cli/src/services/managed_telemetry.rs`: `RuntimeTelemetry::resolve` and `resolve_when_ready` (drop the `ready` parameter; keep the policy-first gate), `inspect_health` and `inspect_health_with` (drop the `readiness` parameter; replace `hint_present` with an `auth_db_present` probe), `readiness_hint_present` (replace it with `auth_db_path().is_ok_and(|p| p.is_file())`), `ManagedTelemetryHealth::DisabledByConfig { readiness_hint_present }` (rename the field to `auth_db_present`).
    - `cli/src/services/doctor/render.rs:318-330` and its tests (around line 1320): the renamed field. Keep the note text.
    - `cli/src/services/auth_command/mod.rs`: `run_telemetry_headers` and `telemetry_headers_with` (replace `ready_probe` with the `auth_db` existence probe; keep the ordering policy → tripwire → probe → repository → lookup).
    - `cli/src/services/telemetry_readiness.rs`: delete `ReadinessState`, `is_ready`, `state`, `state_at`, `is_ready_at`, and their tests, so nothing becomes dead code. `mark_ready`, `clear`, and their helpers stay until T04.
    - `cli/src/services/auth_db/multiprocess_tests.rs`: rewrite `doctor_telemetry_health_classifies_real_auth_db_states_without_mutating_them` to vary credential state instead of marker state. Drop the marker assertion in `claude_hook_with_telemetry_disabled_by_config_never_resolves_the_credential`.
    - `cli/src/app/doctor_reconciliation_cli_tests.rs` (around lines 640-790): stop writing `FAR_FUTURE_READINESS`.
  - Steps:
    1. Delete the readiness probes.
    2. Rename the existing unit tests to marker-free names. For example, `claude_hook_without_readiness_never_looks_up_credentials` becomes a missing-`auth.db` case covered by the T01 mapping, `health_without_readiness_or_with_expired_readiness_never_looks_up_credentials` is removed, and the `*_never_probes_readiness_*` tests assert that neither the lookup nor the `auth_db_present` probe runs when disabled, except where the note needs the probe.
  - Dependencies: T01, T02
  - Done when:
    - No runtime, doctor, or helper code path reads `telemetry-ready.json`.
    - A valid credential without the marker yields `Managed`, `healthy`, and a bearer.
    - Marker presence, absence, expiry, or corruption has no effect.
    - Disabled policy still short-circuits before any lookup.
    - The helper on a machine without `auth.db` fails closed without resolving the repository.
  - Verify:
    - A new `valid_credential_without_readiness_marker_resolves_managed_telemetry_and_headers` multiprocess test: seed without the marker, then assert hook fallback is none, doctor reports `healthy`, and the helper prints the bearer for the bound repository.
    - A new `legacy_readiness_marker_is_ignored` table test with marker cases (absent, valid, expired, malformed) crossed with credential cases (valid, none). It asserts the doctor label matches the credential case only, and that the marker bytes are unchanged.
    - The existing 32-process concurrent hook-span test and the 32-process missing-DB helper test pass without seeding the marker.
    - `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml managed_telemetry`, `... telemetry_headers`, `... doctor`, and `... multiprocess` pass.
  - Context synchronization: pending

- [ ] T04: `Remove readiness marker writes from login and logout and delete the module` (status:todo)
  - Task ID: T04
  - Scope: In — the login, logout, and session-replacement lifecycle, plus deleting the marker module and path. Out — any change to provisioning conditions, enrollment, policy gates, or storage APIs.
  - Files and functions:
    - `cli/src/services/auth_command/mod.rs`:
      - `provision_telemetry_credential`: `Issued` iff `issue_and_save_telemetry_credential` is `Ok`, then `enroll_current_repository_if_unbound`; otherwise `Unavailable`.
      - Delete `record_telemetry_readiness`.
      - `run_logout` and `run_logout_with`: drop the `clear_readiness` parameter and the first step.
      - `save_new_session_tokens` and `replace_session_with`: drop the `clear_readiness` parameter; the order becomes invalidate → save.
      - Remove `use crate::services::telemetry_readiness;`.
    - Delete `cli/src/services/telemetry_readiness.rs`.
    - `cli/src/services/mod.rs`: remove `pub mod telemetry_readiness;`.
    - `cli/src/services/default_paths.rs`: remove `telemetry_readiness_path`, and update any path-catalog regression test that lists it.
    - `cli/src/services/auth_db/multiprocess_tests.rs`: `seed()` stops calling `mark_ready`.
  - Steps:
    1. Make the changes above.
    2. Replace `telemetry_readiness_is_recorded_only_after_credential_persistence` with a `provisioning_result_reflects_credential_persistence` test over the `issue_and_save` outcome, if `provision_telemetry_credential` needs a seam. Otherwise rely on the existing provisioning tests.
    3. Update the event-order assertions to `["invalidate", "save"]` and `["delete-workos", "delete-telemetry"]`.
    4. Add `logout_then_lookup_reports_not_configured`: real sandboxed DB, seed, logout path deletes, then a lookup classifies `Missing`.
  - Dependencies: T03
  - Done when:
    - Login and logout neither write nor remove any readiness file.
    - The login result reflects only credential persistence.
    - A disabled-telemetry login issues nothing.
    - Logout under `auth.mode: disabled` still deletes both rows.
    - No reference to `telemetry_readiness`, `telemetry_readiness_path`, `mark_ready`, or `telemetry-ready.json` remains under `cli/`.
  - Verify:
    - `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml auth_command` passes.
    - `... logout_then_lookup_reports_not_configured` passes.
    - `nix shell nixpkgs#ripgrep -c rg -n "telemetry_readiness|telemetry-ready|mark_ready" cli/` returns nothing.
    - `nix flake check` passes.
  - Context synchronization: pending

## Open questions

- On Linux, the keyring adapter has no non-interactive switch, so a locked Secret Service collection might show an unlock prompt. This limitation already exists. Removing the marker means more hook invocations reach Secret Service: every admitted hook while `auth.db` exists and telemetry is enabled, instead of only marker-ready installs. Is this acceptable for this plan? The smaller and safer alternative is a separate follow-up that adds a Linux non-interactive guard. Folding that into this plan would mean changing the Keychain adapter, which is listed as out of scope.
- `OPT_IN_NOTE` now keys off whether `auth.db` exists, not off the marker, so some users who never provisioned telemetry will also see it: those who logged in while telemetry was disabled, and those with a legacy setup-created `auth.db`. Keep the note as it is, or drop it entirely? Dropping it would also remove the `auth_db_present` probe.
