# Plan: claude-auth-billing-observations

## Change summary

Extend the existing `sce hooks claude-model-state` hook so every valid Claude
`SessionStart` (all sources: `startup`, `resume`, `clear`, `compact`, with or
without `model`) also classifies Claude Code's configured authentication
provider, billing mode, selected authentication environment key, and detection
confidence from the presence of documented environment variables, and stores the
current classification for the session in the existing repository-scoped Agent
Trace DB (`RepositoryAgentTraceDb`). The Agent Trace DB is the source of truth;
no new hook, database, service, event store, or telemetry pipeline is
introduced. Model-selection and `claude_model_state` behavior are preserved
unchanged, and `PostModelSwitch` is ignored for auth.

Environment variables describe configuration, not proof of successful
authentication or actual billing, so every recognized signal is `Inferred` and
the absence of any usable signal is `Unknown`. Persistence is one latest-state
row per canonical `claude:<session_id>`, written by a single atomic upsert that
changes the row only when the classification changes. No historical transition
log is kept: the request names no concrete downstream consumer of transitions,
and the latest-state shape mirrors the existing `claude_model_state` register.

Migration `008_claude_session_auth_state` is additive. Today hook readiness
(`open_repository_db_for_hook_runtime` → `ensure_schema_ready_for_hooks` →
`TursoDb::migration_metadata_problems`) requires exact ledger parity, so a new
migration would make every repository hook fail open until `sce setup` reruns.
The setup fast path (`open_repository_db_concurrently_safe`) uses the same
strict check to decide whether to run migrations, so the strict check itself
must not change. Instead, the hook-runtime open path alone uses a narrow
readiness variant that accepts either the full ledger or the full ledger minus
exactly `008_claude_session_auth_state`; the auth write is skipped when `008` is
not applied. Setup, the concurrent-first-open repair recheck, and doctor keep
strict parity, so setup still applies `008` and doctor still reports the schema
as not ready until setup reruns. Unexpected-ID (downgrade) handling is
unchanged.

Exporter, `sce sync`, control-plane, warehouse, and web changes are not
implemented. Observations stay local until a future export change.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the
check that proves it. `/validate` runs these checks; no task in the stack
performs final validation.

- [ ] AC1: The pure detector maps each documented signal to the expected provider, billing mode, typed env key, and `Inferred` confidence, and never returns a verified confidence or `Subscription` billing: Bedrock/Vertex/Foundry flags yield their provider with `CloudProvider` billing and take precedence over Anthropic credentials; `ANTHROPIC_AUTH_TOKEN` takes precedence over `ANTHROPIC_API_KEY` and yields `Unknown` billing (never `ApiKey`); `ANTHROPIC_API_KEY` without a custom base URL yields `Anthropic` with `ApiKey` billing; a custom `ANTHROPIC_BASE_URL` with either credential yields `CustomGateway` with `Unknown` billing; `CLAUDE_CODE_OAUTH_TOKEN` alone yields `Anthropic` with `Unknown` billing; more than one cloud flag, or no signal, yields `Unknown` provider, billing, and confidence with no env key.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_auth_detection`
- [ ] AC2: Migration `008_claude_session_auth_state` creates `claude_session_auth_state` keyed by `session_id` with `CHECK`-constrained enum columns and non-negative `observed_at_ms`, and has no column capable of holding a credential value or hash; migrations 001–007 are unchanged.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_repository_migrations`; `git diff --exit-code -- cli/migrations/agent-trace-repository/00[1-7]_*.sql`
- [ ] AC3: Repeated identical upserts leave one unchanged row (including `observed_at_ms`); a changed classification updates the row in place; the latest-state read returns the exact canonical session's row and never another session's.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_session_auth_state`
- [ ] AC4: On a pre-`008` repository DB, existing hooks (including `claude-model-state` model writes, `diff-trace`, and `conversation-trace`) still persist and the auth write is skipped without error; setup applies `008` on that DB; doctor still reports the schema not ready before setup; a DB missing any other migration, or carrying an unexpected migration ID, is still rejected by the hook runtime.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_session_auth_schema_compat`
- [ ] AC5: `sce hooks claude-model-state` on `SessionStart` (startup and resume, with and without `model`) stores the auth classification under `claude:<session_id>`, changes it only when the classification changes across resumes and duplicate executions, leaves `claude_model_state` writes identical to current behavior, ignores `PostModelSwitch` for auth, and returns empty stdout on every path including DB-open and write failures.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_model_state`
- [ ] AC6: Sentinel credential values supplied to the hook appear in no DB row, no captured log output, and no hook stdout.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_auth_state_no_secret`
- [ ] AC7: Durable context documents detection rules and limitations, the table schema, hook integration, migration-008 compatibility, and that the data stays local until a future `sce sync`/warehouse export change, without specifying downstream routes, cursors, object layouts, or warehouse schemas.
  - Validate: inspect `context/sce/claude-session-auth-state.md`, `context/sce/agent-trace-db.md`, `context/sce/agent-trace-db-schema.md`, and `context/context-map.md`.

