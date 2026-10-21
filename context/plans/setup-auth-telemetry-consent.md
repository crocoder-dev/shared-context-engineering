# Plan: setup-auth-telemetry-consent

## Change summary

Interactive `sce setup` gains three steps: it asks for the auth mode (`disabled | optional | mandatory`, default `optional`), it asks for explicit SCE telemetry consent (default No), and it logs the user in. Login is offered under `optional`, required under `mandatory`, and skipped under `disabled`. Both answers are saved to the **global user config** (the user layer of the access policy) by an atomic writer with optimistic concurrency. They are never written to the repository's `.sce/config.json`. Login reuses the existing `sce auth login` path. It runs after the answers are saved, so telemetry credential provisioning sees the new consent. On reruns, saved answers suppress the prompts. A new interactive-only `sce setup --reconfigure` flag asks every saved setup answer again, with the saved value as the default. Non-interactive, hooks-only, and context-only setup keep their current behavior: no new prompts, no access-policy writes, no login, and no credential-store access.

This extends the existing setup prompt seam (`SetupTargetPrompter` → `resolve_setup_dispatch` → `SetupCommand::execute`) and the access-policy work from `auth-telemetry-policy-config` and `remove-telemetry-readiness-marker`. It does not touch the resolver's precedence. It completes the "future interface" that `auth-telemetry-policy-config` recorded: a setup step writes `auth.mode` and `telemetry.enabled` into the global config, and `resolve_access_policy` stays unchanged. It also addresses that plan's open question about existing logged-in users silently losing managed export.

## Design

### Problem

Since `auth-telemetry-policy-config`, managed SCE telemetry is off unless the user hand-edits `telemetry.enabled: true` into `${config_root}/sce/config.json`. `auth.mode` can also only be set by editing JSON. Setup asks about Agent Trace auto-sync ("Requires an SCE account") but never offers login. `mandatory` therefore has no flow that actually produces an authenticated install. The Claude-native telemetry stage tells users to "run 'sce auth login'" even when they are already in an interactive session that could do it.

### Current implementation findings

- **CLI parsing.** `cli/src/cli_schema.rs` `Commands::Setup` has these flags: `--opencode|--claude|--pi|--codex|--all` (mutually exclusive), `--non-interactive`, `--hooks`, `--repo` (requires `--hooks`), `--bootstrap-context`, and repeatable `--workflow`. `cli/src/services/parse/command_runtime.rs` maps them into `SetupCliOptions` and calls `convert_setup_command`.
- **`SetupCliOptions` → `SetupRequest`** (`cli/src/services/setup/mod.rs` `resolve_setup_request`). It produces one of three shapes:
  - `config_mode: Some(SetupMode::Interactive)`: no target and no `--hooks`. `install_hooks` is forced true.
  - `Some(SetupMode::NonInteractive(target))`: a target flag was given.
  - `None`: hooks-only (`--hooks`) or `context_only` (`--bootstrap-context`, which must be used alone).
  - Interactive mode always runs from the current working directory, because `--repo` requires `--hooks`.
- **Prompt seam.** `trait SetupTargetPrompter` has four methods: `prompt_targets`, `prompt_optional_workflows`, `prompt_agent_trace_auto_sync`, and `prompt_attribution_hooks_enabled`. `InquireSetupTargetPrompter` implements it by delegating to `cli/src/services/setup/prompt.rs`, which uses `inquire` `MultiSelect`/`Confirm` with a Yes default. Cancel maps to `Ok(None)`; `NotTTY` maps to a fixed `bail!`. This is the only prompt seam. The test fake `FakePrompter` lives in `setup/command.rs` tests.
- **`resolve_setup_dispatch(mode, prompter, optional_workflow_defaults, configured_targets, PersistedSetupConfirmations)`.**
  - Interactive mode prompts for targets, then workflows. For each of `agent_trace_auto_sync` and `attribution_hooks_enabled`, it reuses the persisted value instead of prompting when one exists. Any `None` answer returns `SetupDispatch::Cancelled`.
  - Non-interactive mode returns `Proceed` with `TargetPersistence::Merge` and no behavior values.
  - Persisted answers can never be changed through setup today. The only way to change them is to edit JSON.
- **`PersistedSetupConfirmations`** (`Copy`, `Default`) is read by `persisted_setup_confirmations(repository_root)` from the repository-local file only, through `persisted_file_config`. That function returns `None` for a missing or invalid file.
- **`SetupCommand::execute`** (`setup/command.rs`) runs these steps in order:
  1. `resolve_setup_repository`: Git gate, plus a remote preflight through `resolve_agent_trace_storage_runtime_config`. An unreadable global config or an invalid explicit config already fails here.
  2. Dispatch, with all prompts. Cancellation returns "Setup cancelled. No files were changed."
  3. `bootstrap_context_baseline`.
  4. The lifecycle providers Config → LocalDb → AuthDb → AgentTraceDb → [Hooks]. `AuthDbLifecycle::setup` only creates the auth DB parent directory; it never opens `auth.db` or resolves the Keychain key.
  5. `run_setup_for_targets`: asset install, then `persist_integration_targets`.
  6. When Claude is selected, `claude_telemetry_report`. This resolves policy through `config::resolve_auth_runtime_config(repository_root)`, installs only when telemetry is `Enabled`, and always prints `AUTH_PENDING_LINE` after a fresh configure.
- **`persist_integration_targets`.**
  - It skips an invalid repository-local file and returns `Ok(())`.
  - It bootstraps the file when it is missing.
  - It round-trips the file through `serde_json::Value`, which does not preserve key order because the `preserve_order` feature is off.
  - It writes with a non-atomic `fs::write`.
- **`cli/src/services/setup/claude_telemetry.rs`.** `write_atomically` writes a sibling temp file, then renames it, preserving the existing file mode or using `0o600` for a new file. This is the existing safe-write utility to reuse.
- **Access policy** (`cli/src/services/config/resolver.rs`).
  - `resolve_runtime_config_with` records the per-layer `AccessPolicyLayer` and then calls the pure `resolve_access_policy(user, repository, explicit, degraded)`.
  - The global path comes from the private `resolve_default_global_config_path()`.
  - `ResolvedAccessPolicy` (`config/types.rs`) exposes `auth_mode: ResolvedValue<AuthMode>` with its deciding source, `telemetry: TelemetryPolicyState`, `telemetry_enabled()`, `auth_allowed()`, and `auth_disabled_config_file()`.
  - `AuthMode` and its `as_str`/`parse` are `pub(crate)`.
  - `schema::parse_file_config(raw, path, ConfigPathSource)` yields `FileConfig.auth_mode` / `telemetry_enabled` as `Option<FileConfigValue<_>>`, so "key absent" and "explicit value" are already distinguishable.
