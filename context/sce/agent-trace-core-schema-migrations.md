# Agent Trace Core Schema Migrations

## Scope

- Implements T10 for plan `agent-trace-attribution-no-git-wrapper`.
- Defines foundational local persistence schema for Agent Trace ingestion.
- Covers only core entities: `repositories`, `commits`, `trace_records`, `trace_ranges`.

## Code ownership

- Migration entrypoint: `cli/src/services/local_db.rs` (`apply_core_schema_migrations`).
- Shared local DB connection helper: `cli/src/services/local_db.rs` (`connect_local`).

## Migration contract

- Migrations are idempotent and upgrade-safe via `CREATE TABLE IF NOT EXISTS` and `CREATE INDEX IF NOT EXISTS`.
- Reapplying migrations must succeed on both empty and preexisting local DB states.
- Core schema statements are deterministic and owned in one ordered list (`CORE_SCHEMA_STATEMENTS`).

## Core tables

- `repositories`: repository identity root (`canonical_root`) plus VCS provider marker.
- `commits`: per-repository commit identity (`commit_sha`), optional parent SHA, and idempotency key capture.
- `trace_records`: canonical stored Agent Trace payload envelope per commit (content type, notes ref, payload JSON, quality status, recorded timestamp).
- `trace_ranges`: flattened line-range attribution rows linked to a trace record.

## Indexes

- `idx_commits_repository_commit_sha` on `commits(repository_id, commit_sha)`.
- `idx_trace_records_repository_commit` on `trace_records(repository_id, commit_id)`.
- `idx_trace_ranges_record_file` on `trace_ranges(trace_record_id, file_path)`.

## Verification evidence

- `nix flake check`

## Related context

- `context/sce/agent-trace-post-commit-dual-write.md`
- `context/sce/agent-trace-rewrite-trace-transformation.md`
- `context/plans/agent-trace-attribution-no-git-wrapper.md`
