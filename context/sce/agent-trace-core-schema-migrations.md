# Retired Local DB Empty-File Baseline

Historical reference only. The active local DB migration contract is now documented in [local-db.md](local-db.md).

## Scope

- Captures the state after `agent-trace-removal-and-hook-noop-reset` T01.
- Defines the retired minimal local DB runtime baseline for the neutral local DB path.
- The file creation/open-only baseline is no longer the active local DB schema state.

## Code ownership

- Retired runtime bootstrap seam: `cli/src/services/local_db.rs` (`ensure_local_db_ready_blocking`).
- Retired shared local DB connection helper: `cli/src/services/local_db.rs` (`connect_local`).

## Current contract

- Historical behavior: `resolve_local_db_path` resolved the canonical per-user state path.
- Historical behavior: the retained bootstrap seam opened/created the local Turso file and returned its path when runtime callers invoked it.
- Historical behavior: no schema bootstrap ran.
- Current active behavior: `LocalDb::new()` runs embedded migrations from `cli/migrations/`.

## Observable consequences

- This file should not be used as current-state guidance for local DB schema behavior.
- Current local DB schema behavior is documented in [local-db.md](local-db.md).
- Local hook runtime trace persistence remains disconnected unless explicitly documented elsewhere.

## Removed behavior

- `apply_core_schema_migrations` is no longer part of the active runtime surface.
- The ordered schema statement list for Agent Trace persistence is no longer present in `cli/src/services/local_db.rs`.
- Hosted reconciliation schema ingestion is also absent from the current local DB bootstrap path.

## Verification evidence

- `nix flake check`

## Related context

- `context/sce/agent-trace-post-commit-dual-write.md`
- `context/sce/agent-trace-rewrite-trace-transformation.md`
- `context/plans/agent-trace-attribution-no-git-wrapper.md`
