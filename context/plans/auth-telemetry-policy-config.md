# Plan: auth-telemetry-policy-config

## Change summary

Add two config sections to `sce/config.json` (global and repo-local): `auth.mode` (`disabled | optional | mandatory`, default `optional`) and `telemetry.enabled` (`true | false`, default `false`). The CLI resolves both into one effective access policy, computed in one place, and enforces it where SCE uses authentication or exports data: `sce auth login`, `whoami`, `sync`, post-commit auto-sync, managed telemetry startup, telemetry-credential provisioning, `sce auth telemetry-headers`, the Claude telemetry install step in setup, and doctor's read-only telemetry health check.

This extends the existing config resolver (`cli/src/services/config/`) and the existing consumer seams (`ResolvedAuthRuntimeConfig`, `ResolvedHookRuntimeConfig`, `ResolvedObservabilityRuntimeConfig`). It adds no new config file, resolver, or policy service. Local-only behavior (Agent Trace persistence, mutation-scope hooks, attribution, bash policy) does not change. The existing `otel` endpoint block stays separate from `telemetry.enabled`. Telemetry export becomes opt-in, which changes current behavior: after upgrading, an installation without an explicit global `telemetry.enabled: true` stops managed SCE export (see Migration).

## Design

### Problem

Today, telemetry export and cloud access depend only on credential state. After `sce auth login`, a telemetry credential is provisioned automatically. Claude hooks then export spans while the `telemetry-ready.json` hint exists, and post-commit auto-sync uploads data whenever `agent_trace.auto_sync` is true. No setting lets a user:

- opt out of telemetry while staying logged in,
- forbid authentication or cloud access on a machine, or
- keep a repository from turning these features on.

When the user is logged in, hook processes read the encrypted `auth.db` and its OS-credential-store key (non-interactively). With an explicit policy, users who have not consented never reach that path.

### Current implementation findings

- **Config file model:** `cli/src/services/config/schema.rs`.
  - `parse_file_config` validates each file against the generated schema (`SCE_CONFIG_SCHEMA_JSON`) and against `TOP_LEVEL_CONFIG_KEYS`. Unknown keys fail.
  - It maps sections through `map_agent_trace_config` and `map_otel_config` into `FileConfig`, wrapping each value as `FileConfigValue { value, source: ConfigPathSource }`.
- **Merge and resolution:** `cli/src/services/config/resolver.rs::resolve_runtime_config_with`.
  - Merges layers key by key, with the last layer winning (`global -> local`).
  - Because of that merge, the per-layer values are gone by the time `RuntimeConfig` is built. Consent rules need them, so the policy must be captured during the merge loop.
  - An invalid default-discovered layer is skipped and recorded in `validation_errors`.
  - An explicit `--config` or `SCE_CONFIG_FILE` file replaces discovery entirely (no global layer). An invalid explicit file is fatal.
  - `resolve_config_paths` discovers the local file as `<cwd>/.sce/config.json`, not relative to the Git root.
- **Resolver wrappers:** `resolve_auth_runtime_config` (used by login, whoami, and sync), `resolve_hook_runtime_config` (used by post-commit auto-sync), `resolve_observability_runtime_config` (used by app startup), and `resolve_agent_trace_storage_runtime_config`. Each re-runs the full resolution and projects one slice of it. These are the extension points.
- **Types:** `cli/src/services/config/types.rs` has `ValueSource`, `ConfigPathSource`, `ResolvedValue<T>`, and the projected `Resolved*RuntimeConfig` structs.
- **Rendering:** `cli/src/services/config/render.rs::format_show_output` and `format_validate_output`. Warnings come from `RuntimeConfig.validation_warnings` plus the skipped-file `validation_errors`.
- **Schema source:** `config/pkl/base/sce-config-schema.pkl`. `otel` describes endpoints only. Managed export always uses the code-owned `MANAGED_OTLP_DEFAULTS`.
- **App startup:** `cli/src/app.rs::run_with_dependency_check_and_streams`.
  - Computes `managed_span_kind` (Claude hooks → `ClaudeHook`; other hooks, `setup`, and `auth telemetry-headers` → none; everything else → `Command`).
  - Then calls `RuntimeTelemetry::resolve(span_kind, logger)`.
  - `build_startup_context` already resolves the full config through `resolve_observability_runtime_config(&cwd)`.
- **Managed telemetry:** `cli/src/services/managed_telemetry.rs`.
  - `RuntimeTelemetry::resolve` → `resolve_when_ready(telemetry_readiness::is_ready(..), load_telemetry_credential(), ..)`.
  - Lookups use `CredentialAccess::NonInteractive` under `CREDENTIAL_LOOKUP_DEADLINE`.
  - `TelemetryFallbackReason` classifies Noop outcomes.
  - `inspect_health` / `inspect_health_with` back `sce doctor` (`cli/src/services/doctor/mod.rs`, `doctor/managed_telemetry.rs`).
- **Telemetry credential storage:** `cli/src/services/telemetry_credential_storage.rs`.
  - Functions: `load_telemetry_credential`, `load_repository_telemetry_credential`, `bind_repository_telemetry`, `delete_telemetry_credential`.
  - Reads go through the existing-DB path in `cli/src/services/token_storage.rs` (`open_existing_auth_db(access)`). It never creates `auth.db` or a key.
- **Auth commands:** `cli/src/services/auth_command/mod.rs`.
  - `run_login`: interactive `load_tokens`, renewal or device flow, then `provision_telemetry_credential`. Provisioning calls `issue_and_save_telemetry_credential`, `telemetry_readiness::mark_ready`, and `enroll_current_repository_if_unbound`.
  - `TelemetryProvisioningResult` has two values, `Issued | Unavailable`.
  - `run_logout`: clears the hint, then deletes both credential rows through interactive access. A missing DB returns `false`.
  - `run_whoami`: interactive `load_tokens`. Without tokens it renders an unauthenticated success.
  - `run_telemetry_headers` / `telemetry_headers_with`: gated on the readiness hint and the destination tripwire, then repository resolution and `load_repository_telemetry_credential`.
- **Sync:** `cli/src/services/sync/sync.rs::run_current_sync_without_progress` builds `AuthenticatedControlPlaneClient` with `CredentialAccess::NonInteractive`. `sync/command.rs::classify_sync_error` maps auth failures to `UserError::NotAuthenticated`.
- **Auto-sync:** `cli/src/services/hooks/commit_hooks.rs::run_post_commit_subcommand` reads `resolve_hook_runtime_config(root).agent_trace_auto_sync` and then calls `sync::auto_sync::launch(root)`, a detached `sce sync --format json`.
- **Claude-native telemetry:** `cli/src/services/setup/claude_telemetry.rs::install_claude_telemetry` and `setup/config_merge.rs::merge_claude_telemetry_settings`.
  - Setup writes them into *user-level* Claude settings whenever the Claude target is selected (`setup/command.rs::render_claude_telemetry_section`).
  - Written keys: `CLAUDE_CODE_ENABLE_TELEMETRY=1`, `OTEL_*_EXPORTER=otlp`, the managed endpoint, and `otelHeadersHelper = sce auth telemetry-headers`.
  - `setup/config_merge.rs` holds only the Claude and OpenCode settings merges. Repo-local `.sce/config.json` persistence is `setup/mod.rs::persist_integration_targets`, which writes only `integrations`, `agent_trace.auto_sync`, and `policies.attribution_hooks.enabled`.
- **Errors:** `cli/src/services/error.rs::UserError` (`NotAuthenticated`, `AuthStorageUnavailable`, …) owns stable keys and messages.

### Configuration schema and Rust types

```json
{
  "auth": { "mode": "disabled" | "optional" | "mandatory" },
  "telemetry": { "enabled": true | false }
}
```

- Pkl (`config/pkl/base/sce-config-schema.pkl`):
  - `auth` is an object with `additionalProperties = false`. `mode` is a string enum, default `"optional"`.
  - `telemetry` is an object with `additionalProperties = false`. `enabled` is a boolean, default `false`. Its description must say that it controls SCE-managed export and the Claude headers helper, not the `otel` destination.
