# Decision: Single-owner, disposable Turso Sync replica boundary for the Agent Trace DWH

Date: 2026-08-08
Status: Accepted
Plan: `context/plans/agent-trace-dwh-turso-sync-replica.md`
Task: T01, T02, T03

## Context

The `2026-08-08-agent-trace-dwh-schema-identity-contract` decision introduced
the Agent Trace DWH destination schema and its explicit-path `AgentTraceDwhDb`
adapter, but deliberately left "a sync URL, credentials, ETL state
transitions, bridge locking, or CLI lifecycle behavior" out of scope, and
repository context still described the DWH as having no canonical local sync
path. A future ETL bridge process needs exactly one place to open a local
Turso Sync connection against the remote DWH, without risking two processes
racing on the same local sync file or the sync boundary silently reusing the
source capture database's multiprocess-WAL settings.

## Decision

Introduce `AgentTraceDwhReplica` (`cli/src/services/agent_trace_dwh_replica/`)
as the sole owner of a Turso Sync connection to a repository's disposable
`agent-trace-sync.db`: exactly one OS process may hold it at a time, enforced
by a non-blocking `BridgeLock` acquired before any Turso access; the replica
is fully reconstructible from the remote DWH and carries no state that is not
recoverable through a fresh bootstrap; callers must supply
`local_path`/`database_url`/`auth_token` explicitly, since the boundary never
discovers, persists, or rotates credentials itself and redacts the token from
every error; and the replica's Turso Sync connection never enables
`experimental_multiprocess_wal`, keeping that setting exclusive to the source
capture database.

## Rationale

A single-owner, lock-guarded, disposable replica is the only design that lets
a future ETL bridge process pull from and push to the remote DWH without
risking split-brain local state or blocking the unrelated multiprocess-WAL
source capture path. Requiring explicit caller-supplied credentials (rather
than the replica discovering or storing them) keeps this boundary decoupled
from OAuth/credential-persistence work that has not been designed yet, and
keeps the replica safe to open in tests and future bridge processes alike
without a hidden credential store to reason about.

## Alternatives considered

- **Let any process open the Turso Sync connection directly** — simpler, but
  risks two processes racing on the same local sync file with no ownership
  guarantee, and offers no natural place to enforce "never enable
  multiprocess WAL on this path."
- **Have the replica discover or persist its own credentials** — would let
  callers omit configuration, but couples this storage boundary to
  unfinished OAuth/credential-discovery design and risks the token leaking
  into diagnostics or disk state before that design exists.
- **Provision/repair DWH schema locally on open** — would make replica open
  self-sufficient, but risks a local replica silently diverging from the
  remote's authoritative migration contract; rejected in favor of verifying
  readiness only and failing loudly on mismatch.

## Compatibility and risks

- Net-new boundary, not wired into any lifecycle provider, doctor/setup flow,
  CLI command, or background sync, so this decision has no runtime
  compatibility impact yet.
- Risk: a future ETL/CLI integration could bypass `AgentTraceDwhReplica` and
  open a competing Turso Sync connection directly, reintroducing the
  split-brain risk this boundary exists to prevent; mitigated by this being
  the only Turso Sync builder in the codebase today and by this decision
  recording the single-owner rule for future reviewers to enforce.
- Risk: a caller could still leak a raw auth token by capturing the
  `AgentTraceDwhReplicaConfig` value directly instead of only its errors;
  mitigated by giving `AgentTraceDwhReplicaConfig` a redacted `Debug`
  implementation.

## Guardrails

- `AgentTraceDwhReplica::open` must acquire the `BridgeLock` before any Turso
  Sync builder, local file, or network access.
- No code path may call `.experimental_multiprocess_wal(true)` when opening
  `agent-trace-sync.db`.
- The replica must not locally provision or migrate DWH schema; a missing or
  incompatible remote schema is a reported failure, not a repair target.
- `AgentTraceDwhReplica` remains the only owner of a Turso Sync builder in
  this codebase; application and hook processes must not open the replica
  path directly.

## Consequences

- A future ETL bridge process can rely on `AgentTraceDwhReplica` for safe
  concurrent-open rejection, credential-safe error reporting, and
  reconstruction after local data loss, without re-deriving any of that
  policy itself.
- Losing or deleting the local `agent-trace-sync.db` is always recoverable
  through a fresh `open()` bootstrap against the remote, so it never needs
  backup/retention handling of its own.
- Any future credential-discovery or OAuth design must hand
  `AgentTraceDwhReplica` an explicit token rather than teaching the replica
  to look one up itself, unless a later decision revisits this guardrail.

## Follow-up

None.

## References

- Plan: [`agent-trace-dwh-turso-sync-replica`](../plans/agent-trace-dwh-turso-sync-replica.md)
- Task: T01, T02, T03
- Current-state context: [`agent-trace-dwh-replica.md`](../sce/agent-trace-dwh-replica.md), [`agent-trace-dwh-db.md`](../sce/agent-trace-dwh-db.md), [`shared-turso-db.md`](../sce/shared-turso-db.md), [`default-path-catalog.md`](../cli/default-path-catalog.md)
- Evidence: [`agent_trace_dwh_replica/replica.rs`](../../cli/src/services/agent_trace_dwh_replica/replica.rs), [`agent_trace_dwh_replica/lock.rs`](../../cli/src/services/agent_trace_dwh_replica/lock.rs)
- Related decision: [`agent-trace-dwh-schema-identity-contract`](2026-08-08-agent-trace-dwh-schema-identity-contract.md)
