# Agent Trace Reconciliation Schema Ingestion

## Scope

- Implements T11 for plan `agent-trace-attribution-no-git-wrapper`.
- Adds hosted-rewrite persistence schema slices for reconciliation bookkeeping and replay-safe mapping ingestion.
- Covers schema/migration behavior only; provider webhook transport remains out of scope.

## Code ownership

- Migration entrypoint: `cli/src/services/local_db.rs` (`apply_core_schema_migrations`).
- Schema statement source of truth: `cli/src/services/local_db.rs` (`CORE_SCHEMA_STATEMENTS`).

## Migration contract

- New reconciliation entities are installed idempotently with `CREATE TABLE IF NOT EXISTS` and `CREATE INDEX IF NOT EXISTS`.
- Reapplying migrations on preexisting DB state remains upgrade-safe.
- Run-level and mapping-level replay protection is enforced through per-repository idempotency uniqueness.

## Reconciliation tables

- `reconciliation_runs`: stores provider, run status lifecycle, run timing, and per-repository idempotency key.
- `rewrite_mappings`: stores old/new commit SHA mapping outcomes, confidence, mapping status, and per-repository idempotency key per mapping row.
- `conversations`: stores canonical conversation URLs per repository/source for hosted reconciliation linkage.

## Indexes

- `idx_reconciliation_runs_repository_status` on `reconciliation_runs(repository_id, status)`.
- `idx_rewrite_mappings_run_old_sha` on `rewrite_mappings(reconciliation_run_id, old_commit_sha)`.
- `idx_rewrite_mappings_repository_old_sha` on `rewrite_mappings(repository_id, old_commit_sha)`.
- `idx_conversations_repository_source` on `conversations(repository_id, source)`.

## Validation coverage

- Schema existence + index presence checks in `core_schema_migrations_create_required_tables_and_indexes`.
- Upgrade-safe reapplication checks in `core_schema_migrations_are_upgrade_safe_for_preexisting_state`.
- Reconciliation replay/query checks in `reconciliation_schema_supports_replay_safe_runs_and_mapping_queries`.

## Verification evidence

- `cargo fmt --manifest-path cli/Cargo.toml -- --check`
- `cargo test --manifest-path cli/Cargo.toml core_schema_migrations`
- `cargo test --manifest-path cli/Cargo.toml reconciliation_schema_supports_replay_safe_runs_and_mapping_queries`
- `cargo build --manifest-path cli/Cargo.toml`

## Related context

- `context/sce/agent-trace-core-schema-migrations.md`
- `context/sce/agent-trace-rewrite-trace-transformation.md`
- `context/plans/agent-trace-attribution-no-git-wrapper.md`
