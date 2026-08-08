# Plan: agent-trace-dwh-schema-identity-contract

## Change summary

Introduce the first versioned Agent Trace DWH destination contract as a migration set and dedicated `TursoDb<DbSpec>` boundary that are separate from the repository-scoped `agent-trace.db` source schema. The DWH schema will preserve repository identity, source-database lineage, complete conversation parts, verbatim Agent Trace JSON, transformed code-change metrics, integrity hashes, and independently scoped extraction watermarks without introducing ETL, sync, provisioning, credentials, or CLI behavior.

Document the source-versus-DWH architecture and the composite identities that make future ingestion deterministic and idempotent across repositories and independently created source databases.

## Acceptance criteria

- [ ] AC1: A fresh Agent Trace DWH database initializes exactly the `repositories`, `source_instances`, `etl_watermarks`, `messages`, `message_parts`, `agent_traces`, and `code_changes` contract tables through a dedicated migration set, and DWH migration metadata reports the baseline migration as applied.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_dwh_db`
- [ ] AC2: The DWH uniqueness contract admits overlapping local part and diff-trace integer IDs across source instances and repositories, while rejecting duplicate message logical identities and duplicate Agent Trace logical identities.
  - Validate: targeted DWH schema tests insert the requested coexistence and duplicate cases and pass under `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_dwh_db`
- [ ] AC3: Watermarks are independently keyed by repository, source instance, and extensible source-table text, and deterministic message-part reconstruction uses the declared repository/session/message/time/source-part ordering index.
  - Validate: targeted DWH schema tests exercise independent watermark rows, inspect the required index, and assert deterministic query ordering under `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_dwh_db`
- [ ] AC4: The DWH schema stores complete message-part text and Agent Trace JSON without truncation or normalization columns, stores the required hash fields, preserves source timestamps as integers, and adds only the requested access-pattern indexes without ingestion-order foreign keys.
  - Validate: inspect `cli/migrations/agent-trace-dwh/001_dwh_schema.sql` and run the fresh-schema assertions in the targeted DWH tests.
- [ ] AC5: A dedicated DWH database adapter can initialize an explicitly selected database and verify migration readiness without owning a sync URL, credentials, ETL state transitions, bridge locking, or CLI lifecycle behavior.
  - Validate: targeted adapter tests initialize a fresh explicit-path DWH DB and pass its readiness check; inspect the adapter for the absence of sync/control-plane fields and command wiring.
- [ ] AC6: Durable architecture documentation distinguishes repository `agent-trace.db` source storage from the append-oriented Agent Trace DWH and records every repository/source/message/part/trace/code-change identity rule plus the deterministic idempotent ETL intent.
  - Validate: inspect the updated Agent Trace database, shared Turso, architecture, context-map, and glossary documentation for the DWH boundary and identity contract.

### Full validation

- `nix flake check`
- `nix run .#pkl-check-generated`

### Context sync

- Update `context/sce/agent-trace-db.md` to distinguish the repository source schema from the new DWH destination and link the DWH contract.
- Add focused durable DWH context and register it in `context/context-map.md`.
- Update `context/architecture.md` and `context/glossary.md` with the DWH boundary, append-oriented role, and identity terminology; update `context/sce/shared-turso-db.md` for the new concrete `DbSpec` consumer.

## Constraints and non-goals

- **In scope:** `cli/migrations/agent-trace-dwh/`, a dedicated service module/spec over the existing Turso adapter, service registration, schema/readiness tests, and DWH architecture/identity documentation.
- **Out of scope:** Changes to `cli/migrations/agent-trace-repository/`; ETL extraction, transformation execution, hashing implementation, watermark advancement, Turso Sync, local `agent-trace-sync.db`, pull/push, bridge locks, provisioning, credentials, CLI wiring, retention, search, dashboards, and physical `sessions`, `commits`, `models`, `code_change_files`, or raw DWH `diff_traces` tables.
- **Constraints:** Use the build-time migration auto-discovery convention and one UTC text representation (`strftime('%Y-%m-%dT%H:%M:%fZ', 'now')`) for DWH metadata timestamps; preserve source event timestamps as integer milliseconds; do not add foreign keys that constrain independent or out-of-order batch ingestion; keep `source_table` extensible text rather than a database enum; do not define hash computation in this plan.
- **Non-goal:** Expose or operate a local or remote DWH through setup, doctor, trace, sync, or other user-facing commands.

## Assumptions

- Because this PR explicitly excludes a local sync database and provisioning, the DWH `DbSpec` has no canonical production path and is initialized through existing explicit-path Turso constructors, matching the repository-scoped adapter pattern.
- The DWH adapter reuses the existing `agent_trace_db` retry-policy key rather than adding configuration surface before DWH runtime wiring exists.
- Schema tests remain adapter-local in the current binary crate, following the existing repository Agent Trace DB schema-test harness; this plan does not introduce a library target solely to relocate database tests.
- Database-generated UTC defaults apply to `first_seen_at`, `updated_at`, and `ingested_at`; callers may still supply explicit values during future ETL.

