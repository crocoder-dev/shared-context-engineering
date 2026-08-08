# Conversation Messages ETL

`cli/src/services/conversation_messages_etl.rs` owns the incremental messages
pipeline from a repository-scoped Agent Trace database into the Agent Trace
DWH. It is a CLI-independent table runner: it does not pull or push the DWH
replica, retrieve credentials, or call CLI orchestration.

## Source and progress

`MessagesEtl` verifies the source `RepositoryMetadata` and uses its
`source_instance_id`. Each bounded batch reads exactly
`id > watermark ORDER BY id ASC LIMIT batch_size` in a short plain `BEGIN`
transaction. Only transient database/table lock contention receives the shared
bounded retry; transformation and destination work begin after the source
snapshot ends. The default batch size is 500 and callers can provide a
validated positive size.

Progress is independently stored under
`(repository_id, source_instance_id, "messages")`. An absent watermark is zero.
The runner repeats batches until extraction is empty and reports extracted,
inserted, already-present, batch, and before/after watermark counts.

## Destination identity and atomicity

A DWH message is logically identified by
`(repository_id, session_id, message_id)`, excluding `source_instance_id`.
Missing rows preserve the source session, message, validated `user` or
`assistant` role, and `generated_at_unix_ms`, together with repository and source
lineage. A matching role/timestamp replay increments `already_present`; a role
or timestamp mismatch returns an integrity conflict naming the repository,
session, and message identity and never overwrites the row.

Each batch commits lineage dimensions, message facts, and the messages
watermark in one `BEGIN IMMEDIATE` destination transaction. A conflict,
transformation failure, or other batch error rolls all of them back so the
source batch can be replayed completely. Parts, conversation orchestration,
source update CDC, and remote synchronization remain separate ETL concerns.

See also: [agent-trace-etl.md](agent-trace-etl.md),
[agent-trace-dwh-db.md](agent-trace-dwh-db.md),
[agent-trace-db.md](agent-trace-db.md), [shared-turso-db.md](shared-turso-db.md),
and [../context-map.md](../context-map.md).
