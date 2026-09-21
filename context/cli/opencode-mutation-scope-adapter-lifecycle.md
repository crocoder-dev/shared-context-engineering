# OpenCode mutation-scope adapter lifecycle and recovery

How the OpenCode mutation-scope adapter
(`cli/src/services/hooks/opencode_mutation_scope/`) drives the T01-frozen
lifecycle boundaries onto the generic in-process ingress seam. Shipped by the
`opencode-mutation-scope-integration` plan's **T04**. For the scope model,
identity encoding, wire contract, plugin, and attribution boundary, see
[`opencode-mutation-scope-integration.md`](opencode-mutation-scope-integration.md).

The adapter processes one hook event at a time under a per-`git-dir` boundary
lock (`opencode-mutation-scope-boundary.lock`), serialising boundary work across
concurrent OpenCode processes.

- **Attempt phases.** An attempt is `PendingStart` → `Active` → `PendingAbandon`.
  `PendingAbandon` means exact terminal evidence has been observed, the OpenCode
  execution is considered finished, but the generic protocol scope may still be
  live and its ambiguous filesystem interval is not yet rebaselined. A
  `PendingAbandon` attempt never returns to `Active`, is never treated as a
  reusable `Start`, and is not removed until the generic `abandon` has
  definitely succeeded.
- **Start** is durable before the seam call: a `PendingStart` attempt is
  persisted, the ingress `start` boundary is driven, then the attempt flips to
  `Active`. `bash` starts on `ShellEnv` only; `write`/`edit`/`apply_patch` start
  write-ahead on `ToolExecuteBefore`. A replayed `Start` whose `(session_id,
  call_id)` is already `PendingAbandon` is refused (`TerminalAttemptBlocked`) and
  fails closed — the terminal identity is never reactivated.
- **Close** is a successful `ToolExecuteAfter` — it drives the ingress `close`
  and removes the attempt. An `After` that finds a `PendingStart` or
  `PendingAbandon` attempt consumes the interval and abandons instead (see
  **Abandon**). A late `ToolError` after a completed `Close` finds no attempt and
  is a harmless no-op.
- **Abandon (exact only) + durable terminal intent + ambiguity consumption.**
  `ToolError` for a tracked tool retires exactly its `(session_id, call_id)`
  attempt. Classification happens before any Git/state work, so a `Delegation` or
  `Untracked` `ToolError` is zero-footprint. For a tracked one, under the
  boundary lock:
  1. `begin_terminal_cleanup` persists the doomed attempt(s) as `PendingAbandon`
     **and** the recovery generation in a single durable write — *before* any
     seam call, so a crash or transient failure cannot lose the intent;
  2. `resolve_recovery` drives an ineligible ingress `flush` while the doomed
     scope and any siblings are still live — the runtime resolves it to
     `IneligibleUnscoped` and advances the cursor past the ambiguous interval;
  3. it drives the ingress `abandon` for each doomed scope, removing the attempt
     from adapter state **only after** that `abandon` returns success;
  4. it drives a second `flush` to clear the rebaseline that `abandon` arms so
     surviving scopes keep their **future** intervals;
  5. recovery clears only once the whole sequence for that generation completes.

  Surviving attempts stay `Active` and untouched. So

  ```text
  Start(A)  Start(B)  mutate(A)  mutate(B)  Abandon(A)  Close(B)
  ```

  can never yield `AiExclusive(B)` for the interval that could contain A's
  changes, while B may still attribute mutations it makes **after** the
  consuming flush.
- **Transient cleanup failure is retried, never dropped.** If the first `flush`,
  the `abandon`, or the rebaseline `flush` fails, `resolve_recovery` relinquishes
  recovery to `Pending{generation}` and returns — the doomed attempt stays
  `PendingAbandon` and (if `abandon` had not yet succeeded) stays in adapter
  state. A new tracked `Start` while recovery is unresolved claims the flush
  (`FlushClaimed`) and replays the whole idempotent `flush`/`abandon`/`flush`
  sequence before it may itself be admitted; a duplicate `ToolError` for the same
  identity also replays it. Only after the sequence completes is the attempt
  forgotten and recovery cleared. The generic `abandon` is idempotent (an
  already-abandoned scope settles as a terminal no-op) and repeating the ambiguity
  `flush` is safe, so replay needs no fragile sub-step bookkeeping. Generation
  ownership still prevents a stale `flush`/`complete` from clearing a newer
  recovery requirement.
- **Broad asynchronous events are non-authoritative.** `SessionIdle`,
  `SessionError`, `SessionDeleted`, and `ServerDisposed` are asynchronous /
  fire-and-forget relative to the synchronous mutation boundary (D10), so a
  delayed one can arrive after a newer tracked call already started, and one
  OpenCode process's `ServerDisposed` cannot be distinguished from another's
  (the wire event carries only checkout identity). They therefore drive **no**
  live-scope abandonment — only exact `ToolError` causal evidence retires an
  attempt. Lingering unconfirmed scopes after a crash or a missing terminal
  event are accepted (D3 keeps them non-AI); this trades availability for
  soundness, consistent with D10/D11. No TTL recovers from this.
