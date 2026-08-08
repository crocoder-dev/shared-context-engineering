# Plan: stable-agent-trace-source-instance-identity

## Change summary

Extend the repository-scoped Agent Trace database metadata so each independently created database lineage has a stable `source_instance_id` in addition to the logical `repository_id`. New databases generate one UUID-style value once; existing databases preserve it across reopen and setup, while migrated baseline databases acquire one exactly once through the supported setup/initialization path. The repository adapter will expose both metadata values through a typed API and will retain repository-mismatch failures.

Add a repository Agent Trace migration after the existing baseline without rewriting migration `001`. Use the existing UUID dependency and atomic conditional initialization so concurrent callers converge on the stored source identity. Keep hook/readiness paths non-mutating with respect to schema migrations: an old database lacking the required migration reports the existing setup guidance, while source-instance repair occurs only through the safe initialization/setup path. Update focused Agent Trace context documentation to distinguish logical repository identity, database-lineage identity, and checkout identity.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the check that proves it. `/validate` runs these checks; no task in the stack performs final validation.

- [x] AC1: A newly initialized repository Agent Trace database stores the expected `repository_id` and one valid, non-empty UUID-style `source_instance_id`; reopening it and repeating setup never changes either value.
  - Validate: Repository adapter/storage tests create, reopen, and repeatedly initialize the same database and assert identical typed metadata values.
- [x] AC2: Two independently created database files for the same `repository_id` receive different source-instance identities, while clones/worktrees that resolve the same physical repository-scoped file observe the same identity.
  - Validate: Storage/adapter tests compare metadata from two explicit paths and from repeated resolution of one repository-scoped path.
- [x] AC3: Existing repository databases with the old baseline metadata receive one persisted source identity through the supported migration/setup path, and that identity remains stable after reopening.
  - Validate: A fixture created with migration `001_repository_schema` is upgraded through setup initialization; the test asserts the new column/value and stable reopen result.
- [x] AC4: Concurrent initialization of one missing source identity has one persisted winner and every caller returns that same `source_instance_id`; no caller replaces an existing valid identity.
  - Validate: Concurrent adapter/storage initialization tests assert all returned metadata values equal the value read from `repository_metadata` afterward.
- [x] AC5: Repository metadata validation still rejects a database whose stored `repository_id` differs from the expected repository, and valid existing metadata is returned rather than regenerated.
  - Validate: Existing mismatch coverage plus new typed metadata tests assert the stable mismatch diagnostic and preservation behavior.
- [x] AC6: Normal hook/readiness opens do not apply migrations or silently repair an old schema; a database missing the required migration fails readiness with actionable `sce setup` guidance, while source-instance initialization is available through the setup-safe path.
  - Validate: Hook/readiness tests open an old-schema fixture without migrations, assert no schema mutation and setup guidance, and separately exercise setup initialization/repair.
- [x] AC7: Downstream Rust callers can obtain `repository_id` and `source_instance_id` from `RepositoryAgentTraceDb` through the typed metadata API rather than ad hoc metadata SQL.
  - Validate: Compile-time/use-site tests call the adapter metadata API and assert its `RepositoryMetadata` result.
- [x] AC8: Agent Trace documentation explains the distinction between logical repository identity, source database-lineage identity, and checkout identity, including shared physical DB behavior, independent DB behavior, and the future ETL lineage tuple.
  - Validate: Documentation inspection of the updated Agent Trace DB/storage context and synchronized root context confirms all required distinctions and tuple terminology.

### Full validation

Repository-wide checks `/validate` runs after the last task, regardless of which criterion they map to.

- `nix flake check`
- `nix run .#pkl-check-generated`
- `git diff --check`

### Context sync

- Update `context/sce/agent-trace-db.md` and `context/cli/agent-trace-storage.md` with the migration, typed metadata, initialization, and hot-path readiness contract.
- Update `context/overview.md`, `context/architecture.md`, `context/glossary.md`, and `context/context-map.md` where their current repository Agent Trace claims say the schema is fresh-only, source identity is absent, or hooks may migrate without the new readiness boundary.

