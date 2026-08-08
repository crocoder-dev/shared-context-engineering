# Conversation ETL

`cli/src/services/conversation_etl.rs` exposes `ConversationEtl`, the
CLI-independent composition boundary for the repository-source conversation
pipelines. Callers provide a repository ID, an already-open
`RepositoryAgentTraceDb`, and the lock-owning `AgentTraceDwhReplica`:

```rust
ConversationEtl::default().run(repository_id, source, replica)?;
```

The runner verifies source metadata once, then executes the existing
`MessagesEtl` and `PartsEtl` table runners. It reports
`ConversationEtlStats` with the complete table-level message and part stats.
Batch sizes default to 500 and can be configured independently.

Messages and parts do not share a transaction or watermark. Each table commits
its own facts, lineage dimensions, replay/conflict checks, and
`(repository_id, source_instance_id, source_table)` watermark. Consequently,
parts may be ingested before their parent message, and a no-op table does not
prevent the other table from advancing. Parts with equal timestamps are
reconstructed by `generated_at_unix_ms, source_part_id`.

This composition layer owns no credentials, remote transport, pull/push calls,
CLI command wiring, scheduling, or control-plane behavior. See
[conversation-messages-etl.md](conversation-messages-etl.md),
[conversation-parts-etl.md](conversation-parts-etl.md),
[agent-trace-dwh-replica.md](agent-trace-dwh-replica.md), and
[agent-trace-etl.md](agent-trace-etl.md).
