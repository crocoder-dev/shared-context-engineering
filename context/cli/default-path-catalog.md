# CLI Default Path Catalog

`cli/src/services/default_paths.rs` is the canonical owner for production CLI path definitions.

## Scope

- per-user persisted paths
- repo-relative CLI paths
- install/runtime target paths
- embedded asset paths
- context-document path constants used by CLI-owned workflows

## Current path families

### Per-user persisted paths

- global config: `<config_root>/sce/config.json`
- auth tokens: `<state_root>/sce/auth/tokens.json`
- local DB: `<state_root>/sce/local.db`
- agent-trace DB: `<state_root>/sce/agent-trace.db`

### Repo-relative paths

- `.sce/`, `.sce/config.json`, `.sce/sce.log`
- `.opencode/`, `.opencode/opencode.json`
- `.claude/`
- `.git/`, `.git/hooks/`, `.git/COMMIT_EDITMSG`
- `context/`, `context/plans/`, `context/decisions/`, `context/handovers/`, `context/tmp/`

### Embedded/install paths

- `assets/generated/config/`
- `assets/hooks/`
- OpenCode plugin/runtime/catalog targets under `.opencode/`
- required git hook install targets under `.git/hooks/`

## Contract

- Production CLI code should define named path accessors or constants in `default_paths.rs`, not introduce new hardcoded path owners elsewhere.
- `cli/src/services/config.rs` now resolves the default repo-local config path through `RepoPaths::sce_config_file()` during config discovery.
- `cli/src/services/doctor/inspect.rs` now resolves the repo-local config path through `RepoPaths::sce_config_file()` for local-config health reporting and validation.
- `cli/src/services/doctor/inspect.rs` also resolves OpenCode manifest/plugin/runtime/preset locations through shared `RepoPaths` and `InstallTargetPaths` accessors instead of owning those paths locally.
- `cli/src/services/setup.rs` now resolves setup target directory names and required hook identifiers through `default_paths.rs` constants/accessors instead of owning those path literals locally.
- `cli/src/services/default_paths.rs` includes a regression test that scans non-test Rust source under `cli/src/` and fails when new centralized production path literals appear outside the default-path service.

See also: [cli-command-surface.md](./cli-command-surface.md), [../architecture.md](../architecture.md), [../context-map.md](../context-map.md)
