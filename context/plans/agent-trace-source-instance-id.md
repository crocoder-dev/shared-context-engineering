# Plan: agent-trace-source-instance-id

## Change summary

Give every repository-scoped `agent-trace.db` a second, independent identity alongside the existing logical `repository_id`: a `source_instance_id` that identifies one physical database lineage. Two independently created `agent-trace.db` files for the same logical repository (for example, two different machines cloning the same repo) must end up with different `source_instance_id` values, generated exactly once per physical database and stable across reopen, `sce setup` reruns, and process restarts. This is purely a local storage identity addition — no remote ingestion, sync, or DWH behavior is implemented or designed here. It recreates the useful part of the abandoned PR #186 without any of its retired architecture (no DWH, no `agent-trace-sync.db`, no ETL, no Turso Sync).

The schema gains an additive migration (`002_repository_source_instance_id.sql`) on `repository_metadata`, defaulting existing/placeholder rows to an empty string. Application code — never SQL — replaces that placeholder with a generated identity through a concurrency-safe atomic claim, so concurrent SCE processes opening the same database converge on one winner instead of racing to overwrite each other. Repository metadata initialization becomes typed (`RepositoryMetadata { repository_id, source_instance_id }`) and is threaded through `ResolvedAgentTraceStorage` so setup, lifecycle, and hook-runtime callers all observe the same identity. High-frequency hook runtime keeps its existing no-migration boundary: it must never apply migration `002` itself, even though it is allowed to initialize `source_instance_id` once the column is already present from a prior `sce setup`.

## Acceptance criteria

- [ ] AC1: A fresh repository-scoped `agent-trace.db` receives a valid, non-empty `source_instance_id` on first initialization, and `repository_id` remains correct.
  - Validate: `nix flake check` (runs the new repository-adapter unit tests covering fresh initialization).
- [ ] AC2: `source_instance_id` is stable across database reopen, repeated `verify_or_initialize_repository_metadata` calls (repeated `sce setup`), and is never derived from `repository_id`, remote URL, checkout ID, filesystem path, hostname, or user/workspace identity.
  - Validate: `nix flake check` (repository-adapter reopen/repeat/independent-DB unit tests).
- [ ] AC3: Concurrent initialization of the same physical database converges on exactly one persisted `source_instance_id`; a losing racer's generated candidate is discarded in favor of the winner's stored value, and an already-valid `source_instance_id` is never overwritten.
  - Validate: `nix flake check` (concurrent-initialization unit test in `cli/src/services/agent_trace_db/repository.rs`).
- [ ] AC4: `resolve_agent_trace_storage` (setup/lifecycle path) returns the same typed `RepositoryMetadata` that database verification/initialization produced, alongside the existing `db`/`db_path`/`checkout_id` fields.
  - Validate: `nix flake check` (`agent_trace_storage` unit tests asserting `ResolvedAgentTraceStorage.metadata`).
- [ ] AC5: High-frequency hook runtime resolution never applies migration `002`: before `sce setup` (missing DB, or a baseline-only DB that has migration `001` but not `002`), hook resolution fails with the existing `sce setup` guidance and leaves the stored migration metadata unchanged; after `sce setup`, hook resolution succeeds and returns the same `RepositoryMetadata` setup produced.
  - Validate: `nix flake check` (`agent_trace_storage` hook-runtime resolution unit tests: before-setup missing DB, before-setup baseline-only schema, after-setup parity).
- [ ] AC6: `sce setup` Agent Trace diagnostics report the source-instance ID alongside the existing repository ID line, without introducing workspace or remote-ingestion concepts.
  - Validate: Inspect `format_repository_storage_setup_message` output in `cli/src/services/agent_trace_db/lifecycle.rs` and its covering test.
- [ ] AC7: A baseline-only fixture (only the original `001` repository schema, no `source_instance_id` column) migrates cleanly to `002`, gets a populated `source_instance_id`, and preserves that value across reopen.
  - Validate: `nix flake check` (baseline-fixture migration unit test in `cli/src/services/agent_trace_db/repository.rs`).

### Full validation

- `nix flake check`

### Context sync

- `context/cli/agent-trace-storage.md`
- `context/sce/agent-trace-db.md`
- `context/context-map.md` (only if either linked domain-file summary needs a one-line update to stay accurate)