### Full validation

Repository-wide checks `/validate` runs after the last task, regardless of
which criterion they map to.

- `nix run .#pkl-check-generated`
- `nix flake check`

### Context sync

- `context/sce/claude-session-auth-state.md` (new: detection rules and limitations, schema, hook integration, migration compatibility, local-only status)
- `context/sce/agent-trace-db.md` (migration list, adapter methods, hook-runtime readiness variant, migration-008 note)
- `context/sce/agent-trace-db-schema.md` (new table columns)
- `context/sce/agent-trace-export-readers.md` (new table is local-only with no reader method)
- `context/context-map.md` and `context/glossary.md` (new entry and terms)

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** a new pure detection module next to `cli/src/services/hooks/claude_model_state.rs`; `cli/migrations/agent-trace-repository/008_claude_session_auth_state.sql`; typed payload/SQL in `cli/src/services/agent_trace_db/mod.rs` and methods in `repository.rs`; the hook-runtime readiness path in `cli/src/services/agent_trace_storage/mod.rs`; `claude_model_state.rs`; tests that enumerate the repository migration list; durable context files listed under Context sync.
- **Out of scope:** `AgentTraceExportReader`, `sce sync`, control-plane ingestion, S3, ClickHouse, `packages/warehouse`, web UI, and any design of them; new hook registrations or Pkl hook config changes; `PostModelSwitch` auth handling; reading Claude credential files, keychain, `apiKeyHelper`, or settings files; historical transition logging; historical backfill; other agents (OpenCode, Codex, Pi); unrelated refactoring of `claude_model_state.rs`.
- **Constraints:** static dispatch, no `dyn`/`Box`, no new dependencies, typed enums for classification with string mapping only at the SQL boundary, existing `anyhow::Result` conventions in the DB and hook layers with no new error abstraction, rustdoc-only comments, no credential values or hashes in logs/rows/telemetry, hooks never migrate and always fail open with empty stdout, strict ledger parity unchanged for setup, repair, and doctor.
- **Non-goal:** a generic migration-compatibility framework, a generic session-metadata store or event store, a second hook, or any new telemetry/export path.

## Assumptions

- Precedence follows the Claude Code authentication documentation: cloud-provider flags (`CLAUDE_CODE_USE_BEDROCK`, `CLAUDE_CODE_USE_VERTEX`, `CLAUDE_CODE_USE_FOUNDRY`) → `ANTHROPIC_AUTH_TOKEN` → `ANTHROPIC_API_KEY` → `CLAUDE_CODE_OAUTH_TOKEN` → interactive `/login` credentials (not observable, so `Unknown`). More than one cloud flag set is a provider conflict and classifies as `Unknown`.
- A cloud flag counts as set only for truthy values (`1`/`true`, case-insensitive); credential variables count as set when present and non-empty. Only presence/truthiness is read; values are never retained beyond the check.
- Classification table (all recognized signals are `Inferred`; environment variables describe authentication configuration, not actual billing):

  | Selected signal | Provider | Billing mode | Auth env key |
  |---|---|---|---|
  | `CLAUDE_CODE_USE_BEDROCK` | `AwsBedrock` | `CloudProvider` | `ClaudeCodeUseBedrock` |
  | `CLAUDE_CODE_USE_VERTEX` | `GoogleVertex` | `CloudProvider` | `ClaudeCodeUseVertex` |
  | `CLAUDE_CODE_USE_FOUNDRY` | `MicrosoftFoundry` | `CloudProvider` | `ClaudeCodeUseFoundry` |
  | `ANTHROPIC_AUTH_TOKEN`, no custom base URL | `Anthropic` | `Unknown` | `AnthropicAuthToken` |
  | `ANTHROPIC_AUTH_TOKEN`, custom base URL | `CustomGateway` | `Unknown` | `AnthropicAuthToken` |
  | `ANTHROPIC_API_KEY`, no custom base URL | `Anthropic` | `ApiKey` | `AnthropicApiKey` |
  | `ANTHROPIC_API_KEY`, custom base URL | `CustomGateway` | `Unknown` | `AnthropicApiKey` |
  | `CLAUDE_CODE_OAUTH_TOKEN` | `Anthropic` | `Unknown` | `ClaudeCodeOauthToken` |
  | No signal, or more than one cloud flag | `Unknown` | `Unknown` | none (confidence `Unknown`) |

  `ANTHROPIC_AUTH_TOKEN` may authenticate against a proxy or gateway, so it never implies API-key billing; OAuth authentication never implies a paid subscription. `ANTHROPIC_BASE_URL` is a classification input without an `AuthEnvKey` variant because it is not a credential.
