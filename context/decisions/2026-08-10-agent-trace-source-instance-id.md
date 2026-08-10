# Decision: Give repository-scoped Agent Trace DBs a physical-instance identity independent of repository identity

Date: 2026-08-10
Status: Accepted
Plan: `context/plans/agent-trace-source-instance-id.md`
Task: T01, T02, T03, T04, T05

## Context

`RepositoryAgentTraceDb` identifies a database by `repository_metadata.repository_id`,
a logical Git-repository identity shared by every clone/checkout of the same
repository (see [`repository identity`](../glossary.md)). That identity alone
cannot distinguish two independently created physical `agent-trace.db` files for
the same logical repository — for example, the same repository cloned on two
different machines. A prior, abandoned design (PR #186) attempted to solve
physical-instance identification bundled together with a full remote-ingestion
architecture (a data warehouse, `agent-trace-sync.db`, ETL, and Turso Sync), and
was retired without shipping. The durable, reusable part of that design — a
stable per-physical-database identity — was worth recreating on its own, without
committing to any remote-ingestion architecture.

## Decision

Add `source_instance_id` as a second, independent identity column on
`repository_metadata`, added by the additive migration
`002_repository_source_instance_id.sql`. It identifies one physical database
lineage: generated once by application code the first time a given physical
`agent-trace.db` is initialized, and stable across reopen, `sce setup` reruns,
and process restarts. It is never derived from `repository_id`, remote URL,
checkout ID, filesystem path, hostname, or user/workspace identity, so two
independently created databases for the same logical repository always diverge.
Concurrent first opens of the same physical database converge on exactly one
persisted value through an atomic `UPDATE ... WHERE source_instance_id = ''`
claim; a losing racer's generated candidate is discarded, and an already-valid
stored value is never overwritten. `RepositoryMetadata { repository_id,
source_instance_id }` is the typed result threaded through
`ResolvedAgentTraceStorage` for both the setup/lifecycle resolution path
(`resolve_agent_trace_storage`, which may run migration `002`) and the
no-migration hook-runtime resolution path (`resolve_agent_trace_storage_for_hook_runtime`,
which never runs migration `002` or any migration).

## Rationale

Splitting physical-instance identity from logical-repository identity as two
columns on the same row keeps both concepts queryable together without
conflating them: `repository_id` answers "which logical repository," while
`source_instance_id` answers "which physical database." Generating the value in
application code rather than SQL keeps the identity's shape (UUID v4 today)
free to evolve without a schema dependency, and validating it only as
"non-empty once trimmed" (`is_valid_source_instance_id`) avoids hard-coding
UUID-v4 parsing into downstream consumers. The atomic claim pattern is the
minimal concurrency-safe primitive needed to guarantee exactly one winner
across concurrent SCE processes opening the same new database, without
introducing a separate locking mechanism.

## Alternatives considered

- **Derive the identity from a stable local signal (hostname, filesystem path,
  or checkout ID)** — rejected: none of these are guaranteed stable or unique
  per physical database file, and the request explicitly ruled this out to keep
  the identity meaningful even if a database file is copied or moved.
- **Recreate the full PR #186 architecture (DWH, `agent-trace-sync.db`, ETL,
  Turso Sync) alongside the identity column** — rejected: that architecture was
  abandoned and is unrelated to the local storage-identity problem; bundling it
  back in would reintroduce the same retired complexity this plan exists to
  avoid.
- **Let hook-runtime resolution run migration `002` like any other migration**
  — rejected: it would blur the existing no-migration boundary between
  high-frequency hook-runtime DB access and migration-running setup/lifecycle
  access, a separation this plan preserves and extends rather than erodes.

## Compatibility and risks

- Additive migration only: `001_repository_schema.sql` is untouched, and
  existing/placeholder rows default `source_instance_id` to an empty string,
  so no existing database is invalidated.
- A losing racer under concurrent first-open must discard its generated
  candidate; the atomic claim and always-re-read pattern makes this safe, but
  any future caller of `verify_or_initialize_repository_metadata` must
  preserve that re-read rather than trusting its own candidate.
- Hook-runtime resolution failing closed (no migration fallback) on a
  baseline-only or missing database is intentional: a pre-`002` or pre-setup
  repository must fail with `sce setup` guidance rather than silently
  migrating from the high-frequency hook path.

## Guardrails

- No remote ingestion, sync, DWH, or `agent-trace-sync.db` behavior is
  designed or implied by this decision; `source_instance_id` is local storage
  identity only.
- No workspace, host, or user/account identity is added to the local
  repository Agent Trace DB by this decision.
- `sce doctor`'s read-only diagnose surface stays unchanged; only `sce setup`
  diagnostics report the new identity.

## Consequences

- Every repository-scoped `agent-trace.db` now carries a stable, physically
  unique identity that a future remote-ingestion consumer could use to
  distinguish rows originating from different physical databases for the same
  logical repository — without this decision committing to what that consumer
  looks like.
- The setup/lifecycle-vs-hook-runtime resolution split, already implicit in
  the codebase, is now an explicit, separately named, separately tested
  boundary (`resolve_agent_trace_storage` vs.
  `resolve_agent_trace_storage_for_hook_runtime`), which future callers must
  choose between deliberately rather than defaulting to one path.

## Follow-up

None.

## References

- Plan: [`agent-trace-source-instance-id`](../plans/agent-trace-source-instance-id.md)
- Task: T01, T02, T03, T04, T05
- Current-state context: [`context/cli/agent-trace-storage.md`](../cli/agent-trace-storage.md)
- Current-state context: [`context/sce/agent-trace-db.md`](../sce/agent-trace-db.md)
- Related decision: [`Retire the legacy checkout-scoped Agent Trace DB surface`](2026-07-17-retire-legacy-agent-trace-db.md)
