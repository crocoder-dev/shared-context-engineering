# Plan: Agent Trace normalized DB persistence

## Change summary

Replace the current placeholder `cli/migrations/001_create_agent_traces.sql` migration with a normalized Agent Trace persistence schema and add a typed LocalDb insertion API that accepts the existing Rust `AgentTrace` struct, serializes the full trace payload, and normalizes its file/conversation/range data into queryable tables.

User-confirmed decisions:

- The existing `001_create_agent_traces.sql` migration is placeholder-only and should be replaced rather than extended with `002`.
- Existing local DB data from the placeholder schema is disposable.
- The insertion API should accept the existing `AgentTrace` struct and perform normalization internally.
- Do not change or align the existing Agent Trace version contract as part of this work; leave current code/schema version behavior as-is.

## Success criteria

1. `cli/migrations/001_create_agent_traces.sql` defines the normalized Agent Trace schema instead of the current single `trace_json` placeholder table.
2. The schema stores the full serialized trace record plus normalized rows for files, conversations, and ranges.
3. Existing placeholder local DBs are handled deterministically under the disposable-data decision, without silently leaving an incompatible old `agent_traces` table in place.
4. `LocalDb` exposes a typed insertion method that accepts `&AgentTrace`, serializes the complete trace JSON, and inserts normalized rows in a transaction-like all-or-nothing flow supported by the Turso API.
5. Duplicate trace IDs have deterministic behavior documented. Dedicated `local_db.rs` test coverage was removed after validation per user request.
6. Migration/bootstrap behavior and insertion of a minimal valid trace record with one file, one conversation, and one range are implemented and documented. Dedicated `local_db.rs` tests for these paths were removed after validation per user request.
7. No CLI command, hook runtime, sync command, or Agent Trace version-shape behavior is changed in this plan.
8. Context files are updated only where durable current-state DB/persistence contracts change.
9. Final validation and cleanup are completed.

## Constraints and non-goals

- In scope: replacing the placeholder `001` migration, LocalDb migration/bootstrap support needed for the replacement, typed insert API, normalization tests, and focused context updates.
- In scope: retaining `trace_json` as the canonical full-payload storage while adding normalized tables for queryable structure.
- In scope: deterministic handling for pre-existing placeholder DB files because local placeholder data is disposable.
- Out of scope: adding a new `002` migration for this replacement.
- Out of scope: changing `AgentTrace`, contributor taxonomy, JSON schema versioning, version string formatting, UUID/timestamp generation, or minimal generator behavior.
- Out of scope: wiring persistence into hooks, `diff-trace`, `sce sync`, post-commit runtime, or any CLI command surface.
- Out of scope: cloud sync, hosted ingestion, retry queues, or non-local storage.

## Proposed schema shape

The implementation should finalize exact SQL names/types locally, but the intended normalized shape is:

- `agent_traces`
  - `trace_id TEXT PRIMARY KEY` from `AgentTrace.id`
  - `version TEXT NOT NULL`
  - `timestamp TEXT NOT NULL`
  - `trace_json TEXT NOT NULL`
  - `created_at TEXT NOT NULL DEFAULT (datetime('now'))`
- `agent_trace_files`
  - `id INTEGER PRIMARY KEY AUTOINCREMENT`
  - `trace_id TEXT NOT NULL REFERENCES agent_traces(trace_id) ON DELETE CASCADE`
  - `file_index INTEGER NOT NULL`
  - `path TEXT NOT NULL`
  - unique `(trace_id, file_index)`
- `agent_trace_conversations`
  - `id INTEGER PRIMARY KEY AUTOINCREMENT`
  - `file_id INTEGER NOT NULL REFERENCES agent_trace_files(id) ON DELETE CASCADE`
  - `conversation_index INTEGER NOT NULL`
  - `contributor_type TEXT NOT NULL`
  - unique `(file_id, conversation_index)`
- `agent_trace_ranges`
  - `id INTEGER PRIMARY KEY AUTOINCREMENT`
  - `conversation_id INTEGER NOT NULL REFERENCES agent_trace_conversations(id) ON DELETE CASCADE`
  - `range_index INTEGER NOT NULL`
  - `start_line INTEGER NOT NULL`
  - `end_line INTEGER NOT NULL`
  - unique `(conversation_id, range_index)`

Useful indexes should cover trace timestamp and file path lookups without over-indexing the MVP.

## Task stack

