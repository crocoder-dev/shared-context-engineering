# SCE setup local bootstrap

## Scope

Task `setup-repo-gate-and-local-config-bootstrap` T02 and `turso-local-db-sync` T04 define the local bootstrap behavior for `sce setup`.

## Behavior

- Any successful `sce setup` run in a git-backed repository creates `.sce/config.json` when the file is absent.
- The bootstrap writes the canonical schema-only JSON payload: `{"$schema": "https://sce.crocoder.dev/config.json"}` (with trailing newline).
- If `.sce/config.json` already exists, the bootstrap step returns `Ok(())` immediately and leaves the file untouched — no merge, no reformat, no overwrite.
- The parent `.sce/` directory is created via `fs::create_dir_all` if missing.
- The setup flow also bootstraps the canonical local DB through `LocalDbLifecycle::setup` and the Agent Trace DB through `AgentTraceDbLifecycle::setup`; both use the shared `TursoDb<M: DbSpec>` adapter.
- The bootstrap runs after the git-repo gate (`ensure_git_repository`) and before config/hooks dispatch, so it applies to all setup modes: config-only, hooks-only, combined, and interactive.

## Implementation

- `cli/src/services/setup/mod.rs` exports `bootstrap_repo_local_config(repository_root: &Path) -> Result<()>`.
- `cli/src/services/local_db/lifecycle.rs` implements `LocalDbLifecycle::setup()` for local DB initialization.
- `cli/src/services/agent_trace_db/lifecycle.rs` implements `AgentTraceDbLifecycle::setup()` for Agent Trace DB initialization.
- The function uses `RepoPaths::sce_config_file()` and `RepoPaths::sce_dir()` from `default_paths` for path resolution.
- The canonical payload constant is `REPO_LOCAL_CONFIG_BOOTSTRAP_PAYLOAD`.
- `cli/src/services/setup/command.rs` derives a repo-root-scoped `AppContext` after `ensure_git_repository`, then aggregates lifecycle providers in config → local_db → agent_trace_db → hooks order; `ConfigLifecycle::setup()` calls `bootstrap_repo_local_config(...)`, `LocalDbLifecycle::setup()` initializes the local DB, and `AgentTraceDbLifecycle::setup()` initializes the Agent Trace DB.

## Relationship to other setup contracts

- The git-repo gate (`ensure_git_repository`) was introduced in T01 of the same plan.
- Local bootstrap (repo config + local DB init) is independent of config install and hook install; it runs before both.
- The bootstrap payload matches the `$schema` declaration accepted by the config service's startup config loading and the Pkl-authored JSON Schema at `config/schema/sce-config.schema.json`.