- Rust (`config/types.rs`). `AuthMode` follows the `LogLevel` parse / `as_str` pattern:

  ```rust
  #[derive(Clone, Copy, Debug, Eq, PartialEq)]
  pub(crate) enum AuthMode { Disabled, Optional, Mandatory }

  #[derive(Clone, Copy, Debug, Eq, PartialEq)]
  pub(crate) enum PolicyLayer { User, Repository }

  #[derive(Clone, Copy, Debug, Eq, PartialEq)]
  pub(crate) enum TelemetryPolicyState {
      Enabled,
      Disabled(TelemetryDisabledReason),
  }

  #[derive(Clone, Copy, Debug, Eq, PartialEq)]
  pub(crate) enum TelemetryDisabledReason {
      NotEnabled,        // no layer enabled it (default)
      UserOptOut,        // user layer enabled=false
      NoUserConsent,     // repository enabled=true without user-layer true
      RepositoryOptOut,  // repository enabled=false
      AuthDisabled,      // effective auth mode is disabled
      InvalidConfig,     // a discovered layer was skipped as invalid
  }

  #[derive(Clone, Debug, Eq, PartialEq)]
  pub(crate) struct ResolvedAccessPolicy {
      pub(crate) auth_mode: ResolvedValue<AuthMode>,         // effective mode, source = deciding layer
      pub(crate) telemetry_enabled: ResolvedValue<bool>,     // configured (merged) value with provenance
      pub(crate) telemetry: TelemetryPolicyState,            // effective state
      pub(crate) background_cloud_allowed: bool,             // false when auth disabled or any discovered layer invalid
  }
  ```

  Small methods on `ResolvedAccessPolicy` (`telemetry_enabled()`, `auth_allowed()`, `auto_sync_allowed(configured: bool)`) let call sites stay one-line checks.
- `schema.rs`: `FileConfig` gains `auth_mode: Option<FileConfigValue<AuthMode>>` and `telemetry_enabled: Option<FileConfigValue<bool>>`. They are mapped by `map_auth_config` / `map_telemetry_config`, mirroring `map_agent_trace_config`. `TOP_LEVEL_CONFIG_KEYS` and its description add `auth` and `telemetry`.

### Policy precedence and effective resolution

Layer classification comes from the existing `ConfigPathSource`:

- **User layer:** `DefaultDiscoveredGlobal`, or the single explicit file (`Flag` or `Env`). An explicit `--config` or `SCE_CONFIG_FILE` is the user's own choice, and it already replaces global discovery.
- **Repository layer:** `DefaultDiscoveredLocal`.
- There is no environment-variable or flag layer for either key.

`resolve_access_policy(user, repo, degraded)` is a pure function. `user` and `repo` are the per-layer `(Option<AuthMode>, Option<bool>)`, captured inside the existing merge loop. `degraded` is `!validation_errors.is_empty()`. Rules, applied in order:

1. **Auth mode (most-restrictive wins, ordered `disabled > mandatory > optional`).**
   - If either layer is `disabled`, the effective mode is `disabled`. A repository therefore cannot override a global prohibition. A repository can also opt itself out of cloud access.
   - Otherwise, if either layer is `mandatory`, the effective mode is `mandatory`.
   - Otherwise the mode is `optional` (the default when both layers omit it).
   - The source is the deciding layer. When both layers agree, the source is the repository layer, matching the existing merge.
2. **Telemetry (consent-gated).** The first matching rule wins:
   - `degraded` → `Disabled(InvalidConfig)`.
   - Effective auth is `disabled` → `Disabled(AuthDisabled)`.
   - User layer `false` → `Disabled(UserOptOut)`. The global opt-out always wins.
   - Repository layer `false` → `Disabled(RepositoryOptOut)`.
   - User layer `true` → `Enabled`.
   - Repository layer `true` without user `true` → `Disabled(NoUserConsent)`.
   - Otherwise → `Disabled(NotEnabled)`.
3. **Background cloud operations** (post-commit auto-sync, hook-time credential reads) are allowed only when the effective auth mode is not `disabled` and the policy is not `degraded`. If an invalid file is skipped, nothing it might have prohibited can happen in the background through fallback defaults. Commands the user runs explicitly (`login`, `whoami`, `sync`) use the auth mode resolved from the remaining valid layers. An invalid explicit file stays fatal, as today.
4. **Contradictions** are deterministic `validation_warnings`, never errors, because each file can be valid on its own. Exact strings are fixed in T02:
   - Repository `telemetry.enabled: true` without user consent.
   - Any layer with `telemetry.enabled: true` while the effective auth is `disabled`.
   - `agent_trace.auto_sync: true` while the effective auth is `disabled` (auto-sync is skipped).
   - Repository `auth.mode: mandatory` overridden by a user-layer `disabled`.
   - A skipped invalid layer forcing telemetry and background cloud operations off.

### Authentication and telemetry behavior matrix

"Rejected" means a new `UserError::AuthDisabledByConfig` (key `auth.disabled_by_config`, runtime class). Its fixed message names `auth.mode` and the config file that set it. A rejection happens before any `auth.db` or credential-store access.

| Operation | `disabled` | `optional` | `mandatory` |
| --- | --- | --- | --- |
| `sce auth login` | Rejected | Allowed (current flow) | Allowed (current flow) |
| Telemetry provisioning inside login | n/a | Only when telemetry is `Enabled`. Otherwise skipped with result `disabled`: no issuance, no hint write, no repository enrollment | Same as `optional` |
| `sce auth logout` | Allowed (interactive deletion, explicit user action; no policy check) | Allowed | Allowed |
| `sce auth whoami` | Rejected | Unauthenticated → current success output | Unauthenticated → `UserError::NotAuthenticated` (nonzero) |
| `sce sync` | Rejected | Current behavior (missing or expired credentials → `NotAuthenticated`) | Same as `optional` |
| Post-commit auto-sync | Silently skipped (no spawn, `debug` log) | Spawned when `auto_sync` is true and background cloud is allowed | Same as `optional` |
| Managed telemetry init (commands and Claude hooks) | Noop `disabled_by_config` | Telemetry `Enabled` → current hint plus non-interactive lookup. Otherwise Noop `disabled_by_config` before the hint or `auth.db` is read | Same as `optional` |
| `sce auth telemetry-headers` | Fails closed with the existing fixed diagnostic | Telemetry `Enabled` → current behavior. Otherwise fails closed before repository resolution or `auth.db` | Same as `optional` |
| Claude hook execution (local work) | Unchanged | Unchanged | Unchanged (never prompts, never blocks on auth) |
| `sce doctor` managed telemetry row | `[PASS] Managed telemetry (disabled by config)`, no `auth.db` read | Telemetry not `Enabled` → same PASS row; otherwise current check | Same as `optional` |
| Setup Claude telemetry install | Skipped with a fixed "disabled by SCE config" line | Telemetry not `Enabled` → skipped with the same line; otherwise current install | Same as `optional` |

Repository binding, credential identity (`sub`, `workspace_id`), expiry checks, and the readiness hint keep their current behavior whenever an operation is allowed. The policy check is an extra precondition and replaces nothing.

`mandatory` enforcement boundary: it changes only explicit interactive results (`whoami` fails when unauthenticated). Cloud commands already require credentials. Hooks and background work never initiate login and never fail because of missing auth in any mode.

### Telemetry coverage limits (not claimed as full opt-out)

- `telemetry.enabled: false` stops SCE-managed OTLP export from the CLI, telemetry credential issuance, loading, and headers-helper output, and new Claude telemetry installs by setup.
- It does **not** remove Claude-native telemetry settings that an earlier `sce setup` already wrote to user-level Claude settings. With those settings, Claude Code keeps exporting to the managed collector without the `Authorization` header, and the collector is expected to reject the data. Claude telemetry from other repositories also shares the one user-level settings file. A real opt-out of Claude-native export needs settings reconciliation, which is out of scope. `sce config show` and the plan's context sync must state this.
- A user `OTEL_*` environment or Claude managed settings remain outside SCE control.

### Backward compatibility and migration