## Constraints and non-goals

- **In scope:** `cli/migrations/agent-trace-repository/002_repository_source_instance_id.sql`, `cli/src/services/agent_trace_db/repository.rs`, `cli/src/services/agent_trace_storage/mod.rs`, `cli/src/services/agent_trace_db/lifecycle.rs`, `cli/src/services/hooks/mod.rs` (hook-runtime storage call site only), and the durable context files listed above.
- **Out of scope:** `sce trace sync`, any HTTP/remote ingestion, control-plane changes, workspace resolution, WorkOS integration, ingestion cursors, DWH schema/ETL, derived `code_changes`, Turso Sync, `agent-trace-sync.db`, replica paths, bridge locks, and any change to `sce doctor`'s read-only diagnose surface (it does not write to the DB today and stays that way).
- **Constraints:** The migration must be additive and must not rewrite `001_repository_schema.sql`. `source_instance_id` must never be generated inside SQL. The existing separation between no-migration hook-runtime DB access and migration-running setup/lifecycle access must be preserved and, where it does not yet exist for this exact concern, established rather than blurred.
- **Non-goal:** Adding `workspace_id` or any user/host identity to the local repository Agent Trace DB. Designing remote ingestion, even though it is the eventual consumer of `source_instance_id`.

## Assumptions

- `is_valid_source_instance_id` validates "non-empty once trimmed" rather than strict UUID-v4 parsing, per the request's own instruction not to make downstream code depend on the identity being UUID-shaped forever; `generate_source_instance_id` still produces UUID v4 strings today.
- `sce doctor`'s diagnose path is read-only and does not open the DB for write today; per the request's "keep this diagnostic only" framing for `sce setup`, doctor's diagnose surface is left unchanged rather than being taught to also initialize/display `source_instance_id`.
- Hook-runtime initialization reuses the same `verify_or_initialize_repository_metadata` atomic-claim logic as setup once `ensure_schema_ready_for_hooks()` confirms exact migration-metadata parity (which guarantees the `source_instance_id` column already exists), rather than adding a separate schema/column introspection check — the readiness check already proves the precondition the request describes.

## Task stack

