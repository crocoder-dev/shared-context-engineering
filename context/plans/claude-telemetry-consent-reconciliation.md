# Plan: claude-telemetry-consent-reconciliation

## Change summary

PR title: `fix: Reconcile Claude-native telemetry settings with SCE consent`.

Today `sce setup` installs Claude Code's native OpenTelemetry settings into user-level Claude settings when the Claude target is selected and the effective access policy enables telemetry. When the user later withdraws consent (for example `sce setup --reconfigure` answering "Share SCE telemetry?" with No, or choosing account mode `disabled`), setup saves the withdrawal to the global SCE config but leaves the Claude settings in place. SCE's own exporter and the `sce auth telemetry-headers` helper already fail closed, but Claude keeps `CLAUDE_CODE_ENABLE_TELEMETRY=1` and keeps sending OTLP requests to `https://otel.crocoderlab.dev` without an `Authorization` header. The payloads still leave the machine even though the server rejects them. `sce config show` documents this gap with its `telemetry.note` line.

This plan extends the existing Claude telemetry stage with a removal path. When the **global user config** records a withdrawal (`telemetry.enabled: false` or `auth.mode: disabled`), setup removes only the Claude settings it can attribute to SCE. Attribution uses exact-value fingerprint matching against the values SCE writes, anchored on `otelHeadersHelper == "sce auth telemetry-headers"`. Unrelated settings, user-owned or third-party telemetry, and managed (enterprise) settings are preserved. Repository-local opt-outs are never treated as global withdrawal, so another repository's Claude telemetry is not disabled. No marker file, database, or ownership registry is introduced. Install behavior, conflict detection, and the PR #339 non-interactive no-auth-storage contract are unchanged.