- **Existing configs:** files without `auth` or `telemetry` stay valid and resolve to `optional` / telemetry `Disabled(NotEnabled)`.
- **Behavior change for existing logged-in installs:** managed SCE export stops until the user sets `telemetry.enabled: true` in the global config. There is no inference from `telemetry-ready.json` or `auth.db` state, so no hidden state decides consent.
  - Discoverability: doctor's managed-telemetry row reports `disabled_by_config`.
  - When a valid readiness hint exists while telemetry is disabled, doctor's text adds one fixed line naming the opt-in key.
  - `sce auth login` reports `telemetry_provisioning: disabled` with a fixed message naming the key.
- **Stored state** (`auth.db`, telemetry credential, bindings, hint) is left untouched when telemetry is disabled. It is not read, and re-enabling resumes use without a new login while the credential is unexpired.
- **Older SCE binaries reject the new keys** (`additionalProperties: false`, `TOP_LEVEL_CONFIG_KEYS`). An older binary skips the whole discovered file as invalid, which also drops that file's other settings (for example `policies.bash`). Setup therefore does not write either key in this plan. Release notes must call this out.
- **`agent_trace.auto_sync`** keeps its semantics. It is additionally gated by background-cloud permission.

### Risks

- **Privacy:** the Claude-native export described under coverage limits.
  - Mitigation: setup no longer installs it without consent. Documented residual for existing installs.
- **macOS Keychain:**
  - The policy check must run before every `telemetry_readiness` / `auth.db` touch on hook and startup paths. Tests inject lookups that fail if called.
  - `logout` keeps interactive access in every mode, so one Keychain dialog per explicit logout remains possible. This is intended.
  - Non-interactive Keychain semantics stay unchanged.
- **Startup cost on hook paths:** the policy comes from the config resolution `build_startup_context` already performs. No additional file reads.
- **cwd-based local discovery:** Claude hooks and the headers helper resolve `<cwd>/.sce/config.json`. A launch from a subdirectory misses the repository layer, which can only *restrict*, so the failure mode is fail-open toward the user's global consent. This matches the existing config contract and is listed as an open question.
- **Mixed versions:** older binaries reject configs that carry the new keys (see Migration).

### Out of scope, with future interfaces

- Interactive `sce setup` consent questions and auto-login.
  - Interface: a future setup step writes `telemetry.enabled` / `auth.mode` into the **global** config file (the user layer). `resolve_access_policy` needs no change.
  - Keep `AuthMode::parse` / `as_str` and the key names stable for that writer.
- Keychain "Always Allow" verification commands.
  - Interface: they can call `ResolvedAccessPolicy::telemetry_enabled()` / `auth_allowed()` to decide applicability.
- Removing `telemetry-ready.json`. The policy gate sits before the hint, so the hint can later be removed without touching policy code.
- `auth.db` storage or migrations, macOS Keychain or code-signing changes, and Claude settings reconciliation.
  - Interface: reconciliation can consume `TelemetryPolicyState` to decide whether to remove SCE-owned Claude telemetry keys.

## Acceptance criteria

- [x] AC1: `auth.mode` (`disabled|optional|mandatory`) and `telemetry.enabled` (boolean) are accepted in global and repo-local config, unknown sub-keys and wrong types fail validation with JSON-pointer-prefixed errors, and configs without the sections remain valid.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml config::schema`; `nix run .#pkl-check-generated`.
- [x] AC2: Effective policy follows the precedence rules exactly: user telemetry opt-out always wins, repository `true` alone never enables telemetry, repository `false` disables it, user `disabled` auth cannot be overridden by the repository, `auth.mode: disabled` forces telemetry and auto-sync off, and any skipped invalid discovered layer forces telemetry and background cloud operations off.
  - Validate: table-driven `resolve_access_policy` test in `cli/src/services/config/resolver.rs` covering every user × repository × degraded combination; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml access_policy`.
- [x] AC3: `sce config show` reports `auth.mode`, `telemetry.enabled` (value, `source`, `config_source`), and the effective telemetry state with its reason in text and JSON; `sce config validate` and `show` report the defined contradiction warnings verbatim.
  - Validate: exact-output render tests in `cli/src/services/config/render.rs`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml config::render`.
- [x] AC4: With telemetry not `Enabled`, no SCE process (command, Claude hook, doctor, headers helper) reads the readiness hint, opens `auth.db`, resolves the encryption key, issues a telemetry credential, or exports managed spans — even when a valid `auth.db`, telemetry credential, and readiness hint exist.
  - Validate: unit tests with lookups that fail the test if invoked; multiprocess test in `cli/src/services/auth_db/multiprocess_tests.rs` running a Claude hook through `app::run` with an existing credential and `telemetry.enabled: false`, asserting zero exported spans at the loopback collector and an empty-key environment that would fail any key resolution.
- [x] AC5: With `auth.mode: disabled`, `sce auth login`, `whoami`, and `sync` fail with `auth.disabled_by_config` before any credential-store access, post-commit does not spawn auto-sync, and `sce auth logout` still deletes stored credentials.
  - Validate: auth-command, sync-command, and `run_post_commit_subcommand_with` tests with injected storage/spawn closures; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml auth_command sync commit_hooks`.
- [x] AC6: With `auth.mode: mandatory`, unauthenticated `sce auth whoami` exits nonzero with `auth.not_authenticated`, while hooks and auto-sync behave as in `optional`; with `optional`, current outputs are byte-identical.
  - Validate: auth-command tests for both modes with missing and expired stored credentials.
- [x] AC7: Login with telemetry not `Enabled` reports `telemetry_provisioning` `disabled` (text and JSON), makes no issuance request, writes no readiness hint, and enrolls no repository; with telemetry `Enabled` the existing issued/unavailable behavior is unchanged.
  - Validate: `run_login_with_stored_credentials` / provisioning tests with a provisioning closure that fails if called.
- [x] AC8: `sce setup` with the Claude target skips the Claude-native telemetry install with a fixed "disabled by SCE config" line when telemetry is not `Enabled`, leaving user Claude settings byte-unchanged, and installs as today when it is.
  - Validate: `cli/src/services/setup/command.rs` tests around `render_claude_telemetry_section`.
- [x] AC9: `sce doctor` reports `[PASS] Managed telemetry (disabled by config)` / JSON `state: disabled_by_config` without reading `auth.db` when telemetry is not `Enabled`, and post-commit auto-sync readiness reports disabled when auth is disabled.
  - Validate: `inspect_health_with` and doctor rendering tests; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml doctor`.

### Full validation

- `nix flake check`
- `nix run .#pkl-check-generated`
- Manual macOS check (not covered by CI): with an existing logged-in `auth.db` and readiness hint, a locked login Keychain, and global `telemetry.enabled: false`, run a Claude Code session in an SCE repository and `sce doctor`; confirm no Keychain dialog appears, no `sce.hook` spans arrive, and `sce auth logout` shows at most one interactive Keychain prompt.

### Context sync

- `context/cli/config-precedence-contract.md` (new keys, layer classification, precedence rules, warnings, show output)
- `context/sce/telemetry-credential.md` (policy gate before readiness hint, headers helper, setup install gate, coverage limits)
- `context/cli/agent-trace-auto-sync.md` (auth-policy gate on the trigger boundary)
- `context/cli/cli-command-surface.md` (auth command policy outcomes)
- `context/sce/cli-observability-contract.md` (`disabled_by_config` fallback reason)
- `context/sce/auth-db.md` (which paths may reach `auth.db` under each policy)
- `context/glossary.md` (auth mode, effective access policy, user/repository layer)
- `context/context-map.md`
- A decision record for the consent-gated precedence model (`context/decisions/`)

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** `config/pkl/base/sce-config-schema.pkl`; `cli/src/services/config/{types,schema,resolver,render,mod}.rs`; `cli/src/app.rs` (telemetry startup); `cli/src/services/managed_telemetry.rs`; `cli/src/services/doctor/{mod,managed_telemetry,inspect}.rs`; `cli/src/services/auth_command/mod.rs`; `cli/src/services/sync/{sync,command}.rs`; `cli/src/services/hooks/commit_hooks.rs`; `cli/src/services/setup/command.rs`; `cli/src/services/error.rs`; `cli/src/services/auth_db/multiprocess_tests.rs`; the context files listed under Context sync.
- **Out of scope:** new interactive setup questions; automatic login during setup; Keychain Always Allow verification commands; removing `telemetry-ready.json`; `auth.db` storage or migration changes; macOS Keychain implementation or code signing; reconciling or removing existing Claude Code telemetry settings; setup writing `auth`/`telemetry` keys; env-var or CLI-flag overrides for either key.
- **Constraints:** policy resolved once per config resolution through the existing resolver (no second resolver, no global mutable policy state); policy checks precede every readiness-hint, `auth.db`, and credential-store access on startup and hook paths; stable fixed user-facing strings; no secrets in output; deterministic warnings; `optional` mode output for existing commands stays byte-identical; Rust 2021, `anyhow` with context, rustfmt.
- **Non-goal:** a general feature-flag or permissions framework; per-command policy overrides; changing what `mandatory` means for hooks.