- [x] T01: `Add source-instance identity migration, type, and atomic-claim initialization` (status:done)
  - Task ID: T01
  - Goal: Add migration `002_repository_source_instance_id.sql`, the typed `RepositoryMetadata { repository_id, source_instance_id }`, `generate_source_instance_id()`/`is_valid_source_instance_id()` helpers, and change `RepositoryAgentTraceDb::verify_or_initialize_repository_metadata` to return `Result<RepositoryMetadata>` using the atomic `UPDATE ... WHERE source_instance_id = ''` claim pattern (claim with a generated candidate, always re-read, never overwrite an already-valid value).
  - Boundaries (in/out of scope): In — `cli/migrations/agent-trace-repository/002_repository_source_instance_id.sql`, `cli/src/services/agent_trace_db/repository.rs` (type, helpers, method, and its unit tests: fresh DB, stable reopen, repeated initialization, mismatched repository ID unchanged, independent DBs diverge, concurrent initialization converges, migration-from-baseline-only-fixture). Out — any caller outside `repository.rs` (existing callers discard the method's return value via `?;` and keep compiling unchanged); storage-resolution/hook-runtime wiring (T02/T03).
  - Dependencies: none
  - Done when: The migration is additive and auto-discovered; `RepositoryMetadata`/`generate_source_instance_id`/`is_valid_source_instance_id` are exposed from `repository.rs`; `verify_or_initialize_repository_metadata` returns typed metadata with concurrency-safe initialization; all new/updated unit tests in `repository.rs` pass.
  - Verification notes (commands or checks): `nix flake check` (covers `cargo test`/`cargo clippy`/`cargo fmt` for the CLI crate via the Crane check pipeline).
  - Evidence: Added additive migration `cli/migrations/agent-trace-repository/002_repository_source_instance_id.sql` (`ALTER TABLE repository_metadata ADD COLUMN source_instance_id TEXT NOT NULL DEFAULT ''`; auto-discovered by `build.rs`, no `001` rewrite). Added `RepositoryMetadata { repository_id, source_instance_id }`, `generate_source_instance_id()` (UUID v4), `is_valid_source_instance_id()` (non-empty once trimmed) to `cli/src/services/agent_trace_db/repository.rs`. Rewrote `verify_or_initialize_repository_metadata` to return `Result<RepositoryMetadata>`: seeds the row as before, checks `repository_id` for mismatch before touching `source_instance_id`, then claims via `UPDATE repository_metadata SET source_instance_id = ?1 WHERE id = 1 AND source_instance_id = ''` and always re-reads the row afterward so a losing racer's candidate is discarded and an already-valid value is never overwritten. Existing external callers (`agent_trace_storage/mod.rs`, `trace/discovery.rs`) already discarded the prior `Result<()>` via `?`/`.expect(...)` and needed no changes. Added/updated unit tests in `repository.rs`: `open_at_initializes_the_full_schema_from_one_migration` now asserts both `001_repository_schema` and `002_repository_source_instance_id` are applied; `repository_metadata_is_seeded_once_and_validated_on_reopen` asserts a valid non-empty `source_instance_id` that is stable across repeated calls and reopen; new `source_instance_id_is_not_derived_from_repository_id_and_diverges_across_independent_dbs`; new `concurrent_initialization_converges_on_one_source_instance_id` (4 threads racing the claim, all converge on one value); `mismatched_repository_metadata_errors_on_open` now asserts a rejected mismatch leaves the stored `source_instance_id` unchanged; new `baseline_only_fixture_migrates_and_gets_a_stable_source_instance_id` (hand-built pre-002 fixture with `001` recorded and no `source_instance_id` column, migrates cleanly on open, value stable across reopen).
  - Verification run: `nix flake check` passed (`cli-fmt`, `cli-clippy` with `#![deny(clippy::pedantic)]`, and `cli-tests`, including all new/updated tests in `repository.rs`). One process note: the new migration SQL file had to be `git add`-staged for Nix's flake source view (git-tracked files only) to see it — the file is included in this task's change set.

- [x] T02: `Expose repository metadata through setup/lifecycle storage resolution` (status:done)
  - Task ID: T02
  - Goal: Add `metadata: RepositoryMetadata` to `ResolvedAgentTraceStorage` and thread it through the existing `resolve_agent_trace_storage`/`resolve_agent_trace_storage_at_state_root` (setup/lifecycle) resolution path, which keeps its existing fast-path-then-migrate fallback behavior.
  - Boundaries (in/out of scope): In — `cli/src/services/agent_trace_storage/mod.rs` struct/field addition, threading the typed metadata out of `open_repository_db_concurrently_safe`, and updated/added unit tests asserting `ResolvedAgentTraceStorage.metadata`. Out — introducing the dedicated hook-runtime resolution entrypoint (T03); any call-site changes beyond what's needed to keep existing callers compiling against the new field.
  - Dependencies: T01
  - Done when: `ResolvedAgentTraceStorage` carries `metadata`; setup/lifecycle resolution returns it populated from the same verification/initialization call that opens the DB; existing `agent_trace_storage` tests plus new metadata-presence assertions pass.
  - Verification notes (commands or checks): `nix flake check`.
  - Evidence: Added `metadata: RepositoryMetadata` field to `ResolvedAgentTraceStorage` in `cli/src/services/agent_trace_storage/mod.rs`, imported alongside `RepositoryAgentTraceDb`. Changed `open_repository_db_concurrently_safe` to return `(RepositoryAgentTraceDb, RepositoryMetadata)`: both the fast-path branch (`open_without_migrations_at` + optional schema-metadata repair) and the full-init branch (`new_at`) now keep the `RepositoryMetadata` already produced by `verify_or_initialize_repository_metadata` instead of discarding it. `open_storage` destructures `(db, metadata)` and populates the new field; `resolve_agent_trace_storage`/`resolve_agent_trace_storage_at_state_root` are unchanged aside from receiving the new field through `open_storage`. Extended `repeated_resolution_is_idempotent` in `cli/src/services/agent_trace_storage/mod.rs` to assert `first.metadata.repository_id` matches the resolved `repository_id`, `first.metadata == second.metadata` (stable across repeated resolution), and `source_instance_id` is non-empty once trimmed. No other callers needed changes: `lifecycle.rs`, `hooks/mod.rs`, `config/resolver.rs`, `config/types.rs`, and `trace/status.rs` consume `ResolvedAgentTraceStorage` by field access on `db`/`db_path`/`checkout_id`/`repository_identity` and compile unchanged against the additive field.
  - Verification run: `nix flake check` passed (`cli-fmt`, `cli-clippy` with `#![deny(clippy::pedantic)]`, and `cli-tests`, including the extended `repeated_resolution_is_idempotent` test and all existing `agent_trace_storage` tests).

- [ ] T03: `Add no-migration hook-runtime storage resolution and wire hooks to it` (status:todo)
  - Task ID: T03
  - Goal: Add `resolve_agent_trace_storage_for_hook_runtime`/`_at_state_root` in `cli/src/services/agent_trace_storage/mod.rs` — no-migration open, schema-readiness check, narrow concurrent-first-open metadata repair only, and `source_instance_id` initialization gated on readiness already passing (never a migration fallback) — and switch `open_agent_trace_db_for_hook_runtime` in `cli/src/services/hooks/mod.rs` to call it instead of the setup/lifecycle resolver.
  - Boundaries (in/out of scope): In — the new resolution functions and their unit tests (fails before setup on a missing DB, fails before setup on a baseline-only schema without recording migration `002`, succeeds after setup and returns the same `RepositoryMetadata` as the setup path), and the one hook call-site switch. Out — changing `sce trace status`/`sce trace db shell`/`sce doctor` DB-open call sites, which keep using their current resolution paths.
  - Dependencies: T02
  - Done when: Hook runtime never runs migration `002` (or any migration) during normal operation; a baseline-only or missing schema fails hook resolution with the existing `sce setup` guidance; a fully set-up repository resolves successfully for hooks with metadata matching setup's; all new tests pass alongside existing hook-behavior tests.
  - Verification notes (commands or checks): `nix flake check`.

- [ ] T04: `Report source-instance ID in setup diagnostics` (status:todo)
  - Task ID: T04
  - Goal: Extend `AgentTraceDbLifecycle::setup`'s reported message in `cli/src/services/agent_trace_db/lifecycle.rs` to include the source-instance ID alongside the existing repository ID line, sourced from the `RepositoryMetadata` now returned by storage resolution.
  - Boundaries (in/out of scope): In — `RepositoryDatabaseSetup`/`format_repository_storage_setup_message` in `lifecycle.rs` and its covering test. Out — `sce doctor` diagnose output (read-only, unchanged per Assumptions).
  - Dependencies: T02
  - Done when: `sce setup`'s Agent Trace messaging includes an `Agent Trace source-instance ID: ...` line using the resolved metadata, with no workspace/remote-ingestion wording introduced.
  - Verification notes (commands or checks): `nix flake check`; inspect the formatted message in the lifecycle setup test.

- [ ] T05: `Document the repository_id vs source_instance_id identity split` (status:todo)
  - Task ID: T05
  - Goal: Update `context/cli/agent-trace-storage.md` and `context/sce/agent-trace-db.md` to describe the additive `002` migration, the typed `RepositoryMetadata`, the atomic-claim concurrency contract, and the setup/lifecycle vs. hook-runtime resolution split now that both paths are distinct functions; touch `context/context-map.md` only if a linked summary line becomes materially inaccurate.
  - Boundaries (in/out of scope): In — the durable-context files named above. Out — designing or documenting any remote-ingestion consumer of `source_instance_id`; rewriting unrelated sections of those files.
  - Dependencies: T04
  - Done when: Both domain files accurately describe `repository_id` (logical Git repository identity) versus `source_instance_id` (physical database lineage identity), the migration, and the resolution split, matching the code shipped in T01–T04.
  - Verification notes (commands or checks): Review the updated files against the code from T01–T04 for accuracy; no generated output is touched, so `pkl-check-generated` is not required.

## Open questions

None. The request fully specifies the schema change, identity semantics, concurrency pattern, resolution-boundary rules, diagnostics wording, and test coverage; nothing here changes scope, acceptance criteria, or task ordering.
