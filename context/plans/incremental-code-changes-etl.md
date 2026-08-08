# Plan: incremental-code-changes-etl

## Change summary

Implement PR 6's CLI-independent incremental ETL from repository-scoped Agent Trace `diff_traces` rows into the existing DWH `code_changes` table. The runner will use the source row's integer `id` as an independent `(repository_id, source_instance_id, diff_traces)` watermark, snapshot only owned source values in a short read transaction, then strictly normalize patch and structured payloads through the canonical patch services before hashing the exact source payload and deriving code-change metrics.

This extends the completed source identity, DWH schema, single-owner replica, Agent Trace ETL, and conversation ETL boundaries. It preserves `session_id` as the only conversation/code-change join boundary, validates source-lineage-scoped idempotent replay without overwriting conflicts, and commits code-change facts plus watermark atomically. Pull/push, control-plane integration, CLI orchestration, and message-level attribution remain outside the ETL.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the check that proves it. `/validate` runs these checks; no task in the stack performs final validation.

- [ ] AC1: `CodeChangesEtl` exposes a CLI-independent API consistent with the existing table runners, accepts an open repository source and lock-owning `AgentTraceDwhReplica`, obtains `source_instance_id` from repository metadata, uses `diff_traces` as its independent source table, reports extracted/inserted/already-present/batch and before/after watermark stats, and never calls `pull()` or `push()` or acquires credentials.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml code_changes_etl_api`; inspect the runner and replica call paths.
- [ ] AC2: Source extraction uses exactly `id, time_ms, session_id, patch, model_id, tool_name, tool_version, payload_type` with `id > watermark ORDER BY id ASC LIMIT batch_size`, treats a missing watermark as zero, never queries `MAX(id)` or uses timestamps for progress, validates positive batch sizes, and processes all rows through bounded ordered batches.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml code_changes_etl_extraction`.
- [ ] AC3: Each source batch is copied into owned `SourceDiffTrace` values in a short plain read transaction, commits/releases the source snapshot before parsing, hashing, metric derivation, or destination work, retries only existing transient Busy/database-locked contention with the shared bounded retry/rollback mechanics, and permits concurrent source writers to continue.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml code_changes_etl_source`.
- [ ] AC4: `patch` payloads and currently supported `structured` payloads both normalize through one canonical parsing/derivation path into `ParsedPatch`; existing best-effort recent-diff processing may continue to classify malformed rows as skipped, while strict DWH transformation returns an error for malformed, unsupported, or future payload types and never silently treats them as unified patches.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml patch_payload_normalization`; inspect shared parser callers for preserved best-effort behavior.
- [ ] AC5: DWH transformation preserves source metadata (`session_id`, `time_ms`, nullable `model_id`, `tool_name`, nullable `tool_version`, and `payload_type`), retains the source-level `model_id` without deriving a replacement from parsed hunks, derives `files_changed` from `ParsedPatch.files`, derives `lines_added` by counting normalized touched lines with `TouchedLineKind::Added`, derives `lines_removed` by counting normalized touched lines with `TouchedLineKind::Removed` across every file and hunk, and rejects count conversions that cannot be represented by the destination schema.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml code_changes_metrics`.
- [ ] AC6: `patch_sha256` is lowercase hexadecimal SHA-256 of the exact UTF-8 bytes of the original `diff_traces.patch` value for both payload types; normalization never changes the bytes used for hashing.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml code_changes_hash`.
- [ ] AC7: Destination identity is exactly `(repository_id, source_instance_id, source_diff_trace_id)`. A missing identity inserts one `code_changes` row with all required lineage, source metadata, and derived fields; an identical existing row increments `already_present`; any synchronized or derived mismatch returns an integrity error and leaves the existing row unchanged without using silent conflict-ignore or overwrite behavior.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml code_changes_identity`.
- [ ] AC8: Each batch commits repository/source lineage, code-change inserts or verifications, and the `diff_traces` watermark in one destination transaction; transformation failure occurs before destination transaction creation, and any injected destination failure or integrity conflict rolls back all facts, dimensions, and watermark changes so the full batch can be replayed.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml code_changes_atomic`.
- [ ] AC9: Tests prove normal unified patches, valid structured payloads, multiple files, additions/removals including modified/new/deleted files where practical, initial IDs `1..=3`, no-op reruns, growth with IDs `4` and `5`, batch size `2` over five rows, watermark-behind idempotent replay, conflicts, malformed patch/structured/future payload failures, and no watermark advancement past a failed row.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml code_changes_etl`.
- [ ] AC10: Session-level joins remain the only supported conversation relationship: every DWH code-change row preserves `session_id`, code changes can be queried alongside messages/message parts for the same session, no `message_id` is added or inferred, and documentation states that captured data proves only session membership, not causality to an individual message.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml code_changes_session_relationship`; inspect `code_changes` schema/API and the synchronized context documents.
- [ ] AC11: Two source instances of one repository can each ingest local `diff_traces.id = 1` with independent watermarks and coexist in DWH; source contention tests show writers continue and eventually committed rows are observed; a failed batch can be rerun successfully without duplicates.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml code_changes_lineage_contention`.
- [ ] AC12: Durable context documents the source-to-DWH code-activity path, exact source-instance identity and watermark scope, strict transformation behavior, metric/hash definitions, transactional replay/conflict rules, pull/push separation, and the explicit session-only relationship to conversations with no message-level attribution.
  - Validate: inspect `context/sce/agent-trace-etl.md`, `context/sce/agent-trace-dwh-db.md`, `context/sce/agent-trace-dwh-replica.md`, `context/sce/agent-trace-db.md`, `context/overview.md`, `context/architecture.md`, `context/glossary.md`, and `context/context-map.md` against the implementation and tests.

### Full validation

Repository-wide checks `/validate` runs after the last task, regardless of which criterion they map to.

- `nix run .#pkl-check-generated`
- `nix flake check`

