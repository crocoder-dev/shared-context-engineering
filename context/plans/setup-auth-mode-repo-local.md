# Plan: setup-auth-mode-repo-local

## Change summary

Interactive `sce setup` saves both access answers, `auth.mode` and `telemetry.enabled`, to the repository-local `.sce/config.json` of the repository being set up, so each applies only to that repository. Today `persist_global_access_preferences` (`cli/src/services/setup/access_preferences.rs`) writes both to the global user config (`~/.config/sce/config.json`), so a `mandatory` or a telemetry consent chosen in one repository silently applies to every repository on the machine. To make a repository-local consent take effect, the access-policy resolver also changes: a repository `telemetry.enabled: true` now grants consent instead of resolving to `no_user_consent`.

The plan also makes a missing login under `auth.mode: mandatory` visible without blocking local work. `sce doctor` reports a blocking `SCE login` problem and shows this repository's effective telemetry state. Non-interactive setup prints a login reminder. A Claude Code session prints one message per session telling the user to run `sce auth login`.

This replaces the 2026-10-21 decision "Setup writes access-policy answers to the user layer only" and the user-side-only consent rule of the 2026-10-20 decision "Consent-gated precedence for the effective access policy". The prompt seam, the setup login step, and the most-restrictive-wins `auth.mode` precedence are otherwise unchanged. Existing global `auth.mode` / `telemetry.enabled` values are not migrated; the user removes them by hand.

## Design

### Current implementation findings

- `prepare_interactive_setup` (`cli/src/services/setup/command.rs`) reads `read_global_access_preferences(global_config_path)` and folds both keys into `PersistedSetupConfirmations` through `with_global_access_preferences`. `persisted_setup_confirmations(repository_root)` reads only `agent_trace.auto_sync` and `policies.attribution_hooks.enabled` from the repository file.
- The prompt step in `resolve_setup_dispatch` (`cli/src/services/setup/mod.rs`) asks for each access answer when it isn't saved yet (or `--reconfigure` is passed) and the global config is valid. The labels are `SCE account mode for this machine` and a telemetry label ending `Applies to all repositories on this machine.` (`prompt.rs`).
- `SetupAccessGate::from_policy` treats a repository or explicit `auth.mode: disabled` as a lock (`auth_locked_by` filters out only `Global`), and treats a non-global `mandatory` as `mandatory_locked`.
- `persist_setup_access_preferences` writes both answers to the global file before any repository write. The `SCE access:` report renders `saved to global SCE config '<path>'; applies to all repositories on this machine` (`SAVED_ACCESS_SCOPE`).
- `persist_integration_targets` writes the repository file after the target install, skips an invalid file, and uses non-atomic `fs::write`. A shared `atomic_write::write_atomically` exists.
- `resolve_access_policy` (`cli/src/services/config/resolver.rs`) enables telemetry only on a user or explicit `true`. A repository `true` alone gives `Disabled(NoUserConsent)` plus the contradiction warning `telemetry.enabled is true in the repository config but not in the user config; ...`.
- `sce doctor` reports `[PASS] Managed telemetry (disabled by config)` or `(not configured)`, and its opt-in note (`OPT_IN_NOTE`, `cli/src/services/doctor/managed_telemetry.rs`) tells the user to set `telemetry.enabled: true` in the global SCE config. Doctor has no login check.
- Claude-native telemetry settings are written to the user-level `~/.claude/settings.json` (`resolve_claude_settings_path`). The `otelHeadersHelper` (`sce auth telemetry-headers`) resolves the policy from its working directory and fails closed when telemetry is not `Enabled` there.
- The generated Claude `settings.json` (`config/pkl/renderers/claude-content.pkl`) has a `SessionStart` hook running `sce hooks claude-model-state`. The mutation-scope lifecycle returns empty output for `SessionStart`.
- The `auth` / `telemetry` schema sections were added in `d46c03cd` (2026-10-20), after the latest release tag `v0.4.0-pre-alpha-v11`. The schema uses `additionalProperties = false`.

### Target behavior