## Assumptions

- An explicit `--config` / `SCE_CONFIG_FILE` file is the user layer, consistent with it already replacing global discovery.
- Missing `telemetry.enabled` means disabled for every installation, existing ones included. Consent is never inferred from `telemetry-ready.json` or `auth.db`.
- Contradictions across layers are warnings, not validation errors, because each file can be individually valid.
- `sce auth logout` performs no policy check in any mode.
- `mandatory` adds no new hook or background behavior; its only enforced difference today is a nonzero unauthenticated `whoami`.
- Repository-layer discovery keeps the existing `<cwd>/.sce/config.json` rule.

## Task stack

- [x] T01: `Add auth and telemetry sections to the config schema and file model` (status:done)
  - Task ID: T01
  - Scope: In — Pkl `auth` and `telemetry` schema blocks with descriptions and defaults; `AuthMode` in `config/types.rs` (`parse`, `as_str`); `FileConfig.auth_mode` / `FileConfig.telemetry_enabled`; `map_auth_config` / `map_telemetry_config` and `TOP_LEVEL_CONFIG_KEYS` (+ description) in `config/schema.rs`; initializing the new fields in `resolve_runtime_config_with`. Out — merge semantics, effective policy, rendering, and runtime enforcement.
  - Dependencies: none
  - Done when: both sections parse from any layer with correct `ConfigPathSource`; invalid enum values, non-boolean `enabled`, unknown sub-keys, and non-object sections fail with schema-pointer errors; files without the sections parse unchanged.
  - Verify: table-driven parse tests in `config/schema.rs` (valid modes, invalid mode, `"true"` string, unknown sub-key, non-object, omitted); `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml config::schema`; `nix run .#pkl-check-generated`.
  - Completed: 2026-10-20
  - Files changed: `config/pkl/base/sce-config-schema.pkl`, `cli/src/services/config/types.rs`, `cli/src/services/config/schema.rs`, `cli/src/services/config/resolver.rs`, `context/plans/auth-telemetry-policy-config.md`
  - Result: Added Pkl `auth` (`mode` enum `disabled|optional|mandatory`, default `optional`) and `telemetry` (`enabled` boolean, default `false`) objects with `additionalProperties = false`; added `AuthMode` with `parse`/`as_str`; added `ParsedAuthConfigDocument`/`ParsedTelemetryConfigDocument`, `FileConfig.auth_mode`/`telemetry_enabled`, `map_auth_config`/`map_telemetry_config`, and `auth`/`telemetry` in `TOP_LEVEL_CONFIG_KEYS` and its description; `resolve_runtime_config_with` initializes both fields to `None` without merging them. New `access_policy_config_tests` module covers all four `ConfigPathSource` layers × three modes, omitted/empty sections, schema-pointer failures (invalid mode, non-string mode, `"true"`, `1`, unknown sub-keys, non-object sections), the key-allowlist backstop, and the `AuthMode::parse` error.
  - Verify outcome: `config::schema` tests passed (28 passed, 0 failed); `nix run .#pkl-check-generated` passed (142 files); `cargo clippy --all-targets -- -D warnings` via `run-cli-cargo.sh` passed.
  - Deviation: `FileConfig.auth_mode`, `FileConfig.telemetry_enabled`, and `AuthMode::as_str` carry `#[allow(dead_code)]` until T02 consumes them in the merge loop; T02 should remove these attributes.
  - Context impact: localized — config schema and file model only; no runtime behavior change until T02 merges the fields. Durable docs (`context/cli/config-precedence-contract.md`) gain the new keys and layer rules with T02/T03.
  - Context synchronization: synced

- [x] T02: `Resolve the effective access policy from user and repository layers` (status:done)
  - Task ID: T02
  - Scope: In — `PolicyLayer`, `TelemetryPolicyState`, `TelemetryDisabledReason`, `ResolvedAccessPolicy` in `config/types.rs`; per-layer capture inside the `resolve_runtime_config_with` merge loop; pure `resolve_access_policy(user, repo, degraded)` in `config/resolver.rs`; `RuntimeConfig.access_policy`; contradiction strings appended to `validation_warnings`; exposing the policy on `ResolvedAuthRuntimeConfig`, `ResolvedHookRuntimeConfig`, and `ResolvedObservabilityRuntimeConfig`. Out — rendering and consumer enforcement.
  - Dependencies: T01
  - Done when: the resolver implements the precedence rules in Design exactly, with source/provenance of the deciding layer; explicit config files act as the user layer; a skipped invalid discovered layer yields `Disabled(InvalidConfig)` and `background_cloud_allowed = false`; contradiction warnings are deterministic and ordered.
  - Verify: one table-driven test over user {absent, disabled, optional, mandatory} × repository {same} for auth and user {absent, true, false} × repository {absent, true, false} × degraded {false, true} × auth-disabled for telemetry; integration tests through `resolve_runtime_config_with` for explicit-file-as-user-layer, invalid-global-skipped, and invalid-local-skipped; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml access_policy`.
  - Completed: 2026-10-20
  - Files changed: `cli/src/services/config/types.rs`, `cli/src/services/config/resolver.rs`, `cli/src/services/config/schema.rs`, `context/plans/auth-telemetry-policy-config.md`
  - Result: Added `PolicyLayer` (with `from_config_path_source`: `Flag`/`Env`/`DefaultDiscoveredGlobal` → `User`, `DefaultDiscoveredLocal` → `Repository`), `TelemetryPolicyState`, `TelemetryDisabledReason`, and `ResolvedAccessPolicy` (`telemetry_enabled()`, `auth_allowed()`, `auto_sync_allowed(configured)`) in `types.rs`. `resolve_runtime_config_with` captures per-layer `AccessPolicyLayer` values in the merge loop and calls the pure `resolve_access_policy(user, repository, degraded)`. Auth mode: most restrictive wins (`disabled > mandatory > optional`), with ties sourced from the repository layer and `Default` when unset. `telemetry_enabled` is the merged value, repository over user. Telemetry state follows the Design rule chain, and `background_cloud_allowed = !auth_disabled && !degraded`. `RuntimeConfig.access_policy` is exposed on `ResolvedAuthRuntimeConfig`, `ResolvedHookRuntimeConfig`, and `ResolvedObservabilityRuntimeConfig`. Five fixed warning constants (`WARNING_REPOSITORY_TELEMETRY_WITHOUT_USER_CONSENT`, `WARNING_TELEMETRY_ENABLED_WITH_AUTH_DISABLED`, `WARNING_AUTO_SYNC_WITH_AUTH_DISABLED`, `WARNING_REPOSITORY_MANDATORY_OVERRIDDEN_BY_USER_DISABLED`, `WARNING_INVALID_LAYER_DISABLES_CLOUD`) are appended to `validation_warnings` in Design order, after the bash-policy warnings. Removed T01's `#[allow(dead_code)]` from `FileConfig.auth_mode`/`telemetry_enabled`.
  - Verify outcome: `access_policy` filter passed (15 passed, 0 failed: 10 new `resolver::access_policy_tests` covering the 4×4 auth table × degraded, the 3×3 telemetry table × degraded × auth-disabled, repository auth-disabled, warning order, explicit flag/env file as user layer, invalid-global-skipped, invalid-local-skipped, merge-loop warning order, defaults, and projection exposure; plus 5 T01 schema tests). `config::` tests passed (60 passed, 0 failed). `cargo clippy --all-targets -- -D warnings` via `run-cli-cargo.sh` passed.
  - Deviation: `AuthMode::as_str`, the `ResolvedAccessPolicy` helper methods, and the new `access_policy` fields on the three projected structs carry `#[allow(dead_code)]` until consumers (T03 rendering; T04–T08 enforcement) read them. Those tasks should remove the attributes.
  - Context impact: localized — config resolver and types only; no rendering or runtime enforcement change yet. The fixed warning strings and layer classification are durable contract material for `context/cli/config-precedence-contract.md`; T03 renders them.
  - Context synchronization: synced