## Task stack

- [x] T01: `Add the versioned DWH schema and database boundary` (status:done)
  - Task ID: T01
  - Goal: Define and prove the complete initial DWH schema and expose migration initialization/readiness through a dedicated explicit-path Turso adapter.
  - Boundaries (in/out of scope): In — `cli/migrations/agent-trace-dwh/001_dwh_schema.sql`, an `agent_trace_dwh_db` service/spec/type alias, module registration, all requested schema indexes and constraints, and focused fresh-schema/identity/watermark/order/readiness tests. Out — source-schema changes, domain write APIs, ETL/hashing behavior, sync/config/credentials/lifecycle/CLI wiring, and additional fact or dimension tables.
  - Dependencies: none
  - Done when: Build-time migration discovery emits the DWH migration constant; a fresh explicit-path DWH DB records the baseline and passes readiness; all seven required tables, hash/provenance columns, composite keys, no-FK ingestion tolerance, and required indexes are asserted; overlapping local IDs coexist at the correct scopes; duplicate logical messages and traces fail; code-change and watermark lineage cases pass; equal-time parts query deterministically by `source_part_id`; the targeted Rust tests pass.
  - Verification notes (commands or checks): `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_dwh_db`; inspect generated migration naming through the compiled `AGENT_TRACE_DWH_MIGRATIONS` reference.
  - Implementation evidence: Added `cli/migrations/agent-trace-dwh/001_dwh_schema.sql`, a single multi-statement baseline creating `repositories`, `source_instances`, `etl_watermarks`, `messages`, `message_parts`, `agent_traces`, and `code_changes`, with lineage columns (`repository_id`, `source_instance_id`) denormalized as plain `TEXT` on every fact table and no foreign keys. Added `cli/src/services/agent_trace_dwh_db/mod.rs` defining `AgentTraceDwhDbSpec: DbSpec` (explicit-path only, `db_config_key() = "agent_trace_db"`, migrations from `generated_migrations::AGENT_TRACE_DWH_MIGRATIONS`), `pub type AgentTraceDwhDb = TursoDb<AgentTraceDwhDbSpec>`, and `AgentTraceDwhDb::ensure_dwh_schema_ready()`. Registered `pub mod agent_trace_dwh_db;` in `cli/src/services/mod.rs`.
  - Identity design (recorded as approved assumptions): message logical identity is `(repository_id, session_id, message_id)` and Agent Trace logical identity is `(repository_id, agent_trace_id)` — both deliberately exclude `source_instance_id` so re-ingestion of the same deterministic logical event from an independently created source database stays idempotent. Message-part and code-change identity is `(repository_id, source_instance_id, source_part_id | source_diff_trace_id)` — these are raw local autoincrement source IDs, not stable across independently created source databases, so uniqueness is scoped per source instance, letting the same local integer ID coexist across sources/repositories. Hash columns (`text_sha256`, `trace_json_sha256`, `patch_sha256`) store integrity hashes without computing them.
  - Verification outcome: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_dwh_db` — 9 passed, 0 failed. `nix develop -c ./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings` — clean. `nix develop -c ./scripts/run-cli-cargo.sh fmt --manifest-path cli/Cargo.toml` applied.

- [ ] T02: `Document the DWH architecture and identity contract` (status:todo)
  - Task ID: T02
  - Goal: Make the DWH's destination role, append-oriented ETL design, identities, schema/index contract, and deferred integration boundaries durable and discoverable.
  - Boundaries (in/out of scope): In — focused DWH context, context-map registration, source-DWH distinction in Agent Trace DB context, shared Turso consumer documentation, architecture/glossary updates, and notes on SQLite/Turso constraints or ETL prerequisites discovered while implementing T01. Out — implementation changes, operator runbooks for unimplemented sync/ETL, and speculative tables or query contracts.
  - Dependencies: T01
  - Done when: Documentation names the final seven-table schema, all six identity/uniqueness rules, all required indexes, UTC metadata/source-integer timestamp split, no-FK out-of-order ingestion policy, verbatim JSON/full-part-text preservation, hash-column purpose, per-source/table watermark semantics, any discovered SQLite/Turso limitations, and anything the later ETL framework must account for.
  - Verification notes (commands or checks): inspect links from `context/context-map.md`; compare documented schema, identities, and indexes against `cli/migrations/agent-trace-dwh/001_dwh_schema.sql` and the DWH adapter tests.

## Open questions

None. The request fixes the schema scope, identity rules, access patterns, non-goals, and verification cases; remaining adapter-path, retry-policy, timestamp-default, and test-placement choices follow current repository seams and are recorded as assumptions.
