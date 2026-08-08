# Decision: Allow empty-remote auto-initialization of the Agent Trace DWH schema through AgentTraceDwhReplica

Date: 2026-08-08
Status: Accepted
Plan: `context/plans/agent-trace-dwh-turso-sync-replica.md`
Task: `T04, T05, T06, T07, T08`
Supersedes: `context/decisions/2026-08-08-agent-trace-dwh-turso-sync-replica-ownership.md`

## Context

The prior `agent-trace-dwh-turso-sync-replica-ownership` decision required
`AgentTraceDwhReplica::open()` to verify DWH schema readiness only and never
provision or migrate schema, on the grounds that a replica-side migration
risked silently diverging from the remote's authoritative migration contract.
In practice this made every fresh DWH remote (including disposable
integration-test remotes and a genuinely new production DWH) a manual
provisioning step outside any implemented tool, even though
`AgentTraceDwhDbSpec::migrations()` is already the single authoritative
migration list and `AgentTraceDwhDb::run_migrations()` already exists as the
one implementation that applies it. The change request behind this plan's
later tasks (T04–T07) asked for the replica to safely bootstrap that one
narrow case — a genuinely empty remote — while still refusing to touch any
non-empty schema it cannot already recognize as fully ready.

## Decision

`AgentTraceDwhReplica::open()` may now auto-initialize a **genuinely empty**
remote DWH schema, but must never repair, upgrade, or partially complete an
existing non-empty incompatible schema. This is implemented as a
classify-then-branch state machine: `AgentTraceDwhDb::classify_schema_state()`
(`Ready`, `Empty`, `Incompatible(reason)`) inspects the opened connection
after bootstrap. `Ready` is left unchanged. A genuinely `Empty` schema — no
`__sce_migrations` table, none of the seven DWH contract tables, and no other
user-defined table — is initialized locally via the existing
`AgentTraceDwhDb::run_migrations()` and published with a single `push()`,
narrowly recovering from a push failure with exactly one best-effort `pull()`
plus a readiness re-verification (the original push error is returned unless
that re-verification now reports `Ready`, treating a racing initializer's
publish as success). Every other case — an unrelated schema, a partial DWH
schema, or a migration ledger with unexpected entries — classifies
`Incompatible` and fails `open()` loudly, exactly as before.

SCE remains the sole owner of the DWH schema and its migration list;
control-plane's role is unchanged — remote provisioning and credentials only,
with no schema or migration copy/bundle of its own.

## Rationale

Auto-initializing only the genuinely-empty case keeps the original decision's
core safety property — never silently diverge from or repair an existing
remote schema — while removing an unnecessary manual step for the one case
that has exactly one correct outcome: a brand-new remote has no schema to
diverge from, and `AgentTraceDwhDbSpec::migrations()` is already the sole
authoritative source for what that schema must be. Routing initialization
through the existing `run_migrations()` rather than a new implementation
keeps a single migration code path for both local and replica-initialized
schemas. The narrow one-`pull()`-and-re-verify recovery, rather than a
generic retry or swallowed error, keeps the boundary between "another
initializer already finished this" and "something is actually broken"
explicit and auditable.

## Alternatives considered

- **Keep verify-only `open()` (prior decision)** — simplest and safest by
  construction, but leaves every fresh remote requiring a manual or
  external provisioning step this plan's tests and any future ETL bootstrap
  would otherwise need to duplicate.
- **Auto-repair or auto-upgrade a non-empty, non-ready schema** — would make
  `open()` fully self-sufficient for any remote state, but risks silently
  altering or upgrading a schema the replica cannot prove is safe to change;
  rejected as a separate, later design decision, not folded into this one.
- **Let control-plane own or bundle a copy of the DWH schema for
  provisioning** — would move initialization out of the replica boundary
  entirely, but couples control-plane to SCE's migration list and schema
  ownership, which this plan explicitly keeps out of scope.
- **Generic retry/backoff on any push failure** — simpler to write, but
  would blur "another initializer won the race" (safe) with a genuine,
  unresolved push failure (must surface); rejected in favor of the narrow
  one-pull-and-re-verify check with the original error preserved on failure.

## Compatibility and risks

- Narrows, rather than removes, the prior guardrail: the "never repair a
  non-empty incompatible schema" property is unchanged and still enforced by
  `classify_schema_state()`'s `Incompatible` branch.
- Risk: a Turso Sync remote implementation other than the pinned local
  `tursodb --sync-server` could surface real push conflicts under
  concurrent first initialization; the one-`pull()`-and-re-verify recovery
  path exists for exactly this case but was not exercised by an observed
  conflict in this repository's integration harness (T07), so it remains
  unproven against other remote implementations. Mitigated by the recovery
  path never swallowing an unresolved failure — it still surfaces the
  original push error when re-verification does not report `Ready`.
- Risk: a freshly bootstrapped Turso Sync database carries internal
  bookkeeping tables (`turso_cdc`, `turso_cdc_version`,
  `__turso_internal*`) that are not part of the DWH contract;
  `classify_schema_state()` excludes them from its `sqlite_master` scan so a
  genuinely empty Turso Sync remote/replica still classifies `Empty`.
  Mitigated by this being a fixed, named exclusion list rather than a broad
  pattern.
- No runtime compatibility impact: `AgentTraceDwhReplica` is still not wired
  into any lifecycle provider, doctor/setup flow, CLI command, or background
  sync.

## Guardrails

- `AgentTraceDwhReplica::open()` may run `AgentTraceDwhDb::run_migrations()`
  and `push()` only when `classify_schema_state()` returns genuinely `Empty`
  (no ledger table and no other user-defined table at all).
- `open()` must never add, repair, upgrade, or partially complete an
  `Incompatible` schema; it must fail loudly instead.
- `AgentTraceDwhDbSpec::migrations()` remains the sole authoritative
  migration list; no second migration implementation may exist for the DWH
  schema.
- The push-failure recovery path may perform exactly one best-effort
  `pull()` plus one readiness re-verification; it must return the original
  push failure — never a swallowed or generic error — when that
  re-verification does not report `Ready`.
- Control-plane must not gain ownership of, or a bundled copy of, the DWH
  schema or migration list as part of this guardrail revision.

## Consequences

- A future ETL bridge process (or an integration test) can call
  `AgentTraceDwhReplica::open()` against a brand-new, genuinely empty DWH
  remote and get a `Ready` replica back without any external provisioning
  step.
- Two or more local replicas racing to initialize the same empty remote
  converge on exactly one valid schema/ledger (proven by T07's six-way
  concurrent test), so the auto-initialization path is safe to call from
  multiple independent processes without external coordination.
- Automatic repair or upgrade of an existing non-empty DWH schema remains a
  separate, later design decision; this decision does not authorize it.

## Follow-up

- None.

## References

- Plan: [`agent-trace-dwh-turso-sync-replica`](../plans/agent-trace-dwh-turso-sync-replica.md)
- Task: `T04, T05, T06, T07, T08`
- Current-state context: [`agent-trace-dwh-replica.md`](../sce/agent-trace-dwh-replica.md), [`agent-trace-dwh-db.md`](../sce/agent-trace-dwh-db.md)
- Evidence: [`agent_trace_dwh_replica/replica.rs`](../../cli/src/services/agent_trace_dwh_replica/replica.rs), [`agent_trace_dwh_db/mod.rs`](../../cli/src/services/agent_trace_dwh_db/mod.rs)
- Related decision: [`agent-trace-dwh-turso-sync-replica-ownership`](2026-08-08-agent-trace-dwh-turso-sync-replica-ownership.md) (superseded by this record)
