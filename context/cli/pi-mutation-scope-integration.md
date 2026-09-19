# Pi mutation-scope integration

The Pi mutation-scope adapter is SCE's fourth concrete harness producer. It
lives in `cli/src/services/hooks/pi_mutation_scope/` behind the hidden
`sce hooks pi-mutation-scope` command. Its only production dependency inside
the mutation stack is the in-process
`hooks::mutation_scope::run_mutation_scope_from_payload` seam; it does not call
runtime, protocol, or database modules directly.

The adapter is reachable through its own CLI command and is wired into the
canonical generated Pi extension (`config/lib/pi-plugin/sce-pi-extension.ts`)
that `sce setup --pi` installs and an ordinary `pi` invocation auto-discovers.
No launcher, wrapper, or `sce pi` entry point exists or is required:

```text
ordinary `pi`
    ↓ (normal extension auto-discovery)
generated SCE extension (`.pi/extensions/sce/index.ts`)
    ↓ tool_call (bash/edit/write)                  -- fail-closed
fail-closed tracked Start (`sce hooks pi-mutation-scope`)
    ↓ tool_result, tool_execution_end
per-attempt-ordered ToolResult / ToolExecutionEnd delivery
    ↓ (terminal transport ambiguity)
D9 abandon/rebaseline recovery via ToolExecutionAbandon
```

The TypeScript extension's own terminal-delivery tracker keys every in-flight
attempt by `(session_id, tool_call_id)` — the same identity the Rust adapter
uses — so it never races a `tool_execution_end` subprocess against its own
`tool_result` subprocess: `tool_execution_end` delivery always awaits the
exact same attempt's `tool_result` delivery outcome first. Once a `tool_result`
delivery is known to have failed, or an executed attempt's `tool_execution_end`
delivery itself fails, the extension marks that exact attempt unresolved,
denies further tracked Starts while it stays unresolved, and recovers by
sending `ToolExecutionAbandon` (retried with backoff) rather than replaying a
stale `ToolExecutionEnd` — see D9 above and the Rust `PiHookEvent::ExecutionAbandon`
route, which the adapter treats identically to any other exact-attempt abandon
regardless of whether the attempt was still `PendingStart` or already
`Executed`.

`user_bash` (`!`/`!!`) is delivered to SCE's own `user_bash` handler whenever
Pi actually dispatches it there (T03's documented, accepted limitation: a
competing extension registered ahead of SCE may consume the event first, in
which case SCE has nothing to guard and tracked-tool attribution in the same
session is unaffected). When SCE does receive it, the handler arms the T03
external-mutation supervisor (`sce hooks external-mutation-guard`), awaits its
durable `Armed` acknowledgement, and only then returns `operations` that relay
`exec`/cancellation to the supervisor's own control channel — the supervisor,
not Pi/Node, spawns and owns the real shell. `wrappedOperations.exec()`
resolves only on an explicit supervisor `{"status":"result",...}` frame
(`exit_code: null` included, when the supervisor itself reports that as its
authoritative result); losing the control channel before any result frame
rejects the exec Promise rather than fabricating a completed command.

Lifecycle evidence was frozen against pinned Pi `0.80.6`; raw captures are in
[`pi_mutation_scope/fixtures`](../../cli/src/services/hooks/pi_mutation_scope/fixtures/).

## Real interactive smoke evidence

