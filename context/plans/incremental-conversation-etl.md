# Plan: incremental-conversation-etl

## Change summary

Extend the existing CLI-independent Agent Trace ETL bridge with two independently watermarked conversation pipelines: repository-scoped Agent Trace `messages` rows into DWH `messages`, and source `parts` rows into DWH `message_parts`. Both pipelines will reuse the PR4 short-source-snapshot, bounded contention retry, destination transaction, watermark, and stats mechanics while preserving source lineage, logical message identity, supported part types, exact part text, SHA-256 hashes, and deterministic ordering.

Expose a `ConversationEtl` runner and table-level stats without coupling the ETLs to pull/push, credentials, CLI orchestration, control-plane behavior, or the deferred code-change pipeline. The existing DWH baseline migration remains unchanged because it already contains the required destination columns and identity indexes. The source fields synchronized by these high-watermark pipelines are treated as append-only/immutable for ETL purposes; update CDC is deliberately not added.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the check that proves it. `/validate` runs these checks; no task in the stack performs final validation.

- [ ] AC1: Shared ETL mechanics used by `agent_traces`, `messages`, and `parts` include bounded source-contention retry, absent-watermark-as-zero reads, validated batch sizes, atomic watermark upserts, and common batch accounting without introducing a broad generic ETL trait hierarchy; existing Agent Trace ETL behavior remains unchanged.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_etl`
- [ ] AC2: The messages ETL extracts `id, session_id, message_id, role, generated_at_unix_ms` with `id > watermark ORDER BY id ASC LIMIT batch_size` in a short read transaction, obtains and validates `source_instance_id` through repository metadata, and atomically loads each batch with the `messages` watermark.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml conversation_messages_etl`
- [ ] AC3: Message destination identity is `(repository_id, session_id, message_id)`; same role and timestamp replay is counted as `already_present`, while a differing role or `generated_at_unix_ms` fails with a deterministic integrity conflict and rolls back the complete batch and watermark.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml conversation_messages_identity`
- [ ] AC4: The parts ETL extracts `id, type, text, message_id, session_id, generated_at_unix_ms` incrementally in a short source read transaction, accepts exactly `text`, `reasoning`, `patch`, and `question` through the existing `PartType` representation, rejects other values explicitly, preserves text verbatim, and stores lowercase hexadecimal SHA-256 of the exact UTF-8 bytes.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml conversation_parts_etl`
- [ ] AC5: Part destination identity is `(repository_id, source_instance_id, source_part_id)`; matching `session_id`, `message_id`, `part_type`, `text_sha256`, and timestamp is an idempotent replay, any mismatch fails loudly and rolls back the batch, and parts can load without a parent message row.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml conversation_parts_identity`
- [ ] AC6: A conversation-level API exposes independently configurable/default-batched message and part runs with stats matching existing ETL conventions; messages and parts have separate `(repository_id, source_instance_id, source_table)` watermarks, may progress independently, and no conversation-level transaction or shared watermark couples them.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml conversation_etl`
- [ ] AC7: End-to-end tests prove initial and incremental sync, no-op reruns, logical message replay/conflict, exact text/hash and supported part types, equal-timestamp ordering by `source_part_id`, source-lineage ID collisions across source instances, part-before-message ingestion, independent watermarks, source contention with concurrent writers, and injected mid-batch rollback followed by successful replay for both tables.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml conversation_etl`
- [ ] AC8: Durable context documents the messages/parts DWH pipeline, separate watermarks, logical message identity, source-scoped part identity, verbatim text and hashing, deterministic ordering, absence of a message-to-part foreign-key requirement, append-only source assumption, and pull/push as orchestration concerns; source inspection confirms no existing production code intentionally updates synchronized message/part fields beyond schema-maintenance triggers.
  - Validate: inspect the updated conversation ETL, DWH, replica, shared ETL, root architecture/context-map/glossary files and the source `messages`/`parts` writers against the implementation; confirm only `updated_at` triggers issue SQL `UPDATE` for these tables.

### Full validation

Repository-wide checks `/validate` runs after the last task, regardless of which criterion they map to.

- `nix run .#pkl-check-generated`
- `nix flake check`

### Context sync

