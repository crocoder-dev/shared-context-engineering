# SCE setup local bootstrap

## Scope

Task `setup-repo-gate-and-local-config-bootstrap` T02, `turso-local-db-sync` T04, and `setup-bootstrap-context` T01 define the local bootstrap behavior for `sce setup`.

## Behavior

- Any successful `sce setup` run in a git-backed repository creates `.sce/config.json` when the file is absent.
- The bootstrap writes the canonical schema-only JSON payload: `{"$schema": "https://sce.crocoder.dev/config.json"}` (with trailing newline).
- If `.sce/config.json` already exists, the bootstrap step returns `Ok(())` immediately and leaves the file untouched — no merge, no reformat, no overwrite.
- The parent `.sce/` directory is created via `fs::create_dir_all` if missing.
- The setup flow also bootstraps the canonical local DB through `LocalDbLifecycle::setup` and the Agent Trace DB through `AgentTraceDbLifecycle::setup`; both use the shared `TursoDb<M: DbSpec>` adapter.
- After both repository preflights (`ensure_git_repository` and the effective named-remote URL check), setup validates an existing repo-local `.sce/config.json` before prompts, context baseline bootstrap, lifecycle providers, hooks, or target assets. Invalid config stops the run without those side effects; an absent config continues through the normal bootstrap path.
- Config/DB bootstrap runs after those preflights and config validation, and after context baseline bootstrap, before config/hooks dispatch, so it applies to all normal setup modes: config-only, hooks-only, combined, and interactive.

## Context baseline bootstrap

- `sce setup --bootstrap-context` is a non-interactive context-only mode and must be used alone (no target, hooks, non-interactive, or `--repo` flags).
- Context-only setup ensures both repository preflights, validates an existing repo-local config, then creates the baseline durable-context tree and exits without lifecycle providers, integration installs, or prompts.
- Every normal successful setup path also calls the same additive context bootstrap after both repository preflights and config validation, before lifecycle/config install work.
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

## Implementation

- `cli/src/services/setup/mod.rs` exports `bootstrap_repo_local_config(repository_root: &Path) -> Result<()>`, `bootstrap_context_baseline(repository_root: &Path) -> Result<String>`, and `persist_integration_targets(repository_root: &Path, target: SetupTarget, selected_optional_workflows: &[String]) -> Result<()>`, which writes both `integrations.target` and `integrations.optional_workflows`. `run_setup_for_mode` resolves the selection (the selection handed to it, else the persisted value read through the exported `persisted_optional_workflows`, which parses the repo-local file via `parse_file_config`) before installing and persisting it. `cli/src/services/setup/command.rs` resolves the repository root before any prompt so it can seed the interactive prompt from that persisted value, and passes the prompted selection — when the run was interactive — to `run_setup_for_mode` ahead of the request's `--workflow` list.
- `cli/src/services/local_db/lifecycle.rs` implements `LocalDbLifecycle::setup()` for local DB initialization.
- `cli/src/services/agent_trace_db/lifecycle.rs` implements `AgentTraceDbLifecycle::setup()` for Agent Trace DB initialization.
- Repo-local config bootstrap uses `RepoPaths::sce_config_file()` and `RepoPaths::sce_dir()`; context baseline bootstrap uses the shared context accessors including `RepoPaths::context_tmp_gitignore_file()`.
- The canonical payload constant is `REPO_LOCAL_CONFIG_BOOTSTRAP_PAYLOAD`.
- `cli/src/services/setup/command.rs` resolves the effective `agent_trace.repository_remote`, runs both repository preflights, and validates an existing repo-local config before `bootstrap_context_baseline`. Context-only requests return after the baseline. Normal modes then derive a repo-root-scoped `AppContext` and aggregate lifecycle providers in config → local_db → auth_db → agent_trace_db → hooks order; `ConfigLifecycle::setup()` calls `bootstrap_repo_local_config(...)`, `LocalDbLifecycle::setup()` initializes the local DB, `AuthDbLifecycle::setup()` initializes the auth DB, and `AgentTraceDbLifecycle::setup()` initializes the Agent Trace DB.

## Relationship to other setup contracts

- The Git-repo gate (`ensure_git_repository`), effective named-remote URL preflight, and existing-config validation remain the preconditions for every setup write path, including context-only bootstrap. The gate classifies only an explicit Git `not a git repository` result and an actually missing/empty named-remote URL as typed user errors; Git/process/configuration and remote-lookup execution failures remain runtime errors with technical sources preserved.
- The repo-local config preflight is fail-closed only for an existing invalid config, while absent config remains create-if-missing.
- Context baseline bootstrap is independent of config/DB/hooks install and runs before those steps on normal setup paths.
- Local bootstrap (repo config + local DB init) is independent of config install and hook install; it runs before both after context baseline bootstrap.
- The bootstrap payload matches the `$schema` declaration accepted by startup config loading and the Pkl-authored JSON Schema embedded from Cargo `OUT_DIR`.

See also [the fail-closed boundary decision](../decisions/2026-08-26-setup-storage-fail-closed-on-invalid-config.md).