On 2026-09-17, a freshly installed current-branch extension was exercised by
ordinary interactive Pi in a real TUI. Running `!sh -c 'printf "guarded\\n" >>
human.txt; sleep 60'` showed the durable
`<git-dir>/sce/mutation-cursor-tainted` marker while the command was running;
cancelling the command through Pi removed the marker afterward. This proves the
normal Pi → SCE `user_bash` → arm → `Armed` → supervisor execution path and its
cancellation/finalization cleanup. Production-path regressions (real Git/Agent
Trace-DB coverage, D13 guard end-to-end cases, and a pinned Pi capture-replay
smoke — real Pi 0.80.6 lifecycle captures from T01 replayed through the
extension's real registered handlers, not an actual Pi process/session) live
alongside `pi_mutation_scope/mod.rs` and `sce-pi-extension.test.ts`.

A separate, genuine real-Pi-runtime tracked-tool smoke also exists at
`config/lib/pi-plugin/real-pi-runtime-smoke/` (`run.sh`), reproducible with no
model credentials or network access: it builds `sce` from this branch, runs
the real `sce setup --pi` in a scratch Git repo, installs a throwaway
`.pi/extensions/test-provider/` extension that uses `@earendil-works/pi-ai`'s
own official scripted-response test harness (`createFauxCore`) via
`pi.registerProvider(..., { streamSimple })` to script one deterministic
`bash` tool call, and drives it through the SDK's `createAgentSession()` +
`DefaultResourceLoader` — the same `.pi/extensions/` auto-discovery ordinary
`pi` uses. The real SCE extension (installed by the real `sce setup --pi`,
not stubbed) intercepts the tool call and reaches the real
`sce hooks pi-mutation-scope` Rust adapter. `run.sh` asserts, rather than
merely prints, the scratch repo's own repository-scoped Agent Trace DB result
— via exact-cardinality `SELECT COUNT(*)` queries against the pinned
`nix run .#turso` in machine-readable `list` mode — that exactly one `pi`-actor
scope reaches `status = closed`, exactly one `close` boundary event has
`attribution_kind = ai_exclusive` (`tainted = 0`, `failure_kind = healthy`),
and exactly one `mutation_trace_scope_provenance` row has `session_id LIKE
'pi_%'` and `model_id = sce-test-provider/sce-test-model`; any failed
assertion exits non-zero with a diagnostic. This is distinct from, and not a
replacement for, the capture-replay smoke above — see T06 in
`context/plans/pi-mutation-scope-integration.md` for the full evidence and
scope discipline (bash only, per the task's own minimum-smoke guidance).

No live model-authenticated Pi session or native Windows host is available in
this sandbox, so the Windows disposition and the competing-`user_bash`-extension
limitation are proven via a `process.platform` override and simulated dispatch
order, not a live run.

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
`PendingStart`. A genuinely orphaned `PendingStart` (the owning Pi process
crashed, not a tool call still legitimately running) is instead resolved by
the stale-process reconciliation below.

## Stale-process recovery (D10)

Every `PendingStart` attempt records a `ProcessOwner { pid, instance_token }`
captured via `getppid()` at admission time: because `sce hooks
pi-mutation-scope` is invoked synchronously as a direct child of the Pi/Node
process for that exact call (`tool_call` is a blocking pre-execution gate),
the OS-reported parent pid at that moment *is* the owning Pi process, with no
wire-protocol or TypeScript-extension change needed. `is_definitely_dead`
(`pi_mutation_scope/process_owner.rs`) proves death via `kill(pid, 0)` ==
`ESRCH` on Unix, and additionally guards against PID reuse on Linux by
comparing the parent's `/proc/<pid>/stat` start-time field against the
recorded value; a live pid whose instance identity can't be established this
way (non-Linux Unix, or a missing `/proc` entry) is always conservatively
treated as alive. No TTL, elapsed time, or session sweep is used anywhere in
this path.

Every tracked Start admission is itself a reconciliation opportunity, not
merely a lookup keyed on the incoming `(session-id, tool-call-id)`. While
holding the adapter boundary lock, admission first inspects every persisted
`PendingStart`/`Executed` attempt — any session, any prior process, not only
one matching the attempt currently being admitted — and independently proves
each candidate's own recorded owner positively dead via `is_definitely_dead`.
`PendingAbandon` attempts are never included: they already carry durable
terminal recovery intent owned by the pre-existing D8
pending-recovery-resume path. Because a Pi `session_id` is a fresh UUIDv7 per
process (T01), the process that owned a stale attempt is essentially never
the same process driving the *next* `tool_call`, so a same-key replay is not
how this trigger fires in practice — a later, unrelated Pi session's Start is
what discovers and retires it.

Every scope with positive owner-death evidence collected in one pass is
retired together through the existing D8 flush/abandon/flush pipeline in a
single recovery generation (`begin_terminal_cleanup` on the whole batch →
one ambiguity flush → one `abandon` per doomed scope → one rebaseline flush),
before the triggering `tool_call` is admitted — no new recovery mechanism,
D10 reuses D8's pattern end to end. A dead `PendingStart` and a dead
`Executed` attempt are both abandoned/rebaselined identically; a dead
`Executed` attempt is never given a synthetic delayed Close, because the
current Git tree no longer represents the original `tool_execution_end`
observation time (D9). Live and uncertain-owner attempts (a live pid whose
exact process-instance identity can't be established) are left completely
untouched by this scan — this is broad *inspection*, never broad
*inference*: no TTL, no elapsed time, no session sweep, no `ActorKind::Pi`
sweep, and no same-session-predecessor rule ever substitutes for an
attempt's own positive process-death proof. If the flush/abandon/flush
sequence fails partway, recovery stays durably `Pending` and the triggering
Start is denied fail-closed; the next boundary-lock acquisition resumes and
completes it before any new Start can commit.

## Reconciling with the external-mutation guard

A live Pi scope's local attempt state reconciles with a worktree the
[external-mutation guard](mutation-trace-external-mutation-guard.md) abandons
through the same existing Close-failure→abandon fallback the adapter already
uses for any other externally tainted worktree: the adapter has no visibility
into *why* the generic runtime abandoned a scope out from under it (a guard
finishing, another harness's recovery, or otherwise), only that it did, and
the next `tool_result`/`tool_execution_end` for that attempt safely reconciles
through the pre-existing recovery path rather than erroring or resurrecting
the scope. A fresh Pi `tool_call` that races an active guard blocks on the
runtime's own worktree-lock timeout and fails closed, touching no protocol
state, then succeeds normally once retried after the guard releases.

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

Doctor health classification built on this state machine is proven in
[pi-mutation-scope-health.md](pi-mutation-scope-health.md).

See also [`mutation-scope-hook-ingress.md`](mutation-scope-hook-ingress.md),
[`mutation-scope-runtime.md`](mutation-scope-runtime.md), and
[`mutation-trace-external-mutation-guard.md`](mutation-trace-external-mutation-guard.md)
for the harness-neutral `user_bash` guard mechanism this same task added
alongside the adapter.