- [x] T03: `Report auth and telemetry policy in config show and validate` (status:done)
  - Task ID: T03
  - Scope: In — `config/render.rs` text lines and JSON objects for `auth.mode` and `telemetry.enabled` (value, `source`, `config_source`) plus effective telemetry state and reason (`result.resolved.auth`, `result.resolved.telemetry`); contradiction warnings in `show` warnings and `validate` warnings; a fixed note when telemetry is disabled that Claude settings written by earlier setups are not changed. Out — enforcement.
  - Dependencies: T02
  - Done when: `show` text and JSON include the new fields in a deterministic position for defaults, global-only, local-only, and contradictory layers; `validate` stays limited to its existing fields and lists the new warnings.
  - Verify: exact-output tests in `config/render.rs` for default, user-enabled, repo-only-enabled, and auth-disabled-with-telemetry cases in both formats; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml config::render`.
  - Completed: 2026-10-20
  - Files changed: `cli/src/services/config/render.rs`, `cli/src/services/config/types.rs`, `cli/src/services/config/resolver.rs`, `context/plans/auth-telemetry-policy-config.md`
  - Result: `config show` text now lists, directly after `agent_trace.auto_sync` and before `otel.*`, `- auth.mode: <mode> (source: …[, config_source: …])`, `- telemetry.enabled: <bool> (source: …[, config_source: …])`, `- telemetry.effective: enabled` or `- telemetry.effective: disabled (reason: <reason>)`, and, only when disabled, `- telemetry.note: Claude Code telemetry settings written by earlier sce setup runs are not changed by telemetry.enabled.` JSON adds `result.resolved.auth.mode` (`value`/`source`/`config_source`) and `result.resolved.telemetry` with `enabled` (`value`/`source`/`config_source`), `effective` (`state`, `reason` or `null`), and `note` (fixed string, or `null` when enabled). Added `TelemetryPolicyState::as_str`/`disabled_reason` and `TelemetryDisabledReason::as_str` (`not_enabled`, `user_opt_out`, `no_user_consent`, `repository_opt_out`, `auth_disabled`, `invalid_config`). Removed `#[allow(dead_code)]` from `AuthMode::as_str`. `resolve_runtime_config_with` is now `pub(super)` so render tests resolve real layered configs. Contradiction warnings already flow through `validation_warnings` into `show` and `validate`, so no warning code changed. `validate` output fields are unchanged.
  - Verify outcome: `config::render` passed (3 passed, 0 failed). The new `render::tests` module covers default, user-enabled, repo-only-enabled (with repo `mandatory`), and auth-disabled-with-telemetry cases. For each, it checks the exact text policy block and its position, the exact JSON `auth`/`telemetry` objects, show warnings in both formats, and exact `validate` text and JSON. `config::` passed (63 passed, 0 failed). `cargo clippy --all-targets -- -D warnings` via `run-cli-cargo.sh` passed.
  - Deviation: none in scope. Test text assertions strip ANSI escapes so they hold when stdout is a TTY.
  - Context impact: localized — `config show` and `config validate` output contract. `context/cli/config-precedence-contract.md` should document the new show text lines, the JSON `resolved.auth`/`resolved.telemetry` shape, the reason strings, and the fixed telemetry note.
  - Context synchronization: synced

- [x] T04: `Gate managed telemetry startup and doctor health on telemetry policy` (status:done)
  - Task ID: T04
  - Scope: In — `TelemetryFallbackReason::DisabledByConfig` (`disabled_by_config`, non-failure, `debug` log); `RuntimeTelemetry::resolve` / `resolve_when_ready` taking the policy state and returning Noop before the readiness hint or lookup when not `Enabled`; `app.rs` passing the policy from `StartupContext` (the existing `resolve_observability_runtime_config` result); `inspect_health` / `inspect_health_with` and `doctor/managed_telemetry.rs` rendering `[PASS] Managed telemetry (disabled by config)` / JSON `disabled_by_config` plus the fixed opt-in line when a ready hint exists; multiprocess hook regression. Out — headers helper, login, sync.
  - Dependencies: T02
  - Done when: with telemetry not `Enabled`, command and Claude-hook startup and doctor never call `telemetry_readiness` or the credential lookup; with telemetry `Enabled`, behavior and fallback reasons are unchanged.
  - Verify: unit tests with a lookup future that panics if polled and a readiness probe that fails if called; doctor JSON/text tests; multiprocess test in `auth_db/multiprocess_tests.rs` running a Claude hook via `app::run` with a seeded credential, ready hint, empty `SCE_AUTH_DB_ENCRYPTION_KEY`, and `telemetry.enabled: false`, asserting no exported spans; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml managed_telemetry doctor multiprocess`.
  - Completed: 2026-10-20
  - Files changed: `cli/src/services/managed_telemetry.rs`, `cli/src/app.rs`, `cli/src/services/config/types.rs`, `cli/src/services/doctor/mod.rs`, `cli/src/services/doctor/managed_telemetry.rs`, `cli/src/services/doctor/render.rs`, `cli/src/services/doctor/types.rs`, `cli/src/app/doctor_reconciliation_cli_tests.rs`, `cli/src/services/auth_db/multiprocess_tests.rs`, `context/plans/auth-telemetry-policy-config.md`
  - Result: Added `TelemetryFallbackReason::DisabledByConfig` (`disabled_by_config`, non-failure, logged at `debug`). `RuntimeTelemetry::resolve(span_kind, telemetry_enabled, logger)` and `resolve_when_ready(telemetry_enabled, ready_probe, lookup, span_kind)` now return Noop with `disabled_by_config` before the readiness probe closure runs or the lookup future is polled. `AppRuntime.telemetry_enabled` is taken from `StartupContext.observability_config.access_policy.telemetry_enabled()`. `inspect_health(telemetry_enabled)` / `inspect_health_with(telemetry_enabled, readiness_probe, hint_present, lookup, now)` return the new `ManagedTelemetryHealth::DisabledByConfig { readiness_hint_present }` before any readiness read or credential lookup. Hint presence is a file-existence check on `telemetry-ready.json` only; the file is not read. Doctor resolves the policy from the inspected repository root, and a resolution failure counts as not enabled. Text renders `[PASS] Managed telemetry (disabled by config)`, plus, when the hint file exists, the fixed line `Managed telemetry export is off because telemetry.enabled is not true; set telemetry.enabled: true in the global SCE config to resume it.` through a new `DoctorDisplayDetail::Note`, which renders even on PASS rows. JSON reports `state`/`reason` `disabled_by_config` with null kind, severity, and next_action, and no doctor problem. Removed `#[allow(dead_code)]` from `ResolvedObservabilityRuntimeConfig.access_policy` and `ResolvedAccessPolicy::telemetry_enabled`. The allow now sits only on `auth_allowed` and `auto_sync_allowed`.
  - Verify outcome: `managed_telemetry` passed (42 passed, 0 failed), including new tests whose readiness probe panics if called and whose lookup panics if polled, for command and Claude-hook span kinds and for doctor health. `doctor` passed (34 passed, 0 failed), including render and JSON unit tests and a new CLI test, `doctor_with_telemetry_disabled_by_config_never_reads_the_auth_db`. That test covers a fresh state, then an unreadable `auth.db` plus a ready hint under default config, and checks that the auth DB and hint stay byte-identical and no warnings appear. `multiprocess` passed (18 passed, 0 failed), including the new `claude_hook_with_telemetry_disabled_by_config_never_resolves_the_credential`. Full CLI test suite passed (682 passed, 0 failed). `cargo clippy --all-targets -- -D warnings` via `run-cli-cargo.sh` passed. `cargo fmt` applied.
  - Deviation: The managed endpoint is hard-coded and cannot be redirected to a loopback collector. The multiprocess regression therefore proves "no export" through the hook's debug log: with a seeded credential, ready hint, and empty encryption key, `telemetry.enabled: false` logs only fallback reason `disabled_by_config` (Noop telemetry, no lookup). The contrast run with `enabled: true` logs `database_error`, which shows the test detects a lookup. The auth DB stays byte-identical. The doctor opt-in line keys off hint-file presence rather than hint validity, so AC4's "never reads the readiness hint" still holds. Two existing doctor CLI tests now pass an explicit `telemetry.enabled: true` config, because the default policy disables telemetry.
  - Context impact: localized — managed telemetry startup and doctor health contract. `context/sce/cli-observability-contract.md` should list the `disabled_by_config` fallback reason and the startup policy gate. `context/sce/telemetry-credential.md` should record the policy gate before the readiness hint and lookup. The doctor managed-telemetry output contract gains the `disabled_by_config` row, JSON state, and fixed opt-in line.
  - Context synchronization: synced