- **Login** (`cli/src/services/auth_command/mod.rs`).
  - `run_login(format)` resolves policy from `current_dir` and runs `ensure_auth_allowed`. It loads stored tokens with `CredentialAccess::Interactive`, which opens or creates `auth.db` and resolves the Keychain key.
  - If the stored tokens renew, it provisions (only when telemetry is `Enabled`) and renders the refresh result. Otherwise it runs the WorkOS device flow (`write_login_prompt` prints straight to stdout), saves the tokens, and provisions.
  - Provisioning calls `enroll_current_repository_if_unbound()`, which reads `current_dir` again.
  - It returns a rendered `String`, so the `TelemetryProvisioningResult` is not visible to callers.
  - The text output contains no tokens.
- **Credential storage.** Only `auth.db` is encrypted (`EncryptedTursoDb<AuthDbSpec>`). `managed_telemetry::auth_db_present()` is a file-existence check that never touches the Keychain.
- **`mandatory` enforcement today.** The only effect is that an unauthenticated `sce auth whoami` fails with `auth.not_authenticated`. Hooks, auto-sync, and startup never block or log in. Setup is not admitted to managed telemetry startup (`app.rs` returns no span kind for `setup`).
- **Errors** (`cli/src/services/error.rs`). `UserError::{NotAuthenticated, AuthStorageUnavailable, LoginAuthStorageUnavailable, AuthDisabledByConfig(AuthPolicyConfigFile), UnexpectedFailure}` have fixed keys and messages. Setup maps most failures to `UnexpectedFailure` with a source.
- **No readiness marker remains** (`remove-telemetry-readiness-marker` is validated). This plan must not reintroduce one.

### Interactive prompt sequence

The existing prompts keep their position and wording. The new prompts come after them, and every prompt is answered before any file is written.

1. `Select setup targets` (existing `MultiSelect`).
2. `Select optional workflows` (existing).
3. `Automatically sync Agent Traces?` (existing; asked only when no value is persisted, unless `--reconfigure`).
4. `Record SCE involvement in Git commits?` (existing; same rule).
5. **New `Select` prompt, `SCE account mode for this machine`.** It is asked when no global `auth.mode` is persisted, unless `--reconfigure`, and is skipped by the gates below. The cursor starts on `optional` on a first run, or on the persisted value with `--reconfigure`. The rows are, in this order:
   - `Optional: use SCE locally and log in when you want SCE cloud features`
   - `Mandatory: require an SCE login when setting up`
   - `Disabled: local only; no SCE login, cloud sync, or telemetry`
6. **New `Confirm` prompt, telemetry consent.** It is asked only when the selected or persisted mode is not `disabled` and no global `telemetry.enabled` is persisted, unless `--reconfigure`. The default is No on a first run, or the persisted value with `--reconfigure`. Label:
   ```
   Share SCE telemetry?
   (Sends SCE usage telemetry to SCE servers using your SCE login. If Claude is
   selected, also configures Claude Code's native telemetry export. Separate from
   Agent Trace sync. Applies to all repositories on this machine.)
   ```
7. **New `Confirm` prompt, login offer.** It is asked only when the prospective mode is `optional` (selected, persisted, or effective) and one of these holds: prompts 5 or 6 were asked in this run, or `auth_db_present()` is false. The default is Yes. Label:
   ```
   Log in to SCE now?
   (Opens a device login in your browser. An existing login on this machine is
   reused. You can log in later with 'sce auth login'.)
   ```

Gates on prompts 5–7. They are computed from data that is already loaded, with no credential access:

- **Effective `auth.mode` is `disabled` with a repository or explicit deciding source** (`auth_disabled_config_file()` is `Repository`, `ConfigFlag`, or `ConfigEnv`): prompts 5–7 are skipped. Setup cannot lift that restriction. The report names the deciding file through the existing `auth_disabled_by_config_message` wording.
- **The global config exists but is invalid:** prompts 5 and 6 are skipped, because the answers could not be saved. Prompt 7 follows the normal rule.
- **The effective mode is already `mandatory`** (any layer), **or prompt 5 selected `mandatory`:** prompt 7 is skipped, because login will be required.
- **Prompt 5 selected `disabled`:** prompts 6 and 7 are skipped. The global `telemetry.enabled` value is left untouched, and it stays ineffective because policy reports `auth_disabled`.

Cancelling at any prompt (Esc or Ctrl-C) returns `SetupDispatch::Cancelled` before any write, so the existing "Setup cancelled. No files were changed." stays true.

The telemetry prompt never takes its default from: the Claude target, an existing `auth.db`, `agent_trace.auto_sync`, an old telemetry credential, or existing Claude settings. Its only inputs are the persisted global value and the first-run default `false`.

### Configuration ownership and persistence

| Choice | Written to | Never written to | Rationale |
|---|---|---|---|
| `auth.mode` | Global user config `${config_root}/sce/config.json` (`resolve_default_global_config_path`) | Repository `.sce/config.json`, explicit `--config` / `SCE_CONFIG_FILE` files | Credentials (`auth.db`, Keychain) are per-user per-machine. The repository file is committed and shared, so writing a restriction there would be team-policy authoring, and older binaries skip the whole file when it holds keys they reject. This matches the `auth-telemetry-policy-config` future interface. |
| `telemetry.enabled` | Global user config | Repository and explicit files | Only the user side can grant consent. A repository `true` grants nothing, so setup must never produce one. |
| Agent Trace auto-sync, attribution | Repository config (unchanged) | n/a | Existing behavior. |

- **The resolver's precedence is not changed.** The writer changes only the user layer, so the following still hold:
  - A repository or explicit `auth.mode: disabled` still wins (`disabled > mandatory > optional`).
  - A repository or explicit `telemetry.enabled: false` still restricts telemetry.
  - Global `true` in a repository that opted out stays `repository_opt_out`.
