# Mutation-scope health status

`sce doctor` reports the *runtime* liveness of each mutation-scope adapter's
persisted recovery bookkeeping at `<git-dir>/sce/{adapter}-mutation-scope-state.json`,
alongside its existing structural registration checks (files, hook entries,
Codex trust/policy state). Structural health alone cannot detect a durable
recovery wedge in that state — Claude's original incident (a repository-wide
`PreToolUse` lockout from a stale `recovery_pending = true` with leftover
`attempts`) was invisible to `sce doctor` until this feature existed.

## Shared status vocabulary

`MutationScopeHealthStatus` (`cli/src/services/hooks/mutation_scope_health.rs`)
is defined once and reused by all four adapter classifiers and by doctor. No
adapter redefines it.

- **`Healthy`** — no persisted recovery condition blocks normal future
  mutation-capable admission. An absent state file is `Healthy` (never run, or
  never set up), not "unexercised."
- **`Recovering`** — the persisted state reflects unfinished recovery work,
  but the adapter has a proven normal lifecycle/admission path that advances
  it without manual intervention once currently executing hook calls finish.
- **`Blocked`** — the persisted state is valid, admission is currently denied,
  and — once the hook call that produced it has returned — no future ordinary
  lifecycle/admission event can clear it on its own. This is the status that
  detects the original Claude incident shape.
- **`Invalid`** — the state file cannot be safely interpreted (malformed JSON,
  unsupported version, read error) or is a structurally impossible persisted
  combination proven unreachable by the adapter's own state machine. Never
  used merely because admission is currently blocked.

Each classification (besides the trivial `Healthy`/read-error cases) is proven
by driving the adapter's real recovery/dispatch code, not inferred from enum
or variant names. Classifier functions are pure and read-only: doctor never
writes to a state file, matching the existing `ServiceLifecycle::diagnose` vs
`fix` split.

## Per-adapter classifiers

| Adapter | Classifier | Detailed mapping and reasoning |
| --- | --- | --- |
| Claude Code | `claude_mutation_scope::health::classify_health` | [claude-mutation-scope-integration.md](../cli/claude-mutation-scope-integration.md) ("Mutation-scope health") |
| Codex | `codex_mutation_scope::health::classify_health` | [codex-mutation-scope-health.md](../cli/codex-mutation-scope-health.md) |
| OpenCode | `opencode_mutation_scope::health::classify_health` | [opencode-mutation-scope-health.md](../cli/opencode-mutation-scope-health.md) |
| Pi | `pi_mutation_scope::health::classify_health` | [pi-mutation-scope-health.md](../cli/pi-mutation-scope-health.md) |

Each adapter's `RecoveryState`/attempt-phase machine differs; only that
adapter's own doc is authoritative for which combinations are reachable and
why they land on a given status. This file owns only the shared vocabulary
and how doctor consumes it.

## How doctor consumes it

`inspect_mutation_scope_health` (`cli/src/services/doctor/inspect.rs`) calls
the matching classifier once per integration target `sce doctor` already
resolves as configured or detected (the same `integrations.target`/detected-
directory resolution the structural integration checks use) — never for a
target that is not resolved. Doctor never inspects or interprets a
`RecoveryState` enum or an `attempts` list itself; it only consumes the
four-way status each adapter module already computed.

Doctor is a read-only snapshot: a currently executing hook process can, in a
narrow window, hold a state shape mid-transition (for example Claude's
`recovery_pending = true` with non-empty `attempts` between
`mark_recovery_pending()` and `remove_attempt()` inside one still-running
hook call). Health classifies the durable state observed at inspection time
against whether normal future adapter lifecycle events can recover it,
assuming the operation that produced it has stopped progressing; it is not a
distributed-process liveness oracle.

## Doctor problem and readiness mapping

Each non-healthy status produces one `DoctorProblem` in a new
`mutation_scope_health` category, consistent with AC6 of the
`doctor-mutation-scope-health` plan:

| Status | `ProblemKind` | Severity | Fixability | `next_action` | Readiness contribution |
| --- | --- | --- | --- | --- | --- |
| `Healthy` | none | — | — | — | none |
| `Recovering` | `MutationScopeHealthRecovering` | Warning | `no_action_required` | `no_action_required` | overall readiness may remain `ready` |
| `Blocked` | `MutationScopeHealthBlocked` | Error | `manual_only` | `manual_steps` | overall readiness becomes `not_ready` |
| `Invalid` | `MutationScopeHealthInvalid` | Error | `manual_only` | `manual_steps` | overall readiness becomes `not_ready` |

`sce doctor --fix` never deletes a state file, clears `recovery_pending`/a
`RecoveryState` generically, resets attempts, or fabricates an abandon
outside an adapter's own real protocol operations: doctor cannot safely
discard persisted mutation-scope lifecycle/recovery evidence whose meaning
and recovery obligations belong to the adapter state machine, and it never
calls `remove_attempt()` or rewrites a state file's JSON itself. For example,
Claude's unresolved-abandonment case is documented in D12/D19 in
[claude-mutation-scope-integration.md](../cli/claude-mutation-scope-integration.md);
other adapters can be `Blocked` by different durable evidence, such as
OpenCode's stale outstanding `PendingStart` with no unresolved abandonment
at all.