- `BillingMode::Subscription` is retained in the enum, string mapping, and `CHECK` constraint, but no environment-only signal produces it; the detector cannot establish subscription billing from the permitted inputs.
- `DetectionConfidence` has only `Inferred` and `Unknown`: no permitted input can verify authentication, so a `Verified` variant would be dead code. It can be added when a verifying source exists.
- The hook reads the environment it inherits from Claude Code (which includes `settings.json` `env` values); the detector takes an explicit input struct so tests never mutate process environment.
- Persistence is `INSERT ... ON CONFLICT (session_id) DO UPDATE SET ... WHERE` any classification column differs, so identical replays are no-ops (leaving `observed_at_ms` as the time the current classification was first observed), changes update in place, and no read-before-write or explicit transaction is needed.
- The hook-runtime readiness variant lives in the repository adapter as one named constant for the single optional migration ID, not a general mechanism; the auth write checks the migration ledger for `008` and skips when absent. When a future migration lands, the constant is revisited rather than extended into a list.
- The canonical session ID comes from the existing `prefixed_diff_trace_session_id(CLAUDE_TOOL_NAME, raw)`; repository isolation comes from the per-repository DB file, so no repository column is added. Auth configuration is process-level, so the key has no `agent_id`.

## Task stack

- [x] T01: `Add pure Claude auth and billing detector` (status:done)
  - Task ID: T01
  - Scope: In — `AuthProvider`, `BillingMode`, `AuthEnvKey`, `DetectionConfidence` (`Inferred`, `Unknown`) enums with stable snake_case string mapping; an explicit input struct of signal presence; a pure deterministic `detect_claude_auth` function implementing the documented precedence; a constructor building the input from an env-lookup closure without retaining values; unit tests covering every row of the classification table: each cloud flag and its precedence over Anthropic credentials, `ANTHROPIC_AUTH_TOKEN` precedence over `ANTHROPIC_API_KEY` and `Unknown` billing (not `ApiKey`), `ANTHROPIC_API_KEY` as inferred `ApiKey`, custom-gateway classification for both credentials, OAuth-only as `Unknown` billing (not `Subscription`), provider conflict, and missing configuration. Out — persistence, hook wiring.
  - Dependencies: none
  - Done when: the detector and enums compile with no `dyn`/`Box`/new deps, hold no credential value fields, never return a verified confidence or `Subscription` billing, and all unit tests pass.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_auth_detection`
  - Completed: 2026-11-20
  - Files changed: `cli/src/services/hooks/claude_auth_detection.rs` (new), `cli/src/services/hooks/mod.rs`
  - Result: Added private `hooks::claude_auth_detection` with `AuthProvider`, `BillingMode`, `AuthEnvKey`, `DetectionConfidence` (`Inferred`, `Unknown`) and `as_str`/`from_str` snake_case mapping; `ClaudeAuthSignals` (presence-only bools) built by `from_env_lookup` closure; `ClaudeAuthDetection` result; `detect_claude_auth` implementing the documented precedence and provider-conflict rule. 10 unit tests cover every classification-table row, falsy flags, empty credentials, an exhaustive no-`Subscription` sweep over all signal combinations, and string round-trips. No `dyn`/`Box`, no new dependencies, no value-holding fields.
  - Verify outcome: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_auth_detection` passed (10 passed, 0 failed); `clippy --all-targets` passed; `cargo fmt --check` passed after formatting.
  - Deviations/assumptions: A custom base URL means `ANTHROPIC_BASE_URL` is non-empty and not `https://api.anthropic.com` (trimmed, trailing `/` stripped, case-insensitive). Because the binary crate denies warnings and the module is unused until T03/T04, its `mod` declaration carries `#[cfg_attr(not(test), expect(dead_code, ...))]`, which must be removed when the module is wired in.
  - Context impact: localized — new internal module with no runtime wiring; durable context for detection rules is owned by T05.
  - Context synchronization: synced