- **Setup never lifts a global prohibition silently.** A persisted global `disabled` suppresses prompt 5 on reruns. It changes only through `--reconfigure`, which shows the current value as the default. Any write to global `auth.mode` is reported as `auth.mode: <mode> (saved to global SCE config '<path>'; applies to all repositories on this machine)`.
- **The writer records explicit choices only.** Declining telemetry writes `telemetry.enabled: false` and accepting writes `true`. Choosing a mode writes `auth.mode`, including an explicit `optional`. A key that is absent stays absent until the user answers.
  - **States distinguished at read time** (`GlobalAccessPreferences`): global file missing, key absent (meaning "not asked yet"), explicit `false`, explicit `true`, and global file invalid (degraded: no prompt, no write).
  - **States read from the resolved policy:** repository restriction (`RepositoryOptOut`, repository-sourced `AuthDisabled`), auth-disabled effective state, and degraded config (`InvalidConfig`).
  - The report shows the effective state with its reason, never the saved value alone.
- **Safe write algorithm.** `persist_global_access_preferences` reuses the setup safe-write helper:
  1. Read the file's bytes. A missing file starts as `{"$schema": <sce_config_schema_url()>}`. An unreadable file is an error.
  2. Parse with `parse_file_config(.., DefaultDiscoveredGlobal)`. On failure, refuse to write and return a typed `InvalidGlobalConfig` error. The invalid file is never overwritten.
  3. Mutate only `auth.mode` and/or `telemetry.enabled` in the `serde_json::Value`, preserving every other key and nested key.
  4. Serialize as pretty JSON plus a trailing newline, then validate the new bytes with `parse_file_config` before writing.
  5. If the bytes are unchanged, return `Unchanged` and do not write (idempotence).
  6. Re-read the file. If its bytes differ from step 1, return a `ConcurrentModification` error and do not write.
  7. Create the parent directory, then call `write_atomically`: a sibling temp file, the existing mode or `0o600`, then rename.
  - **Guarantees and limits:** the rename is atomic on one filesystem, and the step 6 check is best-effort optimistic concurrency, not a lock.
  - Schema unknown keys never reach step 3, because the schema rejects them at step 2.
  - Key order changes on the first rewrite, as with the existing repository writer.
- **Failure handling.** An invalid, unreadable, read-only, or concurrently modified global file fails setup before any repository write, with a fixed message naming the file and suggesting `sce config validate` and a rerun. Consent is never reported when the write did not succeed.

### Decision matrix

The "Effective" column is computed by re-resolving `resolve_auth_runtime_config(repository_root)` after the global write. "Login" is decided from the effective mode, not from the prompt answer.

| Selected mode | Telemetry answer | Global write | Effective (no other layers) | Login behavior | Telemetry provisioning |
|---|---|---|---|---|---|
| disabled | not asked | `auth.mode: disabled` (telemetry untouched) | auth disabled, telemetry `auth_disabled` | Never. No `auth.db` or Keychain access. If `auth.db` exists, one line hints `sce auth logout`. | none |
| disabled | (true not offerable) | n/a | n/a | n/a | the conflict cannot be entered, because prompt 6 is skipped |
| optional | false | `optional`, `false` | telemetry `user_opt_out` | Offered. Declining or failing is reported, and setup succeeds. | `disabled` |
| optional | true | `optional`, `true` | telemetry enabled | Offered. On success, provisioning runs inside login. | Issued only after successful login. If login is declined, consent stays saved and a later explicit `sce auth login` provisions. |
| mandatory | false | `mandatory`, `false` | telemetry `user_opt_out` | Required: renew stored credentials, otherwise the device flow. Failure is a nonzero setup exit. | `disabled` |
| mandatory | true | `mandatory`, `true` | telemetry enabled | Required. | Issued after successful login. |

Rows with other layers present. In each, the precedence comes from the resolver and is not recomputed by setup.

| Other layers | Setup behavior |
|---|---|
| Repository or explicit `auth.mode: disabled` | Prompts 5–7 skipped, no global write for access keys, no login. Report: auth disabled by `<file>`. |
| Repository `auth.mode: mandatory`, user answers `optional` | Global `optional` is written. Effective `mandatory` (repository), so login is required. |
| Repository `telemetry.enabled: false`, user answers Yes | Global `true` is written. Effective `repository_opt_out`. Login provisioning reports `disabled`. Report: `telemetry: disabled (reason: repository_opt_out)`. |
| Repository `telemetry.enabled: true`, user answers No | Global `false` is written. Effective `user_opt_out`. The repository cannot grant. |
| Global config invalid | Prompts 5–6 skipped. Effective telemetry `invalid_config`. Login follows the effective mode. |
| `SCE_CONFIG_FILE` with `telemetry.enabled: false` | Global consent is written. Effective `user_opt_out` (explicit). Reported as such. |

### Non-interactive and partial setup contract

| Invocation | New prompts | Access-key writes | Login / credential store | Unchanged behavior |
|---|---|---|---|---|
| `sce setup` (interactive) | Yes (per the gates) | Global only | Per the matrix | Targets, workflows, behavior prompts, hooks |
| `sce setup --reconfigure` | All persisted answers asked again, with persisted defaults | Global only | Per the matrix | Same as interactive |
| `sce setup --{opencode,claude,pi,codex,all} --non-interactive` (also without `--non-interactive`, which is the same mode) | None | None | None. Never prompts, never runs the device flow, never opens `auth.db` | Asset install, target merge, Claude telemetry gated by the existing effective policy |
| `sce setup --hooks [--repo]` | None | None | None | Hook install |
| `sce setup --bootstrap-context` | None | None | None | Context baseline only |

No new non-interactive consent or mode flags are added. Unattended consent must be granted by deliberately editing the global config, which already works. The new `--reconfigure` is rejected with a fixed validation message when combined with a target flag, `--non-interactive`, `--hooks`, `--repo`, or `--bootstrap-context`.

### Setup lifecycle order

1. Resolve the repository and run the preflights (existing).
2. Read persisted state:
   - repository confirmations, targets, and workflows (existing);
   - global access preferences (new, file read only);
   - the effective policy via `resolve_auth_runtime_config(repository_root)` (new, config only);
   - `auth_db_present()` (file existence only).