- [x] T05: `Fail closed in the telemetry headers helper when telemetry is disabled` (status:done)
  - Task ID: T05
  - Scope: In — `auth_command::run_telemetry_headers` resolving the policy from the working directory and `telemetry_headers_with` taking a `telemetry_enabled` input checked before readiness, repository resolution, and lookup. Out — Claude settings changes.
  - Dependencies: T02
  - Done when: telemetry not `Enabled` (including `auth.mode: disabled` and invalid discovered config) yields the existing fixed `Telemetry credential is unavailable.` with empty stdout, without calling the repository resolver or lookup; enabled behavior is unchanged.
  - Verify: extend `telemetry_headers_fail_closed_before_lookup_without_repository_or_on_conflict` into a table including the policy-disabled row with resolver/lookup closures that fail if called; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml telemetry_headers`.
  - Completed: 2026-10-20
  - Files changed: `cli/src/services/auth_command/mod.rs`, `cli/src/services/config/types.rs`, `cli/src/app.rs`, `cli/src/services/telemetry_credential_storage.rs`, `cli/src/services/auth_db/multiprocess_tests.rs`, `context/plans/auth-telemetry-policy-config.md`
  - Result:
    - `run_telemetry_headers` resolves the policy from the current directory through `config::resolve_auth_runtime_config`. A missing working directory or a resolution failure counts as not enabled.
    - `telemetry_headers_with(working_dir, telemetry_enabled, destination_conflict, ready_probe, resolve_repository, lookup)` checks `telemetry_enabled` (and the destination conflict) first. It then checks the working directory, then runs the readiness probe, then calls the repository resolver and the lookup.
    - Readiness is now a lazy `FnOnce() -> bool` probe, so `telemetry-ready.json` is never read when the policy is off.
    - The failure path still returns the fixed `Telemetry credential is unavailable.` runtime error, with empty stdout.
    - Removed `#[allow(dead_code)]` from `ResolvedAuthRuntimeConfig.access_policy`.
    - Updated the existing call sites in the app stall test and the storage headers test to pass `telemetry_enabled = true` and `|| true`.
  - Verify outcome:
    - `telemetry_headers` passed (11 passed, 0 failed). The rewritten `telemetry_headers_fail_closed_before_lookup_without_repository_or_on_conflict` is now a table: policy disabled, not ready, destination conflict, no working dir, no repository.
      - Every row records whether the readiness probe, the repository resolver, and the lookup were called. Lookup is never called. For the policy-disabled row, neither the probe nor the resolver runs.
      - The enabled path still renders `{"Authorization":"Bearer tok-scoped"}`.
    - New `telemetry_headers_policy_follows_auth_and_invalid_config` resolves real layered configs and checks the cases below. Only the global `true` case resolves to enabled:
      - global `true` → enabled
      - default → disabled
      - global `auth.mode: disabled` → disabled
      - repo `auth.mode: disabled` → disabled
      - invalid repo layer → disabled
    - `telemetry_headers_serve_only_the_bound_repository` (multiprocess, through `app::run`) also checks that the bound repository with an explicit `telemetry.enabled: false` config fails closed with the fixed diagnostic.
    - Full CLI suite passed (683 passed, 0 failed). `cargo clippy --all-targets -- -D warnings` via `run-cli-cargo.sh` passed. `cargo fmt` applied.
  - Deviation:
    - The destination-conflict check now also runs before the readiness probe. It is a pure env check, so behavior is unchanged.
    - The multiprocess headers children now pass an explicit `SCE_CONFIG_FILE` with `telemetry.enabled: true`, because the default policy disables telemetry. The config file is written once, before concurrent spawns.
    - Process: implementation edits started before the gate was shown. The `approved` token pre-authorized them, and T04's work was staged, so the baseline-relative diff is the unstaged delta.
  - Context impact: localized — headers-helper contract. `context/sce/telemetry-credential.md` should record that `sce auth telemetry-headers` checks the effective telemetry policy (resolved from its working directory) before the destination tripwire, readiness hint, repository resolution, and `auth.db` lookup, and fails closed with the existing fixed diagnostic.
  - Context synchronization: synced

- [x] T06: `Enforce auth mode and telemetry consent in auth login and whoami` (status:done)
  - Task ID: T06
  - Scope: In — `UserError::AuthDisabledByConfig` (`auth.disabled_by_config`) in `error.rs`; `run_login` and `run_whoami` checking the policy from `resolve_auth_runtime_config` before any `token_storage` call; `TelemetryProvisioningResult::Disabled` (status `disabled`, fixed message naming `telemetry.enabled`) and skipping `provision_telemetry_credential` when telemetry is not `Enabled`; `mandatory` unauthenticated `whoami` → `NotAuthenticated`; logout unchanged. Out — sync, setup.
  - Dependencies: T02
  - Done when: the auth rows of the behavior matrix hold for all three modes; `optional` outputs are byte-identical to today; logout under `disabled` still deletes both credential rows.
  - Verify: table-driven tests over mode × stored credentials {none, valid, expired} using injected storage/provision closures that fail if called where access is forbidden; login text/JSON rendering tests for `disabled` provisioning; `run_logout_with` test under `disabled`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml auth_command error`.
  - Completed: 2026-10-20
  - Files changed: `cli/src/services/error.rs`, `cli/src/services/auth_command/mod.rs`, `cli/src/services/config/types.rs`, `context/plans/auth-telemetry-policy-config.md`
  - Result:
    - Added `UserError::AuthDisabledByConfig(AuthPolicyConfigFile)` (key `auth.disabled_by_config`, runtime class). `AuthPolicyConfigFile` is `Global | Repository | ConfigFlag | ConfigEnv`. Each variant has a fixed message: `Authentication is disabled by \`auth.mode: disabled\` in <file>.`, a blank line, then `To use SCE authentication, set \`auth.mode\` to \`optional\` or \`mandatory\` in that file, then retry.`
    - `run_login` and `run_whoami` resolve config through `resolve_auth_runtime_config` first. They delegate to the new `run_login_with` / `run_whoami_with`, which call `ensure_auth_allowed` before the injected `load_tokens`. That check maps `auth_mode.source` to the config file named in the message.
    - Added `TelemetryProvisioningResult::Disabled` (status `disabled`, message ``Telemetry credential provisioning is disabled by SCE config. Set `telemetry.enabled: true` in the global SCE config to enable it.``). Text login renders it as `Note: <message>` under the headline.
    - `provision_if_enabled` returns `Disabled` without running provisioning (no issuance, no readiness hint, no enrollment) on both the renewal path and the text/JSON device-flow paths.
    - `whoami` with no stored credentials returns `UserError::NotAuthenticated` under `mandatory` and keeps the existing unauthenticated success render under `optional`.
    - Logout is unchanged.
    - Removed `#[allow(dead_code)]` from `ResolvedAccessPolicy::auth_allowed`.
    - Removed the doc comments on `UserError`, `AuthStorageFailure`, and the new enum in `error.rs`, at the user's request.
  - Verify outcome:
    - `auth_command` passed (20 passed, 0 failed).
    - New tests:
      - `login_follows_auth_mode_across_stored_credentials`: 3 modes × {none, valid, expired}. Under `disabled`, storage is never read.
      - `login_skips_provisioning_unless_telemetry_is_enabled`: the provisioning closure panics if called.
      - `whoami_follows_auth_mode_across_stored_credentials`: `optional` and unauthenticated gives byte-identical output; `mandatory` and none, or expired, gives `auth.not_authenticated`.
      - `auth_disabled_rejection_names_the_deciding_config_file`
      - `login_reports_disabled_telemetry_provisioning_in_text_and_json`
      - `logout_deletes_both_credential_rows_when_auth_is_disabled_by_config`
    - `error` filter: the new `auth_disabled_by_config_has_stable_runtime_catalog_mapping_per_file` passed. In 2 of 5 filtered runs, the unrelated `app::tests::managed_export_marks_failed_output_and_exports_final_error_log` failed intermittently: it runs `sce help` against a loopback collector, hit its 5 s receive timeout, and passed on rerun. It does not touch auth code.
    - Full CLI suite passed (690 passed, 0 failed).
    - `cargo clippy --all-targets -- -D warnings` via `run-cli-cargo.sh` passed. `cargo fmt` applied.
  - Deviation:
    - `ResolvedValue` records a layer kind, not a path, so the rejection message names the config layer (global, repository `.sce/config.json`, `--config` file, `SCE_CONFIG_FILE` file) rather than a filesystem path.
    - `whoami` now resolves config before reading storage. With an invalid explicit config file, an unauthenticated `whoami` now fails with the config error instead of rendering the unauthenticated success.
    - Logout takes no policy input, so its `disabled` test resolves a real `auth.mode: disabled` config and then shows that `run_logout_with` still clears readiness and deletes both rows.
  - Context impact: localized — the auth command contract. `context/cli/cli-command-surface.md` should record that `login` and `whoami` are rejected with `auth.disabled_by_config` before credential-store access, `login` reports `telemetry_provisioning` `disabled` without telemetry consent, and unauthenticated `whoami` under `mandatory` fails with `auth.not_authenticated`. `context/sce/telemetry-credential.md` should record that provisioning during login is gated on the telemetry policy. `context/sce/auth-db.md` should note that `login` and `whoami` do not reach `auth.db` under `disabled`.
  - Context synchronization: synced

