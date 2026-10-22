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

- [ ] AC1: Interactive setup writes the answered `auth.mode` and `telemetry.enabled` to the repository `.sce/config.json`, writes no access keys to the global config, and keeps unrelated repository keys.
  - Validate: setup command tests in `cli/src/services/setup/command.rs`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup`.
- [ ] AC2: A rerun suppresses each access prompt when the repository file holds its value, `--reconfigure` asks again with that value as the default, and a global value no longer suppresses a prompt.
  - Validate: `resolve_setup_dispatch` / `prepare_interactive_setup` tests with `FakePrompter` for the repo-saved, global-only, and `--reconfigure` cases.
- [ ] AC3: The repository writer refuses an invalid, concurrently modified, or symlinked file and leaves it byte-identical, and setup then aborts before the target install.
  - Validate: repository writer unit tests plus one setup command abort test.
- [ ] AC4: Global or explicit `disabled` skips the access prompts and global or explicit `mandatory` suppresses the login offer, while repository values are only prompt defaults.
  - Validate: `SetupAccessGate::from_policy` and dispatch tests per layer.
- [ ] AC5: A repository `telemetry.enabled: true` with no `false` in any layer gives effective telemetry `enabled`. Any `false` still disables it, and `no_user_consent` no longer exists.
  - Validate: `resolve_access_policy` tests in `cli/src/services/config/resolver.rs`; `sce config show --format json` shows `result.resolved.telemetry.effective.state` `enabled` in a repository with only a repository `true`.
- [ ] AC6: Under `mandatory` with no valid stored credential, `sce doctor` reports a blocking `SCE login` failure with the `sce auth login` fix. It also shows the repository's effective telemetry state and its source.
  - Validate: doctor render tests with exact text and JSON assertions; `nix run .#sce -- doctor` in a repository with `auth.mode: mandatory` and no login.
- [ ] AC7: Non-interactive setup under effective `mandatory` prints the fixed login reminder line, exits 0, and never touches `auth.db` or the credential store.
  - Validate: setup command test for `SetupMode::NonInteractive` under `mandatory` asserting the exact line and no credential-store access.
- [ ] AC8: A Claude `SessionStart` under `mandatory` without a valid stored credential emits exactly one `systemMessage` with `SCE login required for this repository. Run 'sce auth login'.`, and emits nothing when logged in or not `mandatory`. The hook always succeeds.
  - Validate: hook tests for each case; `nix run .#pkl-check-generated` if the generated Claude settings change.
- [ ] AC9: A new ADR supersedes the 2026-10-21 decision and the user-side-only consent rule of the 2026-10-20 decision, and durable context no longer says setup writes access keys globally or that only the user side grants consent.
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

- [ ] T02: `Let repository telemetry.enabled grant consent` (status:todo)
  - Task ID: T02
  - Scope: In — `resolve_access_policy` telemetry state (any `false` disables, otherwise any `true` enables), removal of `TelemetryDisabledReason::NoUserConsent` and its contradiction warning, the `sce config show` telemetry note text, resolver and render tests. Out — `auth.mode` precedence, setup, doctor.
  - Dependencies: none
  - Done when: AC5 holds; the remaining contradiction warnings and reason strings are unchanged.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml resolver`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml config`.
  - Context synchronization: pending