- **Writer:** setup writes the `auth.mode` and `telemetry.enabled` answers to `<repository root>/.sce/config.json`. The repository writer has the same guardrails as the global writer: write explicit answers only, preserve unrelated keys, validate with `parse_file_config`, skip no-op writes, write atomically (temp file plus rename, keeping the mode), refuse when the bytes changed since the first read, refuse to replace a symlink, and never rewrite an invalid file. A missing file is bootstrapped first. It runs before the target install, so a failed write aborts setup before any other repository file changes. Setup no longer writes access keys to the global config.
- **Saved answers:** prompt suppression and the `--reconfigure` defaults come from the repository file. An invalid repository file skips the access prompts.
- **Locks:** a repository value is setup's saved answer, not a lock. Global and explicit values are the locks: `disabled` there skips the access prompts, and `mandatory` there suppresses the login offer.
- **Telemetry precedence:** any `false` in any layer disables telemetry (`user_opt_out` for user or explicit, `repository_opt_out` for repository). Otherwise any `true` in any layer enables it. `auth.mode: disabled` and a degraded config still disable it first. The `no_user_consent` reason and its contradiction warning are removed.
- **Wording:** the prompt labels and the `SCE access:` saved lines say `saved to repository SCE config '<path>'; applies only to this repository`. Doctor's opt-in note and the `sce config show` telemetry note point at the repository config.
- **Doctor:** a new `SCE login` row. When the effective mode is `mandatory` and no valid stored credential is found, it is a blocking `[FAIL]` that names the config file setting `mandatory`, with the fix `Run 'sce auth login'`. The managed-telemetry row reports the effective state for the inspected repository (`enabled for this repository`, or `disabled (reason: <reason>)`) and its source layer.
- **Non-interactive setup:** when the effective mode is `mandatory`, setup completes and adds the fixed line `SCE login: required by auth.mode: mandatory for this repository; run 'sce auth login' if you are not logged in.`. It doesn't open `auth.db` or the credential store, and it exits 0.
- **Claude session notice:** at `SessionStart`, when the effective mode for the session's working directory is `mandatory` and no valid stored credential is found, the hook emits one Claude `systemMessage`: `SCE login required for this repository. Run 'sce auth login'.`. It never fails the hook, emits nothing otherwise, and runs once per session start.

## Acceptance criteria

- [x] AC1: Interactive setup writes the answered `auth.mode` and `telemetry.enabled` to the repository `.sce/config.json`, writes no access keys to the global config, and keeps unrelated repository keys.
  - Validate: setup command tests in `cli/src/services/setup/command.rs`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup`.
- [x] AC2: A rerun suppresses each access prompt when the repository file holds its value, `--reconfigure` asks again with that value as the default, and a global value no longer suppresses a prompt.
  - Validate: `resolve_setup_dispatch` / `prepare_interactive_setup` tests with `FakePrompter` for the repo-saved, global-only, and `--reconfigure` cases.
- [x] AC3: The repository writer refuses an invalid, concurrently modified, or symlinked file and leaves it byte-identical, and setup then aborts before the target install.
  - Validate: repository writer unit tests plus one setup command abort test.
- [x] AC4: Global or explicit `disabled` skips the access prompts and global or explicit `mandatory` suppresses the login offer, while repository values are only prompt defaults.
  - Validate: `SetupAccessGate::from_policy` and dispatch tests per layer.
- [x] AC5: A repository `telemetry.enabled: true` with no `false` in any layer gives effective telemetry `enabled`. Any `false` still disables it, and `no_user_consent` no longer exists.
  - Validate: `resolve_access_policy` tests in `cli/src/services/config/resolver.rs`; `sce config show --format json` shows `result.resolved.telemetry.effective.state` `enabled` in a repository with only a repository `true`.
- [x] AC6: Under `mandatory` with no valid stored credential, `sce doctor` reports a blocking `SCE login` failure with the `sce auth login` fix. It also shows the repository's effective telemetry state and its source.
  - Validate: doctor render tests with exact text and JSON assertions; `nix run .#sce -- doctor` in a repository with `auth.mode: mandatory` and no login.
- [x] AC7: Non-interactive setup under effective `mandatory` prints the fixed login reminder line, exits 0, and never touches `auth.db` or the credential store.
  - Validate: setup command test for `SetupMode::NonInteractive` under `mandatory` asserting the exact line and no credential-store access.
- [x] AC8: A Claude `SessionStart` under `mandatory` without a valid stored credential emits exactly one `systemMessage` with `SCE login required for this repository. Run 'sce auth login'.`, and emits nothing when logged in or not `mandatory`. The hook always succeeds.
  - Validate: hook tests for each case; `nix run .#pkl-check-generated` if the generated Claude settings change.
- [x] AC9: A new ADR supersedes the 2026-10-21 decision and the user-side-only consent rule of the 2026-10-20 decision, and durable context no longer says setup writes access keys globally or that only the user side grants consent.
  - Validate: inspect `context/decisions/` and the files listed under Context sync; `nix shell nixpkgs#ripgrep -c rg -n "applies to all repositories on this machine|for this machine|no_user_consent" context cli/src` returns no matches outside superseded ADRs and completed plans.

### Full validation

- `nix flake check`
- `nix run .#pkl-check-generated`

### Context sync

- `context/cli/access-policy.md` (resolution, contradiction warnings, reasons, Setup, Enforcement).
- `context/sce/setup-repo-local-config-bootstrap.md` (access prompts, Reconfigure, access preferences, report lines, setup login, non-interactive reminder).
- `context/sce/telemetry-credential.md` (doctor health check and its opt-in note, Claude-native telemetry scope).
- `context/glossary.md` (setup access preferences, auth login, doctor entries).
- `context/decisions/` new ADR, with supersession notes on `2026-10-21-setup-writes-access-answers-to-user-layer-only.md` and `2026-10-20-consent-gated-access-policy-precedence.md`.