3. Dispatch: all prompts. Cancellation means no writes.
4. **Persist the global access preferences.** This is the first write. Failure aborts with nothing changed in the repository.
5. Context baseline (existing).
6. Lifecycle providers (existing; no credential access).
7. Target assets and repository config (existing).
8. Re-resolve the effective policy, then run the **login step**:
   - skip when not applicable;
   - under optional, run login if the user accepted;
   - under mandatory, run login.
   Login reads the policy written in step 4, so provisioning sees the new consent.
9. Claude-native telemetry stage (existing gate). The "authentication pending" line is shown unless step 8 returned `TelemetryProvisioningResult::Issued`.
10. Report: an `SCE access` section with the saved values (or "unchanged"), the effective auth mode and telemetry state with reason from step 8's resolution, and the login outcome from the actual login result.

Why this order:

- Prompts come before writes, which preserves the cancellation contract.
- The global write comes before repository writes, so a failed consent write leaves nothing half-done.
- Login comes after persistence, which provisioning requires.
- Login comes after asset installation, so a slow or failed device flow does not leave the repository uninstalled.
- The Claude stage comes after login, so its report reflects the real provisioning.

Atomicity and recovery:

- **Atomic:** the global config write (temp file plus rename).
- **Not transactional across steps:** there is no rollback.
- **Rerun-safe:** every step is resumable on rerun, because each step is idempotent. Saved answers suppress prompts, asset install is idempotent, and login renews or reuses credentials.

| Failure | Outcome |
|---|---|
| Cancel at any prompt | `Setup cancelled. No files were changed.` (exit 0, existing) |
| Optional login declined | Success. Line: `SCE login: skipped; run 'sce auth login' to log in.` |
| Optional login fails (WorkOS unavailable, device flow expired or denied, Keychain unavailable) | Success. Line: `SCE login: failed: <fixed UserError message>`, followed by the retry hint. |
| Mandatory login fails | Nonzero exit with new `UserError::SetupLoginRequired` (key `auth.setup_login_required`, runtime class). Fixed message: local setup completed; run `sce auth login`, then rerun `sce setup`. The underlying login error is kept as the source. |
| Global config write fails | Nonzero exit before any repository write, with a fixed message naming the file. |
| Repository config write fails | Existing behavior ("Setup assets were installed … but failed to update repo-local config"). The global answers stay saved, and a rerun does not ask them again. |
| Claude settings conflict | Existing skip lines (unchanged). |
| Already authenticated | Optional: offered only when answers were asked this run or `auth.db` is absent. Mandatory: renew without the device flow. |
| Stored credentials expired or unrenewable | `run_login` already falls back to the device flow. |

## Acceptance criteria

- [ ] AC1: On a first interactive run with no global access keys, setup asks the existing four prompts unchanged, then the auth-mode `Select` (cursor on `optional`), telemetry consent (default No), and, under `optional`, the login offer (default Yes). Cancelling any prompt writes no file and prints `Setup cancelled. No files were changed.`
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup::` dispatch tests using a recording `FakePrompter` that assert the call order, the defaults passed, and the cancellation result at each new prompt.
- [ ] AC2: Setup writes `auth.mode` and `telemetry.enabled` only to the global config. The repository `.sce/config.json` never gains `auth` or `telemetry` keys from setup. Unrelated global keys are preserved. A second identical run does not rewrite the file. Existing file mode is preserved, and a new file is `0o600`.
  - Validate: table-driven tests for `persist_global_access_preferences` (missing file, empty object, unrelated keys, explicit values, idempotent rerun, mode preservation) and an execute-level test asserting the bytes of the repository config.
- [ ] AC3: An invalid, unreadable, or concurrently modified global config is never overwritten. An invalid one causes the access prompts to be skipped. A write failure aborts setup before any repository write and never reports consent.
  - Validate: writer tests for invalid JSON, schema-invalid file, read-only directory, and a changed-bytes race (injected re-read), plus a dispatch test for the invalid-global gate.
- [ ] AC4: A repository or explicit `auth.mode: disabled` skips all access prompts and login. A repository `telemetry.enabled: false` keeps telemetry disabled after the user consents. A repository `telemetry.enabled: true` never enables telemetry without global consent. A repository `mandatory` forces the login to be required.
  - Validate: table-driven tests for the dispatch gate and the login-action decision built from `resolve_auth_runtime_config_with` over temp global, repository, and explicit files.
- [ ] AC5: The login step is skipped under effective `disabled` without calling any credential-store or login closure. Under `optional` it runs only after the user accepts, and declining or failing leaves setup successful. Under `mandatory` it always runs, and failure exits nonzero with `auth.setup_login_required`. Telemetry is provisioned only when the re-resolved policy is `Enabled`.
  - Validate: `setup_login_action` table test and a generic execute-step test with login closures that panic when called. Use the existing `auth_command` provisioning tests to cover `Disabled` versus `Issued`.
- [ ] AC6: On rerun, persisted global and repository answers suppress their prompts. `sce setup --reconfigure` asks all of them again, with the persisted values as defaults. `--reconfigure` combined with a target flag, `--non-interactive`, `--hooks`, `--repo`, or `--bootstrap-context` fails validation.
  - Validate: `resolve_setup_request` and `resolve_setup_dispatch` tests; a `cli_schema`/parse test for the flag.
- [ ] AC7: Non-interactive target runs, `--hooks`, and `--bootstrap-context` produce byte-identical output to today. They ask no new prompts, write no access keys, and never invoke login or credential storage.
  - Validate: the existing setup command tests stay green, plus a dispatch test asserting `Proceed` carries no access values and `SetupLoginIntent::NotOffered` for every `SetupMode::NonInteractive` target.
- [ ] AC8: The setup report states the effective auth mode, the effective telemetry state with its reason (from re-resolution, not from the saved value), and the actual login outcome, and contains no token or credential value. `sce auth login` output stays byte-identical.
  - Validate: exact-output render tests for the `SCE access` section; the existing `auth_command` render tests stay green; a test asserting the login report text contains no `access_token` / `refresh_token` values.
- [ ] AC9: The Claude-native telemetry report omits the "authentication pending" line when this setup run's login issued a telemetry credential, and keeps it otherwise.
  - Validate: `render_claude_telemetry_section` tests in `cli/src/services/setup/command.rs`.

### Full validation

- `nix flake check`
- `nix run .#pkl-check-generated` (only if schema or Pkl inputs change; none are planned)