- [x] T02: `Add migration 008 with hook-runtime compatibility for pre-008 databases` (status:done)
  - Task ID: T02
  - Scope: In — `008_claude_session_auth_state.sql` creating `claude_session_auth_state (session_id TEXT PRIMARY KEY, provider, billing_mode, auth_env_key NULLable, confidence, observed_at_ms INTEGER NOT NULL CHECK (>= 0))` with `CHECK` constraints on enum columns; a hook-runtime readiness method on `RepositoryAgentTraceDb` that accepts the full ledger or the full ledger minus exactly `008_claude_session_auth_state`; switching only `open_repository_db_for_hook_runtime` to it; updating tests that enumerate `AGENT_TRACE_REPOSITORY_MIGRATIONS`; tests on pre-`008` and current DBs proving hooks open, setup applies `008`, doctor reports not-ready before setup, and missing-other or unexpected IDs are still rejected. Out — auth read/write helpers, setup fast path, repair recheck, doctor readiness semantics, migrations 001–007.
  - Dependencies: none
  - Done when: existing hook paths open a pre-`008` DB, setup and doctor keep strict parity, unrelated migration gaps still fail readiness, and the targeted tests pass.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_repository_migrations`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_session_auth_schema_compat`; `git diff --exit-code -- cli/migrations/agent-trace-repository/00[1-7]_*.sql`
  - Completed: 2026-11-20
  - Files changed: `cli/migrations/agent-trace-repository/008_claude_session_auth_state.sql` (new), `cli/src/services/agent_trace_db/repository.rs`, `cli/src/services/agent_trace_storage/mod.rs`
  - Result: Added migration `008_claude_session_auth_state` creating `claude_session_auth_state` (`session_id TEXT PRIMARY KEY`, `CHECK`-constrained `provider`/`billing_mode`/`confidence`, nullable `CHECK`-constrained `auth_env_key`, `observed_at_ms INTEGER NOT NULL CHECK (>= 0)`). Added `HOOK_RUNTIME_OPTIONAL_MIGRATION_ID` and `RepositoryAgentTraceDb::ensure_schema_ready_for_hook_runtime` (strict parity, or applied set equal to expected set minus exactly `008`); split the legacy `001` metadata repair into a shared record step plus strict (`repair_missing_repository_schema_migration_metadata`) and hook-runtime (`..._for_hook_runtime`) rechecks; only `open_repository_db_for_hook_runtime` uses the hook-runtime variants. Setup fast path, repair recheck, and doctor (`open_existing_schema_ready_at`) keep strict parity. 8 new tests: migration list and table constraints (`agent_trace_repository_migrations_*`), and pre-`008` hook open with model-state and diff-trace writes, current-DB hook open, setup applying `008`, doctor rejecting pre-`008`, and hook rejection for other missing migrations and unexpected IDs (`claude_session_auth_schema_compat_*`).
  - Verify outcome: `agent_trace_repository_migrations` passed (2 passed); `claude_session_auth_schema_compat` passed (6 passed); `git diff --exit-code` on migrations 001–007 passed; full CLI suite passed (871 passed, 0 failed); `clippy --all-targets` clean; `cargo fmt` applied.
  - Deviations/assumptions: No pre-existing test hardcoded the migration list (the build script discovers migrations), so the enumeration test is new; existing pre-007 slicing fixtures work unchanged.
  - Context impact: localized — new repository migration and a hook-runtime-only readiness variant; durable context (`agent-trace-db.md`, `agent-trace-db-schema.md`, new `claude-session-auth-state.md`) is owned by T05.
  - Context synchronization: synced