- [x] T07: `Gate sce sync and post-commit auto-sync on auth policy` (status:done)
  - Task ID: T07
  - Scope: In — `sync::sync::run_current_sync_without_progress` returning a policy rejection before storage/credential access when auth is `disabled`, mapped by `sync/command.rs::classify_sync_error` to `AuthDisabledByConfig`; `commit_hooks::run_post_commit_subcommand` deciding launch from `ResolvedHookRuntimeConfig` (`auto_sync && background_cloud_allowed`); doctor `post_commit_auto_sync` reporting disabled when auth policy prevents it. Out — changing the auto-sync launcher or sync protocol.
  - Dependencies: T02, T06
  - Done when: `sce sync` under `disabled` fails with `auth.disabled_by_config` and no credential-store access; post-commit with `auto_sync: true` under `disabled` or degraded config never calls the launcher and still succeeds; `optional`/`mandatory` behavior is unchanged.
  - Verify: `run_post_commit_subcommand_with` table test with a launch closure that records calls across {auto_sync false/true} × {disabled, optional, mandatory, degraded}; sync classification test; doctor auto-sync state test; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml commit_hooks sync doctor`.
  - Completed: 2026-10-20
  - Files changed: `cli/src/services/sync/sync.rs`, `cli/src/services/sync/command.rs`, `cli/src/services/hooks/commit_hooks.rs`, `cli/src/services/config/types.rs`, `cli/src/services/config/resolver.rs`, `cli/src/services/doctor/inspect.rs`, `cli/src/services/auth_command/mod.rs`, `cli/src/app/doctor_reconciliation_cli_tests.rs`, `context/plans/auth-telemetry-policy-config.md`
  - Result:
    - Added `TraceSyncError::AuthDisabled(AuthPolicyConfigFile)`. It is not an authentication failure and has no storage failure.
    - `run_current_sync_without_progress` now resolves `resolve_auth_runtime_config` first. When auth is disabled it returns `AuthDisabled` before Agent Trace storage resolution and before the control-plane client (and so before any credential-store access) is built.
    - `classify_sync_error` maps `AuthDisabled` to `UserError::AuthDisabledByConfig` (key `auth.disabled_by_config`), ahead of the storage and authentication checks.
    - Added `ResolvedAccessPolicy::auth_disabled_config_file()`, which maps the deciding `auth.mode` source to `AuthPolicyConfigFile`. `auth_command::ensure_auth_allowed` now uses it, so login, whoami, and sync share one mapping.
    - `run_post_commit_subcommand_with` now takes a `ResolvedHookRuntimeConfig` resolver (production: `config::resolve_hook_runtime_config`). It launches auto-sync only when `access_policy.auto_sync_allowed(agent_trace_auto_sync)` is true. When auto-sync is configured but suppressed, it logs debug event `sce.agent_trace.auto_sync_skipped_by_policy`. The hook still succeeds either way.
    - `resolve_agent_trace_auto_sync_runtime_config` now also returns the access policy. Doctor's `post_commit_auto_sync.enabled` is now the configured value gated by `background_cloud_allowed`. So auth `disabled`, or a degraded config, reports state `disabled` / `[PASS] Post-commit Agent Trace auto-sync (disabled by config)`. `source` and `config_source` still describe `agent_trace.auto_sync`.
    - Removed `#[allow(dead_code)]` from `ResolvedAccessPolicy::auto_sync_allowed` and `ResolvedHookRuntimeConfig.access_policy`; `config/types.rs` now has no `dead_code` allows from this plan.
  - Verify outcome:
    - `commit_hooks sync doctor` filters passed (78 passed, 0 failed).
    - New `post_commit_launches_auto_sync_only_when_background_cloud_is_allowed`: real layered configs across {auto_sync false, true} × {disabled, optional, mandatory, degraded (invalid repo layer)}, with a recording launch closure. Auto-sync launches only for `true` + `optional`/`mandatory`. The skip debug event is logged only when auto-sync is configured but suppressed. Every row returns the success output.
    - New `sync_rejects_disabled_auth_before_storage_or_credential_access`: a non-Git temp dir with repository `auth.mode: disabled` yields `AuthDisabled(Repository)` rather than a storage `Runtime` error. This proves the gate runs before storage and the client are built.
    - New `classify_sync_error_maps_disabled_auth_to_auth_disabled_by_config`: covers all four config-file variants (key and message).
    - New CLI test `doctor_reports_post_commit_auto_sync_disabled_when_auth_is_disabled`: repo `auto_sync: true` reports enabled; adding `auth.mode: disabled` gives JSON `{state: disabled, enabled: false, source: config_file, config_source: default_discovered_local}` and the `(disabled by config)` PASS text row.
    - `auth_command` passed (20 passed) after the shared-mapping refactor.
    - Full CLI suite passed (694 passed, 0 failed). `cargo clippy --all-targets -- -D warnings` via `run-cli-cargo.sh` passed. `cargo fmt` applied.
  - Deviation:
    - Sync now resolves the auth config before the Agent Trace storage config. When both would fail, a config error now surfaces before a storage error; both are still runtime failures.
    - Doctor JSON `post_commit_auto_sync.enabled` now reports the effective (policy-gated) value instead of the raw configured value.
    - `optional`/`mandatory` sync and post-commit behavior is unchanged: the policy check only adds a precondition.
  - Context impact: localized — the sync command, the post-commit auto-sync trigger, and the doctor auto-sync row.
    - `context/cli/agent-trace-auto-sync.md` should record that post-commit launches only when `auto_sync` is true and background cloud operations are allowed (auth not `disabled`, no skipped invalid layer), with the debug skip event, and that doctor reports `disabled` in those cases.
    - `context/cli/cli-command-surface.md` should record that `sce sync` is rejected with `auth.disabled_by_config` before storage or credential access.
    - `context/sce/auth-db.md` should note that `sync` does not reach `auth.db` under `disabled`.
  - Context synchronization: synced

