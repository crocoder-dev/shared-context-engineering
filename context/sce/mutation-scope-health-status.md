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

`sce doctor --fix` never mutates adapter recovery state (deleting a state
file, clearing `recovery_pending`, resetting a `RecoveryState`, or
fabricating an abandon): doctor cannot safely discard persisted
mutation-scope lifecycle/recovery evidence whose meaning and recovery
obligations belong to the adapter state machine. For example, Claude's
unresolved-abandonment case is documented in D12/D19 in
[claude-mutation-scope-integration.md](../cli/claude-mutation-scope-integration.md);
other adapters can be `Blocked` by different durable evidence, such as
OpenCode's stale outstanding `PendingStart` with no unresolved abandonment
at all.

`Blocked` and `Invalid` are `manual_only`: doctor cannot repair them, and
`--fix` renders a deterministic manual-remediation result naming the real
adapter state file path, stating plainly that no safe generic recovery
command exists yet, and never recommending deletion of the state file.

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