### Context sync

- Update `context/overview.md`, `context/architecture.md`, and `context/glossary.md` with the completed code-change ETL and session-level relationship.
- Update `context/sce/agent-trace-etl.md`, `context/sce/agent-trace-dwh-db.md`, `context/sce/agent-trace-dwh-replica.md`, and `context/sce/agent-trace-db.md` from their current “code-change ETL future work” statements to the implemented behavior.
- Add a focused `context/sce/code-changes-etl.md` domain document and register it in `context/context-map.md`.

## Constraints and non-goals

- **In scope:** shared parser refactoring needed for one strict DWH normalization path; `SourceDiffTrace` extraction; strict patch/structured transformation; exact payload hashing; normalized metrics; source-lineage-scoped `code_changes` loading; independent watermarking; stats; focused ETL, rollback, replay, source-contention, and session-join tests; and durable documentation.
- **Out of scope:** message-level attribution or `message_id` on any diff/code-change row; post-commit intersection ETL; commits table; file-level destination rows; raw diff-trace archival; control-plane calls; DWH provisioning; credential retrieval; OAuth; `sce` sync wiring; background syncing; analytics UI; and remote-to-source synchronization.
- **Constraints:** reuse `RepositoryAgentTraceDb::verify_or_initialize_repository_metadata`, `AgentTraceDwhReplica`, `services::etl` helpers, `parse_patch`, structured-patch derivation, `ParsedPatch`, `sha2`, and existing Turso transactions; do not change live source capture semantics; do not hold source read transactions during transformation or destination work; do not use timestamps or a separate `MAX(id)` query for progress; do not change `cli/migrations/agent-trace-dwh/001_dwh_schema.sql` unless implementation proves an existing contract column is insufficient; do not create a generic ETL trait hierarchy; and do not add a new dependency when existing crates suffice.
- **Non-goal:** infer causality between a diff trace and a conversation message. `session_id` is the supported join boundary until source capture supplies explicit message attribution metadata.

