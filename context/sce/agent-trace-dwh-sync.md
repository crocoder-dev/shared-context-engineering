# Agent Trace DWH Sync Orchestrator

`AgentTraceDwhSync` in `cli/src/services/agent_trace_dwh_sync.rs` is the single orchestration boundary composing an [`AgentTraceDwhReplica`](agent-trace-dwh-replica.md) with the three independent ETL bridges — `AgentTraceEtl` (see [agent-trace-etl.md](agent-trace-etl.md)), `ConversationEtl` (see [conversation-etl.md](conversation-etl.md)), and `CodeChangesEtl` (see [code-changes-etl.md](code-changes-etl.md)) — into one sync call. It extends nothing in any of those four modules: it calls their existing public APIs unmodified.

> This file currently documents the core service shape delivered by plan task T01 (`context/plans/agent-trace-dwh-sync.md`): the `run()` sequence and its stats/error shape, proven against a fresh empty remote and a following no-op run. Stage-identified failure semantics, push-failure recovery, fresh-replica reconstruction, multi-repository/multi-source-instance/cross-client convergence, and the full set of durable invariants land with that plan's later tasks and its final documentation task (T08), which supersedes and expands this file.

## Shape

`AgentTraceDwhSync { agent_trace_etl: AgentTraceEtl, conversation_etl: ConversationEtl, code_changes_etl: CodeChangesEtl }` implements `Default`, reusing each ETL's own default batch sizing — the orchestrator adds no configuration of its own.

`run(&self, repository_id: &str, source: &RepositoryAgentTraceDb, replica_config: AgentTraceDwhReplicaConfig) -> Result<AgentTraceDwhSyncStats, AgentTraceDwhSyncError>` performs exactly one sequence, holding the bridge lock for its full duration:

1. `AgentTraceDwhReplica::open(replica_config)` — one call, consuming the caller-supplied config.
2. `replica.pull()`.
3. `AgentTraceEtl::run(repository_id, source, &replica)`.
4. `ConversationEtl::run(repository_id, source, &replica)`.
5. `CodeChangesEtl::run(repository_id, source, &replica)`.
6. `replica.push()` — only on full success of every prior step.

No global transaction wraps the three ETLs: each still commits its own facts and watermark independently, exactly as it does when called directly through the replica. The opened replica is dropped when `run()` returns (success or failure), releasing the bridge lock.

## Stats and errors

`AgentTraceDwhSyncStats { pulled_changes: bool, agent_traces: AgentTraceEtlStats, conversation: ConversationEtlStats, code_changes: CodeChangesEtlStats }` is returned only on full success.

`AgentTraceDwhSyncError` is a stage-tagged enum (`ReplicaOpen`, `Pull`, `AgentTraceEtl`, `ConversationEtl`, `CodeChangesEtl`, `Push`) with manual `Debug`/`Display`/`std::error::Error`, mirroring `AgentTraceDwhReplicaError`'s pattern. `run()` short-circuits at the first failing stage: no later stage runs, and `push()` never runs unless every ETL succeeded. `ReplicaOpen`/`Pull`/`Push` wrap an already token-redacted `AgentTraceDwhReplicaError`; the three ETL-stage variants wrap `anyhow::Error`, which never observes the caller's auth token in the first place, since ETL never touches credentials.

## Observed Turso Sync behavior: `pulled_changes` after a fresh open

Because `run()` opens a brand-new replica connection every call (`replica_config` is consumed, not held across calls), the `pull()` immediately following *any* prior session's successful `push()` — including this orchestrator's own immediately preceding `run()` call — reports `pulled_changes == true`. This happens even when the pulled data already exactly matches what's on disk: the new connection has no local record that it (or the previous session sharing its local file) already observed that push, so it must reconcile once. `pulled_changes` only settles to `false` once a `run()` call observes no push from any source since the previous `run()`'s own reconciliation pull.

Practical effect: a caller cannot treat `pulled_changes == true` as evidence that new *logical* rows arrived — it only means this session's replica needed at least one reconciliation round-trip. The ETL stats (`extracted`/`inserted` per stage) remain the authoritative signal for whether any new source data was processed; they are unaffected by this reconciliation echo and read zero on a genuine no-op run regardless of `pulled_changes`.

## See also

[agent-trace-dwh-replica.md](agent-trace-dwh-replica.md), [agent-trace-etl.md](agent-trace-etl.md), [conversation-etl.md](conversation-etl.md), [code-changes-etl.md](code-changes-etl.md), [../glossary.md](../glossary.md), [../context-map.md](../context-map.md).