- [x] T08: `Skip Claude-native telemetry install in setup without telemetry consent` (status:done)
  - Task ID: T08
  - Scope: In — `setup/command.rs::render_claude_telemetry_section` (or its caller) resolving the policy for the setup repository and skipping `install_claude_telemetry` with a fixed `Claude telemetry: skipped; SCE telemetry is disabled by config.` line when not `Enabled`. Out — removing or reconciling existing Claude settings, setup prompts, writing config keys.
  - Dependencies: T02
  - Done when: setup with the Claude target leaves user Claude settings byte-unchanged and prints the skip line when telemetry is not `Enabled`, and installs exactly as today when it is.
  - Verify: tests alongside `setup_claude_telemetry_installs_when_claude_is_selected` for enabled and disabled policies asserting file bytes and report text; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup_claude_telemetry`.
  - Completed: 2026-10-20
  - Files changed: `cli/src/services/setup/command.rs`, `context/plans/auth-telemetry-policy-config.md`
  - Result:
    - Added the fixed constant `CLAUDE_TELEMETRY_DISABLED_BY_CONFIG_LINE` (`Claude telemetry: skipped; SCE telemetry is disabled by config.`) in `setup/command.rs`.
    - `render_claude_telemetry_section(telemetry_enabled, settings_path, managed_settings_path)` returns that line before `install_claude_telemetry` runs when telemetry is not enabled, so user Claude settings are neither read nor written.
    - `claude_telemetry_report(repository_root)` resolves `config::resolve_auth_runtime_config(repository_root)` (global layer plus `<repository_root>/.sce/config.json`); a resolution failure counts as not enabled.
    - The enabled path is unchanged.
  - Verify outcome:
    - `setup_claude_telemetry` passed (6 passed, 0 failed).
    - New `setup_claude_telemetry_skips_install_when_telemetry_is_disabled_by_config`: the disabled policy returns exactly the skip line, creates no settings file when none exists, and leaves an existing settings file byte-identical. The enabled policy on the same file installs with the exact configured and authentication-pending report and keeps existing keys.
    - New `setup_claude_telemetry_policy_follows_layered_config`: through the resolver seam, only global `telemetry.enabled: true` is enabled. Default, repo-only `true`, repo `false` opt-out, repo `auth.mode: disabled`, and an invalid repo layer are all disabled.
    - Existing setup telemetry tests now pass `telemetry_enabled = true`.
    - Full CLI suite passed (696 passed, 0 failed). `cargo clippy --all-targets -- -D warnings` via `run-cli-cargo.sh` passed. `cargo fmt` applied.
  - Deviation: none. The baseline was a clean tree at `9674e199`.
  - Context impact: localized — the setup Claude telemetry install step. `context/sce/telemetry-credential.md` (or the setup context it references) should record that setup installs Claude-native telemetry only when the effective telemetry policy for the setup repository is `Enabled`; otherwise it prints the fixed skip line and leaves user Claude settings unchanged. Settings written by earlier setups are not removed.
  - Context synchronization: synced

## Open questions

- Existing logged-in users silently lose managed SCE export on upgrade, because a missing `telemetry.enabled` means disabled. Doctor and login now say so, but nothing prompts the user. Is that acceptable, or should the release ship with the future setup consent question? Inferring consent from an existing `telemetry-ready.json` was rejected because it makes hidden state decide consent.
- `mandatory` changes very little today. Cloud commands already require credentials, and hooks must never block, so the only enforced difference is a nonzero unauthenticated `whoami`. It is kept so the future setup and auto-login work has a stable mode to build on. If no near-term consumer exists, shipping only `disabled | optional` would be the smaller version.
- Telemetry opt-out does not stop Claude-native export from settings written by earlier setups, and those settings are user-level, shared across repositories. Should a follow-up plan for SCE-owned Claude settings reconciliation be scheduled now, so the opt-out can eventually be described as complete?
- Repository-layer discovery uses `<cwd>/.sce/config.json`. A Claude hook or headers helper launched from a subdirectory sees only the user layer, so a repository opt-out can be missed there. Fixing this means Git-root-relative discovery for every config consumer, which is broader than this plan.

## Validation Report

**Status:** validated  
**Date:** 2026-10-20

### Commands run

- `nix flake check` -> exit 0 (all 6 checks passed: cli-tests, cli-clippy, cli-fmt, pkl-generated-check, sce-cli-generated-input-check, codex-hook-command-check)
- `nix run .#pkl-check-generated` -> exit 0 (ephemeral Pkl generation passed, 142 files)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml config::schema` -> exit 0 (28 passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml access_policy` -> exit 0 (16 passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml config::render` -> exit 0 (3 passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml auth_command` -> exit 0 (20 passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml sync` -> exit 0 (40 passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml commit_hooks` -> exit 0 (5 passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml doctor` -> exit 0 (35 passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup_claude_telemetry` -> exit 0 (6 passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml multiprocess` -> exit 0 (18 passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml telemetry_headers` -> exit 0 (11 passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml managed_telemetry` -> exit 0 (42 passed)
- Manual macOS Keychain check -> not run (validation host is x86_64-linux; plan marks it as not covered by CI)

### Success-criteria verification

- [x] AC1: auth/telemetry config sections accepted, invalid shapes rejected with pointers, absent sections valid -> `config::schema` `access_policy_config_tests` (5 tests) passed; `pkl-check-generated` passed
- [x] AC2: effective policy precedence -> `config::resolver::access_policy_tests` (consent/opt-out, most-restrictive auth, repo auth disabled forces telemetry off, invalid global/local layers degrade policy) passed
- [x] AC3: config show/validate report policy and warnings -> `show_json_reports_resolved_auth_and_telemetry_policy`, `show_text_reports_access_policy_after_agent_trace_and_lists_policy_warnings`, `validate_lists_policy_warnings_without_policy_fields` passed
- [x] AC4: no readiness/auth.db/key/credential/export access without telemetry consent -> `claude_hook_with_telemetry_disabled_by_config_never_resolves_the_credential` (multiprocess), `telemetry_disabled_by_config_never_probes_readiness_or_looks_up_credentials`, `health_disabled_by_config_never_probes_readiness_or_looks_up_credentials`, `telemetry_headers_policy_follows_auth_and_invalid_config` passed
- [x] AC5: `auth.mode: disabled` rejects login/whoami/sync before storage, no post-commit auto-sync, logout still deletes -> `whoami_follows_auth_mode_across_stored_credentials` (0 storage reads when disabled), `auth_disabled_rejection_names_the_deciding_config_file`, `sync_rejects_disabled_auth_before_storage_or_credential_access`, `classify_sync_error_maps_disabled_auth_to_auth_disabled_by_config`, `post_commit_launches_auto_sync_only_when_background_cloud_is_allowed`, `logout_deletes_both_credential_rows_when_auth_is_disabled_by_config` passed
- [x] AC6: `mandatory` unauthenticated whoami fails with `auth.not_authenticated`; `optional` unchanged -> `whoami_follows_auth_mode_across_stored_credentials` covers all modes × none/valid/expired credentials; passed
- [x] AC7: login without telemetry consent reports `disabled` provisioning and makes no issuance -> `login_skips_provisioning_unless_telemetry_is_enabled`, `login_reports_disabled_telemetry_provisioning_in_text_and_json` passed
- [x] AC8: setup skips Claude telemetry install without consent, settings byte-unchanged -> `setup_claude_telemetry_skips_install_when_telemetry_is_disabled_by_config`, `setup_claude_telemetry_policy_follows_layered_config` passed
- [x] AC9: doctor reports disabled_by_config without reading auth.db; auto-sync readiness disabled when auth disabled -> `doctor_with_telemetry_disabled_by_config_never_reads_the_auth_db`, `doctor_reports_post_commit_auto_sync_disabled_when_auth_is_disabled`, `telemetry_disabled_by_config_reports_no_problem_and_a_stable_json_state`, `telemetry_disabled_by_config_renders_pass_row_with_opt_in_line_only_for_ready_hint` passed

### Failed checks and follow-ups

- None.

### Residual risks

- The manual macOS check (locked login Keychain, no Keychain dialog, no `sce.hook` spans, at most one logout prompt) was not run; macOS Keychain behavior is unverified on real hardware.
- Flake checks ran only for x86_64-linux; darwin and aarch64-linux systems were omitted.
- Open questions in this plan (silent loss of managed export on upgrade, minimal `mandatory` semantics, legacy Claude-native settings, `<cwd>`-relative repository discovery) remain unresolved product decisions.