## Assumptions

- The existing DWH `code_changes` schema is sufficient: its source-lineage identity, preserved metadata, metric columns, and `patch_sha256` column require no migration.
- The strict structured-payload adapter will wrap or refactor the existing Claude structured derivation so the best-effort recent-window caller retains its current skip classification, while DWH ETL receives `Result<ParsedPatch>` for supported `patch` and `structured` payloads.
- `model_id` and `tool_version` are preserved as nullable source values exactly as extracted; `session_id` is preserved exactly as stored in the source database, including any producer prefix.
- Metric counts are converted to the destination integer type with checked conversion; an impossible overflow is a strict transformation failure rather than truncated output.
- Test-only destination-failure injection is acceptable for proving atomic rollback and does not become a production error path.

## Task stack

- [x] T01: `Extract one canonical strict diff-trace normalization path` (status:complete)
  - Task ID: T01
  - Goal: Make unified and structured diff payloads available through one parser boundary that supports strict DWH errors without changing best-effort recent-diff behavior.
  - Boundaries (in/out of scope): In — parser/structured-patch helper extraction, strict `patch` and `structured` dispatch, explicit unsupported-payload errors, preservation of existing Claude structured formats and model provenance, and pure malformed/valid normalization tests. Out — source SQL, DWH writes, watermarks, ETL orchestration, message attribution, and destination schema changes.
  - Dependencies: none
  - Done when: a strict helper returns `Result<ParsedPatch>` for both supported payload types, malformed or unsupported inputs fail explicitly, structured payloads use the existing derivation logic rather than a second parser, and current recent-diff best-effort callers still classify malformed rows as skipped with their existing behavior.
  - Verification notes (commands or checks): `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml patch_payload_normalization`; `nix develop -c sh -c 'cd cli && cargo fmt'`.
  - Completion evidence: Added public `normalize_diff_trace_payload` dispatching supported `patch` and `structured` payloads into `ParsedPatch`, with explicit malformed, unsupported, and negative-time errors. Reused the strict helper in recent diff-trace parsing while preserving best-effort skip accounting and row-level model provenance injection. Added focused valid/failure normalization tests.
  - Verification: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml patch_payload_normalization` passed (3 tests); `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_db` passed (20 tests); `nix develop -c sh -c 'cd cli && cargo fmt'` passed; `git diff --check` passed.

- [x] T02: `Add bounded diff-trace extraction and deterministic transformation` (status:complete)
  - Task ID: T02
  - Goal: Copy source `diff_traces` rows safely and transform them after the source snapshot into owned code-change values with exact hashing and normalized metrics.
  - Boundaries (in/out of scope): In — `SourceDiffTrace`, exact projection/query, shared source contention retry, `TransformedCodeChange`, strict parser invocation, source metadata preservation, checked metric derivation, and raw-payload SHA-256 tests. Out — destination transactions, watermark mutation, identity replay, replica orchestration, and context documentation.
  - Dependencies: T01
  - Done when: extraction uses only the specified ordered integer-ID query in a short read transaction; transformation starts after extraction returns; patch and structured rows retain their discriminator and metadata; metrics count `ParsedPatch` files/touched-line kinds; and exact source-payload hashes match deterministic expected values.
  - Verification notes (commands or checks): `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml code_changes_etl_extraction`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml code_changes_metrics`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml code_changes_hash`; `nix develop -c sh -c 'cd cli && cargo fmt'`.
  - Completion evidence: Added `cli/src/services/code_changes_etl.rs` with the exact ordered `diff_traces` projection, short read-transaction extraction using shared bounded contention retry, owned source rows, strict patch/structured normalization, source metadata preservation, checked `ParsedPatch` file/touched-line metrics, and lowercase SHA-256 hashing of the original payload bytes. Registered the module in `cli/src/services/mod.rs`; destination loading and watermark mutation remain absent.
  - Verification: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml code_changes_etl_extraction` passed (2 tests); `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml code_changes_metrics` passed (2 tests); `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml code_changes_hash` passed (1 test); `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml code_changes_etl` passed (7 tests); `nix develop -c sh -c 'cd cli && cargo fmt'` passed; `git diff --check` passed.