### Context sync

- `context/cli/access-policy.md`: the "Setup does not enforce it yet" status; a setup enforcement bullet; setup as the user-layer writer.
- `context/cli/config-precedence-contract.md`: setup-written global access values, and `--reconfigure` on the setup-written behavior values.
- `context/sce/setup-repo-local-config-bootstrap.md`: prompt sequence, rerun suppression, `--reconfigure`.
- `context/cli/cli-command-surface.md`: `sce setup --reconfigure`, the login step in setup, `auth.setup_login_required`.
- `context/sce/telemetry-credential.md`: setup consent prompt, provisioning through setup login, the Claude pending line.
- `context/sce/cli-error-code-taxonomy.md`: the new error key.
- `context/glossary.md`: "setup access preferences", if introduced.
- `context/context-map.md`
- A decision record for "setup writes access-policy answers to the user layer only".

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:**
  - `cli/src/cli_schema.rs` and `cli/src/services/parse/command_runtime.rs` (the `--reconfigure` flag)
  - `cli/src/services/setup/{mod,prompt,command,claude_telemetry}.rs`
  - a new `cli/src/services/setup/access_preferences.rs`
  - `cli/src/services/config/{resolver,mod}.rs` (visibility of `resolve_default_global_config_path` / `AuthMode` re-export only)
  - `cli/src/services/auth_command/mod.rs` (login report refactor)
  - `cli/src/services/error.rs` (one `UserError` variant)
  - the context files listed under Context sync
- **Out of scope:**
  - `auth.db` schema or migrations
  - restoring `telemetry-ready.json` or any marker
  - Keychain ACLs and code signing
  - Linux Secret Service changes
  - Claude-native settings cleanup or reconciliation
  - resolver precedence changes or rewrites
  - new credential caches
  - new agent integrations
  - making the repository-config writer `persist_integration_targets` atomic
  - non-interactive consent or mode flags
  - a general `sce config set` command
- **Constraints:**
  - Policy is resolved only through `resolve_auth_runtime_config` / `resolve_access_policy`, with no duplicate precedence logic in setup.
  - No `dyn`, boxed futures, or a new prompt framework. Static generics and closures are used for test seams, matching `run_login_with`.
  - Fixed, deterministic user-facing strings.
  - No secrets in output.
  - `sce auth login` and non-interactive setup outputs stay byte-identical.
  - No new dependencies. `inquire::Select` is already in the `inquire` crate.
  - Rust 2021, `anyhow` with context, rustfmt, clippy clean.
  - No code comments (user preference).
- **Non-goal:** a general preferences framework, per-repository auth-mode authoring by setup, or changing what `mandatory` means for hooks and background work. Hooks never log in or block.

## Assumptions

- `auth.mode` set through setup is a per-user, per-machine preference stored in the global config. The prompt title and report say "this machine" or "all repositories". This rests on the `auth-telemetry-policy-config` future interface and on the per-user `auth.db`.
- Answering "No" to telemetry writes an explicit global `false`. This disables managed telemetry in every repository on the machine, consistent with consent being user-level. `--reconfigure` reverses it.
- The new prompts follow the existing four rather than reordering them, so the "Automatically sync Agent Traces?" prompt still comes before the auth-mode prompt. The existing resolver warning covers auto-sync being enabled together with auth `disabled`.
- The login offer defaults to Yes. Pressing Enter on a prompt labelled "Log in to SCE now?" counts as an explicit user action for Keychain access.
- Login inside setup uses the setup repository root as its working directory. Interactive setup always runs from the current directory, and the resolver discovers the same Git root from any subdirectory.
- A failed mandatory login exits nonzero without rolling back the installed assets or the saved answers. Every step is idempotent on rerun.

## Task stack

- [x] T01: `Add an atomic global-config writer for setup access preferences` (status:done)
  - Task ID: T01
  - Scope: In:
    - New `cli/src/services/setup/access_preferences.rs` with `GlobalAccessPreferences` (`Missing | Valid { auth_mode: Option<AuthMode>, telemetry_enabled: Option<bool> } | Invalid`), `read_global_access_preferences(path)`, and `persist_global_access_preferences(path, auth_mode: Option<AuthMode>, telemetry_enabled: Option<bool>) -> Result<GlobalWriteOutcome { Unchanged, Written }>`. Typed errors: `InvalidGlobalConfig`, `ConcurrentModification`, I/O with context.
    - Move `write_atomically` and `apply_mode` from `claude_telemetry.rs` to setup-module scope (`pub(super)`), unchanged in behavior, with `claude_telemetry.rs` using the moved helper.
    - Make `resolver::resolve_default_global_config_path` `pub(crate)` and re-export `AuthMode` through `config`.
    - Out: prompts, dispatch, `execute` wiring, login.
  - Dependencies: none
  - Done when:
    - The writer follows the safe-write algorithm in Design: it preserves unrelated keys, refuses invalid files without touching them, validates the output with `parse_file_config`, skips the write for unchanged bytes, detects changed bytes before the rename, and preserves the file mode (`0o600` for a new file).
    - Claude telemetry install tests are unchanged and green.
    - Any temporary `#[allow(dead_code)]` on the new items is listed in the completion record for T05 to remove.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml access_preferences`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_telemetry`
  - Completed: 2026-10-21
  - Files changed:
    - `cli/src/services/setup/access_preferences.rs` (new)
    - `cli/src/services/setup/atomic_write.rs` (new)
    - `cli/src/services/setup/claude_telemetry.rs`
    - `cli/src/services/setup/mod.rs`
    - `cli/src/services/config/resolver.rs`
  - Result:
    - Added `GlobalAccessPreferences { Missing, Valid { auth_mode, telemetry_enabled }, Invalid }`, `read_global_access_preferences`, `persist_global_access_preferences -> Result<GlobalWriteOutcome { Unchanged, Written }, GlobalAccessPreferencesError { InvalidGlobalConfig, ConcurrentModification, Io }>`, and a private `persist_global_access_preferences_with` that injects the pre-rename re-read for tests.
    - Moved `write_atomically` and `apply_mode` verbatim into `setup/atomic_write.rs` (`pub(super)`). `claude_telemetry.rs` now imports `write_atomically` from there.
    - `resolve_default_global_config_path` is now `pub(crate)`.
    - `AuthMode` was already reachable as `config::AuthMode` through `pub use types::*`, so no new re-export was added.
  - Deviations and assumptions:
    - "Unchanged" compares parsed JSON values rather than raw bytes. A file whose values already match is not rewritten, even if its formatting differs.
    - Calling the writer with no answers returns `Unchanged` and never creates a file.
  - Temporary dead-code allowances for T05 to remove: `#[allow(dead_code)]` on `pub(crate) mod access_preferences;` in `cli/src/services/setup/mod.rs`.
  - Verify outcomes:
    - `access_preferences`: passed (8 tests). Coverage: missing file, empty object, unrelated keys, explicit values, idempotent rerun, no-answer no-op, mode preservation (0o644) and new-file 0o600, invalid JSON / schema-invalid / unknown key / non-UTF-8 left untouched, injected concurrent modification, read-only directory I/O error, reader missing vs. absent keys.
    - `claude_telemetry`: passed (14 tests, unchanged).
    - Additional check: `clippy --all-targets -- --deny warnings` is clean.
  - Context impact: internal (new crate-private setup helper; no user-visible behavior, CLI surface, or contract change until T04/T05 wire it in)
  - Context synchronization: synced