- [x] T01: `Replace placeholder migration with normalized Agent Trace schema` (status:done)
  - Task ID: T01
  - Goal: Rewrite `cli/migrations/001_create_agent_traces.sql` from the placeholder blob-only table into the normalized Agent Trace schema.
  - Boundaries (in/out of scope): In - normalized SQL tables, constraints, foreign keys, uniqueness, and minimal useful indexes for trace timestamp/file path lookup. Out - Rust insertion API, runtime wiring, version contract changes, and any `002` migration.
  - Done when: the `001` migration no longer represents the placeholder-only `agent_traces(id, trace_json, created_at)` table and instead creates the finalized normalized schema with full-payload storage plus file/conversation/range tables.
  - Verification notes (commands or checks): review SQL for SQLite/Turso compatibility; later tasks add migration execution tests.
  - Completed: 2026-04-28
  - Files changed: `cli/migrations/001_create_agent_traces.sql`; context sync touched `context/sce/local-db.md`, `context/sce/agent-trace-core-schema-migrations.md`, `context/context-map.md`, and `context/glossary.md`.
  - Evidence: SQLite in-memory `executescript` check passed for the migration tables/indexes; `nix build .#checks.x86_64-linux.cli-tests --print-out-paths` succeeded with `/nix/store/88ry9vw1jf6siq2lywbz60dzkmrahkv9-sce-cli-tests-test-0.2.0`; `nix run .#pkl-check-generated` reported generated outputs up to date; `nix flake check` passed.
  - Notes: The migration now defines normalized `agent_traces`, `agent_trace_files`, `agent_trace_conversations`, and `agent_trace_ranges` tables with full-payload `trace_json`, uniqueness constraints, cascading foreign keys, and timestamp/path indexes. Runtime bootstrap handling for old placeholder DB files remains intentionally deferred to T02. Context sync classified this as a localized local-DB schema/current-state update; `context/overview.md`, `context/architecture.md`, and `context/patterns.md` were verified without edits.

- [x] T02: `Make LocalDb bootstrap handle replaced placeholder schema deterministically` (status:done)
  - Task ID: T02
  - Goal: Ensure `LocalDb::new()` cannot silently run against an old placeholder `agent_traces` table after `001` is replaced.
  - Boundaries (in/out of scope): In - migration/bootstrap logic in `cli/src/services/local_db.rs`, placeholder-schema detection, deterministic disposable-data reset or actionable failure/remediation, and migration smoke coverage. Out - preserving old placeholder rows, adding a forward `002` migration, and changing default DB path policy.
  - Done when: fresh DB bootstrap creates the normalized schema, and an existing placeholder-shaped DB is handled according to the disposable-data decision rather than producing later insert failures from missing normalized columns/tables.
  - Verification notes (commands or checks): isolated LocalDb migration tests using a temporary DB path or test seam; `nix build .#checks.x86_64-linux.cli-tests`.
  - Completed: 2026-04-28
  - Files changed: `cli/src/services/local_db.rs`; `context/sce/local-db.md`; `context/overview.md`; `context/architecture.md`; `context/patterns.md`; `context/glossary.md`; `context/context-map.md`; plan evidence updated in `context/plans/agent-trace-normalized-db-persistence.md`.
  - Evidence: `nix build .#checks.x86_64-linux.cli-tests --print-out-paths` succeeded with `/nix/store/jjckcn3k1lkyll49x99wbaq0zl5mklz2-sce-cli-tests-test-0.2.0`; `nix build .#checks.x86_64-linux.cli-clippy --print-out-paths` succeeded with `/nix/store/c19f4mw24paz89yd6bxmq1is0iikrkk0-sce-cli-clippy-clippy-0.2.0`; `nix build .#checks.x86_64-linux.cli-fmt --print-out-paths` succeeded with `/nix/store/kv0pnzjq4xd6ibcsxzq4kaiyv10x8nvv-sce-cli-fmt-fmt-0.2.0`; `nix run .#pkl-check-generated` reported generated outputs up to date; `nix flake check` passed.
  - Notes: `LocalDb::new()` now opens through a path-based internal seam, inspects any existing `agent_traces` table before migrations, drops the known retired placeholder shape (`id`, `trace_json`, `created_at`) under the disposable-data decision, fails early with remediation for unknown incompatible `agent_traces` schemas, and runs embedded migrations with Turso `execute_batch` so the multi-statement normalized `001` migration bootstraps fresh DBs. The dedicated `local_db.rs` tests that covered fresh bootstrap, placeholder reset, and incompatible-schema failure were removed after validation per user request. Typed insertion remains deferred to T03.