- [ ] T03: `Load source-lineage code changes atomically with replay validation` (status:todo)
  - Task ID: T03
  - Goal: Insert or verify transformed code-change rows and advance the `diff_traces` watermark in one destination transaction.
  - Boundaries (in/out of scope): In — repository/source lineage ensuring, source-scoped destination lookup, full synchronized/derived content comparison, explicit conflict errors, insert/already-present accounting, watermark upsert, and test-only failure injection/rollback tests. Out — source extraction, parser changes, public run loop, pull/push, credentials, message joins, and documentation.
  - Dependencies: T02
  - Done when: missing `(repository_id, source_instance_id, source_diff_trace_id)` rows insert without conflict-ignore semantics; identical rows count as already present; every compared field mismatch fails without overwrite; code-change rows, dimensions, and watermark roll back together on any destination error; and replay/conflict/rollback tests pass.
  - Verification notes (commands or checks): `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml code_changes_identity`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml code_changes_atomic`; `nix develop -c sh -c 'cd cli && cargo fmt'`.

- [ ] T04: `Expose CodeChangesEtl and prove incremental session-level behavior` (status:todo)
  - Task ID: T04
  - Goal: Add the replica-owned public runner and end-to-end coverage for batching, failure boundaries, independent source instances, contention, and session-level conversation queries.
  - Boundaries (in/out of scope): In — default/configurable batch runner, `CodeChangesEtlStats`, metadata-derived source identity, initial/no-op/growth/batch-boundary runs, invalid-row watermark stopping, source-instance coexistence, source writer contention, rollback/replay, session query coverage with messages/parts, and pull/push absence inspection. Out — CLI/control-plane wiring, orchestration, message-level attribution, commits, and remote synchronization.
  - Dependencies: T03
  - Done when: `CodeChangesEtl::default().run(repository_id, source, replica)` processes only rows above the independent watermark; invalid transformations fail before destination writes; watermarks never advance past failed rows; all requested incremental/replay/conflict/lineage/contention/session scenarios pass; and ETL does not call transport or credential code.
  - Verification notes (commands or checks): `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml code_changes_etl`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml code_changes_lineage_contention`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml code_changes_session_relationship`; `nix develop -c sh -c 'cd cli && cargo fmt'`.

- [ ] T05: `Document the session-level code-change ETL contract` (status:todo)
  - Task ID: T05
  - Goal: Make the implemented code-change source/DWH boundary, strictness, metrics, hash, identity, rollback, and conversation relationship durable and discoverable.
  - Boundaries (in/out of scope): In — focused code-change ETL context, context-map registration, updates to related ETL/source/DWH/replica and root context, and documentation of the session-only join boundary and lack of message causality. Out — implementation changes, CLI/control-plane docs, new attribution metadata, and unrelated context cleanup.
  - Dependencies: T04
  - Done when: durable context matches the code and tests, explicitly states `session_id` is the supported relationship between conversations and code changes, explicitly states there is no reliable message-level attribution, and removes stale “code-change ETL remains future work” claims without implying message causality.
  - Verification notes (commands or checks): inspect the listed context files against `cli/src/services/code_changes_etl.rs` (or the final module path), the source schema, DWH schema, parser helpers, and test coverage; `git diff --check`.

## Open questions

None. The request fixes the source projection, identity, strictness, metrics, hash, session boundary, orchestration exclusions, and test obligations; the existing DWH schema and ETL infrastructure supply the remaining local seams.
