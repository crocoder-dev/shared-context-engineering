# SCE setup local bootstrap

## Scope

Task `setup-repo-gate-and-local-config-bootstrap` T02, `turso-local-db-sync` T04, and `setup-bootstrap-context` T01 define the local bootstrap behavior for `sce setup`.

## Behavior

- Any successful `sce setup` run in a git-backed repository creates `.sce/config.json` when the file is absent.
- Interactive setup asks two independent post-selection confirmations. The first displays:
  ```
  Automatically sync Agent Traces?
  (Requires an SCE account. Sends Agent Traces from supported AI coding tools
  to SCE servers so they can be stored and viewed in your account.)
  ```
  The second displays:
  ```
  Record SCE involvement in Git commits?
  (Adds SCE metadata to commits created or assisted by SCE.)
  ```
  Both default to Yes, and the prompt seam carries each answer separately through setup dispatch so declining one does not change the other. Each is skipped when the repository config already holds its value (`agent_trace.auto_sync`, `policies.attribution_hooks.enabled`), unless [`--reconfigure`](#reconfigure) is passed.
- Interactive setup then resolves SCE access choices in dispatch, after those two confirmations and before any file write:
  - `SCE account mode for this machine` (`Select`; cursor on `optional`). It is skipped when the global user config already holds `auth.mode` (unless `--reconfigure`), or when that config is invalid.
  - `Share SCE telemetry?` (`Confirm`; default No). It is skipped when the global user config already holds `telemetry.enabled` (unless `--reconfigure`), when that config is invalid, or when the selected or persisted mode is `disabled`.
  - `Log in to SCE now?` (`Confirm`; default Yes). It is offered only when the prospective mode is `optional`, no repository or explicit layer enforces `mandatory`, and either an access prompt was asked in this run or no `auth.db` file exists.
  - A repository or explicit (`--config` / `SCE_CONFIG_FILE`) `auth.mode: disabled` skips all three prompts. Cancelling any of them is the standard setup cancellation.
  - Inputs are read without credential access: the resolved access policy, the global user config file, and the presence of the `auth.db` file.
  - The answers are carried on the dispatch result (`auth_mode`, `telemetry_enabled`, and a `SetupLoginIntent` of `NotOffered | Declined | Requested`). Only answers given in this run are carried; a prompt suppressed by a persisted value carries `None`. The login intent feeds the [setup login step](#setup-login).
  - The answers are saved to the global user config before any repository write (see [Global access preferences](#global-access-preferences)).
  - Non-interactive, hooks-only, and context-only setup never read these inputs or prompt.

### Reconfigure

`sce setup --reconfigure` is interactive setup that asks every persisted answer again: auto-sync, attribution, account mode, and telemetry consent. Each prompt's default is the saved value (falling back to the first-run default), so a persisted global `disabled` puts the account-mode cursor on `disabled`. The repository/explicit `disabled` lock and the invalid-global gate still skip the access prompts. New answers go through the same writers as a first run.

- It must be used alone (apart from `--workflow`). Combining it with a target flag, `--non-interactive`, `--hooks`, `--repo`, or `--bootstrap-context` fails request validation with `Option '--reconfigure' cannot be used with '<flag>'. Try: run 'sce setup --reconfigure' alone to change saved setup answers interactively.`
- A global `mandatory` that `--reconfigure` changes to `optional` gets the login offer, because only a repository or explicit `mandatory` suppresses it.
- A plain interactive run where a saved answer skipped a prompt adds `- Run 'sce setup --reconfigure' to change saved setup answers.` to the `SCE access` section.
- The bootstrap writes the canonical JSON payload with the versioned schema declaration and explicit opt-ins for both setup-controlled behaviors: `{"$schema": "https://sce.crocoder.dev/v<version>/config.json", "agent_trace": {"auto_sync": true}, "policies": {"attribution_hooks": {"enabled": true}}}` (where `<version>` is the CLI release version, with a trailing newline).
- If `.sce/config.json` already exists, the bootstrap step returns `Ok(())` immediately and leaves the file untouched — no merge, no reformat, no overwrite. A later interactive target-install write may merge the two answered behavior values, while non-interactive setup leaves existing behavior keys and omissions unchanged.
- The parent `.sce/` directory is created via `fs::create_dir_all` if missing.
- The setup flow also bootstraps the canonical local DB through `LocalDbLifecycle::setup` and the Agent Trace DB through `AgentTraceDbLifecycle::setup`; both use the shared `TursoDb<M: DbSpec>` adapter.
- After both repository preflights (`ensure_git_repository` and the effective named-remote URL check), setup consumes the shared resolver's degraded result for an invalid default-discovered repo-local `.sce/config.json`; the file remains untouched and the run continues through prompts, context baseline bootstrap, lifecycle providers, hooks, and target assets. An absent config continues through the normal bootstrap path, while explicit config selections remain fatal.
- Config/DB bootstrap runs after those preflights and after context baseline bootstrap, before config/hooks dispatch, so it applies to all normal setup modes: config-only, hooks-only, combined, and interactive. If the repo-local config is invalid, target and optional-workflow persistence is skipped rather than rewriting it.

## Global access preferences

Setup saves `auth.mode` and `telemetry.enabled` only to the global user config, never to the repository `.sce/config.json` or an explicit config file ([ADR](../decisions/2026-10-21-setup-writes-access-answers-to-user-layer-only.md)). This applies to interactive runs only; non-interactive, hooks-only, and context-only runs never write access keys and print no `SCE access` section.

- **Order:** the write runs right after dispatch and before `bootstrap_context_baseline`, so it is setup's first write. Cancelling a prompt still writes nothing.
- **Writer:** `persist_global_access_preferences` (`cli/src/services/setup/access_preferences.rs`). It writes explicit answers only, preserves unrelated keys, validates the result with `parse_file_config`, and skips the write when the values are already current. The write is a sibling temp file plus a rename, keeping the existing mode (`0o600` for a new file). It is refused when the file's bytes changed after the first read. When a write is needed and the path is a symlink (valid or dangling, detected with `fs::symlink_metadata`), the writer refuses with `SymlinkManaged` and leaves the link and its target unchanged, because a rename would replace the link. A symlinked file whose values are already current is still a no-op.
- **Invalid global config:** when the file is already invalid at read time, the access prompts are skipped and nothing is written; the file stays byte-identical.
- **Write failure** (unwritable, became invalid, or concurrently modified): setup exits with a runtime error (`SCE-ERR-RUNTIME`) before `context/` or `.sce/` is touched. The error is three fixed lines: `Failed to save SCE access preferences to the global SCE config '<path>'.`, then the writer's error, then `No repository files were changed. Run 'sce config validate' to check the file, then rerun 'sce setup'.` For a symlink-managed file, the writer's error says SCE cannot modify it and to update its source directly (for example, Nix or Home Manager), and the last line is `No repository files were changed. Rerun 'sce setup' after updating the config source.`
- **Report:** after the target install, setup re-resolves the policy for the repository root and prints an `SCE access:` section between the install section and the Claude telemetry stage:
  ```
  SCE access:
  - auth.mode: <mode> (saved to global SCE config '<path>'; applies to all repositories on this machine)
  - telemetry.enabled: <bool> (saved to global SCE config '<path>'; applies to all repositories on this machine)
  - effective auth.mode: <mode> (source: <source>[, config_source: <config_source>])
  - effective telemetry: enabled | disabled (reason: <reason>)
  ```
  - When nothing was written, the two saved lines are replaced by `- saved access preferences: unchanged (global SCE config '<path>')`.
  - The source wording matches `sce config show`, and the reason strings are the [access-policy reasons](../cli/access-policy.md#reporting). The effective lines come from re-resolution, so, for example, a repository `telemetry.enabled: false` still reports `disabled (reason: repository_opt_out)` after global consent.
  - When a repository or explicit file locks auth to `disabled`, the section adds the first line of the matching `auth.disabled_by_config` message, which names that file.
  - When the effective mode is `disabled` and an `auth.db` file exists, it adds `- SCE login: stored SCE credentials remain on this machine; run 'sce auth logout' to remove them.`
  - When a saved answer suppressed a prompt in this run, it ends with the [`--reconfigure`](#reconfigure) hint line.

## Setup login

Interactive setup runs login after the `SCE access` section and before the Claude telemetry stage. It never runs for non-interactive, hooks-only, or context-only setup. The action is decided by `setup_login_action` (`cli/src/services/setup/command.rs`) from the same re-resolved effective policy the `SCE access` section reports, plus the dispatch's `SetupLoginIntent`, not from the prompt answer alone:

| Effective `auth.mode` | Login intent | Action |
|---|---|---|
| `disabled` | any | Skipped. No `auth.db` or credential-store access. |
| `mandatory` | any | Login is required. |
| `optional` | `Requested` | Login runs. |
| `optional` | `Declined` / `NotOffered` | Skipped. |

- **Login path:** the step calls `auth_command::run_login_in(<repository root>, text)`, the same flow as `sce auth login` from that directory. Stored credentials are renewed when possible, with the device flow as the fallback. Because the global access answers were saved first, telemetry provisioning and repository enrollment inside login see the new consent.
- **Output:** a section after `SCE access:` with one fixed outcome. No token or credential value is printed.
  - effective `disabled`: `SCE login: skipped; SCE authentication is disabled by config.`
  - declined: `SCE login: skipped; run 'sce auth login' to log in.`
  - not offered (an existing `auth.db` and no access prompt asked this run): `SCE login: not requested; run 'sce auth login' to log in or refresh credentials.`
  - success: the `sce auth login` text report.
  - optional failure: `SCE login: failed: <fixed UserError message>`, then `Run 'sce auth login' to retry.` Setup still succeeds. An uncatalogued login failure renders the `UnexpectedFailure` message.
- **Mandatory failure:** setup exits with `UserError::SetupLoginRequired` (`auth.setup_login_required`, `SCE-ERR-RUNTIME`, exit code 4) and keeps the login error as the source. The fixed message says local setup completed and tells the user to run `sce auth login`, then rerun `sce setup`. Installed assets and saved answers are not rolled back.
- **Claude telemetry stage:** the Claude report is a single `configured` line when the repository is locally authorized to export: `auth.db` already exists, the repository identity resolves from the setup repository root, and a valid, unexpired telemetry credential is bound to this repository for the credential's user and workspace. Otherwise it adds `Claude telemetry: not ready for this repository; ...`, which tells the user to run `sce auth login` in the repository with the SCE account it is enrolled under. The check reuses `load_repository_telemetry_credential` with non-interactive credential access under `CREDENTIAL_LOOKUP_DEADLINE`, never creates `auth.db` or a key, never rebinds a repository, and fails closed. It runs only during interactive `sce setup`, when telemetry is enabled and the install is `configured`. Non-interactive and target-flag runs (`sce setup --claude`, `--all`, with or without `--non-interactive`) never run the check, so they never open `auth.db` or the OS credential store; after a `configured` install they print only `Claude telemetry: configured; repository authorization was not checked during non-interactive setup.` and report neither authorized nor not ready. It does not depend on whether this run issued a credential, and it does not mean telemetry reached the OTLP backend (see [telemetry credential: Claude-native telemetry](telemetry-credential.md)).

## Context baseline bootstrap

- `sce setup --bootstrap-context` is a non-interactive context-only mode and must be used alone (no target, hooks, non-interactive, or `--repo` flags).
- Context-only setup ensures both repository preflights, then creates the baseline durable-context tree and exits without lifecycle providers, integration installs, or prompts; invalid default-discovered repo-local config remains untouched.
- Every normal successful setup path also calls the same additive context bootstrap after both repository preflights, before lifecycle/config install work.
- Baseline paths: `context/overview.md`, `context/architecture.md`, `context/patterns.md`, `context/glossary.md`, `context/context-map.md`, `context/plans/`, `context/handovers/`, `context/decisions/`, `context/tmp/`, and `context/tmp/.gitignore`.
- Create-if-missing only: existing files and directory contents are left untouched; missing individual paths are restored even when `context/` already exists.
- New Markdown files use neutral headings/placeholders; `context-map.md` links baseline entry points without inventing repository details; `context/tmp/.gitignore` ignores scratch content while retaining itself (`*\n!.gitignore\n`).
- Deterministic success messaging includes `Context baseline ensured.`

## Post-install integration target persistence

After config asset installation succeeds for a non-interactive target (`--opencode`, `--claude`, `--pi`, `--codex`, or `--all`), setup persists the selected target(s) into `.sce/config.json` under `integrations.target`:

- `--opencode` records `["opencode"]`.
- `--claude` adds `"claude"` to an existing array (e.g. `["opencode"]` → `["opencode", "claude"]`).
- `--pi` adds `"pi"` the same way.
- `--codex` adds `"codex"` the same way.
- `--all` records `["opencode", "claude", "pi", "codex"]` atomically. (`--both` was removed when `--all` was introduced.)
- Repeated runs are idempotent — existing targets are deduplicated; previously unrelated config keys (`$schema`, `log_level`, etc.) are preserved.
- If the config file does not exist, it is bootstrapped first, then the targets are written.
- `--hooks` only setup does not modify `integrations.target`.

## Optional-workflow selection persistence

The same write also records the run's resolved optional-workflow selection under `integrations.optional_workflows`:

- Precedence: an interactively answered multi-select is the exact selection for the run; otherwise a supplied `--workflow <slug>` list (repeatable) is; when neither is present the persisted `integrations.optional_workflows` is read back and reused, so a repeat `sce setup --claude --non-interactive` never silently uninstalls a previously selected optional workflow. The prompt's pre-checked rows come from the same persisted value (or from `--workflow` when it was supplied), so accepting the prompt unchanged records what a rerun would have kept.
- The resolved selection filters what is installed, and catalog-derived pruning (see [setup-no-backup-policy-seam.md](setup-no-backup-policy-seam.md)) removes an unselected optional workflow's command file and skill directory left behind by an earlier run, so deselection is effective on both a first-time install and a later reinstall.
- A run that resolves to an empty selection records `[]`. Deselecting is therefore expressed by installing without that slug, not by a separate uninstall step.
- The persisted set is repository-wide, not per target: a `--all` run records one selection covering `.opencode/`, `.claude/`, and `.pi/`.
- Unknown slugs are rejected during request resolution, before any file or config write.

## Interactive behavior persistence

- The two interactive confirmations are persisted with the successful target-install write at `agent_trace.auto_sync` and `policies.attribution_hooks.enabled`. Enter accepts each Yes default; an explicit No changes only its corresponding value.
- Non-interactive setup supplies no behavior selections. A newly created config therefore retains both explicit `true` bootstrap values, while an existing config's explicit values and omitted keys remain untouched.
- The write preserves unrelated top-level and nested behavior keys, existing integration merge behavior, pretty JSON formatting, and the trailing newline. Invalid default-discovered repo-local config remains byte-preserved and skips this persistence, as it does for integration selections.
- These setup values only configure the existing runtime gates: auto-sync remains a config-file-only post-commit launch choice, and attribution retains its environment/config precedence, `SCE_DISABLED` handling, overlap evidence requirement, and canonical trailer behavior.

## Implementation

- `cli/src/services/setup/mod.rs` exports `bootstrap_repo_local_config(repository_root: &Path) -> Result<()>`, `bootstrap_context_baseline(repository_root: &Path) -> Result<String>`, and `persist_integration_targets(...) -> Result<()>`, which writes integration selections and, when supplied by interactive setup, the two behavior values. `run_setup_for_mode` resolves the selection (the selection handed to it, else the persisted value read through the exported `persisted_optional_workflows`, which parses the repo-local file via `parse_file_config`) before installing and persisting it. `cli/src/services/setup/command.rs` resolves the repository root before any prompt so it can seed the interactive prompt from that persisted value, and passes the prompted selection and behavior values — when the run was interactive — to `run_setup_for_mode` ahead of the request's `--workflow` list.
- `cli/src/services/local_db/lifecycle.rs` implements `LocalDbLifecycle::setup()` for local DB initialization.
- `cli/src/services/agent_trace_db/lifecycle.rs` implements `AgentTraceDbLifecycle::setup()` for Agent Trace DB initialization.
- Repo-local config bootstrap uses `RepoPaths::sce_config_file()` and `RepoPaths::sce_dir()`; context baseline bootstrap uses the shared context accessors including `RepoPaths::context_tmp_gitignore_file()`.
- The canonical payload constant is `REPO_LOCAL_CONFIG_BOOTSTRAP_PAYLOAD`.
- `cli/src/services/setup/command.rs` resolves the effective `agent_trace.repository_remote` and runs both repository preflights before `bootstrap_context_baseline`; default-discovered invalid config is handled by the shared resolver and is not rewritten. Context-only requests return after the baseline. Normal modes then derive a repo-root-scoped `AppContext` and aggregate lifecycle providers in config → local_db → auth_db → agent_trace_db → hooks order; `ConfigLifecycle::setup()` calls `bootstrap_repo_local_config(...)`, `LocalDbLifecycle::setup()` initializes the local DB, `AuthDbLifecycle::setup()` initializes the auth DB, and `AgentTraceDbLifecycle::setup()` initializes the Agent Trace DB.

## Relationship to other setup contracts

- The Git-repo gate (`ensure_git_repository`) and effective named-remote URL preflight remain the preconditions for every setup write path, including context-only bootstrap. The gate classifies only an explicit Git `not a git repository` result and an actually missing/empty named-remote URL as typed user errors; Git/process/configuration and remote-lookup execution failures remain runtime errors with technical sources preserved.
- Default-discovered invalid repo-local config is degradable and never rewritten; explicit `--config` / `SCE_CONFIG_FILE` selections remain fatal, while absent local config remains create-if-missing.
- Context baseline bootstrap is independent of config/DB/hooks install and runs before those steps on normal setup paths.
- Local bootstrap (repo config + local DB init) is independent of config install and hook install; it runs before both after context baseline bootstrap.
- The bootstrap payload matches the `$schema` declaration accepted by startup config loading and the Pkl-authored JSON Schema embedded from Cargo `OUT_DIR`; its explicit `agent_trace.auto_sync: true` and `policies.attribution_hooks.enabled: true` values are distinct from the runtime resolver fallbacks for omitted values.

See also [the degraded discovered-config boundary decision](../decisions/2026-09-04-setup-storage-degrade-invalid-discovered-config.md) and the superseded [fail-closed boundary decision](../decisions/2026-08-26-setup-storage-fail-closed-on-invalid-config.md).