`Invalid` is always `manual_only`: a state file doctor cannot safely
interpret is never auto-repaired. `Blocked`, however, has a second, separate
fact beyond its health status: **repairability**. Health
(`healthy`/`recovering`/`blocked`/`invalid`, above) and repairability
(`AutoFixable`/`ManualOnly`, `Repairability` in
`cli/src/services/hooks/mutation_scope_health.rs`, kept as a distinct type
from `MutationScopeHealthStatus`) are modeled as separate facts on purpose:
a `Blocked` problem record's repairability can change as the adapter's own
positive evidence changes (for example, a dead owner becoming provably dead
only after its process actually exits), while its health stays `Blocked`
until either an ordinary lifecycle event or a repair actually clears it.

For a `Blocked` row, each adapter owns its own `assess_repairability(git_dir)
-> Repairability` and, when `AutoFixable`, `repair_blocked(git_dir,
repository_root, logger, seam) -> Result<RepairOutcome>` (Claude and OpenCode
only — Codex and Pi never classify `Blocked` today, so neither defines these
functions). `AutoFixable` requires positive, freshly-reprovable evidence, not
a timestamp, file age, or generic "clear the state" fallback:

- **Claude** (`claude_mutation_scope::health`) is `AutoFixable` only when
  every currently persisted attempt is already `PendingAbandon` (an
  established, durably-recorded abandon intent for all of them); any
  `PendingStart`/`Active` attempt with no established abandon intent forces
  the whole adapter `ManualOnly`. `repair_blocked` re-reads and re-proves
  that same condition inside one lock-protected, read-only state
  transaction, then retries each attempt's already-established seam
  `abandon` call independently outside the lock, removing only the ones that
  succeed and clearing `recovery_pending` only once none remain.
- **OpenCode** (`opencode_mutation_scope::health`) is `AutoFixable` only when
  every currently `PendingStart` attempt has a recorded owner (PID +
  `/proc` start-time identity, stamped at allocation) the shared
  `mutation_scope_owner::is_definitely_dead` proves dead; a legacy attempt
  with no recorded owner, a live owner, or an unprovable owner is
  `ManualOnly`. `repair_blocked` acquires the adapter's `AdapterBoundaryLock`,
  re-proves the same all-or-nothing dead-owner condition inside one
  lock-protected state transaction, transitions the qualifying attempts to
  `PendingAbandon`, then drives the existing `flush`/`abandon`/`flush` seam
  sequence with the state lock released.

`sce doctor --fix` (`execute_doctor_with_lifecycle_providers` in
`cli/src/services/doctor/mod.rs`, dispatched by
`repair_blocked_mutation_scope_targets` in `cli/src/services/doctor/inspect.rs`)
runs this repair as one further step, positioned after the existing
`ServiceLifecycle`/merge-target repairs and before the final diagnosis that
produces the fix-mode report. For each row the *initial* diagnosis found
`Blocked`, it calls that adapter's `assess_repairability` fresh; when
`AutoFixable`, it calls `repair_blocked` through the real production
mutation-scope ingress seam, then immediately re-reads a fresh
`classify_health` rather than trusting `repair_blocked`'s `Ok(())` return —
only when that fresh read is `Healthy`/`Recovering` does doctor record a
`Fixed` fix result, so a `Fixed` result and a final report still
`Blocked`/`Invalid` for that target can never coincide. Doctor itself never
acquires or holds an adapter's state lock and never holds one across the
seam call; that serialization is entirely adapter-owned (OpenCode's
`AdapterBoundaryLock` around the whole repair; Claude's own per-transaction
state lock with no added boundary lock). A `ManualOnly` row is never passed
to `repair_blocked` at all and falls through unchanged to the existing
generic manual-result handling, which still renders a deterministic
manual-remediation result naming the real adapter state file path, stating
plainly that no safe generic recovery command exists for it, and never
recommending deletion of the state file — the `DoctorProblem`'s own
rendered `fixability`/`remediation` text does not yet vary by this
repairability fact (still the shared `manual_only` wording below regardless
of a target's true repairability); that rendering surface is a separate,
later concern from the repair pipeline described here.

`Recovering` is a distinct fixability, `no_action_required`: doctor performs
no repair *because none is needed*, not because remediation is merely
unimplemented. This is deliberately not `manual_only` — there is no manual
action for an operator to take, and `sce doctor --fix` must not render a
manual-repair result for it. The generic remediation text is adapter-neutral
by construction: it states that the persisted state has *a* proven ordinary
lifecycle/admission path capable of advancing recovery without manual
intervention, that not every subsequent tool call is necessarily
recovery-capable for every adapter, and that the operator should simply
rerun `sce doctor` later if the state is still recovering after further
adapter activity. It never promises that a specific next event (e.g. "the
next mutation-capable tool call") will clear it: T03's Codex mapping proves a
`Pending` state with outstanding attempts can require a same-lane successor
before an unrelated call denies without advancing recovery, and T05's Pi
mapping proves a sequence of `Pending -> duplicate existing-key Start ->
still Pending -> fresh Start -> recovery`, where an intervening duplicate
Start does not itself advance recovery. Only each adapter's own classifier
`reason` is authoritative about which specific event actually advances
recovery for that adapter and persisted shape.

## Output shape

`--format json` carries a top-level `mutation_scope_health` array, one entry
per resolved target: `target` (`claude`/`opencode`/`pi`/`codex`), `status`
(`healthy`/`recovering`/`blocked`/`invalid`), `reason` (always present), and
`detail` (present when the classifier attached machine detail, e.g. a parse
error). Human text renders one "Agent tracing" row (the operator-facing
display name for this internal mutation-scope health concept) beneath each
resolved target's existing areas, using the same `[PASS]`/`[WARN]`/`[FAIL]`
compact/expand convention as every other doctor row — see
[doctor-human-text-contract.md](doctor-human-text-contract.md).

See also [doctor operator contract](agent-trace-hook-doctor.md).