## Task context synchronization lifecycle

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** `cli/src/services/setup/` (access preferences, command, mod, prompt), `cli/src/services/config/resolver.rs` and its config types/render, `cli/src/services/doctor/`, the Claude `SessionStart` hook handler under `cli/src/services/hooks/` and its Pkl hook wiring if needed, inline tests, the new ADR, and the context files listed under Context sync.
- **Out of scope:** migrating or deleting existing global access values, `auth.mode` precedence (most restrictive still wins), moving Claude-native telemetry settings out of user-level `~/.claude/settings.json`, changing Claude telemetry removal rules, OpenCode/Pi/Codex session notices, and blocking local SCE features when not logged in.
- **Constraints:** hooks never fail or block a tool call or commit because of a missing login; non-interactive setup keeps its no-credential-store-access contract; keep stdout/stderr contracts and fixed strings apart from the wording changes listed; no new dependencies.
- **Non-goal:** a gitignored per-user repository override file. Values go to the committed `.sce/config.json`.

## Assumptions

- "Local" means the repository's committed `.sce/config.json`, so saved answers, including telemetry consent, apply to everyone who uses the repository. The user chose to store telemetry consent there knowing it replaces the consent-gated rule.
- Repository access values are treated as setup's saved answers, not team locks, because setup can't tell which values it wrote.
- Leftover global values from earlier setup runs stay in effect: a global `mandatory` still wins, and a global `telemetry.enabled: false` still opts out.
- "Valid stored credential" for doctor and the session notice uses the non-interactive credential lookup under `CREDENTIAL_LOOKUP_DEADLINE`, as the setup authorization check does, and fails closed: an unavailable store counts as not logged in. Non-interactive setup doesn't do this lookup, so it prints its reminder whenever the mode is `mandatory`.
- The Claude session notice uses the `SessionStart` hook's JSON `systemMessage` output. Whether it extends `sce hooks claude-model-state` or adds a new hook entry is decided in T06.

## Task stack