- [x] T03: `Add typed AgentTrace insertion API to LocalDb` (status:done)
  - Task ID: T03
  - Goal: Add a LocalDb method that accepts `&AgentTrace`, serializes the complete trace to `trace_json`, and inserts normalized rows for files, conversations, and ranges.
  - Boundaries (in/out of scope): In - method shape, serialization, ordered normalization from existing struct fields, all-or-nothing insert behavior, deterministic duplicate trace ID behavior, and focused unit tests. Out - query API beyond what tests need, CLI/hook/sync wiring, and changing Agent Trace generation.
  - Done when: inserting a minimal trace stores one parent trace row, one file row, one conversation row, and one range row with values matching the supplied `AgentTrace`; duplicate trace IDs behave deterministically and are tested.
  - Verification notes (commands or checks): targeted LocalDb tests; `nix build .#checks.x86_64-linux.cli-tests`.
  - Completed: 2026-04-28
  - Files changed: `cli/src/services/local_db.rs`; plan evidence updated in `context/plans/agent-trace-normalized-db-persistence.md`.
  - Evidence: direct `cargo test local_db` was blocked by the repository bash policy (`use-nix-flake-check-over-cargo-test`); `nix build .#checks.x86_64-linux.cli-tests --print-out-paths` succeeded with `/nix/store/1h5ddggnjmrsrg4khr56wwq8c31b04h6-sce-cli-tests-test-0.2.0`; `nix build .#checks.x86_64-linux.cli-clippy --print-out-paths` succeeded with `/nix/store/k9jk7sqq92bkg3jjzpfh0ri13p3f61ry-sce-cli-clippy-clippy-0.2.0`; `nix build .#checks.x86_64-linux.cli-fmt --print-out-paths` succeeded with `/nix/store/92aipz74gg9wmm2r6hdbc6gn216ih4ir-sce-cli-fmt-fmt-0.2.0`; `nix run .#pkl-check-generated` reported generated outputs up to date; `nix flake check` passed.
  - Notes: `LocalDb::insert_agent_trace(&AgentTrace)` now serializes the full payload into `agent_traces.trace_json` and inserts normalized file, conversation, and range rows in vector order inside an explicit transaction. Duplicate trace IDs deterministically fail through the `agent_traces.trace_id` primary key and roll back without adding child rows. The dedicated `local_db.rs` tests that covered minimal valid trace insertion, duplicate trace IDs, and partial-row rollback were removed after validation per user request. Context sync classified this as a localized local-DB persistence update; durable context now documents the typed API, duplicate-ID behavior, rollback behavior, normalized persistence, and continued non-wiring to hooks/sync.

- [x] T04: `Update current-state context for normalized local DB persistence` (status:done)
  - Task ID: T04
  - Goal: Sync durable context so future sessions know the local DB stores Agent Trace payloads through normalized persistence rather than the old placeholder blob-only table.
  - Boundaries (in/out of scope): In - focused updates to `context/sce/local-db.md`, `context/glossary.md`, `context/context-map.md`, and any directly affected CLI/SCE context file. Out - broad historical rewrites, completed-plan history cleanup, and unrelated Agent Trace runtime docs.
  - Done when: context describes the normalized schema, typed insertion API, placeholder replacement decision, and continued non-wiring to hooks/sync where applicable.
  - Verification notes (commands or checks): manual context review against code truth; `nix run .#pkl-check-generated` if generated outputs are touched by implementation.
  - Completed: 2026-04-28
  - Files changed: `context/overview.md`; plan evidence updated in `context/plans/agent-trace-normalized-db-persistence.md`.
  - Evidence: manually reviewed `cli/migrations/001_create_agent_traces.sql`, `cli/src/services/local_db.rs`, `context/sce/local-db.md`, `context/glossary.md`, `context/context-map.md`, `context/architecture.md`, `context/patterns.md`, and related historical Agent Trace DB context against code truth; stale empty-file-baseline references were searched and none remain; `nix run .#pkl-check-generated` reported generated outputs up to date; `nix flake check` passed.
  - Notes: Durable context already described the normalized schema, typed `LocalDb::insert_agent_trace(&AgentTrace)` API, retired-placeholder reset/fail-fast handling, rollback/duplicate-ID behavior, and continued non-wiring to hooks/`diff-trace`/`sce sync`. This task corrected the overview navigation so future sessions use `context/sce/local-db.md` as the active local DB contract and treat `agent-trace-core-schema-migrations.md` as historical only. Context-sync classification: context-only current-state drift repair; root overview edit required, no code changes.

