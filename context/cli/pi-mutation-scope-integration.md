# Pi mutation-scope integration

The Pi mutation-scope adapter is SCE's fourth concrete harness producer. It
lives in `cli/src/services/hooks/pi_mutation_scope/` behind the hidden
`sce hooks pi-mutation-scope` command. Its only production dependency inside
the mutation stack is the in-process
`hooks::mutation_scope::run_mutation_scope_from_payload` seam; it does not call
runtime, protocol, or database modules directly.

The adapter is reachable through its own CLI command today, but no real Pi
session drives it yet — wiring the actual TypeScript extension's `tool_call`
handler (and its `user_bash` external-mutation guard call site) to this
command is a separate, later task. Lifecycle evidence was frozen against
pinned Pi `0.80.6`; raw captures are in
[`pi_mutation_scope/fixtures`](../../cli/src/services/hooks/pi_mutation_scope/fixtures/).

## Scope model and coverage

The attribution unit is one independently mutation-capable **tracked** Pi tool
execution, never a session, turn, or agent loop. Each tracked execution
receives one fresh `ScopeId`.

| Pi tool class | Tool names | Mutation-scope behavior |
| --- | --- | --- |
| `TrackedMutation` | `bash`, `edit`, `write` | one attempt, one scope, write-ahead `Start`, terminal `Close` |
| `Untracked` | `read`, `grep`, `find`, `ls`, and every custom/unknown tool name | neutral pass-through; no scope, `Start`, terminal bookkeeping, or recovery state |

Unlike OpenCode, Pi has no per-tool exclusivity gate and no separate
delegation classification: `tool_call` is the single universal pre-execution
gate for every built-in tool, including `bash` (there is no OpenCode-style
`shell.env` split), and Pi has no dedicated "spawn a subagent" tool the
adapter needs to treat specially — a custom tool that itself launches another
Pi process stays untracked unless explicitly classified, and that child
process's own tracked tools are attributed independently if it also loads the
SCE extension.

Custom and unknown tool names are untracked even when they mutate: a
`pi.registerTool`-defined tool sharing a name with a tracked built-in is still
admitted as tracked by the name-based allowlist (matching classification's
by-name tolerance), but a plugin tool under any other name is never assumed
mutation-capable from its schema or description alone. False negatives are
preferred to false-positive AI attribution.

## Attempt-phase lifecycle: no `Active` phase

Pi's event stream is `tool_execution_start` → `tool_call` → [`tool_result`] →
`tool_execution_end`. `tool_execution_start` fires unconditionally, for every
registered extension, **before** `tool_call` — including for a call `tool_call`
later blocks or throws on — so it carries zero evidentiary value for "the tool
actually began executing" and drives no state transition; the adapter parses
it only to keep the wire protocol total, and may surface it as telemetry.

`tool_call` is the real fail-closed gate: an admitted attempt is durably
recorded as `PendingStart` before the generic `start` boundary commits, and
`PendingStart` is already the adapter's correct resting state for an admitted,
not-yet-confirmed attempt — there is no further "mark active" write once
`start` succeeds, unlike the OpenCode/Codex adapters' `PendingStart` →
`Active` transition. Concretely, `AttemptPhase` has three variants, not four:

```text
PendingStart --(tool_result observed)--> Executed --(tool_execution_end, Close succeeds)--> [removed]
PendingStart --(tool_execution_end, no tool_result seen)--> abandoned (D7) --> [removed]
```

`tool_result` is present if and only if the tool's `execute()` body actually
ran (success and failed-but-executed alike), and is the sole signal that
transitions `PendingStart` → `Executed`. `tool_execution_end` fires
unconditionally too, even for a blocked/thrown call — it only means Close when
a `tool_result` for the same `toolCallId` was already observed; otherwise it
means the admitted attempt never executed, and the adapter abandons it (never
closes it) through the same flush/abandon/flush recovery pattern the other
adapters use, falling back to abandon on a Close-call failure as well.

### Why `PendingStart` never blocks a sibling admission

The OpenCode/Codex adapters treat any lingering `PendingStart` attempt,
anywhere in adapter state, as checkout-wide crash-recovery ambiguity that
blocks every new admission — sound for them because their own `PendingStart`
is a narrow window between admission and marking `Active`, both inside one
boundary-lock-held invocation, so observing it at the *start* of a fresh
invocation can only mean the previous invocation crashed mid-flight.

That invariant does not transfer to Pi: `PendingStart` is Pi's normal,
possibly long-lived resting state for the tool's *entire* execution window.
Reusing the OpenCode/Codex check would serialize every concurrent Pi tool
call behind whichever one started first, contradicting the requirement that
parallel tool executions stay distinct live scopes. Pi's admission therefore
fails closed only on a lingering `PendingAbandon` (the D7/D8 abandon
pipeline) or a non-`Clear` recovery state — never on a sibling's
`PendingStart`. Detecting a genuinely orphaned `PendingStart` (the adapter's
own process crashed mid-`start`, not a tool call still legitimately running)
is left to stale-process reconciliation, a separate later task.

## Scope identity

```text
pi-tool-v1|n=<attempt-seq>|s=<len>:<session-id>|c=<len>:<tool-call-id>
```

The live-attempt key is `(session-id, tool-call-id)`: a replay while that
attempt is live resolves to the existing attempt and `ScopeId`. A checkout-local
monotonic `next_attempt_seq` counter, persisted alongside the attempt list,
is what makes a *new* attempt after the old one's terminal cleanup receive a
fresh `attempt_seq` and therefore a distinct `ScopeId` — so a reused
`toolCallId` can never reactivate a terminal scope, the one property
OpenCode/Codex get for free because their identifiers are never reused across
a session.

## Provenance and session identity

`session_id` is prefixed `pi_<uuid>` via the same `prefixed_diff_trace_session_id`
helper the diff-trace intake already uses for Pi; `model_id` is
`<provider>/<id>` as observed directly on `ctx.model` at the exact `tool_call`
handler invocation, normalized through a new `normalize_pi_model_id` beside
the existing Codex/OpenCode normalizers, or `NULL` when unavailable. Model
absence never blocks a tracked tool.

## Durable state

`<git-dir>/sce/pi-mutation-scope-state.json`, written with the same
lock/write-temp/sync/atomic-rename discipline as the other adapters' state
files, holding `next_attempt_seq`, a recovery generation/phase, and the live
attempt list. It is bookkeeping only, never attribution evidence, and is never
held while invoking the generic mutation-scope runtime.

See also [`mutation-scope-hook-ingress.md`](mutation-scope-hook-ingress.md),
[`mutation-scope-runtime.md`](mutation-scope-runtime.md), and
[`mutation-trace-external-mutation-guard.md`](mutation-trace-external-mutation-guard.md)
for the harness-neutral `user_bash` guard mechanism this same task added
alongside the adapter.