- [x] T01: `Add repository-local access preferences writer` (status:done)
  - Task ID: T01
  - Scope: In — a repository `auth.mode` / `telemetry.enabled` reader and writer in `cli/src/services/setup/access_preferences.rs` (or a sibling module) reusing `write_atomically`, plus unit tests. Out — setup wiring, resolver, report wording, docs.
  - Dependencies: none
  - Done when: the writer writes only explicit answers, bootstraps a missing file, preserves unrelated keys, skips no-op writes, writes atomically, and refuses invalid, concurrently modified, and symlinked files without changing them; targeted tests pass.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml access_preferences`.
  - Completed: 2026-10-22
  - Files changed: `cli/src/services/setup/access_preferences.rs`, `cli/src/services/setup/command.rs`, `cli/src/services/setup/mod.rs`
  - Result: The writer core now depends on the config layer (`ConfigLayer::Global` / `Repository`). The layer picks the validation source, the labels in errors and I/O context, and the bootstrap value. A missing repository file starts from the existing repo-local bootstrap payload. Added `read_repository_access_preferences` and `persist_repository_access_preferences`, which return `RepositoryAccessPreferencesError` with repository-specific text. The global API and its error strings are unchanged. The shared types were renamed `GlobalAccessPreferences` → `AccessPreferences` and `GlobalWriteOutcome` → `AccessWriteOutcome`; `command.rs` and `mod.rs` only follow the rename. The two new entry points carry `#[cfg_attr(not(test), expect(dead_code))]` until T03 wires them in, and T03 must remove it. Added 7 inline unit tests covering bootstrap, explicit-only answers, preserved keys, no-op skip, invalid/concurrent/symlink refusal with unchanged bytes, and atomic replace keeping the mode.
  - Verify outcome: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml access_preferences` passed (7 passed). Extra checks: the `setup` filter passed (69 passed), and `clippy --all-targets --all-features -- -D warnings` was clean.
  - Context impact: none — internal helper with no behavior change until T03 wires it; durable context updates belong to T07.
  - Context synchronization: synced

- [x] T02: `Let repository telemetry.enabled grant consent` (status:done)
  - Task ID: T02
  - Scope: In — `resolve_access_policy` telemetry state (any `false` disables, otherwise any `true` enables), removal of `TelemetryDisabledReason::NoUserConsent` and its contradiction warning, the `sce config show` telemetry note text, resolver and render tests. Out — `auth.mode` precedence, setup, doctor.
  - Dependencies: none
  - Done when: AC5 holds; the remaining contradiction warnings and reason strings are unchanged.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml resolver`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml config`.
  - Completed: 2026-10-22
  - Files changed: `cli/src/services/config/resolver.rs`, `cli/src/services/config/types.rs`, `cli/src/services/config/render.rs`
  - Result: `resolve_access_policy` now checks in this order: `invalid_config`, then `auth_disabled`, then user/explicit `false` → `user_opt_out`, then repository `false` → `repository_opt_out`, then any `true` → `Enabled`, otherwise `not_enabled`. A new helper, `resolve_telemetry_enabled`, sets the reported `telemetry.enabled` value and source from the layer that decides the outcome: the first `false` across explicit, user, repository, otherwise the first `true` across explicit, repository, user. Before, it was the first layer with any value. Removed `TelemetryDisabledReason::NoUserConsent`, `WARNING_REPOSITORY_TELEMETRY_WITHOUT_USER_CONSENT`, and the `user_telemetry_*` helpers. The other warnings and reason strings are unchanged. For `not_enabled`, `sce config show` now uses the new `TELEMETRY_NOT_ENABLED_NOTE`, which begins `Set telemetry.enabled: true in the repository SCE config to enable SCE telemetry for this repository.` and then repeats the unchanged Claude-settings removal text. Every other disabled reason keeps the existing note. Added 5 resolver tests and 2 render tests.
  - Verify outcome: `resolver` filter passed (25 passed); `config` filter passed (91 passed). Extra checks: `setup` filter passed (69 passed); `clippy --all-targets --all-features -- -D warnings` was clean; the built `sce config show --format json` in a temp repository with only a repository `telemetry.enabled: true` (isolated `XDG_CONFIG_HOME`) gave `effective.state` `enabled`, `enabled.config_source` `default_discovered_local`, `note` null, and no warnings.
  - Context impact: behavior change to the effective access policy — `no_user_consent` and its warning are gone, a repository `true` grants consent, the reported `telemetry.enabled` source is the deciding layer, and there is a new `not_enabled` note. Affects `context/cli/access-policy.md` (resolution flow, contradiction warnings, reason strings, `telemetry.note`); the ADR supersession belongs to T07.
  - Context synchronization: synced

- [x] T03: `Save and read setup access answers in the repository layer` (status:done)
  - Task ID: T03
  - Scope: In — `prepare_interactive_setup` reads saved answers from the repository file and writes new ones through the T01 writer before the target install; remove setup's global access write; `SetupAccessGate::from_policy` locks on global/explicit instead of repository; the invalid repository file gates the access prompts; prompt labels, `SCE access:` saved lines, and write-failure text naming the repository file; setup tests. Out — login step behavior, non-interactive setup, doctor, hooks.
  - Dependencies: T01, T02
  - Done when: AC1–AC4 hold in setup command and dispatch tests, and the report shows repository scope for both answers.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup`.
  - Completed: 2026-10-22
  - Files changed: `cli/src/services/setup/access_preferences.rs`, `cli/src/services/setup/command.rs`, `cli/src/services/setup/mod.rs`, `cli/src/services/setup/prompt.rs`
  - Result: `prepare_interactive_setup` gets the global config path no more. It reads saved answers from `<repo>/.sce/config.json` through `read_repository_access_preferences`, and `PersistedSetupConfirmations::with_repository_access_preferences` folds them in. It writes the answers through `persist_repository_access_preferences` inside `prepare_dispatch`, so this runs before the target install. `SetupAccessSave` now holds `repository_config_path`. A failed write maps to `Failed to save SCE access preferences to the repository SCE config '<path>'.` plus the writer error and `REPOSITORY_ACCESS_*_REMEDIATION`, which keeps the existing remediation text. Removed `persist_global_access_preferences`, `GlobalAccessPreferencesError`, and the T01 `dead_code` expects. `global_config_invalid` became `repository_config_invalid`: an invalid repository file skips the access prompts, and an invalid global file no longer does. `SetupAccessGate::from_policy`: `auth_locked_by` drops `Repository` instead of `Global`, and `mandatory_locked` holds for every source except `Default` and the repository file. Wording: `SAVED_ACCESS_SCOPE` is now `applies only to this repository`. Saved and unchanged lines name `repository SCE config '<path>'`. The prompt title is now `SCE account mode for this repository`, and the telemetry label ends `Applies only to this repository.`. Deviation (approved assumption): the Claude-native telemetry removal step still reads only the global preferences (`default_global_preferences()`), because the removal rules are out of scope. So a repository `telemetry.enabled: false` answer gives `SkipDisabled`, not `Remove`. T07 should record this in the ADR. Added 4 setup command tests: repository-only write with preserved keys and the exact report; saved-answer prompt suppression, global-only prompting, `--reconfigure` defaults, the repository/global `disabled` and global `mandatory` gates, and invalid-file skip; per-layer gate locks; the symlink abort with the exact error and an untouched target. The recording `FakePrompter` now logs each prompt with its default.
  - Verify outcome: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup` passed (73 passed). Extra checks: `clippy --all-targets --all-features -- -D warnings` was clean, `cargo fmt` was applied, and `rg "applies to all repositories on this machine|for this machine|Applies to all repositories|global_config_invalid" cli/src` returned no matches.
  - Context impact: user-visible behavior change. Interactive setup now saves `auth.mode` / `telemetry.enabled` to the repository `.sce/config.json`. Saved answers and prompt suppression come from the repository file. Global or explicit values are the locks. Prompt labels and the `SCE access:` lines say repository scope. Affects `context/sce/setup-repo-local-config-bootstrap.md` (access prompts, Reconfigure, access preferences, report lines), the Setup section of `context/cli/access-policy.md`, and the glossary entry for setup access preferences. The ADR belongs to T07.
  - Context synchronization: synced

- [x] T04: `Report mandatory login and repository telemetry state in sce doctor` (status:done)
  - Task ID: T04
  - Scope: In — a doctor `SCE login` check (blocking `[FAIL]` under effective `mandatory` without a valid stored credential, naming the deciding config file, fix `Run 'sce auth login'`); the managed-telemetry row and JSON show the repository's effective telemetry state and source; `OPT_IN_NOTE` points at the repository config; doctor render tests. Out — `sce doctor --fix` behavior, credential creation, other doctor rows.
  - Dependencies: T02
  - Done when: AC6 holds with exact text and JSON assertions; doctor still never creates `auth.db` or a credential-store key.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml doctor`.
  - Completed: 2026-10-22
  - Files changed: `cli/src/services/doctor/access.rs` (new), `cli/src/services/doctor/managed_telemetry.rs`, `cli/src/services/doctor/mod.rs`, `cli/src/services/doctor/render.rs`, `cli/src/services/doctor/types.rs`, `cli/src/services/lifecycle.rs`
  - Result: A new `doctor/access.rs` holds `DoctorAccess`. It is built once per run from `resolve_observability_runtime_config(repository_root).access_policy`; a failed resolution becomes `invalid_config` with no login check. Under effective `mandatory` it calls `auth_command::repository_telemetry_authorized`, the same non-interactive, deadline-bounded, fail-closed lookup setup uses. That lookup checks `auth.db` exists before opening it, so doctor never creates it. Other modes skip the lookup. A missing login adds a `SceLoginRequired` problem (`Error`, `ManualOnly`, `global_state`, next action `auth_login`). Its summary is `auth.mode: mandatory is set in the <config file>, but no valid SCE login was found.` and its remediation is `Run 'sce auth login'.`. The config file labels are `repository SCE config (`.sce/config.json`)`, `global SCE config`, `SCE config file passed with `--config``, and `SCE config file named by `SCE_CONFIG_FILE``. The new Environment row reads `SCE login` (`[FAIL]` with details, or `[PASS]`), or `SCE login (not required)`. The managed-telemetry row label is now `Managed telemetry: enabled for this repository` or `Managed telemetry: disabled (reason: <reason>)`, with ` (not configured)` added when telemetry is enabled but no credential is stored. A `Source: <layer>` detail shows on every row, including pass rows. The source is the deciding `telemetry.enabled` layer, the `auth.mode` source for `auth_disabled`, and none for `invalid_config`. The existing credential warnings are unchanged. `OPT_IN_NOTE` now says `... set telemetry.enabled: true in the repository SCE config to enable it for this repository.` and shows only for `not_enabled` when an auth DB exists. JSON adds a top-level `sce_login` {`state`: `not_required`|`logged_in`|`not_logged_in`, `auth_mode`, `source`, `config_source`} and `managed_telemetry.policy` {`state`, `reason`, `source`, `config_source`}. Existing keys are unchanged. `HealthProblemKind::SceLoginRequired` was added so the doctor/lifecycle kind mappings stay exhaustive. To keep `render_report_json` under clippy's line limit, the `post_commit_auto_sync` JSON moved into a helper with no output change. Approved assumptions: I chose the row labels, source wording, and JSON field names; the plan did not fix them. Added 6 render tests with exact text and JSON assertions: mandatory without login, mandatory with login, non-mandatory skipping the lookup (the lookup closure panics if called), repository consent enabled, `not_enabled` with the opt-in note, and user opt-out without the note.
  - Verify outcome: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml doctor` passed (27 passed). Extra checks: `clippy --all-targets --all-features -- -D warnings` was clean, and `cargo fmt` was applied. I ran the built `sce doctor` in a temp git repository with `.sce/config.json` `{"auth":{"mode":"mandatory"}}` and isolated `HOME` / `XDG_CONFIG_HOME` / `XDG_STATE_HOME`. It printed `[FAIL] SCE login` with the exact problem and remediation. It printed `[PASS] Managed telemetry: disabled (reason: not_enabled)` / `Source: default`. JSON had `sce_login.state` `not_logged_in` with `config_source` `default_discovered_local`. No `auth*` file was created under the isolated directories.
  - Context impact: user-visible behavior change to `sce doctor` text and JSON: a new `SCE login` row and blocking problem, new managed-telemetry row labels and the `Source:` detail, new `OPT_IN_NOTE` text, and new `sce_login` / `managed_telemetry.policy` JSON objects. Affects `context/sce/telemetry-credential.md` (doctor health check and opt-in note), the doctor entries in `context/glossary.md`, and any doctor output description in `context/cli/cli-command-surface.md`.
  - Context synchronization: synced

- [x] T05: `Print a login reminder in non-interactive setup under mandatory` (status:done)
  - Task ID: T05
  - Scope: In — non-interactive and target-flag setup add the fixed `SCE login: required by auth.mode: mandatory for this repository; run 'sce auth login' if you are not logged in.` line when the effective mode is `mandatory`; setup tests. Out — running login, credential lookup, exit-code changes, hooks-only and context-only modes.
  - Dependencies: T03
  - Done when: AC7 holds; non-`mandatory` runs and interactive runs print no new line.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup`.
  - Completed: 2026-10-22
  - Files changed: `cli/src/services/setup/command.rs`
  - Result: Added `SETUP_LOGIN_REMINDER_LINE` (`SCE login: required by auth.mode: mandatory for this repository; run 'sce auth login' if you are not logged in.`) and `non_interactive_login_reminder`. That function resolves the access policy for the repository root and returns the line only when the effective `auth.mode` is `mandatory`. A failed policy resolution returns no line, so setup never fails or changes its exit code because of the reminder. A new `non_interactive_followup` helper replaces the inline non-interactive `else` branch of `SetupCommand::execute`. It pushes the reminder as its own section after the target install output and before the Claude telemetry section, then returns the existing non-interactive Claude telemetry action. The helper exists only to keep `execute` under clippy's line limit. Only non-interactive and target-flag setup reach that branch. Interactive runs keep their login step, and hooks-only and context-only runs never reach it. The reminder only resolves config: no `auth.db` open, no credential lookup. Approved assumptions: the reminder's placement, and no reminder when resolution fails. Added 2 tests. One covers repository, global, and explicit `mandatory`, which print the exact line, and repository `optional`, `disabled`, and the default, which print nothing; it also asserts no `auth*` file is created under the fixture directory. The other covers resolution failure, which prints nothing.
  - Verify outcome: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup` passed (75 passed). Extra checks: `clippy --all-targets --all-features -- -D warnings` was clean, and `cargo fmt` was applied. The built `sce setup --opencode` was run in a temp git repository with a remote and isolated `HOME` / `XDG_CONFIG_HOME` / `XDG_STATE_HOME`. With `.sce/config.json` `{"auth":{"mode":"mandatory"}}` it exited 0 and printed the exact reminder line. With `optional` it exited 0 and printed no `SCE login` line. No `auth*` file was created under the isolated directories.
  - Context impact: user-visible output addition to non-interactive setup: a fixed reminder line under effective `mandatory`. Affects the non-interactive reminder part of `context/sce/setup-repo-local-config-bootstrap.md` and the Setup / Enforcement part of `context/cli/access-policy.md`.
  - Context synchronization: synced

- [x] T06: `Notify Claude sessions when an SCE login is required` (status:done)
  - Task ID: T06
  - Scope: In — at Claude `SessionStart`, resolve the policy for the session working directory and, under `mandatory` without a valid stored credential, emit one `systemMessage` `SCE login required for this repository. Run 'sce auth login'.`; Pkl hook wiring if a new hook entry is needed; hook tests. Out — OpenCode/Pi/Codex notices, blocking the session, repeated notices within a session.
  - Dependencies: T04
  - Done when: AC8 holds; the hook exits successfully in every case, including policy-resolution and credential-lookup failures.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude`; `nix run .#pkl-check-generated` when Pkl changes.
  - Completed: 2026-10-22
  - Files changed: `cli/src/services/hooks/claude_model_state.rs`
  - Result: The notice is added to the existing `sce hooks claude-model-state` handler, which the generated Claude settings already run at `SessionStart`. No new hook entry or Pkl change was needed. A new `session_login_notice` helper takes injected policy and credential seams. It returns nothing unless the payload's `hook_event_name` is `SessionStart`. It then resolves the access policy for the hook working directory through `resolve_auth_runtime_config` and, only under effective `mandatory`, runs `auth_command::repository_telemetry_authorized`. That is the same non-interactive, deadline-bounded, fail-closed lookup doctor uses, and it never creates `auth.db`. With no valid credential it returns `{"systemMessage":"SCE login required for this repository. Run 'sce auth login'."}` (constant `SESSION_LOGIN_REQUIRED_MESSAGE`). `run_claude_model_state_subcommand` computes the notice right after reading stdin and before the clock and model-state persistence. It returns the notice as the hook output, so a model-state failure never suppresses it. A failed policy resolution returns nothing, and model-state behavior is unchanged. The hook still always returns successfully. It fires once per `SessionStart` event. Approved assumption: extend `claude-model-state` rather than add a new hook entry. Added 5 inline tests: mandatory without login emits exactly the one-key `systemMessage`; mandatory with login emits nothing; `optional` / `disabled` emit nothing and never run the lookup; non-`SessionStart`, eventless, and non-JSON payloads never resolve the policy; a resolution failure emits nothing.
  - Verify outcome: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude` passed (43 passed; the 5 new tests ran under the `claude_model_state` filter, 7 passed). `nix run .#pkl-check-generated` was not needed because no Pkl changed. Extra checks: `clippy --all-targets --all-features -- -D warnings` was clean, and `cargo fmt` was applied. I ran the built `sce hooks claude-model-state` in a temp git repository with isolated `HOME` / `XDG_CONFIG_HOME` / `XDG_STATE_HOME`. A `SessionStart` under repository `mandatory` printed exactly the `systemMessage` JSON and exited 0. `PostModelSwitch` under `mandatory`, `SessionStart` under `optional`, and `SessionStart` with an invalid `.sce/config.json` printed nothing to stdout and exited 0. The existing invalid-config startup diagnostic still goes to stderr. No `auth*` file was created.
  - Context impact: user-visible addition. At Claude `SessionStart`, `sce hooks claude-model-state` may now emit a `systemMessage` login notice under effective `mandatory` without a valid stored credential. Affects the Enforcement part of `context/cli/access-policy.md` and any description of `claude-model-state` hook output (for example the Claude hook entries in `context/glossary.md` or `context/sce/` hook docs).
  - Context synchronization: synced