- [x] T03: `Add Claude session auth state upsert and read interface` (status:done)
  - Task ID: T03
  - Scope: In — typed `ClaudeSessionAuthState` payload using the T01 enums; parameterized atomic change-only upsert SQL; `RepositoryAgentTraceDb::upsert_claude_session_auth_state` that returns a skipped outcome when `008` is not in the ledger; `claude_session_auth_state_by_session(session_id)` returning the typed row or none; row-to-enum parsing that rejects unknown stored values; tests for identical replay no-op, change update, session isolation, read, and pre-`008` skip. Out — hook wiring, export reader, history.
  - Dependencies: T01, T02
  - Done when: helpers follow the existing `claude_model_state` delegation pattern, the write is a single statement without explicit transaction or prior read of the row, and targeted tests pass.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_session_auth_state`
  - Completed: 2026-11-20
  - Files changed: `cli/src/services/agent_trace_db/mod.rs`, `cli/src/services/agent_trace_db/repository.rs`, `cli/src/services/agent_trace_storage/mod.rs`, `cli/src/services/hooks/mod.rs`
  - Result: Added `ClaudeSessionAuthState` (T01 enums, nullable `auth_env_key`) and `ClaudeSessionAuthStateUpsertOutcome` (`Upserted { rows_changed }`, `SkippedMissingMigration`); `UPSERT_CLAUDE_SESSION_AUTH_STATE_SQL` (`INSERT ... ON CONFLICT (session_id) DO UPDATE ... WHERE` provider/billing_mode/confidence `<>` or `auth_env_key IS NOT` the stored value) and `SELECT_CLAUDE_SESSION_AUTH_STATE_SQL`; `upsert_claude_session_auth_state_with` / `claude_session_auth_state_by_session_with` / row parser using the T01 `from_str` mappings (unknown stored values error). `RepositoryAgentTraceDb::upsert_claude_session_auth_state` checks the `__sce_migrations` ledger for `HOOK_RUNTIME_OPTIONAL_MIGRATION_ID` and returns `SkippedMissingMigration` when absent, otherwise runs the single upsert statement (no transaction, no prior row read); `claude_session_auth_state_by_session` delegates to the read helper. `hooks::claude_auth_detection` is now `pub(crate)`. 5 new tests: identical replay no-op (including `observed_at_ms`, for both a keyed and a null-key row), in-place change including clearing `auth_env_key`, exact-session isolation, unknown stored value rejection, and pre-`008` skip.
  - Verify outcome: `claude_session_auth_state` passed (7 passed, including 2 T02 migration tests matching the filter); full CLI suite passed (876 passed, 0 failed); `clippy --all-targets` clean; `cargo fmt --check` passed.
  - Deviations/assumptions: Tests live in the `agent_trace_storage` test module to reuse the current and pre-`008` DB fixtures. The two public adapter methods carry `#[cfg_attr(not(test), expect(dead_code, ...))]` until T04 wires the hook; remove those attributes (and the T01 module-level one) in T04. `expect` seeds liveness, so the helpers they call need no attribute.
  - Context impact: localized — new typed persistence helpers for the T02 table with no runtime wiring; durable context is owned by T05.
  - Context synchronization: synced

- [ ] T04: `Store auth state from every claude-model-state SessionStart` (status:todo)
  - Task ID: T04
  - Scope: In — `claude_model_state.rs`: derive the canonical `claude:` session ID for any valid `SessionStart` from the already parsed payload fields (reusing existing helpers, no second parse path), detect from the inherited environment, and open the DB when an auth candidate, model observation, or bridge candidate exists instead of returning early on model-less events; run the auth upsert independently of model-state persistence so either failing does not affect the other; a dedicated fail-open log event for auth write failures carrying only the session ID and error; tests for startup, resume with change, resume without change, duplicate execution, model-less `SessionStart`, `PostModelSwitch` ignored, DB-open failure, unchanged `claude_model_state` rows, empty stdout, and sentinel credential values absent from rows, logs, and stdout. Out — Pkl hook config, `PostModelSwitch` auth, other hooks, unrelated refactoring.
  - Dependencies: T03
  - Done when: existing `claude_model_state` tests pass unchanged, new hook tests pass, and hook stdout stays empty on every path.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_model_state`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_auth_state_no_secret`
  - Context synchronization: pending

- [ ] T05: `Document Claude session auth state` (status:todo)
  - Task ID: T05
  - Scope: In — new `context/sce/claude-session-auth-state.md` covering detection rules, precedence, the classification table, the distinction between authentication configuration and actual billing (environment signals are `Inferred` configuration evidence; neither `ANTHROPIC_AUTH_TOKEN` nor OAuth implies a billing mode), confidence semantics, limitations (`/login` and `apiKeyHelper` not observable), table schema, hook integration, migration-008 compatibility, and that the data is local-only until a future `sce sync`/warehouse export change; updates to `agent-trace-db.md`, `agent-trace-db-schema.md`, `agent-trace-export-readers.md`, `context-map.md`, and `glossary.md`. Out — code changes; any design of export routes, cursors, storage layouts, or warehouse schemas.
  - Dependencies: T04
  - Done when: context files accurately describe the implemented code and mention future export only as unplanned follow-up work.
  - Verify: inspect the listed context files against `claude_model_state.rs`, the migration, `repository.rs`, and `agent_trace_storage/mod.rs`.
  - Context synchronization: pending

## Open questions

- With no env signals, interactive `/login` subscription users (credentials in the keychain or credential file) always classify as `Unknown`, because reading credential storage is not allowed. Is that coverage acceptable, or should the value of the data be confirmed before shipping?
- Sessions using `apiKeyHelper` cannot be identified, because it lives in settings rather than the environment. Acceptable, or should settings inspection be approved?