- [x] T02: `Return a typed login report from auth login with an explicit working directory` (status:done)
  - Task ID: T02
  - Scope: In:
    - In `cli/src/services/auth_command/mod.rs`, add `pub(crate) struct LoginReport { rendered: String, telemetry: TelemetryProvisioningResult }` and `pub(crate) async fn run_login_in(working_dir: &Path, format: AuthFormat) -> Result<LoginReport, CliError>`.
    - `run_login_in` resolves policy from `working_dir` and passes `working_dir` to repository enrollment, replacing the `current_dir` reads in `enroll_current_repository_if_unbound`.
    - `run_login(format)` becomes `current_dir` + `run_login_in(..).map(|r| r.rendered)`.
    - `run_login_with`, `run_login_with_stored_credentials`, `run_text_login`, and `run_login_json` return `LoginReport`.
    - Out: setup wiring, any change to WorkOS, renewal, credential persistence, or provisioning logic.
  - Dependencies: none
  - Done when: `sce auth login` text and JSON output are byte-identical, existing auth_command tests pass unchanged, and new tests assert that `LoginReport.telemetry` is `Disabled` when policy telemetry is off and `Issued` when the provisioning closure issues, and that enrollment receives the supplied working directory.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml auth_command`
  - Completed: 2026-10-21
  - Files changed:
    - `cli/src/services/auth_command/mod.rs`
  - Result:
    - Added `pub(crate) struct LoginReport { rendered, telemetry }` with `pub(crate)` fields and `pub(crate) async fn run_login_in(working_dir, format) -> Result<LoginReport, CliError>`.
    - `run_login` now reads `current_dir` (same error context as before) and returns `run_login_in(..).rendered`; render functions are untouched, so `sce auth login` text/JSON output is byte-identical.
    - `run_login_with`, `run_login_with_stored_credentials`, `run_text_login`, and `run_login_json` return `LoginReport`; the stored-credential refresh path carries its provisioning result into the report.
    - `provision_telemetry_credential` takes the working directory; `enroll_current_repository_if_unbound` was replaced by `enroll_repository_if_unbound(working_dir)` over a generic seam `enroll_repository_if_unbound_with(working_dir, repository_id_for, bind)`.
    - Policy resolution goes through new `resolve_auth_config_in(working_dir)`; `resolve_current_auth_config` delegates to it.
  - Deviations and assumptions:
    - Existing tests needed mechanical type adaptation only (`.rendered` / `device_login_report()` fixture); their assertions are unchanged.
  - Verify outcomes:
    - `auth_command`: passed (21 tests, including new `login_report_carries_issued_telemetry_when_policy_enables_it` and `repository_enrollment_uses_the_supplied_working_directory`; `LoginReport.telemetry == Disabled` asserted in `login_skips_provisioning_unless_telemetry_is_enabled`).
    - Additional check: `clippy --all-targets -- --deny warnings` is clean.
  - Context impact: internal (crate-private login API seam for setup; no user-visible CLI behavior or output change until T06 wires it in)
  - Context synchronization: synced

- [x] T03: `Add auth-mode, telemetry-consent, and login-offer prompts to the setup prompt seam` (status:done)
  - Task ID: T03
  - Scope: In:
    - In `cli/src/services/setup/prompt.rs`, add `prompt_auth_mode(default: AuthMode)` using `inquire::Select` with the three fixed rows and the cursor on the default, `prompt_telemetry_consent(default: bool)`, and `prompt_login()` (default Yes) with the fixed labels from Design.
    - Give `prompt_agent_trace_auto_sync` and `prompt_attribution_hooks_enabled` a `default: bool` parameter. Current callers pass `true`.
    - Extend `trait SetupTargetPrompter` and `InquireSetupTargetPrompter`.
    - Cancel and `NotTTY` handling match the existing prompts.
    - Update `FakePrompter` in `setup/command.rs` tests to compile.
    - Out: dispatch logic, persistence, login.
  - Dependencies: T01 (`AuthMode` re-export)
  - Done when: the new prompt functions exist with exact label constants and defaults, the auth-mode row mapping is covered by a pure test (row index ↔ `AuthMode`, default cursor), and existing prompt behavior and wording are unchanged.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup::`
  - Completed: 2026-10-21
  - Files changed:
    - `cli/src/services/setup/prompt.rs`
    - `cli/src/services/setup/mod.rs`
    - `cli/src/services/setup/command.rs`
  - Result:
    - `prompt.rs` adds `prompt_auth_mode(default: AuthMode)` (`inquire::Select` over `AUTH_MODE_PROMPT_ROWS` = Optional, Mandatory, Disabled, with `with_starting_cursor(auth_mode_prompt_default_index(default))` and title `prompt_label("SCE account mode for this machine")`), `prompt_telemetry_consent(default: bool)`, and `prompt_login()` (`LOGIN_PROMPT_DEFAULT = true`). Every row label and confirm label is a fixed constant matching Design.
    - `prompt_agent_trace_auto_sync` and `prompt_attribution_hooks_enabled` take `default: bool`. Their wording is unchanged, and `resolve_setup_dispatch` passes `true`.
    - Cancel/Interrupt → `Ok(None)` and `NotTTY` → the existing fixed bail now go through one shared `interactive_prompt_result` helper, used by both `Confirm` and `Select`.
    - `SetupTargetPrompter` and `InquireSetupTargetPrompter` gain `prompt_auth_mode`, `prompt_telemetry_consent`, and `prompt_login`. The test `FakePrompter` in `setup/command.rs` implements them.
  - Deviations and assumptions:
    - The new trait methods and prompt functions have no callers until T04, so they carry temporary dead-code allowances.
  - Temporary dead-code allowances for T04 to remove when it wires dispatch: `#[allow(dead_code)]` on the trait methods `prompt_auth_mode`, `prompt_telemetry_consent`, and `prompt_login` in `cli/src/services/setup/mod.rs`, and on `auth_mode_prompt_title`, `prompt_auth_mode`, `prompt_telemetry_consent`, and `prompt_login` in `cli/src/services/setup/prompt.rs`.
  - Verify outcomes:
    - `setup::`: passed (71 tests). This includes the new `prompt::tests` (row index ↔ `AuthMode` order and labels, the default cursor for every mode, fixed telemetry/login/title labels, and Cancel/Interrupt/NotTTY mapping).
    - Additional check: `clippy --all-targets -- --deny warnings` is clean.
  - Context impact: internal (crate-private prompt seam additions; no user-visible behavior change until T04 wires dispatch)
  - Context synchronization: synced