Base: branch `sce-setup` at `a1712e4b` (includes PR #339, `setup: Skip Claude authorization checks for non-interactive runs`).

## Current implementation findings

All paths are under `cli/src/services/`.

### Keys SCE writes (verified from code)

`setup/config_merge.rs::claude_telemetry_env_entries` plus `merge_claude_telemetry_settings` write exactly these 10 entries:

| Location | Key | Value written |
|---|---|---|
| top level | `otelHeadersHelper` | `sce auth telemetry-headers` (`CLAUDE_OTEL_HEADERS_HELPER_COMMAND`) |
| `env` | `CLAUDE_CODE_ENABLE_TELEMETRY` | `1` |
| `env` | `CLAUDE_CODE_ENHANCED_TELEMETRY_BETA` | `1` |
| `env` | `OTEL_TRACES_EXPORTER` | `otlp` |
| `env` | `OTEL_METRICS_EXPORTER` | `otlp` |
| `env` | `OTEL_LOGS_EXPORTER` | `otlp` |
| `env` | `OTEL_EXPORTER_OTLP_PROTOCOL` | `http/protobuf` (`managed_telemetry::MANAGED_OTLP_PROTOCOL`) |
| `env` | `OTEL_EXPORTER_OTLP_ENDPOINT` | `https://otel.crocoderlab.dev` (`managed_telemetry::MANAGED_OTLP_ENDPOINT`) |
| `env` | `CLAUDE_CODE_OTEL_HEADERS_HELPER_DEBOUNCE_MS` | `300000` |
| `env` | `OTEL_METRICS_INCLUDE_REPOSITORY` | `true` |

SCE never writes `ENABLE_ENHANCED_TELEMETRY_BETA`, which is only recognized as an alias in conflict detection. It also never writes `OTEL_EXPORTER_OTLP_HEADERS`, per-signal endpoints, protocols, or headers.

### Flows

- **Path resolution:** `setup/claude_telemetry.rs::resolve_claude_settings_path` uses `$CLAUDE_CONFIG_DIR/settings.json`, else `~/.claude/settings.json`, and returns `None` when there is no home directory. `managed_claude_settings_path` is `/etc/claude-code/managed-settings.json` on Linux and `/Library/Application Support/ClaudeCode/managed-settings.json` on macOS.
- **Conflict detection:** `config_merge.rs::claude_telemetry_conflict(&Value) -> Option<ClaudeTelemetrySkipReason>`. It returns `UserHelperPresent` for any non-SCE `otelHeadersHelper`. For `env` key classes (switches + alias, privacy, endpoints ×4, headers ×4, protocols ×4, exporters ×3, debounce), a key that is present with a value other than SCE's expected value is a conflict, and so is any `OTEL_*_EXPORTER=none`. Per-signal keys have no expected value, so their mere presence is a conflict. `None` means that every telemetry key present is exactly SCE's value.
- **Install:** `setup/claude_telemetry.rs::install_claude_telemetry(settings_path, managed_path)` reads both files. A managed read error becomes `None`, but malformed managed JSON makes the merge `Err`, which ends as `SkippedInvalid`. It then calls `merge_claude_telemetry_settings`:
  - Managed conflict → `Skipped(ManagedSettingsConflict)`.
  - User conflict → `Skipped(reason)`.
  - Otherwise it inserts the 10 entries and returns `AlreadyCurrent` or `Installed(bytes)`, using `to_string_pretty` + newline. `serde_json` has no `preserve_order`, so keys are re-sorted.
  - Writes go through `setup/atomic_write.rs::write_atomically`: a sibling `.sce-tmp` file, the existing mode kept (`0o600` for a new file), and a rename. Install does no symlink check and no re-read before the rename.
- **Policy gate and reporting:** `setup/command.rs::claude_telemetry_report` resolves `config::resolve_auth_runtime_config(repository_root)`, which gives the **effective** policy for the setup repository, and calls `render_claude_telemetry_section`:
  - Not enabled → the fixed `CLAUDE_TELEMETRY_DISABLED_BY_CONFIG_LINE`. No read or write.
  - Enabled → install. An `Err` from the install, including a write I/O error, is collapsed to `SkippedInvalid`.
  - After a `Configured` install, `RepositoryAuthorizationCheck::for_mode` decides the next step. Interactive runs call `auth_command::repository_telemetry_authorized`. Non-interactive and target-flag runs skip it (`NotChecked`, PR #339).
  - `format_claude_telemetry_report` renders fixed lines.
- **Stage trigger:** `SetupCommand::execute` runs the stage only when `dispatch_targets_claude(...)`. It runs after `run_setup_for_targets`, after the `SCE access` section, and after the setup login step. A mandatory login failure returns `Err` before the Claude stage runs.
- **Consent persistence:** `command.rs::prepare_interactive_setup` → `persist_setup_access_preferences` → `setup/access_preferences.rs::persist_global_access_preferences`. This is the first write of the run. It writes only to the global user config and has symlink refusal (`SymlinkManaged`), re-read concurrency refusal (`ConcurrentModification`), and parse validation. `read_global_access_preferences(path) -> GlobalAccessPreferences::{Missing, Valid{auth_mode, telemetry_enabled}, Invalid}` reads the user layer without credential access. The non-interactive path in `prepare_dispatch` never computes `resolve_default_global_config_path` today.
- **Resolver authority:** `config/resolver.rs::resolve_access_policy`. A user-side `telemetry.enabled: false` → `Disabled(UserOptOut)`, and auth mode is most-restrictive-wins, so a global `auth.mode: disabled` → `Disabled(AuthDisabled)`. Both rules apply in **every** repository on the machine, and no repository or explicit layer can lift them. A repository `false` → `RepositoryOptOut` applies only there. `UserOptOut` is also produced by an explicit `--config`/`SCE_CONFIG_FILE` `false`, so the effective reason alone cannot identify a *global* withdrawal.
- **Helper backstop:** `sce auth telemetry-headers` resolves the policy from its working directory and fails closed when telemetry is not `Enabled`. Repository opt-outs are enforced there, not in Claude settings.
- **Logout:** `auth_command::run_logout` deletes credential rows without a policy check. It never touches Claude settings.
- **Reporting note:** `config/render.rs::TELEMETRY_DISABLED_CLAUDE_SETTINGS_NOTE` is shown whenever telemetry is effectively disabled.

## Consent and ownership semantics

### Global withdrawal (the only removal trigger)

Global withdrawal is a property of the **global user config only**, as read by `read_global_access_preferences` after setup's own write:

- `Valid { telemetry_enabled: Some(false), .. }`, or
- `Valid { auth_mode: Some(AuthMode::Disabled), .. }`.

`Missing`, `Invalid`, and `Valid` with absent or `true` keys are **not** withdrawal. A degraded or invalid global config cannot establish what the user wants, so nothing is removed.

Why this is safe: under the resolver, either value disables SCE telemetry in every repository on the machine. Explicit and repository layers cannot lift it. User-level SCE-owned Claude settings therefore cannot be serving any repository, and removing them cannot break repository A while setup runs in repository B.

These are **not** global withdrawal and never remove anything:

- Repository `telemetry.enabled: false`.
- Repository `auth.mode: disabled`.
- An explicit-layer `false` or `disabled`.
- `no_user_consent`, `not_enabled`, `invalid_config`.

The existing helper and resolver gates keep enforcing them per repository.

### Ownership (who wrote a Claude setting)

| Candidate signal | Safety | Used? |
|---|---|---|
| Every `OTEL_*` / `CLAUDE_CODE_*` key | Unsafe: destroys user or third-party telemetry | No |
| Endpoint == `MANAGED_OTLP_ENDPOINT` alone | Ambiguous: the security doc recommends orgs set this endpoint themselves; a user may set it by hand | Only to report a residue, never to remove |
| `otelHeadersHelper == "sce auth telemetry-headers"` | Strong anchor: SCE-specific command string no other tool writes | Required anchor |
| Anchor + `claude_telemetry_conflict(user) == None` | Every telemetry key present is exactly SCE's value, the same condition under which install would leave the file `AlreadyCurrent` | **Removal predicate** |
| Marker file / ownership registry / DB | Tracks provenance, but adds new durable state, drift (user edits the file without the registry noticing), and multi-machine/dotfile-sync problems; still needs value checks to detect user edits | Rejected: exact-value matching with a required anchor answers every case below without new state |

Classification of user-level settings (pure, no I/O):

1. **No anchor, endpoint ≠ SCE endpoint (or absent):** `NotInstalled`. Unchanged. This covers custom helpers, other exporters, and unrelated env.
2. **No anchor, `env.OTEL_EXPORTER_OTLP_ENDPOINT == MANAGED_OTLP_ENDPOINT`:** `HelperMissing`. Unchanged and reported, because this could be a partially hand-edited SCE install or a hand-made config.
3. **Different helper value:** `NotInstalled`. Another integration owns telemetry, and install would also have skipped (`UserHelperPresent`).
4. **Anchor + `claude_telemetry_conflict == Some(reason)`:** `Modified(reason)`. The user changed an SCE-written value or added a per-signal key, endpoint, or header next to SCE's config. Everything stays unchanged and reported. Removing the shared switches (`CLAUDE_CODE_ENABLE_TELEMETRY`) would disable the user's own exporter, so partial removal is never attempted.
5. **Anchor + no conflict:** `Removed`. Remove `otelHeadersHelper` and each of the 9 SCE `env` keys present whose value equals SCE's value. Everything else in `env` survives, including telemetry-adjacent keys outside the conflict classes (for example `OTEL_RESOURCE_ATTRIBUTES`, `OTEL_LOG_USER_PROMPTS`, `ENABLE_ENHANCED_TELEMETRY_BETA`). Without `CLAUDE_CODE_ENABLE_TELEMETRY` those keys are inert. Drop the `env` key only when it was a non-empty object before removal and is empty after.

Residual ambiguity, accepted and documented: a key the user set before SCE's install with exactly SCE's value (for example `OTEL_METRICS_INCLUDE_REPOSITORY=true`) is indistinguishable from SCE's write and is removed. Install had already merged over it identically, so the file reached that state through SCE.

Managed settings are never written. When the managed-settings file parses and configures telemetry (`otelHeadersHelper` present, or any conflict-class `env` key present), removal of the user-level SCE keys still proceeds, and a fixed note line reports that managed settings also configure telemetry and SCE does not change them. A missing, unreadable, or malformed managed file adds no note and does not block removal. Server-managed settings, MDM plists, `--settings` files, and shell environment are not inspectable. This is the same limit as install.

## Reconciliation algorithm

Pure core in `setup/config_merge.rs`:

```rust
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum ClaudeTelemetryRemovalMerge {
    Removed(Vec<u8>),
    NotInstalled,
    Modified(ClaudeTelemetrySkipReason),
    HelperMissing,
}

pub(crate) fn remove_claude_telemetry_settings(
    existing_bytes: Option<&[u8]>,
    source_path: &str,
) -> Result<ClaudeTelemetryRemovalMerge>;

pub(crate) fn settings_configure_claude_telemetry(settings: &Value) -> bool;
```

1. `None` bytes → `NotInstalled`.
2. Parse JSON. A parse failure, a non-object top level, or a non-object `env` → `Err` naming `source_path`, the same wording family as `merge_claude_telemetry_settings`.
3. Classify per the ownership table, reusing `claude_telemetry_conflict`, `claude_telemetry_env_entries`, and the existing constants.
4. On `Removed`, remove the matched keys, drop `env` only if this removal emptied it, and serialize with `to_string_pretty` + `\n`, the install's format. Return the bytes. Re-parsing the result is part of the unit test, not a runtime step: the value is produced by `serde_json`.

Filesystem shell in `setup/claude_telemetry.rs`:

```rust
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum ClaudeTelemetryRemovalOutcome {
    Removed,
    NotInstalled,
    PreservedModified(ClaudeTelemetrySkipReason),
    PreservedHelperMissing,
    PreservedInvalid,
    PreservedSymlink,
    PreservedReadOnly,
    Failed(String),
}

pub(crate) struct ClaudeTelemetryRemovalReport {
    pub(crate) outcome: ClaudeTelemetryRemovalOutcome,
    pub(crate) managed_configures_telemetry: bool,
}

pub(crate) fn remove_claude_telemetry(settings_path: &Path, managed_settings_path: &Path) -> ClaudeTelemetryRemovalReport;
```

`remove_claude_telemetry` delegates to a private `remove_claude_telemetry_with(settings_path, managed_settings_path, reread)`, mirroring `persist_global_access_preferences_with`, so the concurrency check can be tested.

1. Read the settings file. `NotFound` → `NotInstalled`. Other read errors → `Failed(fixed message + path)`.
2. Read and parse the managed file best-effort and compute `managed_configures_telemetry`. It never gates the result.
3. Call `remove_claude_telemetry_settings`. `Err` → `PreservedInvalid`. `NotInstalled` / `Modified` / `HelperMissing` map 1:1 with **no write**, so a no-op never rewrites the file.
4. On `Removed(bytes)`, check in this order:
   - `fs::symlink_metadata`: a symlink → `PreservedSymlink`.
   - On Unix, no owner write bit (`mode & 0o200 == 0`) → `PreservedReadOnly`.
   - `reread(path)` ≠ original bytes → `Failed(concurrent-modification message)`.
5. `write_atomically(path, &bytes)`. An error → `Failed(error text)`. The temp file is removed on rename failure, as today. Success → `Removed`.
6. `format_claude_telemetry_removal_report(&report, claude_selected) -> Option<String>` renders fixed lines. `NotInstalled` renders the existing `Claude telemetry: skipped; SCE telemetry is disabled by config.` line when Claude is selected, and nothing otherwise. Preserved and failed lines include the settings path and an actionable next step. A failed line states that consent is still withdrawn. The managed note is appended when its flag is set.

There are no traits, no dynamic dispatch, and no new modules. Everything is a static function over existing types.

## Global versus repository behavior

| Situation | Effect on user-level Claude settings | Why |
|---|---|---|
| Global `telemetry.enabled: false` | Reconcile (remove SCE-owned only) | Disables SCE telemetry in every repository |
| Global `auth.mode: disabled` | Reconcile | Most-restrictive auth wins machine-wide → telemetry `auth_disabled` everywhere. This is a machine-level prohibition, so cleanup is consistent with it |
| Repository `telemetry.enabled: false` | Unchanged; existing skip line | Other repositories may rely on the settings; the helper fails closed here |
| Repository `auth.mode: disabled` | Unchanged; existing skip line | Repository-local restriction |
| Explicit (`--config` / `SCE_CONFIG_FILE`) `false` / `disabled` | Unchanged; existing skip line | Per-invocation layer, not a machine-wide decision |
| Global config invalid (degraded) | Unchanged; existing skip line | Consent cannot be established |
| Global keys absent (`not_enabled`) | Unchanged; existing skip line | Never consented ≠ withdrew |

## Triggering and lifecycle

Trigger contract. A pure decision in `setup/command.rs`:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ClaudeTelemetryAction { Install, Remove, SkipDisabled }

fn claude_telemetry_action(
    global: GlobalAccessPreferences,
    effective_enabled: bool,
    claude_selected: bool,
    interactive: bool,
) -> Option<ClaudeTelemetryAction>;
```

- `Remove` when the global user config records a withdrawal **and** (the run is interactive **or** Claude is selected). Global withdrawal implies effective disabled, so `Install` and `Remove` are mutually exclusive by the resolver.
- `Install` when effective telemetry is enabled and Claude is selected. Unchanged.
- `SkipDisabled` when Claude is selected and neither applies. Unchanged line and no I/O.
- `None` otherwise: no line and no I/O.

Answers to the lifecycle questions:

- **`sce setup --reconfigure` withdrawal:** yes. Setup writes the withdrawal to the global config first (existing order), then re-reads it, and the Claude removal runs in the same invocation. It runs even when Claude was installed by a previous run.
- **Claude not selected (interactive):** cleanup still runs. Consent is machine-wide and the SCE-owned keys are inert without it. When nothing SCE-owned exists, no line is printed.
- **Ordinary `sce setup` with consent already withdrawn:** reconciles. The trigger is state-based and idempotent, so a manual edit of the global config is applied by rerunning `sce setup`. No new command is added.
- **Non-interactive / target-flag setup:** reconciles only when Claude is selected (`--claude`, `--all`), the same surface that installs today. It reads the global config path through `resolve_default_global_config_path` + `read_global_access_preferences`, a config-file read only. It never opens `auth.db` or the OS credential store. Hooks-only, context-only, and non-Claude non-interactive runs never touch Claude settings.
- **Placement:** in interactive runs, `Remove` executes and renders **immediately after the `SCE access` section and before the setup login step**, so a mandatory-login failure (`auth.setup_login_required`) cannot skip cleanup. `Install` keeps its current position after login, because its authorization check depends on enrollment done by login. In non-interactive runs the stage position is unchanged.
- **`sce auth logout`:** no change. Logout removes credentials, not consent. After logout the helper fails closed. Coupling the two would make logout rewrite Claude settings that a still-consenting user wants for the next login.
- **Re-enable:** after removal, the settings have no SCE keys and no conflict, so a later run with global `telemetry.enabled: true` and Claude selected installs again through the unchanged install path. Repeated disable runs return `NotInstalled` without writing.
- **No auth dependency:** removal never calls `repository_telemetry_authorized`, `run_login_in`, `managed_telemetry::auth_db_present`, or any token storage.

## Failure and recovery behavior

| Condition | Outcome | Write? | Report |
|---|---|---|---|
| Missing settings file / no home directory | `NotInstalled` | No | Existing skip line if Claude selected, else none |
| Malformed JSON / non-object top level or `env` | `PreservedInvalid` | No | Path + "could not be parsed and were left unchanged" |
| Unreadable file (permission error) | `Failed` | No | Path + error + manual remediation |
| Read-only file (no owner write bit) | `PreservedReadOnly` | No | Path + "is read-only; remove the SCE entries manually" |
| Symlink-managed file | `PreservedSymlink` | No | Path + "update its source directly (for example Nix or Home Manager)" |
| Concurrent modification between read and rename | `Failed` | No | Path + "changed while setup was updating it; rerun 'sce setup'" |
| Partial match (anchor missing, SCE endpoint present) | `PreservedHelperMissing` | No | Path + manual remediation |
| User-modified SCE values / custom OTLP exporter or endpoint beside anchor | `PreservedModified(reason)` | No | Path + reason + manual remediation |
| Custom helper / other integration only | `NotInstalled` | No | As missing |
| Managed settings configure telemetry | Removal proceeds | User file only | Extra fixed note line |
| Atomic replace fails (dir unwritable, rename error) | `Failed` | Temp removed | Path + error |

Ordering and consent:

- The global consent write happens first, unchanged, and is never rolled back or restored when cleanup fails.
- Cleanup failure is **non-fatal**. Setup completes and exits 0, the same posture as existing Claude conflict skips. The rendered line says cleanup did not happen, names the file and the next step, and states that SCE telemetry stays disabled by config.
- The success line is printed only after `write_atomically` returns `Ok`.

## Behavioral matrix

| Global consent (user layer) | Repository / explicit policy | Existing Claude user settings | Mode | Expected behavior |
|---|---|---|---|---|
| `true` | none | none | Claude selected | Install SCE settings (unchanged) |
| `true` | none | SCE-managed | Claude selected | `AlreadyCurrent`, no rewrite (unchanged) |
| `true` | none | custom exporter / endpoint / helper | Claude selected | Preserve; existing conflict skip line (unchanged) |
| `true` | repository `telemetry.enabled: false` | SCE-managed | any | Preserve byte-identical; existing "disabled by config" line if Claude selected |
| `true` | repository `telemetry.enabled: false` | none | Claude selected | Do not install; existing skip line |
| `true` | repository `auth.mode: disabled` | SCE-managed | any | Preserve; existing skip line |
| `true` | explicit `telemetry.enabled: false` | SCE-managed | any | Preserve; existing skip line |
| `false` | any | SCE-managed (exact) | interactive, or Claude selected | Remove SCE keys; keep unrelated keys; report removed |
| `false` | any | SCE-managed + unrelated `env` | interactive, or Claude selected | Remove only the 10 SCE entries; `env` keeps the unrelated keys |
| `false` | any | custom exporter (no SCE helper) | interactive, or Claude selected | `NotInstalled`; preserve |
| `false` | any | SCE helper + user-changed endpoint / added per-signal key | interactive, or Claude selected | `PreservedModified`; preserve all; report |
| `false` | any | SCE endpoint without SCE helper | interactive, or Claude selected | `PreservedHelperMissing`; preserve; report |
| `false` | any | none | any | No-op; no write |
| `false` | any | SCE-managed | non-interactive, Claude not selected | Untouched (not this run's surface) |
| `auth.mode: disabled` | any | SCE-managed | interactive, or Claude selected | Remove (global prohibition) |
| absent | any | SCE-managed (installed through an explicit-file consent) | any | Preserve; existing skip line (never consented ≠ withdrew) |
| invalid global config | any | SCE-managed | any | Preserve; existing skip line |
| `true` again after removal | none | previously cleaned | Claude selected | Reinstall via unchanged install path |
| `false` (second run) | any | previously cleaned | interactive, or Claude selected | `NotInstalled`; no write |

## Acceptance criteria

- [ ] AC1: With global `telemetry.enabled: false` or global `auth.mode: disabled`, removal deletes exactly `otelHeadersHelper` and the 9 SCE `env` entries from SCE-owned settings. Unrelated top-level keys, hooks, permissions, and non-SCE `env` keys survive, and `env` is dropped only when the removal emptied it.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_telemetry_removal` (table-driven ownership tests in `config_merge.rs`; confirm a non-zero match count with `-- --list`).
- [ ] AC2: Settings that are not attributable to SCE stay byte-identical and are reported with a fixed line naming the file: custom helper, other exporter, user-modified SCE values, per-signal additions, SCE endpoint without helper, malformed JSON, symlink, read-only file, and managed settings.
  - Validate: same filter; the shell tests assert file bytes before and after, and `fs::read_link` stays unchanged for the symlink case.
- [ ] AC3: A repository-level or explicit-layer opt-out (`telemetry.enabled: false` or `auth.mode: disabled`) never modifies user-level Claude settings, while global consent stays `true`.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup_claude_telemetry` (repository A/B and explicit-layer cases assert byte-identical settings).
- [ ] AC4: Interactive `sce setup` / `--reconfigure` that records a global withdrawal removes SCE-owned Claude settings in the same run, whether or not Claude is selected and even if a mandatory login fails afterwards. A non-interactive run reconciles only when Claude is selected.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_telemetry_action` plus the `setup_claude_telemetry` integration-style tests in `setup/command.rs`.
- [ ] AC5: Enable → disable → enable → disable cycles are idempotent. A second disable performs no write, and re-enabling reinstalls the same bytes as a first install.
  - Validate: lifecycle test in `setup/command.rs` comparing file bytes and mtime-independent content across the cycle.
- [ ] AC6: Removal never touches `auth.db`, the OS credential store, the repository-authorization lookup, or login, in either interactive or non-interactive mode.
  - Validate: tests pass an authorization closure that panics if called; inspection confirms that `remove_claude_telemetry` and the removal branch call no `auth_command` / `token_storage` / `auth_db_present` function.
- [ ] AC7: A cleanup failure (concurrent modification, unwritable directory) is reported as not done, with the path and a remediation. Setup exits 0, and the withdrawn global consent remains saved.
  - Validate: shell tests with an injected re-read and a `0o555` directory assert the `Failed` line and an unchanged global config.
- [ ] AC8: `sce config show` `telemetry.note` describes the new reconciliation behavior in place of the old "not changed" wording.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml config::render`.
- [ ] AC9: The existing install, conflict-detection, and PR #339 non-interactive tests pass unmodified, apart from tests whose asserted output order intentionally changes.
  - Validate: `nix flake check`.

### Full validation

- `nix flake check`

### Context sync

- `context/sce/telemetry-credential.md`, section "Claude-native telemetry": the removal trigger, ownership predicate, preserved cases, and failure lines; remove "Settings written by earlier setups are not removed."
- `context/cli/access-policy.md`: the Enforcement bullet for `sce setup` Claude telemetry install, the Reporting `telemetry.note` text, and the coverage-limit sentence.
- `context/sce/setup-repo-local-config-bootstrap.md`: the Setup login / Claude telemetry stage ordering (removal before login in interactive runs).
- `context/context-map.md`: the annotations for `telemetry-credential.md` and `access-policy.md`, if they mention the limitation.
- `context/glossary.md`: add "global telemetry withdrawal" only if the term is used across files.

## Task context synchronization lifecycle

- **Task context synchronization:** every task carries `pending | synced | blocked`. A completed task must be `synced` before another task can start or the plan can finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition** beside the status. Never infer `synced` from conversation history; write every lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** `cli/src/services/setup/config_merge.rs`, `cli/src/services/setup/claude_telemetry.rs`, `cli/src/services/setup/command.rs`, `cli/src/services/config/render.rs` (note text only), and the context files listed under Context sync.
- **Out of scope:**
  - Auth DB schema, credential migration, Keychain / Secret Service, WorkOS, `sce auth logout`.
  - Resolver precedence (`resolve_access_policy`).
  - Telemetry readiness markers, repository enrollment, optional login recovery.
  - Other agent integrations (OpenCode, Codex, Pi).
  - Agent Trace auto-sync.
  - Claude managed / server-managed settings writes.
  - `sce doctor` reporting of stale Claude settings.
  - A general settings-management framework.
- **Constraints:**
  - Static functions and existing enums only.
  - Reuse `claude_telemetry_conflict`, `claude_telemetry_env_entries`, `write_atomically`, and `read_global_access_preferences`.
  - Fixed, deterministic output strings held in constants.
  - No comments in code (repository-owner preference).
  - Non-interactive runs must never open `auth.db` or the credential store (PR #339).
- **Non-goal:**
  - An ownership registry, marker file, or install-time snapshot.
  - Partial removal from settings that the user modified.
  - Hardening the existing install path (symlink check, distinct write-failure line); see Open questions.

## Assumptions

- "Global consent" means the global user config layer only (`GlobalAccessPreferences`), per [ADR: setup writes access answers to user layer only](../decisions/2026-10-21-setup-writes-access-answers-to-user-layer-only.md). The explicit layer is per-invocation and is not machine-wide.
- A global `auth.mode: disabled` counts as withdrawal. The resolver already makes it disable telemetry in every repository, so cleanup cannot affect any repository that still needs the settings.
- Cleanup failure is non-fatal (exit 0, explicit failure line). This matches existing Claude conflict skips and the "assets not rolled back" posture of mandatory-login failure.
- Re-serialization with sorted keys on removal is acceptable because install already rewrites the same file that way.
- Removal output position (before login, interactive only) may change the relative order of the Claude line in tests that assert the full interactive output. Install ordering is unchanged.

## Task stack

- [x] T01: `Add pure SCE-owned Claude telemetry removal` (status:done)
  - Task ID: T01
  - Scope:
    - In: `setup/config_merge.rs`:
      - Add `ClaudeTelemetryRemovalMerge`, `remove_claude_telemetry_settings(existing_bytes, source_path)`, and `settings_configure_claude_telemetry(&Value)`, implementing the ownership classification (anchor `otelHeadersHelper`, `claude_telemetry_conflict == None`, exact-value removal of the 10 entries, `env` drop only when emptied, `to_string_pretty` + newline).
      - Add one table-driven test `claude_telemetry_removal_classifies_ownership`. Cases: exact SCE install; SCE + unrelated env/hooks/permissions; partial SCE set (subset of the 9 keys + helper); user-modified endpoint; added per-signal endpoint; added `OTEL_EXPORTER_OTLP_HEADERS`; custom helper; other exporter without helper; SCE endpoint without helper; `env` emptied vs `env` with leftovers; `None` bytes.
      - Add `claude_telemetry_removal_rejects_malformed_json_naming_the_path`.
      - Add `claude_telemetry_removal_then_install_round_trips` (install(remove(install(x))) == install(x), and remove(remove(x)) is `NotInstalled`).
    - Out: filesystem I/O, reporting, triggers.
  - Dependencies: none
  - Done when:
    - The pure function returns the documented variant for every case.
    - Removed output re-parses and contains no SCE key, while preserving every non-SCE key.
    - The existing `telemetry_merge_*` tests are unchanged and pass.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_telemetry_removal -- --list` (non-zero), then without `--list`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml telemetry_merge`.
  - Completed: 2026-10-21
  - Files changed: `cli/src/services/setup/config_merge.rs`
  - Result: Added `ClaudeTelemetryRemovalMerge`, `remove_claude_telemetry_settings`, and `settings_configure_claude_telemetry`, implementing the ownership classification. A different helper or no anchor → `NotInstalled`. No anchor with the SCE endpoint → `HelperMissing`. Anchor with a conflict → `Modified(reason)`. Anchor without a conflict → `Removed`, which drops the helper and the exact-value SCE `env` entries, and drops `env` only when this removal emptied it. Malformed JSON, a non-object top level, or a non-object `env` → `Err` naming the path. Added the three planned tests, with the preserved-case table extracted into the `preserved_removal_cases` helper to satisfy `clippy::too_many_lines`. `remove_claude_telemetry_settings` and `settings_configure_claude_telemetry` carry `#[cfg_attr(not(test), expect(dead_code))]` until T02 wires them in; T02 must remove those attributes.
  - Verify outcomes: `claude_telemetry_removal -- --list` → 3 tests (passed); `claude_telemetry_removal` → 3 passed, 0 failed (passed); `telemetry_merge` → 9 passed, 0 failed, existing tests unchanged (passed). Extra: `run-cli-cargo.sh clippy --all-targets` clean (passed).
  - Context impact: none. This adds internal pure helpers with no callers, and there is no user-visible behavior change until T02/T03.
  - Context synchronization: synced

- [x] T02: `Add guarded filesystem removal and fixed report lines` (status:done)
  - Task ID: T02
  - Scope:
    - In: `setup/claude_telemetry.rs`:
      - Add `ClaudeTelemetryRemovalOutcome`, `ClaudeTelemetryRemovalReport`, `remove_claude_telemetry(settings_path, managed_settings_path)`, and private `remove_claude_telemetry_with(.., reread)`. The order is read → managed note (best-effort) → pure removal → symlink check (`fs::symlink_metadata`) → owner-write-bit check (Unix) → re-read concurrency check → `write_atomically`.
      - Add fixed `const` report lines and `format_claude_telemetry_removal_report(&report, claude_selected) -> Option<String>`.
      - Add table-driven test `claude_telemetry_removal_filesystem_outcomes`. Cases: missing file; exact SCE file (mode `0o640` preserved); invalid JSON byte-identical; symlink with link + target unchanged; `0o444` file unchanged; injected concurrent re-read → `Failed` and file unchanged; `0o555` parent dir → `Failed`; no-op `NotInstalled`/`PreservedModified` leaves bytes **and** inode/mtime untouched; managed file configuring telemetry → note flag, user file still cleaned.
      - Add one report-rendering test covering each variant and the `claude_selected` false/true `NotInstalled` lines.
    - Out: setup wiring, install path changes.
  - Dependencies: T01
  - Done when:
    - Every failure-table row maps to the documented outcome and line, with no write in any non-`Removed` outcome.
    - The success line appears only after `write_atomically` returns `Ok`.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_telemetry_removal`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_telemetry_install`.
  - Completed: 2026-10-21
  - Files changed: `cli/src/services/setup/claude_telemetry.rs`, `cli/src/services/setup/config_merge.rs`, `cli/src/services/setup/command.rs`
  - Result: Added `ClaudeTelemetryRemovalOutcome`, `ClaudeTelemetryRemovalReport`, `remove_claude_telemetry`, and private `remove_claude_telemetry_with(.., reread)`. The steps run in the planned order: read → best-effort managed note via `settings_configure_claude_telemetry` → pure removal → `symlink_metadata` symlink check → owner-write-bit check (Unix; `readonly()` elsewhere) → injected re-read comparison → `write_atomically`. Outcomes other than `Removed` never write. Added fixed text constants and `format_claude_telemetry_removal_report(&report, claude_selected)`. `NotInstalled` renders the existing disabled-by-config line only when Claude is selected. Preserved lines name the path, give a remediation, and say SCE telemetry stays disabled by config. The `Failed` line names the path and detail and says consent remains withdrawn. The managed note is appended as a second line. Deviations: `ClaudeTelemetryRemovalReport` carries `settings_path` so the report lines can name the file. `CLAUDE_TELEMETRY_DISABLED_BY_CONFIG_LINE` moved from `command.rs` to `claude_telemetry.rs` as `pub(crate)` and is imported back, with the same text. The `0o555` parent-directory case is its own test, `claude_telemetry_removal_reports_failed_replace_in_unwritable_directory`, because it needs permissions restored before cleanup. T01's `expect(dead_code)` attributes were removed from `config_merge.rs`. `remove_claude_telemetry` and `format_claude_telemetry_removal_report` now carry `#[cfg_attr(not(test), expect(dead_code))]` until T03 wires them in, and T03 must remove them.
  - Verify outcomes: `claude_telemetry_removal` → 6 passed, 0 failed (3 from T01 + `claude_telemetry_removal_filesystem_outcomes`, `claude_telemetry_removal_reports_failed_replace_in_unwritable_directory`, `claude_telemetry_removal_report_renders_fixed_lines`) (passed); `claude_telemetry_install` → 8 passed, 0 failed (passed). Extra: `run-cli-cargo.sh clippy --all-targets -- -D warnings` clean; `claude_telemetry` filter → 19 passed (passed).
  - Context impact: none. These are internal filesystem helpers with no production callers yet, so user-visible behavior is unchanged until T03 wires the removal into setup.
  - Context synchronization: synced

- [ ] T03: `Trigger Claude telemetry removal on global consent withdrawal in setup` (status:todo)
  - Task ID: T03
  - Scope:
    - In: `setup/command.rs`:
      - Add `ClaudeTelemetryAction` and `claude_telemetry_action(global, effective_enabled, claude_selected, interactive)`, plus a `global_consent_withdrawn(GlobalAccessPreferences) -> bool` predicate (`telemetry_enabled: Some(false)` or `auth_mode: Some(AuthMode::Disabled)` in `Valid`).
      - Interactive path: after `persist_setup_access_preferences`, re-read the global config with `read_global_access_preferences(&access_save.global_config_path)`. On `Remove`, run `remove_claude_telemetry` and push its section right after the `SCE access` section, before `run_setup_login_step`.
      - Non-interactive path: compute `resolve_default_global_config_path` + `read_global_access_preferences` only when Claude is selected. A path or read error counts as not withdrawn.
      - `Install` and `SkipDisabled` keep `render_claude_telemetry_section` and its position unchanged.
      - Tests:
        - Table test `claude_telemetry_action_follows_global_withdrawal_only`. Rows: global false / disabled / true / absent / invalid × repository opt-out × Claude selected × interactive.
        - Lifecycle test `setup_claude_telemetry_consent_cycle_is_idempotent` (enable → disable → disable → enable → disable over one temp settings file).
        - `setup_claude_telemetry_repository_opt_out_preserves_user_settings` (repository A/B and explicit-layer `false` with global `true`; bytes unchanged).
        - `setup_claude_telemetry_removal_never_checks_authorization` (panicking authorization closure; interactive and non-interactive).
    - Out: changes to `prepare_interactive_setup` consent writing, the login step, the install path, and `RepositoryAuthorizationCheck`.
  - Dependencies: T02
  - Done when:
    - Withdrawal recorded in this run or earlier removes SCE-owned settings in interactive runs regardless of target, and in non-interactive runs only with Claude selected.
    - Repository and explicit opt-outs leave settings byte-identical.
    - Removal happens before a possible mandatory-login failure.
    - The existing `setup_claude_telemetry_*` and PR #339 tests pass.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_telemetry_action`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup_claude_telemetry`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup_repository_authorization_is_checked_only_for_interactive_setup -- --exact`. Expected: nonzero test counts and all passing.
  - Context synchronization: pending

- [ ] T04: `Update config show telemetry note for Claude settings reconciliation` (status:todo)
  - Task ID: T04
  - Scope:
    - In: `config/render.rs`:
      - Change `TELEMETRY_DISABLED_CLAUDE_SETTINGS_NOTE` to a fixed string stating that `sce setup` removes SCE-managed Claude Code telemetry settings when the global SCE config sets `telemetry.enabled: false` or `auth.mode: disabled`, and that repository-level opt-outs leave them in place. For example: `Claude Code telemetry settings written by sce setup are removed by the next sce setup run after the global SCE config sets telemetry.enabled to false or auth.mode to disabled; repository-level opt-outs leave them in place.`
      - Existing tests already reference the constant.
    - Out: show/validate structure, JSON field names, and when the note is shown.
  - Dependencies: T03
  - Done when: text and JSON `telemetry.note` render the new constant, and the render tests pass.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml config::render`.
  - Context synchronization: pending

## Compatibility concerns

- **Output order:** in interactive runs that remove settings, the Claude line now appears before the `SCE login` section. Install-path output and order are unchanged.
- **Formatting:** a removal rewrite re-sorts keys and pretty-prints the file, the same effect install already has on this file.
- **Claude Code versions:** removing keys is version-neutral. Claude falls back to its defaults (telemetry off).
- **Older SCE binaries:** an older `sce setup` with telemetry enabled may reinstall the settings. That is expected, because it reflects consent at that point.
- **Shared dotfiles:** when `~/.claude/settings.json` is synced across machines, removal on one machine propagates. The symlink refusal covers the Home Manager/Nix case. Plain-file syncing is the user's choice.

## Open questions

- Should non-interactive `sce setup --claude` / `--all` also remove settings on global withdrawal? This plan says yes, because it is the same surface that installs and needs no auth storage. The narrower alternative is to reconcile only in interactive runs, which leaves non-interactive runs printing the existing skip line over stale settings.
- Should cleanup failure be fatal (non-zero exit) instead of a reported warning? The plan keeps it non-fatal for consistency with conflict skips. A fatal exit would also suppress the remaining sections, because errors return early.
- The existing install path replaces a symlinked `settings.json` through `write_atomically` without the symlink check, and reports write I/O errors as "could not be parsed" because of `unwrap_or(SkippedInvalid)`. Both are pre-existing install defects outside this plan's scope. Fix them in a separate PR, or fold them into T02, since they reuse the same guard?