- Update `context/overview.md`, `context/architecture.md`, and `context/glossary.md` with the conversation ETL boundary and independent watermark/identity terminology.
- Extend `context/sce/agent-trace-etl.md`, `context/sce/agent-trace-dwh-db.md`, `context/sce/agent-trace-dwh-replica.md`, `context/sce/shared-turso-db.md`, and `context/sce/agent-trace-db.md` to describe the implemented messages/parts behavior.
- Add or update a focused conversation ETL domain context file and index it from `context/context-map.md`.

## Constraints and non-goals

- **In scope:** shared PR4 ETL helper extraction; source row models and short snapshot extraction for `messages` and `parts`; deterministic transforms; DWH fact/dimension/watermark transactions; message/part conflict validation; conversation runner/stats; focused filesystem/in-memory-DWH and source-contention tests; and durable documentation.
- **Out of scope:** `diff_traces` to `code_changes`; post-commit intersection ETL; commits, session or model materialization; control-plane calls; DWH provisioning; credential retrieval; OAuth; CLI or background sync orchestration; archive/search; message update CDC; and remote-to-source synchronization.
- **Constraints:** use the existing `RepositoryAgentTraceDb` metadata API; use the existing `AgentTraceDwhReplica` ownership boundary; never call `pull()` or `push()` from ETL; end source read transactions before transformation or destination work; retry only bounded transient source lock contention; do not add a parent-message foreign key; do not modify `001_dwh_schema.sql`; do not use timestamp-only cursors or ordering; and do not add a broad generic ETL trait framework.
- **Non-goal:** detect or reconcile historical updates to rows at or below a committed integer-ID watermark. Fields synchronized from `messages` and `parts` are append-only/immutable after insertion for ETL purposes, even though the source schema's updated-at triggers technically permit SQL updates.

## Assumptions

- The existing DWH baseline columns and indexes are sufficient: message rows store source lineage plus role/timestamp, and message parts store source lineage, source ID, type, text, text hash, and timestamp without a migration.
- `MessageRole` remains the authoritative supported representation for `user` and `assistant`; invalid source message roles fail explicitly rather than bypassing the destination constraint.
- `PartType` remains the authoritative supported representation for `text`, `reasoning`, `patch`, and `question`; unknown source values fail explicitly.
- The default batch size remains `500`, with one validated configuration seam shared by the table runners or an equivalent small local seam consistent with `AgentTraceEtl`.
- `ConversationEtl::run` may execute messages then parts sequentially, but each table commits facts and its own watermark independently; a failure in one table cannot roll back a previously committed batch in the other table.
- Test-only destination failure injection is acceptable for proving atomic rollback and does not become a production error path.

## Task stack

- [x] T01: `Extract shared ETL mechanics without changing Agent Trace behavior` (status:complete)
  - Task ID: T01
  - Goal: Move genuinely reusable PR4 mechanics behind small internal helpers that can serve three table ETLs.
  - Boundaries (in/out of scope): In — shared source-contention classification/retry and rollback hook, watermark read/upsert helpers keyed by repository/source/table, batch-size validation, common table batch-stat shape where it fits existing conventions, and updates to Agent Trace ETL tests/call sites. Out — messages/parts behavior, generic ETL traits, schema changes, API orchestration, and documentation beyond implementation comments.
  - Dependencies: none
  - Done when: `AgentTraceEtl` uses the shared helpers, its existing source snapshot and atomic load tests still pass, and the helper surface is small enough that row extraction/transformation/loading remain table-specific.
  - Verification notes (commands or checks): `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_etl`; `nix develop -c sh -c 'cd cli && cargo fmt'`.
  - Implementation evidence: Added the internal `cli/src/services/etl.rs` helper module for bounded source-contention retry/classification, positive batch-size validation, table watermark reads/upserts, and shared batch accounting. Registered it as `pub(crate) mod etl`; `AgentTraceEtl` now uses these helpers while retaining table-specific extraction, transformation, and loading behavior.
  - Verification evidence: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_etl` passed all 15 focused tests; `nix develop -c sh -c 'cd cli && cargo fmt'` passed; `git diff --check` passed.

- [ ] T02: `Implement independently watermarked messages ETL` (status:todo)
  - Task ID: T02
  - Goal: Add incremental source extraction and transactional DWH loading for logical messages.
  - Boundaries (in/out of scope): In — `SourceMessage`, exact source projection/query, supported role validation, source metadata lookup, logical identity lookup/insert/verification, `messages` watermark handling, stats, and message-focused initial/incremental/replay/conflict/rollback tests. Out — parts, conversation orchestration, code-change ETL, pull/push, and source update tracking.
  - Dependencies: T01
  - Done when: message batches use `id > watermark ORDER BY id ASC LIMIT ?`, source reads are short and contention-safe, missing rows insert all required lineage/content fields, equal role/timestamp logical replays increment `already_present`, differing role or timestamp returns an integrity error containing repository/session/message identity, and every batch failure leaves rows and the `messages` watermark unchanged.
  - Verification notes (commands or checks): `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml conversation_messages_etl`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml conversation_messages_identity`; `nix develop -c sh -c 'cd cli && cargo fmt'`.

