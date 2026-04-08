# Local DB empty-file baseline

## Scope

- Current state after `agent-trace-removal-and-hook-noop-reset` T01.
- Defines the minimal local DB runtime baseline for the current neutral local DB path.
- Covers file creation/open behavior only; schema tables and migrations are not active.

## Code ownership

- Runtime bootstrap seam: `cli/src/services/local_db.rs` (`ensure_local_db_ready_blocking`).
- Shared local DB connection helper: `cli/src/services/local_db.rs` (`connect_local`).

## Current contract

- `resolve_local_db_path` resolves the canonical per-user state path.
- The retained bootstrap seam opens/creates the local Turso file and returns its path when future runtime callers invoke it.
- No schema bootstrap runs.
- No trace, reconciliation, retry, or prompt tables are created as part of runtime readiness.

## Observable consequences

- A newly created DB file is empty until another future task introduces schema creation.
- Doctor/runtime callers may still ensure the file exists, but they must not assume DB tables are present.
- Local DB persistence adapters that previously wrote trace or reconciliation rows are disconnected in the current runtime.

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