- [x] T05: `Validation and cleanup` (status:done)
  - Task ID: T05
  - Goal: Run final validation, remove temporary scaffolding, and record evidence in this plan.
  - Boundaries (in/out of scope): In - full repository validation, pkl generated-output parity check, final context drift check, plan evidence updates, and cleanup of test-only leftovers not intended to remain. Out - new persistence features, runtime wiring, or schema changes beyond prior tasks.
  - Done when: required checks pass, temporary scaffolding is removed or intentionally justified, this plan has validation evidence, and no unplanned scope remains.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; `nix flake check`; inspect changed files for accidental version-contract or runtime-wiring changes.
  - Completed: 2026-04-28
  - Files changed: plan evidence updated in `context/plans/agent-trace-normalized-db-persistence.md`; removed empty ignored temporary log `context/tmp/sce.log`; later removed the dedicated `local_db.rs` tests per user request and updated `context/cli/cli-command-surface.md` to avoid stale LocalDb test references.
  - Evidence: `nix run .#pkl-check-generated` reported generated outputs up to date; `nix flake check` passed before post-validation test removal; after removing the dedicated `local_db.rs` tests, `nix build .#checks.x86_64-linux.cli-tests --print-out-paths`, `nix build .#checks.x86_64-linux.cli-clippy --print-out-paths`, `nix build .#checks.x86_64-linux.cli-fmt --print-out-paths`, and `nix run .#pkl-check-generated` passed; changed-file inspection found no LocalDb/persistence references in `cli/src/services/hooks.rs`, and `AGENT_TRACE_VERSION` remains unchanged at `"0.1"` in `cli/src/services/agent_trace.rs`.
  - Notes: No new CLI command, hook runtime, `diff-trace`, `sce sync`, post-commit, Agent Trace version-shape, or schema behavior was added during validation. Cleanup found only the retained `context/tmp/.gitignore` plus an empty ignored `context/tmp/sce.log`, which was removed. After validation, the user requested removal of the added `local_db.rs` tests; current behavior is implemented and documented without dedicated local DB test coverage in that file. Context-sync classification: validation-only finalization plus post-validation test-removal feedback; root context remains verify-only unless final sync finds drift.

## Open questions

- None.

## Validation Report

### Commands run

- `nix run .#pkl-check-generated` -> exit 0; generated outputs are up to date.
- `nix flake check` -> exit 0; all checks passed.
- Post-validation test-removal checks: `nix build .#checks.x86_64-linux.cli-tests --print-out-paths`, `nix build .#checks.x86_64-linux.cli-clippy --print-out-paths`, `nix build .#checks.x86_64-linux.cli-fmt --print-out-paths`, and `nix run .#pkl-check-generated` -> exit 0.

### Success-criteria verification

- [x] `001_create_agent_traces.sql` defines normalized `agent_traces`, `agent_trace_files`, `agent_trace_conversations`, and `agent_trace_ranges` tables with timestamp/path indexes.
- [x] Full serialized trace payload remains stored in `agent_traces.trace_json`; normalized child rows are inserted for files, conversations, and ranges.
- [x] `LocalDb::new()` deterministically resets the retired placeholder `agent_traces(id, trace_json, created_at)` shape and fails early for unknown incompatible shapes.
- [x] `LocalDb::insert_agent_trace(&AgentTrace)` serializes the full trace and writes normalized rows in an explicit rollback-on-failure transaction.
- [x] Duplicate trace IDs fail through the `agent_traces.trace_id` primary key and are documented; the dedicated duplicate-ID test was removed after validation per user request.
- [x] Migration/bootstrap and minimal insert behavior are implemented and documented; dedicated local DB tests for those paths were removed after validation per user request.
- [x] Inspection confirmed no CLI command, hook runtime, `diff-trace`, `sce sync`, post-commit runtime, or Agent Trace version-shape behavior was wired or changed.
- [x] Durable current-state context is linked from `context/context-map.md`, `context/overview.md`, and `context/sce/local-db.md`.
- [x] Final validation and cleanup completed; empty ignored `context/tmp/sce.log` was removed and only `context/tmp/.gitignore` is intended to remain.

### Failed checks and follow-ups

- None.

### Residual risks

- Dedicated LocalDb regression tests for migration/bootstrap, duplicate trace IDs, and normalized insert rollback were removed per user request, so future regressions in those paths are less directly covered by automated tests.