- [x] T07: `Record repository-scoped access answers and sync context` (status:done)
  - Task ID: T07
  - Scope: In — new ADR under `context/decisions/` superseding `2026-10-21-setup-writes-access-answers-to-user-layer-only.md` and the user-side-only consent rule of `2026-10-20-consent-gated-access-policy-precedence.md` (supersession notes added to both), covering the lock change, the committed-file consent risk, and the older-binary risk; update the context files listed under Context sync. Out — code changes.
  - Dependencies: T03, T04, T05, T06
  - Done when: AC9 holds.
  - Verify: `nix shell nixpkgs#ripgrep -c rg -n "applies to all repositories on this machine|for this machine|no_user_consent" context cli/src`.
  - Completed: 2026-10-22
  - Files changed: `context/decisions/2026-10-20-consent-gated-access-policy-precedence.md`, `context/decisions/2026-10-21-setup-writes-access-answers-to-user-layer-only.md`
  - Result: The replacing ADRs were already recorded during the T02 and T03 context syncs. `2026-10-22-setup-writes-access-answers-to-repository-config.md` covers the lock change, the committed-file consent risk, the older-binary risk, and the T03 Claude-removal deviation. `2026-10-22-repository-telemetry-enabled-grants-consent.md` covers the consent rule. Approved assumption: those two ADRs together serve as the plan's new ADR, so no third ADR was written. T07 added the supersession notes. The 2026-10-21 ADR is now `Status: Superseded`, with a `Superseded by:` line and a note pointing to the repository-config ADR. The 2026-10-20 ADR is now `Status: Accepted (partially superseded)`, with a `Partially superseded by:` line limited to the user-side-only consent rule, and a note saying which parts still apply. The Context sync files (`access-policy.md`, `setup-repo-local-config-bootstrap.md`, `telemetry-credential.md`, `glossary.md`, `context-map.md`) already describe repository-scoped answers, any-layer consent, the doctor `SCE login` row, the non-interactive reminder, and the Claude session notice, so none of them changed. Follow-up outside this task's no-code scope: the login provisioning message `TELEMETRY_PROVISIONING_DISABLED` (`cli/src/services/auth_command/mod.rs`) still says `Set telemetry.enabled: true in the global SCE config to enable it.`. `context/cli/access-policy.md` quotes it exactly.
  - Verify outcome: the planned `rg` check ran. Matches remain only in: completed plans (`auth-telemetry-policy-config`, `setup-auth-telemetry-consent`, `claude-telemetry-consent-reconciliation`); this active plan's own description of the change; the superseded 2026-10-21 ADR; the partially superseded 2026-10-20 ADR; the 2026-10-22 consent ADR and its `context-map.md` index entry, both of which record the removal of `no_user_consent`. There are no matches in `cli/src` or in any current-state context file. AC9 holds.
  - Context impact: decision records only. The two older ADRs now carry supersession status and notes. No current-state context or code behavior changed.
  - Context synchronization: synced

