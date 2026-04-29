# SCE setup local bootstrap

## Scope

Task `setup-repo-gate-and-local-config-bootstrap` T02 and `turso-local-db-sync` T04 define the local bootstrap behavior for `sce setup`.

## Behavior

- Any successful `sce setup` run in a git-backed repository creates `.sce/config.json` when the file is absent.
- The bootstrap writes the canonical schema-only JSON payload: `{"$schema": "https://sce.crocoder.dev/config.json"}` (with trailing newline).
- If `.sce/config.json` already exists, the bootstrap step returns `Ok(())` immediately and leaves the file untouched — no merge, no reformat, no overwrite.
- The parent `.sce/` directory is created via `fs::create_dir_all` if missing.
- The setup flow also bootstraps the canonical local DB by initializing `LocalDb` (which resolves the shared default local DB path and applies embedded migrations).
- The bootstrap runs after the git-repo gate (`ensure_git_repository`) and before config/hooks dispatch, so it applies to all setup modes: config-only, hooks-only, combined, and interactive.

## Implementation

- `cli/src/services/setup/mod.rs` exports `bootstrap_repo_local_config(repository_root: &Path) -> Result<()>`.
- `cli/src/services/setup/mod.rs` exports `bootstrap_local_db() -> Result<()>`.
- The function uses `RepoPaths::sce_config_file()` and `RepoPaths::sce_dir()` from `default_paths` for path resolution.
- The canonical payload constant is `REPO_LOCAL_CONFIG_BOOTSTRAP_PAYLOAD`.
- `cli/src/app.rs` calls `services::setup::bootstrap_repo_local_config(&repository_root)` and then `services::setup::bootstrap_local_db()` in `Command::Setup` dispatch, immediately after `ensure_git_repository`.

## Relationship to other setup contracts

- The git-repo gate (`ensure_git_repository`) was introduced in T01 of the same plan.
- Local bootstrap (repo config + local DB init) is independent of config install and hook install; it runs before both.
- The bootstrap payload matches the `$schema` declaration accepted by the config service's startup config loading and the Pkl-authored JSON Schema at `config/schema/sce-config.schema.json`.