- **Recovery barrier.** Durable state under
  `<git-dir>/sce/opencode-mutation-scope-state.json` (guarded by
  `opencode-mutation-scope-state.lock`, held only for individual file ops,
  **never across a seam call**) carries a generation-tracked
  `Clear`/`Pending`/`Flushing` recovery state plus the per-attempt phase. The
  ambiguity-consuming `flush` runs at abandon time, alongside surviving live
  attempts — it is **not** deferred until `attempts.is_empty()`. A successful
  consume returns recovery to `Clear` with survivors still `Active`; a failed
  step retains `Pending`/recovery-required with the doomed attempt still
  `PendingAbandon`, and any new tracked `Start` while recovery is unresolved
  stays fail-closed and replays the consume. Recovery is complete only when every
  `PendingAbandon` attempt for that generation has been abandoned and the
  rebaseline `flush` has succeeded — a generic pending `flush` that still sees a
  `PendingAbandon` attempt must retire it, not merely clear recovery and admit
  new work. Generation ownership prevents an old `flush` completion from clearing
  a newer recovery requirement. `normalize_recovery_after_boundary_lock_acquired`
  demotes an orphaned `Flushing{g}` (a crashed mid-flush invocation) back to
  `Pending{g}` when the next boundary acquires the lock.
- **Fail-closed.** Any failure to durably establish a tracked `Start` exits the
  adapter non-zero (`SCE could not establish OpenCode mutation attribution for
  this tool execution.`); the generated plugin turns that into a thrown hook that
  blocks the tool. `Close`/terminal paths are best-effort, falling back to
  `abandon_and_consume` on seam failure.
- **No same-session sweep, no TTL.** Attempts are keyed only by `(session_id,
  call_id)`; a second live call runs alongside the first (D9). Abandoning or
  retiring one attempt never sweeps a sibling. Nothing retires a scope on elapsed
  time (D11) — an interrupted tool's interval stays `IneligibleUnscoped` rather
  than risk a false positive while an orphan child mutates.
- **No protocol or Quint change.** This durability contract is entirely adapter
  recovery bookkeeping. The generic protocol already provides `flush`, `abandon`
  (idempotent), confirmation-required attribution, and `needs_rebaseline`; the
  existing Quint checks are re-run only as regression verification.

## Owner evidence and the doctor-repair path

Each `AdapterAttempt` now carries an optional `owner: Option<ProcessOwner>`
(`#[serde(default)]`), stamped with the shared
[`mutation_scope_owner::current_process_owner()`](pi-mutation-scope-integration.md)
when a `PendingStart` attempt is allocated. A state file written before this
field existed deserializes its absence as `owner: None`, never inferred —
`assess_repairability`/`repair_blocked` below treat a missing owner exactly
like a live/unprovable one.

This evidence exists only to back a `doctor`-invoked repair for the
`blocked` `PendingStart` shape described in
[opencode-mutation-scope-health.md](opencode-mutation-scope-health.md#why-a-stale-pendingstart-is-blocked-not-recovering--even-alongside-a-pendingflushing-recovery-generation);
it changes none of the ordinary hook lifecycle above and none of
`classify_health`'s reachable classifications.

- `assess_repairability(git_dir)` reports `AutoFixable` only when every
  currently `PendingStart` attempt has a recorded owner the shared
  `mutation_scope_owner::is_definitely_dead` proves dead; otherwise
  `ManualOnly` (no owner recorded, a live owner, or unprovable liveness).
- `repair_blocked(git_dir, repository_root, logger, seam)` acquires the
  `AdapterBoundaryLock`, normalizes an orphaned `Flushing`, then — in one
  `AdapterStateLock` transaction — re-reads state fresh and re-evaluates
  liveness against the *current* state rather than trusting any earlier
  read. Only attempts still `PendingStart` with a still-dead owner at that
  moment transition to `PendingAbandon`, using the same durable
  `begin_terminal_cleanup` shape the ordinary `ToolError` path already uses.
  The state lock is released before the unmodified `flush`/`abandon`/`flush`
  seam sequence (`resolve_recovery`) runs, so the lock is never held across a
  seam call. A losing re-proof (the attempt is no longer `PendingStart`, or
  no attempt is currently dead-owned) is a safe no-op: no seam call, no
  write. If a seam call fails mid-repair, the resulting `PendingAbandon`/
  `Pending` state is left exactly as an ordinary failed `ToolError` cleanup
  would leave it, so the existing self-healing retry (any later tracked
  admission) resumes and completes it without duplicating or resurrecting
  the attempt.
- Neither function is wired into `doctor` yet — `sce doctor --fix` cannot
  reach `repair_blocked` until a later task dispatches to it from
  `execute_doctor_with_lifecycle_providers`.

## Related context

- [OpenCode mutation-scope integration](opencode-mutation-scope-integration.md)
- [OpenCode mutation-scope health classification](opencode-mutation-scope-health.md)
- [Mutation-scope hook ingress](mutation-scope-hook-ingress.md)
- [Mutation-scope runtime: the harness-adapter contract](mutation-scope-runtime.md)
- [Mutation-scope provenance](mutation-scope-provenance.md)
- [Pi mutation-scope integration](pi-mutation-scope-integration.md) (the shared
  `mutation_scope_owner` liveness primitive this task reuses)