## Constraints and non-goals

- **In scope:** `repository_metadata` schema/migrations, `RepositoryAgentTraceDb` metadata initialization and validation, repository storage/lifecycle/hook readiness boundaries, focused Rust tests, and Agent Trace context documentation.
- **Out of scope:** Turso Sync, ETL/DWH schemas, watermarks, `sce trace sync`, control-plane or machine registration, device syncing, archive logic, row-schema changes solely for ETL, and checkout identity changes.
- **Constraints:** Preserve migration `001_repository_schema`; add a later repository migration. Persist UUID-style text generated with the existing `uuid` crate (UUIDv4 is the current acceptable convention). Never derive the source identity from repository ID, checkout ID, remote, hostname, or path. Preserve repository mismatch diagnostics and the existing setup/readiness lifecycle. Hook/read paths must not run schema migrations.
- **Non-goal:** Do not make `source_instance_id` a checkout identity or create a new database per clone/worktree; all callers using the same physical repository-scoped database share its source identity.

## Assumptions

- The existing `uuid` dependency and its `v4` feature are sufficient; no new dependency is needed.
- SQLite's staged migration limitation is handled by adding the column with a temporary empty/default storage value, then atomically filling it in initialization code and enforcing non-empty/valid UUID semantics through typed validation/readiness. The migration must not invent a pseudo-UUID.
- The current shared storage resolver needs an explicit setup/migration-capable path separate from hook/readiness resolution so existing lazy first-open behavior remains supported without allowing old schemas to be silently migrated from a hot path.

## Task stack

- [x] T01: `Add repository source-instance migration and typed metadata primitives` (status:done)
  - Task ID: T01
  - Goal: Add the post-baseline repository migration and the typed `RepositoryMetadata`/UUID validation primitives needed by the repository adapter.
  - Boundaries (in/out of scope): In — `002` migration under `cli/migrations/agent-trace-repository/`, UUIDv4 generation using the existing dependency, typed metadata representation and validation helpers, build-generated migration wiring as required, and unit-level pure validation coverage. Out — hook/lifecycle call-site behavior, ETL, and documentation.
  - Dependencies: none
  - Done when: The migration extends `repository_metadata` without modifying `001`, generated migration discovery includes it, the storage shape is compatible with old rows that need initialization, and the Rust layer can generate/validate a UUID-style source identity without deriving it from repository data.
  - Verification notes (commands or checks): `nix flake check`; targeted Agent Trace compilation/tests through the repository Cargo wrapper if needed.
  - Evidence: Added `cli/migrations/agent-trace-repository/002_repository_source_instance_id.sql` (`ALTER TABLE repository_metadata ADD COLUMN source_instance_id TEXT NOT NULL DEFAULT ''`, additive to `001`). Added `RepositoryMetadata { repository_id, source_instance_id }`, `generate_source_instance_id()` (UUIDv4 via the existing `uuid` crate), and `is_valid_source_instance_id()` to `cli/src/services/agent_trace_db/repository.rs` (all `#[allow(dead_code)]` pending T02 call-site wiring). Updated the module doc comment and the `open_at_initializes_the_full_schema_from_one_migration` test (renamed `..._from_all_migrations`) to reflect the now-two-migration chain; added tests for ID generation/uniqueness, placeholder rejection, malformed-value rejection, and the fresh-row placeholder value. Migration discovery required no build.rs change — `build.rs` already discovers migration files per directory by numeric filename prefix. Verification: `nix flake check` passed (clippy, fmt, cli-tests, pkl-generated); targeted `repository::` module tests 12/12 passed; `git diff --check` clean. No deviations from the reviewed task boundaries.