- [x] T04: `Resolve access-policy choices and login intent in setup dispatch` (status:done)
  - Task ID: T04
  - Scope: In:
    - In `cli/src/services/setup/mod.rs`, extend `PersistedSetupConfirmations` with `auth_mode: Option<AuthMode>`, `telemetry_enabled: Option<bool>`, and `global_config_invalid: bool`.
    - Add `SetupAccessGate` (computed from the current `ResolvedAccessPolicy` and `auth_db_present`): `auth_locked_by: Option<AuthPolicyConfigFile>` for repository or explicit disabled, `effective_mandatory: bool`, `auth_db_present: bool`.
    - Add `enum SetupLoginIntent { NotOffered, Declined, Requested }`.
    - Add `auth_mode: Option<AuthMode>`, `telemetry_enabled: Option<bool>`, and `login_intent: SetupLoginIntent` to `SetupDispatch::Proceed`.
    - `resolve_setup_dispatch` takes the gate and applies the prompt sequence and gates from Design.
    - Non-interactive returns `None`, `None`, `NotOffered`.
    - In `setup/command.rs`, build the gate from `config::resolve_auth_runtime_config(&repository_root)`, `read_global_access_preferences`, and `managed_telemetry::auth_db_present()`, then pass it in. The new `Proceed` fields are carried but not yet persisted.
    - Out: writing the global config, login execution, `--reconfigure`.
  - Dependencies: T01, T03
  - Done when: table-driven dispatch tests with a recording `FakePrompter` cover:
    - a first run, which asks all prompts in order with the right defaults;
    - persisted global values, which suppress prompts 5 and 6;
    - selected `disabled`, which skips 6 and 7;
    - selected or effective `mandatory`, which skips 7;
    - repository or explicit `disabled`, which skips 5–7;
    - an invalid global config, which skips 5 and 6;
    - the `auth_db_present` and prompts-asked rules for prompt 7;
    - cancellation at prompts 5, 6, and 7, which returns `Cancelled`;
    - every non-interactive target, which prompts nothing and returns `NotOffered`;
    - a telemetry default of `false` whatever the Claude target selection or `auth_db_present` value.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup::`
  - Completed: 2026-10-21
  - Files changed:
    - `cli/src/services/setup/mod.rs`
    - `cli/src/services/setup/command.rs`
    - `cli/src/services/setup/prompt.rs`
  - Result:
    - `PersistedSetupConfirmations` gains `auth_mode`, `telemetry_enabled`, and `global_config_invalid`, filled from `GlobalAccessPreferences` by `with_global_access_preferences`. The repository reader leaves them at their defaults.
    - Added `SetupAccessGate { auth_locked_by, effective_mandatory, auth_db_present }` with `from_policy(&ResolvedAccessPolicy, auth_db_present)`. `auth_locked_by` keeps only `Repository`, `ConfigFlag`, and `ConfigEnv` sources of an effective `disabled`.
    - Added `SetupLoginIntent { NotOffered, Declined, Requested }`. `SetupDispatch::Proceed` carries `auth_mode`, `telemetry_enabled`, and `login_intent`.
    - `resolve_setup_dispatch` takes the gate. A private `resolve_setup_access_choices` applies prompts 5–7 after the existing four, using the Design gates. Cancelling returns `Cancelled`. Non-interactive dispatch returns `None`, `None`, `NotOffered`.
    - `SetupCommand::execute` builds the gate and the global preferences only for `SetupMode::Interactive`, via `resolve_auth_runtime_config`, `resolve_default_global_config_path` + `read_global_access_preferences`, and `managed_telemetry::auth_db_present()`. Non-interactive runs use a default gate and read nothing new. The new `Proceed` fields are not yet persisted.
    - Removed T03's temporary `#[allow(dead_code)]` allowances from the trait methods in `mod.rs` and the prompt functions in `prompt.rs`.
  - Deviations and assumptions:
    - `Proceed` carries only values answered in this run. A prompt suppressed by a persisted value yields `None`, so T05 writes only explicit new answers.
    - The prospective mode for the prompt 6/7 gates is chosen in this order: the selected answer, then the persisted global value, then the effective mode (`mandatory` when `effective_mandatory`, otherwise `optional`).
    - `effective_mandatory` is computed before the global write. T07 must account for it when `--reconfigure` changes a global `mandatory`.
    - T01's module-level `#[allow(dead_code)]` on `access_preferences` stays for T05, because the writer is still unused.
  - Verify outcomes:
    - `setup::`: passed (78 tests). These include the new `setup::tests` dispatch tests with a recording prompter:
      - first-run order and defaults
      - mode and telemetry gates (persisted, selected and persisted `disabled`, repository and explicit lock, invalid global)
      - login gates (`auth_db_present`, prompts asked this run, selected and effective `mandatory`)
      - cancellation at prompts 5, 6, and 7
      - every non-interactive target returning `NotOffered`
      - a telemetry default of `false`
      - the `SetupAccessGate::from_policy` lock sources
    - Additional check: `clippy --all-targets -- --deny warnings` is clean.
  - Context impact: internal, interim. Interactive setup now asks prompts 5–7, but their answers are not persisted or acted on until T05/T06. The report output is unchanged, and non-interactive, hooks-only, and context-only behavior is unchanged. Durable context for the prompt sequence belongs with T05–T07 per the plan's Context sync list.
  - Context synchronization: synced