## Open questions

- Storing `telemetry.enabled: true` in the committed `.sce/config.json` means anyone who clones or pulls the repository is opted in without being asked. The 2026-10-20 decision existed to prevent exactly that. The plan does what you asked. A smaller alternative keeps the per-repository scope and per-person consent: save the answer to a gitignored per-user file such as `.sce/config.local.json`.
- Claude-native telemetry settings stay in the user-level `~/.claude/settings.json`, so Claude Code is set up to export in every repository. In repositories without consent the headers helper fails closed, so export there has no SCE credential. Claude may still try to send to the OTLP endpoint. Moving those settings into the repository's Claude settings is out of scope here.
- Older released binaries (`v0.4.0-pre-alpha-v11` and earlier) reject the `auth` and `telemetry` keys. A teammate on one of them has the whole repository config skipped as invalid, including bash policies and integration targets. The plan records this as a release-coupling risk in the ADR. The safer alternative is to hold the repository write until a release that accepts the keys has shipped.
- Should a team-committed repository value (especially `auth.mode: disabled`) still lock the setup prompts? The plan lets `sce setup --reconfigure` change any repository value.

## Validation Report

**Status:** validated  
**Date:** 2026-10-22

### Commands run

- `nix flake check` -> exit 0 (all 3 checks passed: `cli-tests`, `cli-clippy`, `cli-fmt`)
- `nix run .#pkl-check-generated` -> exit 0 (ephemeral Pkl generation passed: 145 files)
- `nix shell nixpkgs#ripgrep -c rg -n "applies to all repositories on this machine|for this machine|no_user_consent" context cli/src` -> exit 0 (matches only in completed plans, this plan, the superseded/partially superseded ADRs, and the 2026-10-22 consent ADR plus its `context-map.md` index entry recording the removal; none in `cli/src`)
- `nix build .#default` then built `sce config show --format json` in a temp repo with only repository `telemetry.enabled: true` (isolated `HOME`/`XDG_*`) -> exit 0 (`effective.state` `enabled`, `enabled.config_source` `default_discovered_local`, `note` null)
- Built `sce doctor` (text and `--format json`) in a temp repo with repository `auth.mode: mandatory` and no login -> exit 0 (`[FAIL] SCE login` with `auth.mode: mandatory is set in the repository SCE config (`.sce/config.json`), but no valid SCE login was found.` / `Run 'sce auth login'.`; JSON `sce_login.state` `not_logged_in`; no `auth*` file created)
- Built `sce doctor` in the repository-consent temp repo -> exit 0 (`[PASS] Managed telemetry: enabled for this repository (not configured)`, `Source: repository SCE config (`.sce/config.json`)`)

