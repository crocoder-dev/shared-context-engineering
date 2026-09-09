# Mutation-scope provenance (`mutation_trace_scope_provenance`)

Durable, insert-once metadata answering one question the verified
mutation-cursor protocol deliberately does not: given a `ScopeId`, which session
and model did that scope represent? It is stored beside — never inside — the
protocol state persisted by the
[mutation-trace store](mutation-trace-store.md), and reached through that same
`MutationTraceStore` seam.

## Table shape

Migration `005_mutation_scope_provenance.sql` adds a sixth table alongside
`004_mutation_trace_protocol.sql`'s five, keyed `scope_id TEXT PRIMARY KEY` with
`session_id TEXT NOT NULL`, a nullable `model_id TEXT`, and the same `created_at`
default the `004` tables use. It deliberately does not duplicate `actor_kind`,
`worktree_id`, or `status` — those remain owned by `mutation_trace_scopes` — and
it stores no `agent_id`.

`session_id` holds a canonical SCE session identity verbatim (`cc_` for Claude,
`cx_` for Codex); `model_id` is already normalized by its producer and may be
`NULL`. Unknown model information is `NULL`, never guessed.

## Provenance is not protocol state

Provenance is metadata *about* an already-established scope. It is not part of
`ProtocolState`, never enters a `DurableTransition` or the CAS batch, and never
participates in deciding `IneligibleUnscoped` / `AiExclusive` / `AiContended`.
The pure protocol works identically when it is entirely absent. `ScopeId` proves
ownership; scope provenance only describes the owning scope.

## Seams

`MutationTraceStore::register_scope_provenance(&ScopeProvenance { scope_id,
session_id, model_id }) -> Result<ScopeProvenance>` is the write seam, returning
whichever row is stored afterwards.
`MutationTraceStore::load_scope_provenance(scope_id) ->
Result<Option<ScopeProvenance>>` is the read seam; a scope with no provenance
reads back `None`, which is an ordinary absence of metadata rather than an error.

## Insert-once semantics

The first persisted row always wins:

- `session_id` is immutable identity. A `scope_id` belongs to exactly one
  session, permanently.
- `model_id` is immutable first-observed descriptive metadata. A stored `NULL`
  is never backfilled by a later discovery, and a stored model is never cleared
  by a later `None` or overwritten by a disagreeing one. No `UPDATE` path exists.

Every replay carrying the same `session_id` therefore succeeds and returns the
stored row unchanged, whatever its model says. The **only** provenance condition
that returns `Err` is a `scope_id` already bound to a different `session_id`, and
that failure leaves the stored row untouched — session identity is never silently
rewritten. A model disagreement is not a conflict: no attribution decision
depends on this metadata, so a disagreement about it is not a reason to deny a
mutation-capable tool.

## Owning-scope requirement

Like `register_scope`, registration requires its owning row to already exist: the
`mutation_trace_scopes` row for `scope_id` is checked before any insert, so
provenance never creates a scope implicitly. Through the `MutationTraceStore`
write seam, provenance therefore cannot be registered without an existing owning
`mutation_trace_scopes` row, and a missing scope leaves no orphan row behind.
That requirement is enforced in the store rather than by a `FOREIGN KEY`, leaving
`004_mutation_trace_protocol.sql` unchanged.