- [ ] T03: `Implement source-lineage-scoped parts ETL` (status:todo)
  - Task ID: T03
  - Goal: Add incremental, verbatim-preserving message-part extraction and transactional loading.
  - Boundaries (in/out of scope): In — `SourceMessagePart`, exact source projection/query, `PartType` conversion, UTF-8 SHA-256 transform, source-part identity verification/insertion, no-parent requirement, `parts` watermark handling, ordering and part-focused tests, and source-writer contention coverage. Out — messages ETL changes except shared test fixtures, conversation runner, code-change ETL, pull/push, and CDC.
  - Dependencies: T01
  - Done when: parts use `id > watermark ORDER BY id ASC LIMIT ?`, valid types are preserved exactly, unknown types fail, text bytes round-trip exactly, hash values are lowercase SHA-256, identical source-lineage replays are counted without duplication, conflicts fail without overwrite, same local IDs from different source instances coexist, and a part batch succeeds before its parent message exists.
  - Verification notes (commands or checks): `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml conversation_parts_etl`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml conversation_parts_identity`; `nix develop -c sh -c 'cd cli && cargo fmt'`.

- [ ] T04: `Expose ConversationEtl and prove independent table progress` (status:todo)
  - Task ID: T04
  - Goal: Provide the conversation-level API and end-to-end proof that message and part ETLs share mechanics but not progress state.
  - Boundaries (in/out of scope): In — `ConversationEtl`, `ConversationEtlStats`, table-runner composition, replica-owned execution, independent message/part watermark tests, initial/incremental/no-op orchestration tests, out-of-order reconstruction/order checks, source contention integration tests for both source tables, and batch rollback/replay coverage through the public table runners. Out — pull/push calls, credentials, CLI command wiring, background scheduling, code changes, and remote orchestration.
  - Dependencies: T02, T03
  - Done when: callers can run `conversation_etl.run(repository_id, source, replica)`, stats expose both table results, messages and parts can advance independently, parts-before-messages remains valid, equal timestamps reconstruct by `generated_at_unix_ms, source_part_id`, and the API contains no transport or control-plane behavior.
  - Verification notes (commands or checks): `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml conversation_etl`; inspect the runner for absence of `pull()`/`push()` and credential access; `nix develop -c sh -c 'cd cli && cargo fmt'`.

- [ ] T05: `Record the conversation ETL and append-only architecture contract` (status:todo)
  - Task ID: T05
  - Goal: Make the implemented conversation pipeline and its source immutability assumption durable in repository context.
  - Boundaries (in/out of scope): In — focused conversation ETL context, context-map index entry, updates to root architecture/overview/glossary and related Agent Trace ETL/DWH/replica/source documents, and an inspection of production message/part writers for append-only violations. Out — code changes, migration edits, source update CDC, CLI/control-plane docs, and unrelated context cleanup.
  - Dependencies: T04
  - Done when: durable context describes the two source-to-DWH flows, independent watermarks, exact identity/conflict rules, verbatim text/hash, deterministic ordering, no parent FK, pull/push separation, and append-only assumption; the source audit records that current production writes are inserts and only schema-maintenance `updated_at` triggers issue updates, or identifies a concrete architectural conflict if that is no longer true.
  - Verification notes (commands or checks): inspect the documented paths against `cli/src/services/agent_trace_etl/`, the source repository adapter, DWH schema, replica API, and `grep -RInE 'UPDATE[[:space:]]+(messages|parts)' cli/src cli/migrations` output; run `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml conversation_etl`.

## Open questions

None. The request fixes the identity, transaction, ordering, source-safety, API, and non-goal boundaries, and the existing DWH baseline already supplies the required destination schema without a migration.