### Success-criteria verification

- [x] AC1: Interactive setup writes access answers to the repository `.sce/config.json` only, keeping unrelated keys -> setup command tests pass inside `nix flake check` `cli-tests`
- [x] AC2: Repository-saved answers suppress prompts, `--reconfigure` defaults to them, global values no longer suppress -> `FakePrompter` dispatch tests pass in `cli-tests`
- [x] AC3: Repository writer refuses invalid, concurrently modified, or symlinked files byte-identically; setup aborts before target install -> writer unit tests and symlink abort test pass in `cli-tests`
- [x] AC4: Global/explicit `disabled` skips prompts, global/explicit `mandatory` suppresses login offer; repository values are defaults only -> `SetupAccessGate::from_policy` per-layer tests pass in `cli-tests`
- [x] AC5: Repository `true` alone enables telemetry, any `false` disables, `no_user_consent` gone -> resolver tests pass; built `sce config show --format json` shows `enabled`; no `no_user_consent` in `cli/src`
- [x] AC6: `sce doctor` reports blocking `SCE login` failure with fix and shows repository telemetry state and source -> doctor render tests pass; built `sce doctor` text/JSON confirmed in both temp repos
- [x] AC7: Non-interactive setup under `mandatory` prints the fixed reminder, exits 0, no credential-store access -> non-interactive setup tests pass in `cli-tests`
- [x] AC8: Claude `SessionStart` under `mandatory` without login emits exactly one `systemMessage`, nothing otherwise, always succeeds -> `claude_model_state` hook tests pass in `cli-tests`; no Pkl change, `pkl-check-generated` passed
- [x] AC9: New ADRs supersede the 2026-10-21 decision and the user-side-only consent rule; durable context updated -> inspected `context/decisions/` (2026-10-21 `Superseded`, 2026-10-20 `partially superseded`, two 2026-10-22 ADRs); remaining `rg` matches are historical records of the change only

### Failed checks and follow-ups

- None.

### Residual risks

- `TELEMETRY_PROVISIONING_DISABLED` in `cli/src/services/auth_command/mod.rs` still says `Set telemetry.enabled: true in the global SCE config to enable it.` (recorded T07 follow-up, outside this plan's wording changes).
- Committed repository `telemetry.enabled: true` opts in every cloner without asking, and older binaries (`v0.4.0-pre-alpha-v11` and earlier) reject the `auth`/`telemetry` keys; both are recorded open questions/ADR risks.
- Claude-native telemetry removal still reads only global preferences, so a repository `telemetry.enabled: false` answer gives `SkipDisabled` rather than `Remove` (recorded T03 deviation).
- In a bare temp repo, `sce doctor` repeats the same state/repository-identity problems under several Environment rows; this is outside the plan's scope and was not investigated here.