- [x] T02: `Integrate atomic metadata initialization and migration-safe storage paths` (status:done)
  - Task ID: T02
  - Goal: Update `RepositoryAgentTraceDb` and repository storage/lifecycle integration so fresh, existing, and concurrently racing initializations converge on one stable typed metadata record while hot readiness paths remain non-migrating.
  - Boundaries (in/out of scope): In — metadata insert/conditional repair/validation API, preservation of repository mismatch errors, setup-safe initialization of missing source identity, explicit distinction between migration-running setup and no-migration hook/readiness opens, actionable setup guidance for old schemas, and propagation of metadata where the existing storage/lifecycle result surfaces need it. Out — changing Agent Trace row schemas, adding ETL consumers, or changing checkout identity.
  - Dependencies: T01
  - Done when: New DB initialization atomically seeds repository and source identities; existing valid identities are returned unchanged; missing identities are conditionally filled once; races return the stored winner; setup can upgrade old repository baselines; no-migration hook/readiness access neither applies the new migration nor replaces an old-schema error with a silent migration.
  - Verification notes (commands or checks): Targeted `agent_trace_db`, `agent_trace_storage`, lifecycle, and hook/readiness tests via `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml ...` as needed; inspect SQL/schema metadata before and after no-migration opens.
  - Evidence: `RepositoryAgentTraceDb::verify_or_initialize_repository_metadata` (`cli/src/services/agent_trace_db/repository.rs`) now returns typed `RepositoryMetadata`, atomically claims a missing `source_instance_id` via a conditional `UPDATE ... WHERE source_instance_id = ''` (race-safe: every caller re-reads the stored winner), and leaves an existing valid identity untouched; repository-ID mismatch diagnostics are preserved. `cli/src/services/agent_trace_storage/mod.rs` now exposes two resolution paths sharing one `open_storage` core: the existing `resolve_agent_trace_storage`/`_at_state_root` (setup/lifecycle/diagnostic callers) still create and migrate via `open_repository_db_concurrently_safe`, while new `resolve_agent_trace_storage_for_hook_runtime`/`_at_state_root` (used by `hooks/mod.rs::open_agent_trace_db_for_hook_runtime`) call `open_repository_db_for_hook_runtime`, which only opens without migrating, calls `ensure_schema_ready_for_hooks`, and fails with the existing `sce setup` guidance on any missing/incomplete schema — never repairing or migrating. `ResolvedAgentTraceStorage` gained a `pub metadata: RepositoryMetadata` field, propagated into `lifecycle.rs`'s setup message (`Agent Trace source-instance identity: ...`). Removed the now-unneeded `#[allow(dead_code)]` on `RepositoryMetadata`/`generate_source_instance_id`/`is_valid_source_instance_id`. Added/updated tests: `repository.rs::fresh_repository_database_receives_a_valid_source_instance_id_once` (fresh DB gets a valid identity that repeated setup never changes); `agent_trace_storage::hook_runtime_resolution_fails_with_setup_guidance_before_setup_ran` (hook path on a missing DB fails with `sce setup` guidance and creates no schema); `agent_trace_storage::hook_runtime_resolution_succeeds_and_reuses_metadata_after_setup` (hook path after setup returns identical metadata). Verification: `nix flake check` passed (clippy, fmt, cli-tests, pkl-generated); `nix run .#pkl-check-generated` passed; targeted `agent_trace_db::repository::` (12/12), `agent_trace_storage::` (13/13), `services::hooks` (8/8), and `trace::` (35/35) tests passed; `git diff --check` clean. No deviations from the reviewed task boundaries; `sce trace status` and `sce setup` intentionally keep the existing create/migrate-capable resolver, since only the hooks git-hook runtime entry point is the "high-frequency hook path" the plan's non-migrating constraint targets.

