# Conversation Parts ETL

`cli/src/services/conversation_parts_etl.rs` owns the independently watermarked
parts pipeline from a repository-scoped Agent Trace source database into the
DWH `message_parts` fact table. It is CLI-independent: it does not pull or
push the replica, retrieve credentials, call orchestration, or require a
parent message row.

## Source and progress

`PartsEtl` verifies the source repository metadata and uses its
`source_instance_id`. Each bounded batch reads exactly
`id > watermark ORDER BY id ASC LIMIT batch_size` in a short plain `BEGIN`
transaction. Only transient database/table lock contention receives the shared
bounded retry; transformation and destination work start after the source
snapshot ends. The default batch size is 500 and callers can provide a
validated positive size.

Progress is independently stored under
`(repository_id, source_instance_id, "parts")`. An absent watermark is zero.
The runner reports extracted, inserted, already-present, batch, and
before/after watermark counts. A failed destination batch leaves facts,
lineage dimensions, and the watermark unchanged for complete replay.

## Transformation and identity

The source `type` is converted through the existing `PartType` representation;
only `text`, `reasoning`, `patch`, and `question` are accepted. The source text
is copied verbatim and its exact UTF-8 bytes receive a lowercase hexadecimal
SHA-256 hash.

A destination part is identified by
`(repository_id, source_instance_id, source_part_id)`. A matching replay
verifies session, message, type, exact text, hash, and timestamp, then counts
`already_present`; any mismatch is an integrity conflict and never overwrites
the existing row. Because the local source ID is scoped to the source
instance, independently created source databases can contribute the same
integer ID without collision.

The DWH schema has no foreign key from `message_parts` to `messages`, so a part
batch may commit before its parent message exists. When parts share a message
timestamp, consumers reconstruct deterministic order using
`generated_at_unix_ms, source_part_id`.

## Source immutability and boundaries

The integer-ID watermark treats synchronized source fields as append-only for
ETL purposes. Update CDC is deliberately not part of this pipeline. Remote
synchronization, credentials, pull/push orchestration, conversation-level
composition, and code-change ingestion remain separate concerns.

See also: [conversation-messages-etl.md](conversation-messages-etl.md),
[agent-trace-etl.md](agent-trace-etl.md),
[agent-trace-dwh-db.md](agent-trace-dwh-db.md),
[agent-trace-db.md](agent-trace-db.md),
[shared-turso-db.md](shared-turso-db.md), and [../context-map.md](../context-map.md).
