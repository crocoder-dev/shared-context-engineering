# CLI Default Path Catalog

`cli/src/services/default_paths.rs` is the canonical owner for production CLI path definitions.

## Scope

- per-user persisted paths
- repo-relative CLI paths
- install target paths
- embedded asset paths
- context-document path constants used by CLI-owned workflows

## Current path families

### Per-user persisted paths

- global config: `<config_root>/sce/config.json`
- auth DB: `<state_root>/sce/auth.db`
- local DB: `<state_root>/sce/local.db`
- active repository-scoped agent trace DB: `<state_root>/sce/repos/{repository_id}/agent-trace.db` via `agent_trace_db_path_for_repository(repository_id)` (plus `_at(state_root, repository_id)` for explicit roots); rejects empty or path-unsafe repository IDs
- legacy/global agent trace DB fallback: `<state_root>/sce/agent-trace.db`
- legacy per-checkout agent trace DB: `<state_root>/sce/agent-trace-{checkout_id}.db`

### Repo-relative paths

- `.sce/`, `.sce/config.json`, `.sce/sce.log`
- `.opencode/`, `.opencode/opencode.json`
- `.claude/`
- `.pi/`
- `.git/`, `.git/hooks/`, `.git/COMMIT_EDITMSG`
- `context/`, `context/plans/`, `context/decisions/`, `context/handovers/`, `context/tmp/`

### Embedded/install paths

- `assets/generated/config/`
- `assets/hooks/`
- OpenCode plugin/catalog targets under `.opencode/`
- required git hook install targets under `.git/hooks/`

## Contract

- Production CLI code should define named path accessors or constants in `default_paths.rs`, not introduce new hardcoded path owners elsewhere.
- `cli/src/services/config/mod.rs` now resolves the default repo-local config path through `RepoPaths::sce_config_file()` during config discovery.
- `cli/src/services/doctor/inspect.rs` now resolves the repo-local config path through `RepoPaths::sce_config_file()` for local-config health reporting and validation.
- `cli/src/services/doctor/inspect.rs` also resolves OpenCode manifest/plugin/preset locations through shared `RepoPaths` and `InstallTargetPaths` accessors instead of owning those paths locally.
- `cli/src/services/setup/mod.rs` now resolves setup target directory names and required hook identifiers through `default_paths.rs` constants/accessors instead of owning those path literals locally.
- `cli/src/services/default_paths.rs` includes a regression test that scans non-test Rust source under `cli/src/` and fails when new centralized production path literals appear outside the default-path service.
- Active hook runtime no longer resolves or writes collision-safe JSON artifacts under `context/tmp/`; `context/tmp/` remains a repo-relative scratch/session path owned by the default path catalog.
- Active hook runtime and `cli/src/services/agent_trace_db/lifecycle.rs` resolve repository-scoped Agent Trace DB files through `agent_trace_storage` and `agent_trace_db_path_for_repository(repository_id)`.
- `cli/src/services/checkout/mod.rs` retains `agent_trace_db_path_for_checkout(checkout_id)` only for legacy per-checkout DB helper code; active setup/hooks no longer select checkout-scoped paths for new writes.

See also: [cli-command-surface.md](./cli-command-surface.md), [../architecture.md](../architecture.md), [../context-map.md](../context-map.md)
