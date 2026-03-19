# MCP Smart Cache Storage Foundation

## Scope

- This file documents the implemented `sce-mcp-smart-cache-engine` T01 storage foundation in `cli/src/services/mcp.rs`.

## Implemented contract

- Repository root detection uses `git rev-parse --show-toplevel` from the current working directory and returns actionable non-git errors.
- Smart Cache Engine state is rooted at `<state_root>/sce/cache`, where `state_root` follows the existing platform-dependent resolver in `cli/src/services/local_db.rs`.
- Per-repository cache state lives at `<state_root>/sce/cache/repos/<sha256(repo_root)>/cache.db`.
- The cache feature keeps a deterministic global config map at `<state_root>/sce/cache/config.json` keyed by canonical repository root path.
- Config entries currently persist `repository_hash` and `cache_db_path` for each tracked repository.

## Bootstrapped schema

- `file_versions`: canonical per-repository file fingerprint storage (`content_hash`, line count, byte count, timestamps).
- `session_reads`: per-session read tracking keyed by session + repository + relative path, including forced-read and token-savings placeholders for later tasks.
- `cache_stats`: per-repository aggregate counters for tracked files and token-savings totals.

## Verification coverage

- Unit tests cover nested-directory repo-root detection, stable repository hash generation, platform-state-root cache path resolution, config persistence, and DB schema bootstrap.
- Prefer repository-level verification/build entrypoints over direct `cargo` commands for this feature work.
- Verification/build commands for this task slice: `nix flake check` and `nix build .#default`