- [ ] T03: `Save and read setup access answers in the repository layer` (status:todo)
  - Task ID: T03
  - Scope: In — `prepare_interactive_setup` reads saved answers from the repository file and writes new ones through the T01 writer before the target install; remove setup's global access write; `SetupAccessGate::from_policy` locks on global/explicit instead of repository; the invalid repository file gates the access prompts; prompt labels, `SCE access:` saved lines, and write-failure text naming the repository file; setup tests. Out — login step behavior, non-interactive setup, doctor, hooks.
  - Dependencies: T01, T02
  - Done when: AC1–AC4 hold in setup command and dispatch tests, and the report shows repository scope for both answers.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup`.
  - Context synchronization: pending

- [ ] T04: `Report mandatory login and repository telemetry state in sce doctor` (status:todo)
  - Task ID: T04
  - Scope: In — a doctor `SCE login` check (blocking `[FAIL]` under effective `mandatory` without a valid stored credential, naming the deciding config file, fix `Run 'sce auth login'`); the managed-telemetry row and JSON show the repository's effective telemetry state and source; `OPT_IN_NOTE` points at the repository config; doctor render tests. Out — `sce doctor --fix` behavior, credential creation, other doctor rows.
  - Dependencies: T02
  - Done when: AC6 holds with exact text and JSON assertions; doctor still never creates `auth.db` or a credential-store key.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml doctor`.
  - Context synchronization: pending

- [ ] T05: `Print a login reminder in non-interactive setup under mandatory` (status:todo)
  - Task ID: T05
  - Scope: In — non-interactive and target-flag setup add the fixed `SCE login: required by auth.mode: mandatory for this repository; run 'sce auth login' if you are not logged in.` line when the effective mode is `mandatory`; setup tests. Out — running login, credential lookup, exit-code changes, hooks-only and context-only modes.
  - Dependencies: T03
  - Done when: AC7 holds; non-`mandatory` runs and interactive runs print no new line.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup`.
  - Context synchronization: pending

- [ ] T06: `Notify Claude sessions when an SCE login is required` (status:todo)
  - Task ID: T06
  - Scope: In — at Claude `SessionStart`, resolve the policy for the session working directory and, under `mandatory` without a valid stored credential, emit one `systemMessage` `SCE login required for this repository. Run 'sce auth login'.`; Pkl hook wiring if a new hook entry is needed; hook tests. Out — OpenCode/Pi/Codex notices, blocking the session, repeated notices within a session.
  - Dependencies: T04
  - Done when: AC8 holds; the hook exits successfully in every case, including policy-resolution and credential-lookup failures.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude`; `nix run .#pkl-check-generated` when Pkl changes.
  - Context synchronization: pending

- [ ] T07: `Record repository-scoped access answers and sync context` (status:todo)
  - Task ID: T07
  - Scope: In — new ADR under `context/decisions/` superseding `2026-10-21-setup-writes-access-answers-to-user-layer-only.md` and the user-side-only consent rule of `2026-10-20-consent-gated-access-policy-precedence.md` (supersession notes added to both), covering the lock change, the committed-file consent risk, and the older-binary risk; update the context files listed under Context sync. Out — code changes.
  - Dependencies: T03, T04, T05, T06
  - Done when: AC9 holds.
  - Verify: `nix shell nixpkgs#ripgrep -c rg -n "applies to all repositories on this machine|for this machine|no_user_consent" context cli/src`.
  - Context synchronization: pending

## Open questions

- Storing `telemetry.enabled: true` in the committed `.sce/config.json` means anyone who clones or pulls the repository is opted in without being asked. The 2026-10-20 decision existed to prevent exactly that. The plan does what you asked. A smaller alternative keeps the per-repository scope and per-person consent: save the answer to a gitignored per-user file such as `.sce/config.local.json`.
- Claude-native telemetry settings stay in the user-level `~/.claude/settings.json`, so Claude Code is set up to export in every repository. In repositories without consent the headers helper fails closed, so export there has no SCE credential. Claude may still try to send to the OTLP endpoint. Moving those settings into the repository's Claude settings is out of scope here.
- Older released binaries (`v0.4.0-pre-alpha-v11` and earlier) reject the `auth` and `telemetry` keys. A teammate on one of them has the whole repository config skipped as invalid, including bash policies and integration targets. The plan records this as a release-coupling risk in the ADR. The safer alternative is to hold the repository write until a release that accepts the keys has shipped.
- Should a team-committed repository value (especially `auth.mode: disabled`) still lock the setup prompts? The plan lets `sce setup --reconfigure` change any repository value.