- [x] T03: `Add source-instance regression and concurrency coverage` (status:done)
  - Task ID: T03
  - Goal: Cover the complete PR behavior with focused repository adapter/storage tests, including migration fixtures and concurrent missing-identity initialization.
  - Boundaries (in/out of scope): In — fresh DB, stable reopen, repeated setup, same-repository independent files, wrong repository, old-schema migration/setup, concurrent convergence, and old-schema hook/readiness guidance tests; add only small testability seams required by those cases. Out — broad refactors, ETL tests, or full-suite-only cleanup.
  - Dependencies: T02
  - Done when: Every requested behavior has deterministic automated coverage, including assertions that identities are persisted and never replaced and that the no-migration path leaves an old database schema unchanged.
  - Verification notes (commands or checks): Narrow module/name-filtered tests through `scripts/run-cli-cargo.sh`; run the relevant Crane-backed test derivation when filesystem/database tests require the repository validation environment.
  - Evidence: Fresh DB/stable-reopen/repeated-setup and wrong-repository coverage already existed from T01/T02 and needed no changes. Added the remaining gaps: `repository.rs::independently_created_databases_for_the_same_repository_receive_different_source_instance_ids` (AC2 — two `new_at`-created DBs with the same `repository_id` get different, both-valid `source_instance_id`s); `repository.rs::baseline_only_fixture_gains_a_stable_source_instance_id_through_setup_migration` (AC3 — a fixture seeded with only `001_repository_schema` plus a baseline metadata row is upgraded via `run_migrations()` and gets a stable identity across reopen); `repository.rs::concurrent_missing_source_instance_id_initialization_converges_on_one_persisted_winner` (AC4 — 8 threads, each with its own connection to one fresh, unseeded DB file, call `verify_or_initialize_repository_metadata` behind a `Barrier`; every returned value equals the value read back afterward); `agent_trace_storage/mod.rs::hook_runtime_resolution_fails_with_setup_guidance_on_a_baseline_only_schema_without_mutating_it` (AC6 — a migration-`002`-missing fixture at the resolved hook DB path fails `resolve_agent_trace_storage_for_hook_runtime_at_state_root` with `sce setup` guidance and `__sce_migrations` still shows only `001_repository_schema` afterward, proving no migration ran). Testability seam: added `TursoDb::<M>::run_migrations_up_to(count)` (`cli/src/services/db/mod.rs`, `#[cfg(test)]`) so tests can build a fixture that predates later migrations without hand-rolling batch SQL execution; it reuses the existing private `run_embedded_migrations` helper against a `&M::migrations()[..count]` slice. Verification: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_db::repository::` (15/15 passed) and `... agent_trace_storage::` (14/14 passed); full `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml` (193/193 passed); `nix flake check` passed (one transient, reproducibility-confirmed flake in the sandboxed `cli-tests` derivation on the first run, passing cleanly on immediate rebuild — not caused by these changes, no code change made in response); `git diff --check` clean. No deviations from the reviewed task boundaries.

- [x] T04: `Document repository and source-instance lineage semantics` (status:done)
  - Task ID: T04
  - Goal: Update durable Agent Trace documentation to describe source-instance identity and the final migration/readiness contract.
  - Boundaries (in/out of scope): In — Agent Trace DB and storage context plus required root context summaries/map/glossary/architecture repairs, including `(repository_id, source_instance_id, source_table, source_row_id)` future ETL lineage wording and the explicit non-checkout identity distinction. Out — implementation behavior, ETL/DWH work, and unrelated context cleanup.
  - Dependencies: T03
  - Done when: Documentation states that same physical repository-scoped DB users share `source_instance_id`, independently created DB files differ, clones/worktrees share it only when they share the DB, and `source_instance_id` is not checkout identity; current migration and hot-path claims match code.
  - Verification notes (commands or checks): Review the changed context files against the adapter/storage code; run `git diff --check` and the generated-output check if any generated contract is touched.
  - Evidence: `context/sce/agent-trace-db.md` and `context/cli/agent-trace-storage.md` already carried the two-migration chain, typed `RepositoryMetadata`, and hook-runtime no-migration contract from T01–T03's context sync; `context/glossary.md` already had an accurate `source-instance identity` entry distinguishing repository/checkout/source-instance identity — reviewed, no changes needed. Found and fixed three stale/incomplete root-context claims: `context/overview.md` said hook runtime "lazily creates or upgrades" the repository DB (now states it resolves through a separate no-migration path that never creates, migrates, or repairs, failing with `sce setup` guidance); `context/architecture.md`'s and `context/context-map.md`'s `agent_trace_db`/`agent-trace-db.md` descriptions only mentioned the `001_repository_schema.sql` baseline (now note the additive `002_repository_source_instance_id` migration and typed `RepositoryMetadata`). Added the missing AC8 future-ETL-lineage wording to `context/sce/agent-trace-db.md`: a new paragraph states `source_instance_id` identifies a database-lineage (shared by same-physical-DB clones/worktrees, distinct per independently created DB file), is distinct from diagnostic-only never-persisted `checkout_id`, and that a future ETL/DWH consumer is expected to key row provenance on `(repository_id, source_instance_id, source_table, source_row_id)`. Verification: reviewed each changed file's claims against `repository.rs` (`verify_or_initialize_repository_metadata`, migration `002`) and `agent_trace_storage/mod.rs` (hook-runtime-safe vs. setup-safe resolution) from T01–T03; `git diff --check` clean. No deviations from the reviewed task boundaries.

## Open questions

None. The request specifies the identity representation, migration ordering, lifecycle boundary, required tests, and non-goals; the only SQLite detail is recorded as an implementation assumption rather than a scope decision.

## Validation Report

**Status:** validated  
**Date:** 2026-08-08

### Commands run

- `nix flake check` -> exit 0 (all checks passed)
- `nix run .#pkl-check-generated` -> exit 0 ("Ephemeral Pkl generation passed: 101 files")
- `git diff --check` -> exit 0 (no whitespace errors)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_db::repository::` -> exit 0 (15/15 passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_storage::` -> exit 0 (14/14 passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::hooks::` -> exit 0 (8/8 passed)

### Scaffolding removed

- None.

### Success-criteria verification

- [x] AC1: Fresh DB stores valid `repository_id`/`source_instance_id`, stable across reopen/repeated setup -> `repository_metadata_is_seeded_once_and_validated_on_reopen`, `fresh_repository_database_receives_a_valid_source_instance_id_once` pass (15/15 repository suite).
- [x] AC2: Independently created DB files differ; shared-path resolution stays identical -> `independently_created_databases_for_the_same_repository_receive_different_source_instance_ids` pass; storage suite's `repeated_resolution_is_idempotent`, `clones_of_the_same_repository_share_the_db_path_with_distinct_checkout_ids`, `linked_worktree_shares_the_db_path_with_a_distinct_checkout_id` pass (14/14 storage suite).
- [x] AC3: Old baseline fixture gains a stable identity through setup migration -> `baseline_only_fixture_gains_a_stable_source_instance_id_through_setup_migration` pass.
- [x] AC4: Concurrent missing-identity initialization converges on one persisted winner -> `concurrent_missing_source_instance_id_initialization_converges_on_one_persisted_winner` (8-thread barrier test) pass.
- [x] AC5: Repository-ID mismatch still rejected; valid existing metadata preserved -> `mismatched_repository_metadata_errors_on_open` pass.
- [x] AC6: Hook/readiness opens never migrate an old schema and fail with `sce setup` guidance; setup-safe path still repairs -> `hook_runtime_resolution_fails_with_setup_guidance_before_setup_ran`, `hook_runtime_resolution_fails_with_setup_guidance_on_a_baseline_only_schema_without_mutating_it`, `hook_runtime_resolution_succeeds_and_reuses_metadata_after_setup` pass; `services::hooks::` suite (8/8) unaffected.
- [x] AC7: Downstream callers use the typed `RepositoryMetadata` API -> inspected `cli/src/services/agent_trace_storage/mod.rs`, which imports and threads `RepositoryMetadata` from `agent_trace_db::repository` through `ResolvedAgentTraceStorage.metadata` rather than ad hoc metadata SQL.
- [x] AC8: Documentation distinguishes repository/database-lineage/checkout identity and states the future ETL lineage tuple -> inspected `context/sce/agent-trace-db.md` (new paragraph on `source_instance_id` vs. `checkout_id` and the `(repository_id, source_instance_id, source_table, source_row_id)` tuple) plus corroborating edits in `context/overview.md`, `context/architecture.md`, `context/context-map.md`, `context/patterns.md` reflecting the additive `002` migration and non-migrating hook-runtime contract.

### Failed checks and follow-ups

- None.

### Residual risks

- None identified.