- [ ] T05: `Persist global access preferences in setup and report the effective policy` (status:todo)
  - Task ID: T05
  - Scope: In:
    - In `SetupCommand::execute`, call `persist_global_access_preferences` right after dispatch and before `bootstrap_context_baseline`, only when `Proceed` carries an access value.
    - A write failure returns a `CliError` with a fixed message naming the global file, the `sce config validate` hint, and "no repository files were changed".
    - After the target install, re-resolve `resolve_auth_runtime_config(&repository_root)` and render a fixed `SCE access` section:
      - saved or unchanged values with the global path;
      - `auth.mode: <effective> (source: …)`;
      - `telemetry: enabled` or `telemetry: disabled (reason: <reason>)`;
      - the `sce auth logout` hint under effective `disabled` when `auth_db_present()`;
      - the deciding-file line when auth is locked by the repository or an explicit file.
    - The section appears only for interactive runs.
    - Remove T01's temporary dead-code allowances.
    - Out: login execution, `--reconfigure`.
  - Dependencies: T01, T04
  - Done when:
    - Execute-level tests with temp global and repository files (injected global path, `resolve_auth_runtime_config_with`) prove the following:
      - answers land only in the global file;
      - the repository config gains no `auth`/`telemetry` keys;
      - a rerun with identical answers leaves the global bytes unchanged;
      - a read-only or invalid global file aborts before `context/` or `.sce/` are created;
      - a repository `telemetry.enabled: false` with global consent reports `disabled (reason: repository_opt_out)`.
    - Exact-output tests cover the `SCE access` section.
    - Non-interactive output is byte-identical.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup::`
  - Context synchronization: pending

- [ ] T06: `Run optional or mandatory login from interactive setup through auth login` (status:todo)
  - Task ID: T06
  - Scope: In:
    - In `setup/command.rs`, add a pure `setup_login_action(policy: &ResolvedAccessPolicy, intent: SetupLoginIntent) -> SetupLoginAction { Skip(SetupLoginSkip), Run { required: bool } }`.
    - Add a generic step `run_setup_login_step<L, LF>(action, login: L) -> Result<SetupLoginStatus, CliError>`. In production it calls `auth_command::run_login_in(&repository_root, AuthFormat::Text)`.
    - Wire the step after the target install and re-resolution, and before the Claude telemetry stage.
    - Add `UserError::SetupLoginRequired` (key `auth.setup_login_required`, runtime class, fixed message) in `cli/src/services/error.rs`, keeping the login error as the source.
    - Render login status lines for skipped, declined, failed (fixed `UserError` message only), and the authenticated `LoginReport.rendered`.
    - Pass `LoginReport.telemetry == Issued` into `render_claude_telemetry_section` to drop `AUTH_PENDING_LINE`.
    - Out: changes to login internals, hooks, non-interactive modes.
  - Dependencies: T02, T05
  - Done when: table tests cover the following:
    - `setup_login_action` covers every pairing of effective mode with intent;
    - step tests with closures that panic when called prove disabled, declined, and not-offered never invoke login;
    - optional failure is a successful setup with the failure line;
    - mandatory failure returns `auth.setup_login_required`;
    - mandatory success and optional acceptance include the login text;
    - the Claude report omits the pending line only when `Issued`;
    - the setup output contains no token values.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup::`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml error`
  - Context synchronization: pending

- [ ] T07: `Add sce setup --reconfigure to revise persisted setup answers` (status:todo)
  - Task ID: T07
  - Scope: In:
    - Add a `--reconfigure` flag in `cli/src/cli_schema.rs` `Commands::Setup`.
    - Map it in `parse/command_runtime.rs` and add `SetupCliOptions.reconfigure` / `SetupRequest.reconfigure`.
    - `resolve_setup_request` rejects it, with fixed messages, when combined with a target flag, `--non-interactive`, `--hooks`, `--repo`, or `--bootstrap-context`.
    - With it, `resolve_setup_dispatch` ignores persisted values for suppression and passes them as prompt defaults for auto-sync, attribution, auth mode, and telemetry. Locked and invalid-global gates still apply.
    - Add a one-line hint `Run 'sce setup --reconfigure' to change saved setup answers.` to the `SCE access` section when any prompt was suppressed.
    - Out: non-interactive consent flags, `sce config set`.
  - Dependencies: T04, T05
  - Done when: request-validation tests cover every rejected combination; dispatch tests prove that `--reconfigure` asks all four persisted prompts with the persisted defaults (including a persisted global `disabled` as the auth-mode cursor), and that a repository lock still skips the access prompts; setup help lists the flag.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup`; `nix develop -c ./scripts/run-cli-cargo.sh run --manifest-path cli/Cargo.toml -- setup --help`
  - Context synchronization: pending

## Open questions

- **Global versus repository auth mode.** Saving `auth.mode` globally means that choosing `disabled` during setup in one repository turns off login, sync, and telemetry in every repository on the machine. The plan states this in the prompt and report and does not offer a repository-scoped restriction. The alternative is a fourth choice: write `disabled` (or `mandatory`) to the committed `.sce/config.json` as team policy. That alternative changes shared files and breaks older binaries that reject the keys. Is user-level only the right product call?
- **Login offer default.** Default Yes treats pressing Enter as consent to a possible Keychain dialog. Default No is the more conservative choice on privacy, but leaves users who enabled auto-sync unauthenticated. Which one should ship?
- **`--reconfigure` scope.** The plan has it re-ask the two existing repository behavior answers as well, because they are currently impossible to change through setup and a flag named "reconfigure" that skipped them would be confusing. The smaller alternative is to re-ask only the auth-mode and telemetry prompts.
- **Unattended consent.** Without new flags, CI or provisioning scripts can grant consent only by writing the global config file. Is that acceptable, or does a follow-up need a non-interactive `--telemetry`/`--auth-mode` contract?
