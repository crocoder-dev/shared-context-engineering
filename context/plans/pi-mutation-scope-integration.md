# Plan: pi-mutation-scope-integration

## Change summary

Add Pi as the fourth concrete mutation-scope producer, building on the validated Claude Code and Codex mutation-attribution adapters plus the mutation-scope-provenance work already on `main`. This plan is stacked on the still-open OpenCode integration (PR #276) rather than assuming it — see **Stack and base**.

The integration reuses the existing project-local Pi extension rather than introducing a second competing extension. The TypeScript extension remains a thin harness transport layer; a new Rust `pi_mutation_scope` adapter owns event parsing, tool classification, scope identity, durable attempt state, recovery, provenance normalization, and translation into the existing harness-neutral mutation-scope ingress.

One independently executing mutation-capable Pi tool call is one mutation scope. A Pi session, turn, agent loop, or process is not itself a scope.

The initial tracked built-in tool set is `bash`, `edit`, `write`. Known read-only built-ins (`read`, `grep`, `find`, `ls`) are untracked. Custom and unknown tools remain untracked in v1 unless exact Pi evidence establishes a sound capability contract — missed attribution is preferred over claiming AI attribution for an unknown tool.

Pi `!` / `!!` user Bash is human-initiated and never creates a Pi AI mutation scope.

The integration uses the existing generic mutation-scope runtime and existing `mutation_trace_*` storage. It introduces no new Agent Trace schema or mutation-trace SQL migration.

The plan is based on Pi `0.80.6`. T01 freezes the lifecycle evidence for that exact version. Changing the Pi version after T01 begins requires stopping the plan, updating the version policy, and rerunning the complete load-bearing probe matrix.

## Stack and base

- **Predecessor:** PR #276 `opencode-mutation-scope-integration` (head branch
  `opencode-mutation-scope-integration`, head commit `3e01f97f0154135b5b3ffe8aaa3b486bc26fff15`
  as of T02's completion; based on `mutation-scope-provenance`). **PR #276 is
  currently open and unmerged.**
- **This branch:** `pi-mutation-scope-integration` (PR #278) is stacked
  directly on `opencode-mutation-scope-integration`: PR #276's current head is
  the merge base of this branch, and #278 contains only the Pi-layer plan
  commits above it. The OpenCode mutation-scope adapter and the generalized
  per-`ActorKind` confirmation-required predicate (covering `ActorKind::Codex`
  and `ActorKind::OpenCode`) are already present in this branch's history via
  that stacked base. The exact commit count above the predecessor head grows
  as plan tasks land, so it is not recorded here; re-derive it with
  `git log --oneline origin/opencode-mutation-scope-integration..pi-mutation-scope-integration`
  when it matters.
- **Stack invariant, not a plan task:** PR #276's current head must remain an
  ancestor of this branch for as long as #278 is stacked on it. Before
  beginning a task, if PR #276's head has moved (amended or rebased), update
  this branch onto the new predecessor before continuing. If PR #276 merges,
  rebase/retarget #278 onto the branch that now contains the landed OpenCode
  work — normally `main`. This is a Git operation performed outside the task
  stack, not a numbered task; every task below assumes the invariant currently
  holds.
- **Base for the PR while the stack is unmerged:** `opencode-mutation-scope-integration`
  (#276 head), **not** `main`.
- **Final branch comparison** is against `opencode-mutation-scope-integration`,
  not `main`, for as long as #276 remains open. If #276 changes before
  execution time (rebased, amended, or merged to `main`), re-check
  `gh pr view 276` and the actual branch ancestry, rebase onto the current
  predecessor, and update this section plus every task below that assumes a
  specific pre-existing OpenCode/Codex confirmation-required shape.

## Dependency and version policy

Independently of the branch stacking above, this plan also pins:

* `@earendil-works/pi-coding-agent` `0.80.6` as pinned by `config/lib/package.json`
  (confirmed at planning time);
* upstream Pi tag `v0.80.6`, commit `2b3fda9921b5590f285165287bd442a25817f17b`.

No Pi package upgrade belongs in this PR.

If the pinned Pi runtime behaves differently from the lifecycle assumptions below, T01 is a re-planning gate. Do not weaken attribution semantics to make the implementation fit an unexpected lifecycle.

## Design

### D1 — One tool execution is one scope

A mutation scope represents one independently executing mutation-capable tool call.

```text
Pi session
  |
  +-- toolCall A: bash   -> scope A
  +-- toolCall B: write  -> scope B
  +-- toolCall C: read   -> no scope
```

Parallel tool executions must remain distinct live scopes.

A session, turn, agent loop, or Pi process must never be collapsed into a single mutation scope.

The adapter maintains a monotonic checkout-local attempt sequence so a reused Pi `toolCallId` can never reactivate a terminal SCE scope.

Canonical identity:

```text
pi-tool-v1|n=<attempt-seq>|s=<len>:<session-id>|c=<len>:<tool-call-id>
```

The exact live-attempt key is:

```text
(session-id, tool-call-id)
```

A replay while that attempt is live resolves to the existing attempt. A new execution after terminal cleanup receives a new attempt sequence and therefore a new `ScopeId`.

Boundary event IDs derive deterministically from the scope:

```text
<scope-id>|start
<scope-id>|close
```

No timestamp, random UUID, model identifier, PID, or tool argument participates in attribution identity.

### D2 — Conservative tool classification

Pi `0.80.6` has three SCE-supported built-in mutation-capable tools: `bash`, `edit`, `write`. These establish scopes.

Known read-only tools (`read`, `grep`, `find`, `ls`) create no mutation-scope state.

Custom and unknown tool names are untracked in v1. Their schemas or descriptions are not sufficient evidence of mutation capability.

This is deliberately asymmetric:

```text
unknown mutating tool -> possible false negative
unknown read-only tool -> never fabricates an AI scope
```

False negatives are preferred to false-positive AI attribution.

If Pi allows a built-in mutation-capable name to be replaced with behavior that invalidates this classification, T01 must record that and the plan must be revised before T02.

### D3 — Start is write-ahead and fail-closed

The candidate Pi Start boundary is `tool_call`.

The pinned API defines it as occurring before execution and permits the handler to block the tool.

Within the existing SCE Pi extension, handler ordering must be:

```text
bash policy
    ↓
mutation-scope Start
    ↓
existing edit/write diff pre-image capture
    ↓
tool execution
```

For Bash, an SCE bash-policy denial therefore occurs before mutation-scope admission and creates no scope.

For a tracked tool, the mutation-scope handler synchronously invokes:

```text
sce hooks pi-mutation-scope
```

and does not allow the tool to proceed unless the Rust adapter has durably established `Start`.

Transport failure, missing `sce`, timeout, malformed identity, durable-state failure, recovery failure, provenance identity conflict, or generic Start failure all return Pi's normal `{ block: true, reason: ... }` shape.

A tracked mutation must never proceed merely because mutation attribution could not be established.

### D4 — Pi becomes confirmation-required

Pi changes from `requires_boundary_confirmation(Pi) = false` to `requires_boundary_confirmation(Pi) = true`.

The reason is extension ordering.

A successful SCE `tool_call` handler does not itself prove the tool will execute. A later Pi extension can still return `block: true`.

Therefore `Start(Pi A)` followed by a later extension rejecting A must never make A eligible for positive attribution.

Until Pi A reaches its own confirmed post-execution Close, any boundary while A remains unconfirmed resolves to `IneligibleUnscoped`.

A successful Pi Close confirms its own scope exactly like the generalized Codex/OpenCode confirmation rule, already in place via the stacked base (see **Stack and base**).

This change must remain bounded to the existing confirmation-required predicate and corresponding Quint/MBT cases. It must not add Pi-specific fields to `ProtocolState`, `ScopeState`, `MutationEvent`, `Attribution`, or the Quint scope model.

### D5 — `tool_execution_start` is pre-gate telemetry; `tool_result` proves execution began

**Frozen by T01 (`cli/src/services/hooks/pi_mutation_scope/fixtures/NOTES.md`, D5).** The plan originally assumed `tool_execution_start` fires after a successful Start and proves execution began. This is backwards on pinned Pi `0.80.6`: `tool_execution_start` fires unconditionally, for every registered extension, **before** `tool_call` — including for a call `tool_call` later blocks or throws on. Confirmed both live (`captures/bash-success.jsonl`: `tool_execution_start` precedes `tool_call` by ~1.3ms for the same `toolCallId`; `captures/probeC-order-throw.jsonl`: all three extensions' `tool_execution_start` handlers fire before any `tool_call` handler runs) and in the pinned package's own documentation (`docs/extensions.md`, `tool_call` section: "Fired after `tool_execution_start`, before the tool executes").

```text
tool_execution_start   (unconditional, fires for every extension, before tool_call)
tool_call               (the actual gate — can block; see D3)
[tool execute body, if admitted]
tool_result             (only if execute() actually ran — see D6)
tool_execution_end
```

`tool_execution_start` therefore carries zero evidentiary value for "the tool actually began executing." It performs no mutation-scope state transition and must never be used as proof of execution. It may still be surfaced as informational telemetry (e.g. progress UI) but never drives attribution.

The reliable evidence that execution occurred is `tool_result`: present if and only if the tool's `execute()` body actually ran — for success and for a genuine runtime failure alike (a non-zero Bash exit still produces `tool_result`) — and absent whenever `tool_call` prevented execution (D6).

The adapter's checkout-local attempt-phase bookkeeping is:

```text
PendingStart
    ↓ generic Start (tool_call admission) succeeds — attempt stays PendingStart
    ↓ tool_result observed for this exact toolCallId
Executed
    ↓ tool_execution_end paired with an already-observed tool_result
Closed
```

`PendingStart`/`PendingAbandon` naming follows the existing `AttemptPhase` convention already used by `codex_mutation_scope`/`opencode_mutation_scope`; Pi's adapter adds `Executed` as the one phase distinguishing "admitted, execution proven" from "admitted, terminal without execution" (D7). There is no `AwaitingExecution`/`Active` phase keyed on `tool_execution_start`.

### D6 — `tool_result` is execution evidence; `tool_execution_end` is Close only once execution is proven

**Frozen by T01 (NOTES.md, D6).** `tool_execution_end` alone cannot serve as the confirming Close: it fires unconditionally for every `tool_call`, including one blocked or thrown on before execution, with `isError: true` and the block/throw reason as its content (`captures/probeA-block.jsonl`, `captures/probeA-throw.jsonl`, `captures/probeB-later-block.jsonl`).

The corrected rule:

* `tool_result` is the positive evidence that execution occurred for a given `toolCallId` — success and failed-but-executed (`isError: true`, e.g. Bash exit 7 with a partial write already persisted) alike, exactly matching the original D6 intent for the success/failure split.
* `tool_execution_end` is a confirming Close **only when a `tool_result` for the same exact `toolCallId` was already observed**. A `tool_execution_end` with no preceding `tool_result` proves the opposite of a Close — see D7.
* The adapter Closes on this exact pairing (`tool_result` for `toolCallId`, then `tool_execution_end` for the same `toolCallId`) rather than on `tool_execution_end` alone or on `tool_result` alone: `tool_execution_end` remains Pi's own 1:1 terminal-lifecycle event for the `tool_call` it ends, so gating on it (once execution is proven) keeps the adapter's Close aligned with Pi's own "this tool call is finished" signal rather than closing while Pi may still be running trailing per-call bookkeeping. Exactly one Close boundary is produced per executed tool call, unchanged from the original requirement.

Both success and failed-but-executed tools map to the same Close boundary, unchanged from the original D6 intent — only the raw event this decision is keyed on changed.

### D7 — A Start followed by no execution must be abandoned, never closed

**Frozen by T01 (NOTES.md, D7) — the previously open terminal signal is now exact, not a candidate.** If SCE established Start but the tool never executed — for example because a later extension blocked it — there is no legitimate successful Close:

```text
tool_execution_end
AND no tool_result was observed for this exact toolCallId
    => admitted Start (PendingStart), but the tool never executed
    => abandon the scope, never Close it
```

This is an exact, per-`toolCallId`, synchronous signal (`captures/probeA-block.jsonl`, `captures/probeA-throw.jsonl`, `captures/probeB-later-block.jsonl` — all reach `tool_execution_end` with no `tool_result`). No broad lifecycle event or elapsed time is used or needed for this determination:

```text
Do not rely on:
  agent_settled
  session shutdown
  any timeout or TTL
```

Recovery (D8) still owns turning this into a durable abandon/rebaseline.

### D8 — Recovery follows the soundness-first flush/abandon/flush pattern

Pi is confirmation-required and may overlap another live scope, so terminal recovery must preserve the same safety invariant already established for OpenCode.

When an exact terminal condition requires abandoning A:

```text
1. durably record A as PendingAbandon
2. Flush while A and siblings are still live
3. abandon A
4. remove A only after durable abandon succeeds
5. Flush again to consume needs_rebaseline
6. clear recovery only after every step succeeds
```

The first Flush makes the ambiguous interval ineligible while A is still an unconfirmed live scope.

Surviving scopes are not swept:

```text
Start(A)
Start(B)
mutations
A becomes unrecoverably terminal
    ↓
ambiguous interval -> non-AI
abandon(A)
rebaseline
B remains live
B makes later mutation
Close(B)
    ↓
later B interval can still become AI
```

Failed recovery remains fail-closed for subsequent tracked Pi Starts.

Never replay an old Close later merely because its original delivery failed. The current Git tree would no longer represent the original observation time.

### D9 — Transport failure after tool execution cannot be repaired by pretending the observation is current

Start transport is fail-closed because the tool has not executed yet.

Terminal transport is different: once Pi reports `tool_execution_end`, the mutation may already exist.

If the terminal event reaches the Rust adapter but the generic Close fails before durable completion, Rust owns the recovery sequence from D8.

If the TypeScript extension cannot reach the Rust adapter at all after execution:

* retain an in-process unresolved-terminal marker for that exact attempt;
* deny subsequent tracked Starts while the terminal state remains unresolved;
* once the adapter becomes reachable, recover the old scope through abandonment/rebaseline, not a delayed Close;
* if the Pi process dies before that can happen, process-staleness recovery owns it.

Existing conversation-trace and diff-trace delivery remains fail-open. Mutation-scope delivery does not inherit their advisory semantics.

**Relationship to D13 (asymmetric, not a duplicate).** D9 and D13 both guard against ambiguous attribution, but they sit on opposite sides of execution and therefore require different postures:

```text
D9  — terminal transport failure:
      the tool has already executed; the mutation may already
      exist; it cannot be undone
      => unresolved-terminal / recovery semantics required

D13 — fence transport failure:
      the human command has not been allowed to begin yet;
      no mutation has occurred from this attempt
      => block user_bash; no ambiguous human mutation is
         ever introduced
```

Because D13 blocks the human command outright whenever its own fence cannot be proven durably armed, a failed fence-arming attempt never produces a mutation that needs protecting. D13 therefore does not reuse — and must not reuse — this section's in-process unresolved-terminal fallback; there is nothing for that fallback to protect. The two remain independently necessary for their own distinct events: this section protects against losing track of a mutation that has already happened; D13 prevents an ambiguous mutation from happening in the first place.

### D10 — Process death is positive staleness evidence; elapsed time is not

Each durable Pi attempt records enough process ownership information to determine whether the Pi process that owned it is definitely gone.

On a later adapter invocation, an attempt owned by a positively dead process may be recovered through abandonment/rebaseline.

A live PID alone is not enough to prove ownership after PID reuse; the implementation should use the strongest process-instance evidence available without weakening portability. If exact cross-platform process-instance identity cannot be established, a possibly-reused live PID is treated as alive and the stale scope remains conservative.

No timeout or TTL proves death. Do not abandon scopes because they are old. Do not abandon every Pi scope at startup. Do not use `ActorKind::Pi` as evidence of staleness.

### D11 — Pi session/model provenance is admission-time metadata

Every tracked Start carries:

```text
session_id = pi_<Pi session ID>
model_id   = <ctx.model.provider>/<ctx.model.id> | NULL
```

Canonical session prefixing is idempotent.

Model provenance comes from the exact `ctx.model` observed for that Start.

Missing or unusable model evidence yields `NULL`.

Never: infer the model from another session; reuse stale model state; backfill a `NULL` provenance row later; update provenance after model switching; make model absence itself block a tracked tool.

Provenance remains insert-once metadata outside protocol state.

### D12 — Multi-process Pi is normal concurrency

Pi has no need for a special "subagent scope."

If another Pi process/session works on the same checkout, its tracked tools naturally receive their own scopes:

```text
Pi process/session A -> pi-tool scope A
Pi process/session B -> pi-tool scope B
```

Their overlap becomes normal mutation-scope contention.

If an extension/custom tool launches another Pi process, the launching custom tool remains untracked unless explicitly classified; the child Pi's own tracked tools are attributed independently if that child loads the SCE extension.

No parent scope absorbs child mutations.

### D13 — `!` / `!!` user Bash may mutate only for the lifetime of a durable, worktree-wide external-mutation guard

**Corrected twice — the T03-drafted guard-holds-the-lock-itself design closed the marker-only race but has two further, load-bearing soundness holes, both found before T02 began.** An earlier statement of this section (frozen by T01, NOTES.md D13) established that `!`/`!!` user Bash can execute concurrently with an active agent tool call and required a durable worktree-wide `ExternalTaintMarker` armed **before** the command begins. A first correction (superseded by this section) recognized that a one-shot marker is not enough and proposed holding `WorktreeLock`/`ExternalTaintMarker` open for the whole command by making a **separate long-lived guard process**, spawned by the extension, the lock's sole owner — with the actual shell still executed independently, inside Pi/Node, via `createLocalBashOperations().exec()`, and with the guard's "finish" triggered by an explicit signal or by stdin EOF on the pipe to that extension process.

That design conflated "the process holding `WorktreeLock`" with "the process capable of mutating the checkout," and conflated "the control channel to Pi/Node going quiet" with "the human command has stopped." Both are false in general. Pinned Pi `0.80.6` starts its local Bash child on Unix with `detached: process.platform !== "win32"` (`config/lib/node_modules/@earendil-works/pi-coding-agent`, local-bash execution path used by `createLocalBashOperations()`), so the actual mutation-producing process is, by construction, not a normal child of whichever process happens to be waiting on it — it can outlive either:

```text
Sequence 1 — guard-owner death does not prove the human shell is dead:

human shell running (human write #1 already committed)
    ↓
SCE guard process is SIGKILLed (OOM, crash, operator kill -9)
    ↓
guard process's file descriptors close
    ↓
WorktreeLock released                      <- WRONG: nothing about the
                                               guard dying proves the
                                               detached shell is dead
    ↓
a foreign coordinate() call (Claude/Codex/OpenCode/another Pi)
acquires the now-free lock, observes the still-armed
ExternalTaintMarker, self-heals via database_failure + recover,
then evaluates its own boundary against the "clean" recovered tree
    ↓
the human shell is STILL RUNNING and mutates again (human write #2)
    ↓
write #2 can now be folded into a later confirmed AI attribution,
because nothing durable still marks the worktree as guarded

Sequence 2 — control-channel EOF is not execution completion:

human shell running, still mutating
    ↓
the Pi/Node extension process dies (unrelated crash, OOM, user
closes the terminal)
    ↓
the guard's stdin pipe (its connection to Pi/Node) sees EOF
    ↓
the T03-drafted design treats EOF as equivalent to an explicit
finish signal and runs its finish sequence: forced recover,
complete(), release the lock                <- WRONG: no evidence
                                                 the shell stopped,
                                                 only that Pi/Node
                                                 stopped talking
    ↓
the detached shell survives Pi/Node's death and writes again
    ↓
the guard has already released the lock, so this write is unguarded
```

Both sequences violate the invariant this section exists to establish. The fundamental correction:

> Protection may end only when SCE has positive evidence that the actual mutation-producing command — the real shell process, not any control or coordinator process — has stopped being capable of mutating the checkout. Death of a control process, or silence on a control channel, is never sufficient evidence of that on its own.

Pi's `user_bash` lifecycle remains user-initiated and must never establish an `ActorKind::Pi` scope. Ordinary user Bash mutations occurring while no guard and no agent scope are active remain naturally observed as unscoped by the next mutation boundary, unchanged.

**Required architectural correction: the guard's owner process must itself spawn the human shell, not merely coordinate with whatever process happens to run it.** The generic external-mutation supervisor — a new, harness-neutral runtime/ingress component (still T03's responsibility, still not Pi-specific) — owns the whole guarded interval end to end:

```text
Pi wrapped BashOperations.exec()
    ↓ (control channel: command, cwd, env, timeout, cancellation;
       streamed stdout/stderr back)
SCE external-mutation supervisor (new long-lived process, T03)
    ↓
supervisor acquires WorktreeLock, arms ExternalTaintMarker
    (ProtectedWorktree::acquire, unchanged)
    ↓
supervisor acknowledges ARMED to Pi/Node
    ↓
supervisor itself spawns the human shell as its OWN child
    (the same command Pi would otherwise have handed to
    createLocalBashOperations() — now executed by the
    supervisor, not by the Pi/Node process)
    ↓
supervisor streams the shell's stdout/stderr back to Pi/Node
over the control channel, so onData still fires as before
    ↓
supervisor continuously observes shell status, stdout/stderr, and a
kernel-owned lifetime-token pipe inherited by the shell and ordinary
descendants
    ↓
foreground shell terminates, but the supervisor does not finish yet
    ↓
lifetime-token EOF proves no ordinary descendant still owns the
inherited token
    ↓
capture final Git tree; database_failure + recover against the
already-held ProtectedWorktree; commit durably; on success,
ProtectedWorktree::complete() (clears the marker); release the
supervisor's own lock reference by exiting
    ↓
return the real exit result (exit code, output) to Pi/Node
```

Pi/Node no longer calls `createLocalBashOperations().exec()` to run the command itself; the wrapped `exec()` becomes a thin control-channel client of the supervisor. The supervisor performs the actual spawn (replicating the relevant parts of Pi's own local-shell contract — the exact command string via a shell, `cwd`, `env`, and streamed output — since the supervisor is a plumbing process in this codebase's own language, not a caller of Pi's internal TypeScript helper; T03 must document exactly which local-shell semantics it reproduces and cite pinned Pi's own local-execution behavior as the reference it is matching).

**Supervisor-crash safety — a kernel-enforced relationship between shell lifetime and lock lifetime, not merely moving the spawn.** Making the supervisor the shell's parent removes Sequence-1-style ambiguity for the *ordinary* teardown path (the supervisor's own `wait()` is real termination evidence), but it is not yet sufficient on its own: if the supervisor itself is `SIGKILL`ed while its child shell keeps running, the supervisor's file descriptors — including its `WorktreeLock` file descriptor — close, and by default that releases the advisory lock even though the actual mutation-producing shell is still alive. The fix must be kernel-enforced, not merely a longer-lived process tree.

Inspecting the actual implementation (`cli/src/services/mutation_trace/runtime/worktree_lock.rs`) confirms the smallest sound mechanism is available without inventing anything new: `WorktreeLock` wraps a `std::fs::File` and calls `.try_lock()` / `.unlock()` — Rust's standard-library file-locking API, backed on Unix by `flock(2)`. `flock(2)` locks attach to the *open file description*, not to a specific file descriptor number or a specific process: any file descriptor that is a `dup()` of, or is inherited across `fork()`/`exec()` from, the descriptor that took the lock refers to the *same* open file description and therefore holds the *same* lock — the OS releases the lock only once every such descriptor, in every process that holds one, is closed. This is exactly the "smallest sound Unix mechanism" the codebase already has ready to use, chosen instead of inventing a new fencing primitive:

```text
supervisor acquires WorktreeLock
    -> opens/holds an fd referencing the lock file's open file
       description (worktree_lock.rs's `File`)
    ↓
supervisor spawns the human shell, duplicating that same fd into
the child (dup() before exec, with FD_CLOEXEC cleared on the
duplicate so `execve` does not close it) — an ordinary,
undocumented-to-the-shell inherited file descriptor; the shell
does not need to know it exists or do anything with it
    ↓
supervisor and shell now both hold a descriptor referencing the
SAME open file description, and therefore the SAME flock

supervisor is SIGKILLed
    ↓
supervisor's own fd closes — but the shell's inherited duplicate
is still open
    ↓
the flock is STILL HELD (this is exactly the kernel-level
guarantee flock provides across dup'd/inherited descriptors)
    ↓
any foreign coordinate() call still blocks on
ProtectedWorktree::acquire_inner exactly as it did while the
supervisor was alive; it cannot reach the marker or protocol
state regardless of whether the supervisor is alive

the shell eventually exits (normally, or once its own real work
is done) and, absent a descendant that separately inherited and
kept the fd open (see D14), its copy of the descriptor closes
    ↓
the flock is finally released — this is the first and only
moment at which "the actual mutation producer is dead" becomes
kernel-provable
    ↓
the ExternalTaintMarker is STILL ARMED, because no process ever
ran the supervisor's finish sequence to clear it
    ↓
the very next coordinate() call on this worktree, from ANY
harness, whenever it next happens to run, observes a free lock
and a still-armed marker and runs the existing, completely
unmodified "inherited external taint" database_failure + recover
path before processing its own boundary — identical in shape to
every other kind of unresolved marker this codebase already
self-heals today; no new recovery mechanism, no listener, and no
"who finishes when the supervisor is dead" logic is required,
because the pre-existing inherited-taint self-heal already
handles exactly this shape of leftover marker
```

This makes the required property hold structurally rather than by any process's continued aliveness:

```text
actual mutation producer (the shell, and any descendant that still
holds a duplicate of the lock's file descriptor) alive
    =>
the kernel-enforced WorktreeLock remains held, by construction,
regardless of whether the supervisor, Pi/Node, or any other
control process is alive
```

When the supervisor *is* still alive at the moment the shell exits (the ordinary, non-crash path), it observes that termination directly via its own `wait()`/`waitpid` on its child but does not finish until a separate kernel-owned lifetime token also reaches EOF. That token is a pipe whose read end belongs only to the supervisor and whose CLOEXEC-clear writer is inherited by the shell and ordinary descendants; the supervisor closes its own writer after spawn. It continuously consumes stdout/stderr while waiting. Only after shell termination and token EOF does it run forced recovery, `complete()`, and release, so the fd-duplicated WorktreeLock remains active throughout the descendant interval.

**Post-spawn failure rule — no explicit unlock before lifetime completion.** Before
`Command::spawn()` succeeds, ordinary RAII cleanup is safe because no external
mutation producer exists. After successful spawn and before lifetime-token EOF,
any wait, poll, lifetime-read, stream-read, callback panic, or other
supervision failure consumes a dedicated abandonment state: the marker remains
armed and the supervisor closes/relinquishes only its own `WorktreeLock` fd
without `flock(LOCK_UN)`. The shell and descendants retain the inherited fd
for the same open file description, so the kernel keeps the flock authoritative
until the last inheritor closes it. Once that inherited lock becomes free, the
next `coordinate()` observes the armed marker and performs the existing
inherited-taint `database_failure + recover` path before proceeding. If
lifetime EOF has already been observed, ordinary unlock is safe even when
final recovery fails, but the marker remains armed. `GuardEvent::Armed` is
reported only after lifetime-token creation/configuration succeeds; a failure
there emits no event and spawns no shell.

The fd-duplication mechanism above is Unix-specific by construction: it depends on `flock(2)` semantics attaching to the open file description and surviving `dup()`/inheritance across `fork()`/`exec()`, which is a POSIX guarantee with no Windows equivalent for `std::fs::File`'s locking primitive. The separate lifetime token uses the same ordinary Unix fd-inheritance rule: its read end is supervisor-owned and CLOEXEC, its writer is explicitly CLOEXEC-clear, and the supervisor closes its writer after spawn. EOF is positive evidence that the kernel closed the final ordinary writer reference. T03 must keep these ownership and CLOEXEC rules explicit and must not replace token EOF with shell exit, stream EOF, process enumeration, or a timeout.

**Corrected a third time — Windows was previously and incorrectly claimed to need no lifetime protection.** An earlier version of this section stated that Pi's own `detached: process.platform !== "win32"` conditional meant "the underlying detached-survival hazard this whole section addresses does not arise on Windows in the first place," and that T03 could therefore implement "the simpler 'supervisor process tree death is sufficient' story on Windows." **This claim is false and is retracted.** `detached: false` (Node's default, and what pinned Pi passes on Windows) only controls whether Node places the child in a new process group/session on POSIX; on Windows it controls an unrelated flag (`CREATE_NEW_PROCESS_GROUP`/console allocation), and on **neither** platform does a non-detached child's lifetime become tied to its parent's lifetime by default. An orphaned child on Windows, exactly as on Unix, is simply reparented and keeps running when its parent dies — Windows has no default "kill children when parent exits" behavior any more than Unix does. "Pi does not pass `detached: true` on Windows" therefore proves nothing about what happens to the shell if the supervisor is killed on Windows; the detached-survival hazard this whole section addresses is present on **every** platform Pi's local Bash child can outlive its spawner. This is a straightforward category error (a Node.js spawn-option default was treated as an OS-enforced process-lifetime guarantee) and no revision of this plan may repeat it or an equivalent claim for any platform.

**Chosen Windows disposition — Option B: `user_bash` is unconditionally refused on Windows; tracked-tool (`bash`/`edit`/`write`) attribution remains fully enabled there.** Implementing a real Windows process-lifetime primitive for the guard (a Windows Job Object tying the spawned shell's lifetime to a kernel object the supervisor holds, analogous in spirit to the Unix fd-duplication mechanism, or an equivalent Win32 facility) is a substantial new piece of Rust/Win32 engineering — a new dependency surface, new unsafe FFI, and new platform-specific test infrastructure — that this plan's own investigation-only scope must not invent or commit to sight-unseen (per the plan's existing discipline of deriving exact mechanisms from source inspection, not invention, at T03 time). Scoping the feature honestly instead:

* On Windows, SCE's `user_bash` handler always takes the existing guard-establishment-failure branch already specified above (`{ result: { output: "<reason>", exitCode: 1, cancelled: false, truncated: false } }`) — unconditionally, not merely on a transient failure. `session.executeBash()` is therefore **never** called for `!`/`!!` on Windows; no shell — supervised or otherwise — is ever spawned by that path, so there is no detached-shell lifetime to track and D13's hazard cannot arise there at all. This is a hard refusal (the command never executes), not a silent un-hooking that would let the command run unguarded — the distinction the "do not merely disable handling of `user_bash`" requirement exists to enforce.
* This is chosen over disabling Pi's positive mutation attribution entirely on Windows because the hazard this section addresses is specific to `user_bash`'s detached-shell lifetime; D3–D12 (the `tool_call` fail-closed gate, the `tool_result`/`tool_execution_end` Close pairing, conservative recovery, provenance) involve no OS-level process-lifetime assumption and no evidence anywhere in T01 suggests they behave differently on Windows — they are plain JS/TS control flow inside Pi's own Node/Bun runtime, not calls into OS-specific process-lifetime primitives. Refusing only the one hazardous path (`user_bash`) and leaving the unaffected paths (`bash`/`edit`/`write` tool tracking) enabled is the smaller, more precisely targeted safe behavior; disabling all Pi attribution on Windows would be strictly more conservative than necessary and is not required once `user_bash` cannot execute unguarded.
* **Residual verification gap, not a T02 blocker:** T01's fixtures (`fixtures/NOTES.md`, "OS" row) were captured only on Linux (NixOS `x86_64`). The claim just above — that D5–D12 are platform-independent — is a reasonable inference from the mechanism (plain JS event ordering, no OS syscalls) but is not itself T01-verified on Windows. T06 must add a Windows smoke test exercising the tracked `bash`/`edit`/`write` lifecycle (at minimum `tool_call` → `tool_result` → `tool_execution_end` for a success case) before AC1/AC2 can be considered validated cross-platform; until then, Windows tracked-tool support rests on inference from source, not direct evidence, and T06's task record must say so explicitly rather than silently assuming parity with the Linux captures.
* A future PR may add real Windows lifetime protection (Job Object or equivalent) and enable guarded `user_bash` there; that work is out of scope for #278 and must not be implemented speculatively here.

**Begin semantics — fail-closed before execution, using Pi's actual API (no `block`/`reason`), now via the supervisor.** T01 was correct that `user_bash` fires unconditionally and can intercept, but the plan's earlier text incorrectly assumed `tool_call`'s `{ block: true, reason: ... }` shape applies to it. Inspecting the pinned `0.80.6` package directly:

* `UserBashEventResult` (`config/lib/node_modules/@earendil-works/pi-coding-agent/dist/core/extensions/types.d.ts`, ~line 771) is exactly:
  ```ts
  export interface UserBashEventResult {
      operations?: BashOperations;
      result?: BashResult;
  }
  ```
  There is no `block`/`reason` member; that shape belongs only to `ToolCallEventResult` (`tool_call`), a few lines above it in the same file.
* The actual consumption logic, `handleBashCommand()` in `dist/modes/interactive/interactive-mode.js` (~line 4930-4990): if the handler's returned `eventResult.result` is truthy, Pi uses it as a **full replacement** — it builds the UI/history entries from that fabricated `BashResult` and **never calls `session.executeBash()` at all**. Only when `result` is absent does Pi call `session.executeBash(command, onChunk, { excludeFromContext, operations: eventResult?.operations })`, which (`dist/core/agent-session.js`, `executeBash()`) forwards `options.operations ?? createLocalBashOperations({ shellPath })` into `executeBashWithOperations(...)` together with Pi's own `AbortSignal` (`this._bashAbortController.signal`) and streaming callback.
* `BashOperations` (`dist/core/tools/bash.d.ts`) is `exec(command, cwd, { onData, signal?, timeout?, env? }) => Promise<{ exitCode: number | null }>`.

This gives the real mechanism for both halves of D13's fail-closed requirement, revised for the supervisor-owns-the-shell architecture:

* **Guard cannot be established (failure or ambiguous acknowledgement):** the handler returns `{ result: { output: "<reason SCE could not establish the worktree external-mutation guard>", exitCode: 1, cancelled: false, truncated: false } }`. Because a truthy `result` is a full replacement, `session.executeBash()` — and therefore any real shell, supervisor-spawned or otherwise — is **never invoked**.
* **Guard established:** the handler returns `{ operations: wrappedOperations }`. `wrappedOperations.exec(command, cwd, options)` no longer calls `createLocalBashOperations()` itself; it sends `command`/`cwd`/`env` to the already-armed supervisor over the control channel, relays each streamed output chunk to the caller's `onData` as it arrives, forwards `signal`-driven cancellation and `timeout` expiry to the supervisor as explicit cancellation requests (the supervisor is what actually signals the real shell's process group, since only the supervisor holds its pid), and resolves only once the supervisor delivers the shell's real, supervisor-observed exit result — never merely once the control channel goes quiet.

**Crash and death semantics — positive death evidence only, and an explicit, chosen policy for every process that can die mid-command; never a timeout used to infer abandonment.**

* *Supervisor dies mid-command while the shell keeps running* (`SIGKILL`, OOM, crash): per the fd-duplication mechanism above, the `WorktreeLock` is **not** released — the shell's inherited duplicate descriptor keeps the flock held. No foreign `coordinate()` call can proceed past `ProtectedWorktree::acquire_inner` during this window, regardless of the supervisor's death. Only once the shell (and every descendant still holding a duplicate of the fd) exits does the lock free; the very next `coordinate()` call anywhere then self-heals the still-armed `ExternalTaintMarker` via the existing, unmodified inherited-taint path. No PID, timestamp, or TTL is involved at any point; the lock's eventual release, gated on the shell's own fd closing, is the positive death evidence.
* *Pi/Node dies mid-command while the shell is still running* (control-channel process, not the supervisor): **chosen policy — Option A, the supervisor continues execution.** The control channel to Pi/Node closes; the supervisor does not treat that closure as a finish signal, does not kill the shell, and does not run its finish sequence. It keeps holding the lock and marker and keeps waiting on the shell's own termination exactly as if Pi/Node were still alive. When the shell terminates, the supervisor runs its normal finish sequence (forced recover, `complete()`, release) regardless of whether anything is still listening on the (now-dead) control channel — worktree correctness never depends on a listener being present. This is chosen over Option B (the supervisor kills the shell's process group when Pi/Node dies) because killing a running human-typed command merely because the AI harness's own control process happened to crash is a surprising, destructive side effect on work the human explicitly initiated, and D13's invariant does not require it: soundness comes from the guard's lifetime tracking the shell, not from tracking Pi/Node. A future revision could still add Option B as an opt-in policy, but it is out of scope here and must not be silently assumed.
* *Ambiguous begin acknowledgement* (the extension spawns the supervisor and sends its begin request, but the process exits, the pipe breaks, or no acknowledgement arrives before the extension's own bounded wait elapses): the extension must treat this exactly as guard-establishment failure — return the `result` full-replacement, never call `session.executeBash()` — and must additionally terminate the spawned supervisor process before returning, so a supervisor that *did* acquire the lock before the acknowledgement was lost does not linger holding it (and, since no shell was spawned yet in this window, killing the supervisor here releases the lock immediately — the fd-duplication mechanism only matters once a shell exists to inherit the descriptor). This bounded wait is an availability bound on a single establishment attempt, never a staleness determination.
* *Guard finalization fails* (the forced recovery/rebaseline commit does not durably succeed): `complete()` is never called and the marker stays armed; the supervisor exits non-zero. The extension must not claim clean attribution state for the just-finished command — it surfaces the failure, but the human command's already-completed result is still returned to the user (SCE cannot un-execute a finished command); the durable, still-armed marker is what future `coordinate()` calls use to stay conservative until self-healed.
* *Abort/timeout/non-zero exit* (ordinary Pi semantics to preserve): the supervisor accepts a cancellation request from Pi/Node (from `AbortSignal` or Pi's own timeout) and signals the real shell's process group accordingly, exactly as `createLocalBashOperations()` would have — but cancelling the shell does not by itself end the guarded interval; the guard still waits for the shell's own positive termination (which a `SIGKILL` typically produces quickly, but is not assumed instantaneous) before running its finish sequence. A non-zero exit is relayed to Pi/Node like any other exit code and does not change guard behavior.

**Exclusivity — multiple `user_bash` invocations reuse the same lock, no new admission logic.** Pi's TUI already serializes `!`/`!!` execution to one in flight at a time (T01). If a mode ever allows a second concurrent `user_bash` on the same worktree, its supervisor's own `ProtectedWorktree::acquire` simply contends on the same `WorktreeLock` as the first supervisor and, on timeout, is treated as an ordinary guard-establishment failure by the mechanism above — command refused, no execution. This requires no reference counting, no tokens, and no admission code beyond what `WorktreeLock` already does.

**No new protocol or Quint semantics.** `database_failure`, `recover`, `Flush`, `CoordinateError::LockAcquisition`, and `CoordinateError::MarkerClearAfterCommit` are all pre-existing, actor-agnostic, and already exercised by Rust tests and MBT. This section changes *which process* spawns the human shell, *how* the OS-level lock's lifetime is made to durably imply the shell's lifetime (fd duplication, a runtime/OS-level mechanism, not a protocol one), and *what triggers* the existing recovery the runtime already performs — not what the protocol model represents. No `protocol.rs` or `spec/mutation_cursor.qnt` edit is required (see AC18). This task does not need to solve the separately deferred Bash-policy behavior for `!`/`!!` beyond this guard.

**Extension-dispatch interception is an accepted, permanent limitation — not a hole this plan closes.** T01's own evidence (`handleBashCommand()`) establishes that Pi consumes exactly one `user_bash` handler's result — the first one that returns a truthy `operations` or `result` — not a chain where every registered extension's handler runs in sequence the way `tool_execution_start`/`tool_call` do for tool calls (D5). If any other registered Pi extension's `user_bash` handler runs ahead of SCE's and itself returns a result, Pi never invokes SCE's handler at all: no supervisor is spawned, no `WorktreeLock` is acquired, no `ExternalTaintMarker` is armed, and the human command executes with no guard from SCE.

**Corrected — SCE does not attempt to close this by contesting Pi's dispatch order.** Earlier drafts of this section proposed closing this gap with an SCE-hosted Pi launcher that would own `ResourceLoader` construction (`extensionFactories`/`extensionsOverride`), normalize the resolved extension array to guarantee SCE is first and present exactly once (`sceEnforceExtensionOrder`), and ship a `sce pi` entry point or PATH-shadowing wrapper to get sessions into that launcher. That direction is retracted as a deliberate product decision, not merely deprioritized:

* SCE does not redistribute Pi, bundle a JS/Bun runtime, or embed Pi's SDK. Pi remains an independently installed, independently launched dependency.
* There is no SCE-hosted Pi process, no `sce pi` command, and no launcher-owned `ResourceLoader`.
* SCE does not claim, prove, or test that its extension is first or unique in Pi's runtime extension array. `sceEnforceExtensionOrder`, the canonical-inline-factory-identity mechanism, and the disk-duplicate-removal normalization described in earlier drafts of this section do not exist and are not built.
* The supported product path remains exactly: `sce setup --pi` installs the generated extension; the user runs ordinary `pi`; Pi auto-discovers SCE like any other extension.

**The guarantee this task actually provides:**

```text
SCE user_bash handler receives event
    ↓
arm external-mutation supervisor
    ↓
receive durable Armed acknowledgement
    ↓
only then authorize exec
    ↓
supervisor owns actual shell execution and lifetime guard
    ↓
finish forces database_failure + recover before guard completion
```

When SCE's `user_bash` handler is the one Pi invokes, every guarantee elsewhere in this section holds: the guard is durably established before any shell runs, `Armed` alone never authorizes execution, and the supervisor's own lifetime-tracking and forced recovery govern the guarded interval end to end, regardless of which process (Pi/Node, the supervisor, or the shell) dies and when.

**The explicit limitation this task accepts, not solves:**

```text
A competing Pi extension may consume user_bash before SCE.
SCE cannot guard an event Pi never dispatches to it.
This configuration is outside the user_bash safety guarantee.
```

This limitation is specific to `user_bash` extension-dispatch interception. It does not weaken mutation-attribution semantics for ordinary tracked Pi tools (`bash`/`edit`/`write`, D3–D12), which are gated on SCE actually receiving Pi's `tool_call` event for that call, not on SCE's position in the extension array — Pi calls every registered extension's `tool_call` handler in registration order (D5), so a later extension cannot prevent SCE's own `tool_call` handler from running the way `user_bash`'s first-handler-wins dispatch can. This limitation also must never become a runtime self-disable mechanism for tracked-tool attribution: whether another extension might also see `user_bash` first has no bearing on whether `bash`/`edit`/`write` Start/Close is established when SCE's own `tool_call` handler runs.

`sce doctor`/`sce setup --pi` may optionally report, as human-readable diagnostic information only, that another extension is configured in a way that could intercept `user_bash` ahead of SCE under naive on-disk resolution. This is informational only: it is never a prerequisite for installing or using the Pi integration, and a clean report is never treated as proof that `user_bash` will actually reach SCE for a given session.

**Preserving the disk integration.** `sce setup --pi` generates and installs `.pi/extensions/sce/index.ts` exactly as it always has. A normal `pi` invocation discovers it like any other project extension, with no wrapper, launcher, or additional installation step.

### D14 — Detached descendants remain an explicit limitation

A foreground Pi Bash tool can potentially launch a child process that survives the Bash tool's own completion. If the pinned Pi runtime provides no structured lifecycle proving all descendants are dead, `tool_execution_end` cannot prove that a self-detached descendant has stopped mutating.

For the D13 `user_bash` guard specifically, foreground-shell lifetime is not external-mutation lifetime. The supervisor creates a dedicated Unix pipe before spawning: its read end remains with the supervisor, its writer is explicitly CLOEXEC-clear, the shell inherits that writer, and ordinary descendants inherit it under normal Unix fd inheritance. The supervisor closes its own writer after spawn. It does not recover, clear the marker, or release the real WorktreeLock at foreground-shell exit. It waits for shell termination **and** kernel-observable lifetime-token EOF; EOF proves that no ordinary inheritor still owns the token. The final tree is then observed and the existing forced `database_failure + recover` composition runs while the supervisor still owns the ProtectedWorktree. Only durable recovery is followed by marker clear and normal WorktreeLock drop.

This improves the ordinary `cmd &`, `nohup`, or `disown` case without parsing Bash or enumerating processes. The explicit residual limitation remains: a descendant that deliberately closes the inherited lifetime token (or execs into a program that closes non-standard inherited descriptors) can continue mutating after EOF and escape tracking. The supervisor cannot distinguish that deliberate close from genuine completion; output streams are not used as the safety oracle, and their post-token finalization is bounded. This is accepted and documented, not hidden or solved by a TTL.

**Windows scoping.** On Windows, `user_bash` is unconditionally refused (D13's corrected Windows disposition) — no shell is ever spawned via that path, so no descendant question arises for `user_bash` there at all. The general, platform-independent D14 rule above (a foreground Pi tool's own tracked Bash call can launch a surviving descendant) is unchanged and unaffected by this correction, since it concerns `tool_call`-mediated `bash`, not `user_bash`, and `tool_call`'s lifecycle is not part of D13's Windows carve-out.

### T01 evidence corrections (adopted into this design)

T01 (`cli/src/services/hooks/pi_mutation_scope/fixtures/NOTES.md`) froze the pinned Pi `0.80.6` lifecycle and found two load-bearing corrections to this design's original text, both incorporated above:

```text
1. tool_execution_start is pre-gate telemetry, not execution evidence.
   tool_result proves execution occurred.
   tool_execution_end without a preceding tool_result means the tool
   never executed and requires abandon, never Close.        (D5, D6, D7)

2. user_bash can execute concurrently with an active Pi agent tool.
   The overlapping interval requires a durable, worktree-wide
   external-mutation guard whose lifetime spans the entire human
   command — not merely a fence armed before it begins — implemented
   by holding the existing per-worktree WorktreeLock/ExternalTaintMarker
   for that whole interval, so no part of it can ever become part of
   a later confirmed scope's positive attribution, for Pi's own scopes
   or any other harness's.   (D13)
```

Neither finding is a failure of the overall Pi integration approach; both are exactly the kind of refinement T01 exists to surface. Neither weakens the soundness contract: `tool_call`'s fail-closed gate (D3), the confirmation-required design (D4), and conservative recovery (D8–D10) are unchanged. Neither requires a `protocol.rs` or `spec/mutation_cursor.qnt` edit — D5–D7 are adapter-internal (T03) event re-keying, and D13 reuses the already-existing, already-proven `ProtectedWorktree`/`WorktreeLock`/`ExternalTaintMarker`/`database_failure`/`recover` primitives unchanged, held for a longer, explicitly-terminated interval instead of a single boundary (see AC18). T01 itself remains complete and unchanged; see its completed task record below.

## Acceptance criteria

- [x] AC1: Exact lifecycle evidence exists for Pi `0.80.6`, covering `tool_call`, `tool_execution_start`, `tool_execution_end`, `tool_result`, blocking, handler failure, tool failure, interruption, session lifecycle, process death, model observation, extension ordering, and concurrency — including the frozen `tool_execution_start`-before-`tool_call` ordering and the `tool_result`-gates-execution rule (D5/D6/D7).
  - Validate: satisfied by T01's committed fixtures/report (`cli/src/services/hooks/pi_mutation_scope/fixtures/`) with exact Pi version, upstream commit, environment, and event sequences; `/validate` re-confirms this AC against the final implementation, not merely against T01's evidence.
- [x] AC2: `bash`, `edit`, and `write` each establish one independently identified Pi mutation scope before their mutation-capable execution begins.
  - Validate: adapter tests plus live/runtime fixtures.
- [x] AC3: `read`, `grep`, `find`, `ls`, `user_bash`, and representative unknown/custom tools create no Pi mutation scope.
  - Validate: zero-footprint classification and runtime tests.
- [x] AC4: failure to establish a tracked Pi Start blocks the tool before execution.
  - Validate: live probe where the adapter fails and an observable filesystem mutation never occurs.
- [x] AC5: a Pi scope cannot create positive mutation attribution until its own confirming post-execution Close, where Close is keyed on the `tool_result`-then-`tool_execution_end` pairing (D6), never on raw `tool_execution_end`.
  - Validate: Rust protocol tests plus Quint Pi confirmation-required cases.
- [x] AC6: an unconfirmed Pi scope suppresses positive attribution at Claude, Codex, OpenCode, Pi, and Flush boundaries.
  - Validate: protocol/MBT/Quint cross-harness tests.
- [x] AC7: a confirming Pi Close (the D6 `tool_result`-then-`tool_execution_end` pairing) can produce `AiExclusive(Pi)` when it is the only safe live scope and `AiContended` when overlapping confirmation-safe scopes remain.
  - Validate: Rust/Quint reachability tests.
- [x] AC8: an earlier extension or SCE bash policy rejecting a tool before SCE Start creates no scope; a later extension rejecting after SCE Start cannot create positive attribution and is eventually conservatively recovered. A later-extension rejection after a successful SCE Start produces `tool_execution_end` with no preceding `tool_result` for that `toolCallId` (D7); the adapter must abandon, never Close, on that exact signal.
  - Validate: pinned-runtime ordering fixtures plus adapter/runtime regression; fixtures assert the exact `tool_execution_end`-without-`tool_result` pairing, not a broader heuristic.
- [x] AC9: a tracked tool that executes and then reports `isError` still produces `tool_result` (proving execution occurred, per D6) and observes its final Git tree through the same `tool_result`-gated terminal boundary as success.
  - Validate: partial-mutation-then-error regression.
- [x] AC10: simultaneous or overlapping Pi calls remain separate scopes and terminal cleanup of one never implicitly retires another.
  - Validate: concurrency adapter/runtime test.
- [x] AC11: a lost or failed terminal boundary cannot later be replayed as if its observation happened at recovery time.
  - Validate: injected terminal seam failure followed by another filesystem mutation; recovery must discard/rebaseline the ambiguous interval instead of attributing it.
- [x] AC12: stale-process cleanup requires positive process-death evidence and never uses TTL, age, session identity, or ActorKind alone.
  - Validate: live-owner vs dead-owner durable-state tests.
- [x] AC13: Pi Start provenance stores canonical `pi_<sessionID>` plus the exact observed normalized model, or `NULL` when unavailable.
  - Validate: real repository Agent Trace DB and final Agent Trace regressions.
- [x] AC14: existing Pi Bash policy, conversation tracing, edit/write diff tracing, generated extension installation, and doctor behavior remain intact.
  - Validate: existing Pi/config-lib tests, setup smoke, doctor smoke, and generated-output validation.
- [x] AC15: only confirmed exclusive Pi evidence reaches `mutation_ai_patch`; blocked, ambiguous, unconfirmed, abandoned, custom/unknown, and recovery intervals do not.
  - Validate: real Git/DB production-path tests.
- [x] AC16: cross-harness Pi overlap obeys the generalized mutation protocol, at minimum Pi+Claude, Pi+Codex, Pi+OpenCode.
  - Validate: production-path tests against the OpenCode adapter already present in the stacked base (see **Stack and base**), plus Rust/Quint cross-harness tests.
- [x] AC17: no new Agent Trace schema or mutation-trace SQL migration is introduced.
  - Validate: baseline diff over schema/migration paths is empty.
- [x] AC18: the protocol/Quint semantic change is limited to adding the Pi case to the already-generalized confirmation-required predicate. This also covers D13's external-mutation guard: it reuses the existing `ProtectedWorktree`/`WorktreeLock`/`ExternalTaintMarker`/`database_failure`/`recover` primitives unchanged, held for a longer, explicitly-terminated interval whose lifetime is anchored to the actual shell process (via the supervisor spawning it directly and, on Unix, fd-duplicating the lock into it), and adds no new protocol.rs or Quint code; the new long-lived supervisor invocation the mechanism requires (T03) lives in the runtime/ingress layer (`cli/src/services/mutation_trace/runtime/`, `cli/src/services/hooks/mutation_scope.rs`), outside this baseline-diff scope entirely.
  - Validate: targeted baseline diff over `protocol.rs`, `spec/mutation_cursor.qnt`, its documentation, and MBT/refinement surface, showing only Pi-shaped additions.
- [x] AC19: on every platform where guarded Pi `user_bash` attribution is supported, every mutation performed by a `user_bash` execution occurs inside one worktree-wide external-mutation guard whose lifetime spans the entire lifetime of the actual shell process (not the supervisor's, and not the control channel's), and no AI boundary can make any part of that guarded interval positively attributable, for any live harness scope on that worktree (D13's corrected lifetime invariant). On Windows, where guarded `user_bash` attribution is explicitly unsupported for this PR, `user_bash` is unconditionally refused rather than guarded — see the Windows-refusal bullet below — and this AC's guard-lifetime claims apply only to the Unix mechanism.
  - Validate, successful guard, single write: arm the guard while a Pi scope and at least one other-harness scope (Claude, Codex, or OpenCode) are both live and mutating on the same worktree; let the human command execute and mutate; end the guard; then trigger a boundary from the *other* harness's scope (not Pi's own) and assert the forced recovery abandons every live worktree scope before that boundary is evaluated, that neither scope reaches `AiExclusive`/`AiContended` over the guarded interval, and that the interval is excluded from `mutation_ai_patch` (matching AC15).
  - Validate, the mid-command race: with the guard active and a human write already made (write #1), have a foreign harness's boundary attempt to run *while the guard is still active* and assert it does not proceed — it observes `CoordinateError::LockAcquisition` (or the adapter's own conservative retry-later handling of it) and neither reads, mutates, nor clears any protocol or taint state; let a second human write occur (write #2) before the guard ends; end the guard; assert both writes remain excluded from positive AI attribution and no live scope reached `AiExclusive`/`AiContended` for any part of the interval spanning either write.
  - Validate, supervisor dies while the shell is still running (Unix): after the guard is armed and the shell has produced at least one write, `SIGKILL` the supervisor process directly while the shell keeps running; assert `WorktreeLock` remains held (a concurrent foreign `coordinate()` call still blocks/fails closed with `CoordinateError::LockAcquisition`, exactly as if the supervisor were alive) for as long as the shell (or a descendant holding the duplicated fd) is alive; let the shell make a second write and then exit; assert the lock frees only once the shell exits, that `ExternalTaintMarker` is still armed at that point, and that the very next `coordinate()` call on that worktree — from any harness — self-heals via the existing unmodified inherited-taint path before processing its own boundary; assert both writes remain excluded from positive AI attribution.
  - Validate, Pi/Node dies while the shell is still running: kill the Pi/Node control-channel process while the guard is active and the shell is still running; assert the supervisor does not treat this as a finish signal, does not kill the shell, and keeps holding the lock/marker; let the shell make a further write and then exit normally; assert the supervisor still runs its normal finish sequence (forced recover, `complete()`, release) with nothing listening on the dead control channel, and that every write remains excluded from positive AI attribution.
  - Validate, Windows refusal: on Windows, invoke `user_bash`; assert the handler unconditionally returns the `result` full-replacement (never `operations`), that `session.executeBash()` is never called, that no shell — supervised or otherwise — is ever spawned, and that the command's own exit/output is never delivered because it never ran; assert a concurrently live Pi `bash`/`edit`/`write` tracked-tool scope on the same Windows worktree is unaffected and can still separately reach `AiExclusive`/`AiContended` normally, proving the refusal is scoped to `user_bash` alone and does not disable tracked-tool attribution.
- [x] AC20: after a `user_bash`-guarded interval ends and its forced recovery/rebaseline durably succeeds, a fresh, uninterfered-with Pi scope — and a fresh scope from any other harness whose boundary was deferred by the guard — can still reach `AiExclusive` (D13).
  - Validate: guard, recover (abandoning the live scope(s) and rebaselining), then run a clean tracked Pi tool to completion with no further interference; assert it reaches `AiExclusive` and lands in `mutation_ai_patch`. Also validate that a foreign-harness boundary that was deferred (AC19's mid-command-race case) succeeds normally once retried after the guard ends, and that a boundary deferred by the supervisor-death self-heal case above also succeeds normally once retried.
- [x] AC21: if the worktree external-mutation guard cannot be durably established, the underlying Bash execution is never invoked, and the supervisor — not Pi/Node — is confirmed to be the process that actually spawns the real shell once the guard is established.
  - Validate: inject a guard-establishment failure (lock-acquisition timeout, marker-persistence failure, or spawn failure) while a Pi scope (and, in at least one variant, an other-harness scope) is live on the worktree; assert the `user_bash` handler returns Pi's `result` full-replacement (never `operations`), that no real shell is ever spawned by either Pi/Node or a supervisor, that an observable shell mutation never occurs, and that the live scope(s) are unaffected because no human mutation was ever introduced.
  - Validate, ambiguous begin acknowledgement: have the supervisor durably acquire the lock and arm the marker while the caller's acknowledgement is lost or delayed past its bound, before any shell has been spawned; assert the command is still blocked (never executed on an uncertain result) and that the caller terminates the orphaned supervisor process so the lock is promptly released (no shell exists yet to hold a duplicated fd in this window); assert a later boundary on that worktree conservatively recovers/rebaselines it anyway — an accepted false negative (unnecessary abandonment), never a safety violation.
  - Validate, guard finalization failure: force the forced-recovery commit at guard-finish time to fail; assert `complete()` is never called, the marker remains armed, and the supervisor reports failure without claiming clean attribution; assert the next boundary on that worktree self-heals via the existing inherited-taint recovery path.
  - Validate, the real shell's parent is the supervisor: assert the spawned shell process's parent pid is the supervisor's pid (not Pi/Node's), confirming Pi/Node's wrapped `exec()` never itself calls `createLocalBashOperations()`/spawns a shell once a guard exists.
- [x] AC22: `sce setup --pi` installs SCE's canonical generated Pi extension, and a normal `pi` invocation loads it via ordinary auto-discovery and uses it for tracked-tool (`bash`/`edit`/`write`) mutation attribution and for guarded `user_bash` handling whenever Pi actually delivers those events to SCE's handlers. SCE does not claim, prove, or test authority over third-party Pi extension ordering. A third-party extension that consumes `user_bash` before SCE is a documented, accepted boundary of the external-mutation-guard guarantee (D13), not a defect this AC requires closing.
  - Validate: in a scratch repository, run `sce setup --pi`, launch ordinary `pi` (no wrapper, no launcher, no alternate entry point), execute a tracked `bash`/`edit`/`write` tool call, and confirm the corresponding Start/Close reaches `sce hooks pi-mutation-scope` and produces `AiExclusive` attribution when it is the only live scope; separately, invoke `!`/`!!` `user_bash` and confirm SCE's guard establishes and the command executes only through the supervisor. Then register a second extension ahead of SCE in `.pi/settings.json` whose own `user_bash` handler returns a result; assert Pi never dispatches that `user_bash` event to SCE (no guard-establishment attempt occurs), and record this as the documented, accepted limitation rather than a failure — confirm tracked `bash`/`edit`/`write` attribution for the same session is unaffected, since it does not depend on SCE's position in the extension array.
- [x] AC23: control-process death (Pi/Node) never terminates or truncates a running human `user_bash` command, and never causes the guard to end before the actual shell terminates (D13's chosen Option A policy).
  - Validate: kill the Pi/Node process at several points during a running `user_bash` command (before any output, mid-stream, after the shell has already exited but before the supervisor's finish sequence completes) and assert in every case that the shell is never signaled by the supervisor as a result of the control-channel closing, that the guard's finish sequence runs only once the shell itself terminates, and that the shell's own exit code/output — while now undeliverable to the dead Pi/Node process — does not affect worktree correctness.

### Full validation

Run from the repository's prescribed Nix environment.

```text
nix run .#quint -- typecheck spec/mutation_cursor.qnt
nix run .#quint -- test spec/mutation_cursor.qnt
nix build .#checks.x86_64-linux.mutation-trace-quint-connect

nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml pi_mutation_scope
nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace
nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml hooks::

nix run nixpkgs#bun -- test config/lib

nix run .#pkl-check-generated
nix flake check
git diff --check
```

Also verify that the baseline diff introduces no mutation-trace schema migration:

```text
git diff <base> -- \
  config/schema/agent-trace.schema.json \
  cli/migrations/agent-trace-repository/
```

Expected: empty. `<base>` is `opencode-mutation-scope-integration` (#276 head) for as long as that PR remains open — see **Stack and base**.

### Context sync

Expected durable-context impact:

```text
context/cli/mutation-scope-hook-ingress.md
context/cli/mutation-scope-runtime.md
context/cli/mutation-scope-provenance.md
context/cli/mutation-trace-protocol.md
context/cli/mutation-trace-external-taint.md
context/cli/pi-mutation-scope-integration.md
context/sce/agent-trace-hooks-command-routing.md
context/architecture.md
context/context-map.md
context/glossary.md
context/overview.md
spec/mutation_cursor.md
```

Pi generation/setup ownership documentation should be updated only where mutation-scope behavior materially changes the existing extension contract.

Each completed task must finish context synchronization as `synced` before the next task begins.

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** pinned Pi lifecycle evidence (`cli/src/services/hooks/pi_mutation_scope/fixtures/`);
  the Pi mutation-scope adapter (`cli/src/services/hooks/pi_mutation_scope/`) and
  its hidden `sce hooks pi-mutation-scope` route; adding the Pi case to the
  existing confirmation-required protocol predicate and its Quint/MBT
  scenarios; Pi scope identity and durable adapter state; conservative
  recovery/stale-process handling; Pi provenance; existing Pi extension wiring
  (`config/lib/pi-plugin/sce-pi-extension.ts`); generated extension parity;
  setup/doctor preservation; production Git/DB/Agent Trace regressions;
  cross-harness attribution tests against the Claude, Codex, and OpenCode
  adapters already present in the stacked base.
- **Out of scope:** redesigning the generic mutation runtime; a new mutation
  protocol; Agent Trace schema changes; mutation-trace SQL migrations;
  arbitrary custom-tool capability inference; comprehensive attribution for
  third-party custom Pi tools; implementing a native Pi subagent framework;
  changing conversation-trace or diff-trace semantics; redesigning Pi Bash
  policy; policy support for `!` / `!!` unless required for attribution
  soundness; parsing Bash to detect detached descendants; upgrading Pi;
  refactoring Claude/Codex/OpenCode adapters merely to deduplicate Pi code;
  **redoing the OpenCode confirmation-required generalization** — that is
  PR #276's work, which this plan's stacked base already supplies rather than
  reimplements; **an SCE-hosted Pi launcher, a `sce pi` command, Pi SDK
  embedding, or any mechanism claiming SCE is first or unique in Pi's own
  runtime extension array** (retracted — see D13; SCE does not redistribute
  Pi, and the supported product path is `sce setup --pi` followed by an
  ordinary `pi` invocation); npm/Nix/Flatpak release-packaging changes to
  bundle Pi, a JS/Bun runtime, or Pi's `node_modules`.
- **Constraints:** PR #276's commits must remain an ancestor of this branch
  for as long as #278 is stacked on it (see the stack invariant in **Stack
  and base**); no Pi package upgrade; attribution safety outranks preserving attribution coverage; do not
  resolve an uncertain lifecycle by broadening positive AI attribution; reuse
  `ActorKind::Pi` / `"actor_kind":"pi"`, already accepted by the generic
  ingress; production adapter code reaches mutation semantics only through the
  existing `hooks::mutation_scope` ingress seam, never a second `coordinate()`
  path; the adapter-state lock is never held across a `hooks::mutation_scope`
  invocation.
- **Non-goal:** treating `AiExclusive(Pi)` as proof no human edited the
  worktree; inferring staleness from `ActorKind::Pi`, TTL, or age; replaying an
  old Close at recovery time; a long-lived Pi "session" or "agent" scope; a
  Bash-text detached-process detector; proving or testing that SCE's
  extension is unavoidable in Pi's dispatch order — a competing Pi extension
  that consumes `user_bash` before SCE is an accepted, documented limitation
  of the external-mutation-guard guarantee (D13), not a defect future work is
  expected to close.

## Assumptions

- Task numbering follows T01..T06 as given in the original change request, one
  task per design-and-acceptance slice already scoped above.
- File and command naming (`cli/src/services/hooks/pi_mutation_scope/`,
  `sce hooks pi-mutation-scope`) follows the existing `claude_mutation_scope` /
  `codex_mutation_scope` and `claude-mutation-scope` / `codex-mutation-scope`
  precedent exactly, per repository convention.

## Task stack

- [x] T01: `Freeze Pi mutation lifecycle evidence` (status:done)
  - Task ID: T01
  - Scope: In — probing the exact SCE-pinned Pi `0.80.6` runtime and committing
    reproducible evidence under `cli/src/services/hooks/pi_mutation_scope/fixtures/`,
    recording Pi package version, upstream tag/commit, OS/platform, runtime
    mode, test configuration, and extension ordering. Out — writing any
    adapter, protocol, or extension code.
  - Dependencies: none
  - Done when: every load-bearing assumption in D1–D14 has a recorded
    disposition — `PROVEN`, `PROVEN-BY-PINNED-SOURCE`, `DOCUMENTED — NON-LOAD-BEARING`,
    or `UNSUPPORTED` — covering at minimum: bash success/non-zero
    failure/timeout/abort/partial-mutation-then-failure; write success/failure;
    edit success/failure; `tool_call` ordering including handler block/throw
    and SCE Start failure; earlier-extension-blocks-before-SCE and
    later-extension-blocks-after-SCE-Start; `tool_execution_start` ordering;
    `tool_execution_end` success/`isError`; `tool_result` success/`isError`;
    whether blocked calls receive execution/result events; multiple/overlapping
    tool calls and two Pi processes on one checkout; session
    startup/resume/fork/reload/switch/shutdown, `agent_end`, `agent_settled`,
    hard process termination; model available/switch/missing at `tool_call`;
    `user_bash` (`!`/`!!`) and whether it can overlap an active agent tool;
    custom read-only/mutating tools and built-in-name replacement; a foreground
    Bash tool spawning a detached descendant. Probe A (Start transport failure
    proves fail-closed block with no execution and no filesystem side effect),
    Probe B (later-extension rejection after a successful SCE Start proves no
    positive attribution is possible while the scope is unconfirmed), and
    Probe C (exact `tool_call`/`tool_execution_start`/`tool_execution_end`/`tool_result`
    ordering for both success and mutate-then-fail) are explicitly load-bearing
    and must each have recorded evidence. The plan may not proceed to T02 if
    `tool_call` cannot reliably block before mutation execution, no sound
    confirming post-execution boundary exists, later-extension rejection
    invalidates the confirmation-required design, Pi user Bash can overlap AI
    execution in a way the current protocol cannot soundly distinguish, or
    process/recovery semantics cannot conservatively preserve false-positive
    safety — any such finding requires revising this plan rather than
    weakening attribution.
  - Verify: replay/inspect committed captures and compare every load-bearing
    claim with upstream Pi `v0.80.6` source.
  - Completed: 2026-09-11
  - Files changed: `cli/src/services/hooks/pi_mutation_scope/fixtures/NOTES.md`;
    `cli/src/services/hooks/pi_mutation_scope/fixtures/captures/{bash-success,bash-nonzero,bash-detached,write-success,edit-success,readonly-footprint,customtool,parallel,probeA-block,probeA-throw,probeB-later-block,probeC-order-throw,sigint,sessioninfo}.jsonl`;
    `cli/src/services/hooks/pi_mutation_scope/fixtures/probe-plugins/{capture,customtool,order-first,order-last,order-last-fault,sessioninfo}.ts`
    (21 new files; no other paths touched).
  - Result: every load-bearing D1–D14 assumption plus Probes A/B/C now has a
    recorded disposition against pinned Pi `0.80.6` (installed at
    `config/lib/node_modules/@earendil-works/pi-coding-agent`, no upstream Git
    clone available in this sandbox, so citations are against the pinned
    package's own compiled `dist/` and shipped `docs/`, per NOTES.md's
    "Pinned versions" section). Probing used an isolated `HOME`-redirected
    scratch environment with only `auth.json`/`models-store.json` copied in;
    the operator's real `~/.pi/agent/sessions/` count (226) was confirmed
    unchanged before and after. Twelve of fourteen D-items and all three
    probes are `PROVEN` or `PROVEN-BY-PINNED-SOURCE`; two are `DOCUMENTED —
    NON-LOAD-BEARING` (D4, folded into D3 for Pi's simpler single-gate shape).
    **Two load-bearing corrections to the plan's own text were found and must
    be adopted before T02 implements anything:**
    (1) **D5/D6 correction:** `tool_execution_start` fires unconditionally
    *before* `tool_call` for every registered extension (confirmed directly
    against `docs/extensions.md`'s documented lifecycle order and live in
    `bash-success.jsonl`/`probeC-order-throw.jsonl`), so it carries no
    evidentiary value for "execution began" and cannot back the plan's
    `AwaitingExecution` state as literally written. The sound substitute,
    proven in every capture, is `tool_result`: present if and only if the
    tool's `execute()` body actually ran, absent whenever `tool_call` blocked
    or threw. `tool_execution_end` fires unconditionally (even on a blocked
    call) and is only a valid Close when a `tool_result` for the same
    `toolCallId` was already observed; a `tool_execution_end` with no
    preceding `tool_result` is D7's abandon signal instead. This is a
    mechanical re-keying, not a soundness weakening — D3's fail-closed gate
    and D6's success/isError Close treatment are otherwise intact once keyed
    on `tool_result`.
    (2) **D13 triggers the plan's own stated stop condition:** `!`/`!!`
    `user_bash` is TUI-only and unreachable from one-shot probing, but
    `interactive-mode.js`'s `handleBashCommand()` shows `session.executeBash(...)`
    is called unconditionally regardless of `session.isStreaming` — the
    streaming flag only affects where output is displayed, not whether the
    command runs. **User Bash can execute concurrently with an active Pi
    agent tool call.** Per the plan's own D13 text ("If it can, the plan must
    stop and add a sound explicit unscoped/taint boundary before T02"), this
    is a required new T02+ design item: ensuring a `user_bash` mutation
    overlapping a live, unconfirmed Pi scope can never be folded into that
    scope's attribution once it confirms. This is additive, not a
    contradiction of D1–D12, but it is not yet written into any T02–T06 task
    body's "Done when" bullets.
    Full per-assumption evidence, citations, and the disposition/capture
    tables are in `fixtures/NOTES.md`; nothing in either finding invalidates
    `tool_call`'s reliability as a fail-closed gate, the existence of a sound
    confirming Close, the confirmation-required design itself, or conservative
    recovery — so this is not a whole-plan re-planning gate, but neither
    finding may be silently absorbed into T02 without updating T02's (and
    likely T04's) task body to name the `tool_result`-keyed state machine and
    the D13 taint/fence obligation explicitly. **Before approving T02, the
    plan's D5/D6 text and T02/T04's task bodies should be revised to reflect
    these two corrections; T02 as currently worded still describes the
    unrevised `tool_execution_start`-keyed design and omits the D13 fence
    requirement.**
  - Verify outcome: fixtures replayed and cross-checked against the pinned
    package's `docs/extensions.md` lifecycle diagram and `tool_call`/
    `tool_execution_start` section text directly (not merely against the
    subagent's summary) — confirmed the documented order is
    `tool_execution_start` before `tool_call`, matching every capture.
    Spot-checked `probeA-block.jsonl`/`probeB-later-block.jsonl` event
    sequences and confirmed no credentials or operator session data leaked
    into committed captures. Probe-plugin sources confirmed comment-free per
    repository convention. Working-tree diff confirmed limited to the 21
    fixture files listed above; plan file and all other paths untouched by
    the research work itself.
  - Context impact: durable-context classification `pending-review` — this
    finding materially affects the plan's own Design section (D5, D6, D13)
    and T02/T04's task bodies, which is plan content, not the five root
    context files; no root context file (`architecture.md`, `context-map.md`,
    `glossary.md`, `overview.md`, `spec/mutation_cursor.md`) is affected by an
    evidence-only task with zero adapter/protocol code. The Task context
    synchronization phase should confirm this classification and record any
    residual impact.
  - Context synchronization: synced

- [x] T02: `Make Pi a confirmation-required protocol actor` (status:done)
  - Task ID: T02
  - Scope: In — adding the Pi case to the existing generalized confirmation
    predicate (`ClaudeCode -> false`, `Codex -> true`, `OpenCode -> true`,
    `Pi -> true`; the Codex and OpenCode entries already exist in the stacked
    base — see **Stack and base** — so this task's actual diff is adding Pi) in
    `cli/src/services/mutation_trace/protocol.rs`,
    `cli/src/services/mutation_trace/tests.rs`,
    `cli/src/services/mutation_trace/mbt/`, `spec/mutation_cursor.qnt`, and
    `spec/mutation_cursor.md`; adding explicit Pi scenarios (`Start(Pi A)` +
    mutation + another actor's boundary => `IneligibleUnscoped`; `Start(Pi A)` +
    mutation + `Close(Pi A)` => `AiExclusive(A)`; `Start(Pi A)` + `Start(Claude B)`
    + mutation + `Close(Pi A)` => `AiContended`; `Start(Pi A)` + `Start(Codex B)`
    + mutation + `Close(Pi A)` => `IneligibleUnscoped` until Codex B confirms).
    Also proves, via new Rust regression tests only in
    `cli/src/services/mutation_trace/tests.rs` (no `protocol.rs`/Quint semantic
    change — see D13/AC18), that the existing generic
    `database_failure`/`external_taint`/`recover` mechanism composes correctly
    with `Pi -> true`: a live Pi scope on a worktree that becomes externally
    tainted is abandoned by `recover`, never confirmed, and a fresh Pi scope
    started after `recover` clears the taint can still reach `AiExclusive`.
    This is the generic-protocol half of the T01 D13 finding and is unchanged
    by D13's corrected lifetime-guard mechanism (below): the guard changes
    *which process* spawns the human shell, *how* the OS-level lock's
    lifetime is made to durably track the shell's own lifetime (a supervisor
    process, plus, on Unix, fd-duplicating the lock into the spawned shell —
    D13), and *when* the existing `database_failure`/`recover` composition is
    forced — not what that composition means at the protocol level, so this
    task's Pi-actor regression already covers the load-bearing
    generic-protocol claim the guard depends on. Confirmed after inspecting
    the corrected D13 supervisor design: it remains pure runtime/ingress
    composition of already-existing `ProtectedWorktree`/`coordinate()`/
    `database_failure`/`recover`/`Flush` primitives, the pre-existing
    `CoordinateError::{LockAcquisition, MarkerClearAfterCommit}` variants, and
    ordinary OS process/fd mechanics (spawning a child, duplicating a file
    descriptor into it) — no new field on `ProtocolState`/`ScopeState`/
    `Attribution`, no new Quint action or state component, and no
    protocol-level behavior the current model does not already represent. No
    mutation-cursor protocol change is needed for D13 beyond what this task
    already does for `Pi -> true`. The new long-lived supervisor invocation
    itself, and Pi's own call site, are T03's and T05's responsibility, not
    T02's.

    **Hard precondition before any work in this task's own scope begins (added by this amendment):** both of D13's remaining blockers must already be resolved in the plan text —
    ```text
    D13 dispatch-admission strategy = resolved
    D13 platform lifetime strategy  = resolved
    ```
    Both are now resolved in D13 as amended: platform lifetime is Option B
    (fd-duplication on Unix, unchanged; `user_bash` unconditionally refused on
    Windows with tracked-tool attribution otherwise intact); dispatch/array
    admission is Option A per D13 "Corrected a fifth time" — SCE's launcher
    owns `ResourceLoader` construction for the session (`createRuntime` +
    `resourceLoaderOptions.extensionFactories`/`extensionsOverride`, both
    confirmed first-class, publicly-exported Pi `0.80.6` constructor options,
    not an invented API), so the array `ExtensionRunner` is built from is
    SCE-governed, SCE-first, on every rebuild — initial load, every later
    `/reload`/`AgentSession.reload()`, and every `/new`/`/resume`/`/fork` that
    reuses the same `createRuntime` factory — by construction, not by a
    runtime check that can itself be absent when SCE is. This supersedes and
    retracts Option C ("Corrected a fourth time": launcher-refuses-to-exec +
    SCE's own in-process self-check + terminate-on-newly-unsafe-reload) as
    the primary proof; that design is preserved only as optional
    diagnostics/defence-in-depth (see D13). Do not perform any Pi
    confirmation-required protocol edit while either strategy is unresolved;
    since both are resolved by this amendment, T02's own Rust/Quint work
    below may proceed, but this task's own record must restate the
    resolution (not silently inherit it from D13) before being marked done —
    including D13's required invariant ("the authority... must exist outside,
    or below, the replaceable Pi extension set... SCE's own extension cannot
    be the sole watchdog for whether SCE is still present, first, or active
    after an ExtensionRunner replacement") verbatim or by exact
    cross-reference, not merely by section number.

    **Before this task's own Rust/Quint work is considered done, this task
    must also formally freeze — mirroring how T01 froze pinned Pi lifecycle
    evidence into committed fixtures, not merely into plan prose — pinned-source
    evidence answering the following, citing exact files/line numbers from the
    vendored `config/lib/node_modules/@earendil-works/pi-coding-agent` copy of
    `0.80.6` (this amendment already located the answers below during
    plan-correction research; T02 must independently re-confirm each against
    the installed package and commit the citations, not merely copy this
    paragraph):**
    ```text
     1. Who constructs ResourceLoader? — Never AgentSession itself; it is
        handed in via constructor config (agent-session.js line 132:
        `this._resourceLoader = config.resourceLoader`). The caller —
        `createAgentSession`/`createAgentSessionServices`/SCE's own launcher
        — constructs it.
     2. Who constructs ExtensionRunner? — AgentSession._buildRuntime(),
        exclusively from `this._resourceLoader.getExtensions()`
        (agent-session.js lines 2002/2008).
     3. What exact array is passed to ExtensionRunner? — Exactly
        `extensionsResult.extensions`, the return value of
        `resourceLoader.getExtensions()`, unmodified.
     4. Can an external launcher/host provide or override that exact array?
        — Yes: `DefaultResourceLoaderOptions.extensionFactories` (resource-loader.d.ts,
        ~line 70; consumed unconditionally on every `reload()` via
        `loadFinalExtensionSet()`, resource-loader.js lines 366-372) inserts
        launcher-supplied factories independent of on-disk discovery, and
        `DefaultResourceLoaderOptions.extensionsOverride` (resource-loader.d.ts
        line 78; invoked unconditionally at the end of every `reload()`,
        resource-loader.js line 279) lets the launcher reorder/reject the
        complete final array. Both are constructor-bound to one `ResourceLoader`
        instance reused unreplaced for the life of the process.
     5. Can extension discovery be disabled/frozen? — Discovery itself
        (`noExtensions`) can be disabled, but freezing is unnecessary:
        `extensionsOverride` governs the final array regardless of what
        discovery produces, every time.
     6. What exactly does /reload rebuild? — `AgentSession.reload()`
        (agent-session.js lines 2023-2034) calls `this._resourceLoader.reload()`
        (the same instance, same bound options) then `_buildRuntime()`, which
        rebuilds `ExtensionRunner` from that instance's `getExtensions()`.
        Nothing about `/reload` replaces the `ResourceLoader` instance itself.
     7. Can /reload be disabled or intercepted outside extension handlers? —
        Not disabled, and does not need to be: the launcher's bound
        `extensionsOverride` intercepts every `/reload`'s result by
        construction, without any extension-level hook.
     8. Can the launcher host the Pi session programmatically? — Yes:
        `createAgentSession`/`createAgentSessionServices`/`createAgentSessionRuntime`/
        `createAgentSessionFromServices` are exported from the package root
        (dist/index.d.ts, re-exported from sdk.ts), and Pi's own production
        entry point (`dist/main.js` lines 489-598) already uses exactly this
        composition, handing the result to `InteractiveMode`/`runPrintMode`/
        `runRpcMode` (also package-root-exported, dist/modes/index.d.ts) for
        the actual UI.
     9. Does Pi expose an authoritative runner-construction seam? — Yes: the
        `ResourceLoader` the caller supplies is that seam; `ExtensionRunner`
        is always and only built from its `getExtensions()` result.
    10. What happens if SCE's own extension is absent after reload? — For a
        launcher-hosted session, this cannot happen: SCE's presence and
        position come from the launcher's own `extensionFactories`/
        `extensionsOverride`, never from on-disk discovery, so nothing that
        mutates on-disk configuration can make SCE "absent." For a session
        NOT hosted by SCE's launcher (raw `pi`, or SDK embedding bypassing
        SCE's `ResourceLoader` construction), SCE's extension code, if
        present at all, still cannot police its own absence — this remains
        the named, permanent, unclosable "Raw Pi / SDK embedding" boundary.
    11. Which selected mechanism remains active in that case? — None; there
        is no runtime mechanism inside `sce-pi-extension.ts` that this
        design still depends on for soundness. The mechanism is the
        launcher's `ResourceLoader` construction, which is not a member of
        the extension set and therefore cannot be removed by mutating it.
    ```
    **Added by this amendment — the plan's D13 "Canonical SCE runtime-instance
    invariant" identified a further gap (launcher-hosted SCE loading twice, not
    merely not-first) that the eleven answers above do not by themselves close.
    T02 must also independently re-confirm and commit, with exact file/line
    citations against the installed `0.80.6` package, evidence answering:**
    ```text
    12. Does loadFinalExtensionSet() append inline factories to
        already-discovered extensions, or does it replace/merge them? —
        Confirm exactly (resource-loader.js, loadFinalExtensionSet(),
        loadExtensionFactories() call site): this amendment's research
        found an unconditional append (extensionsResult.extensions.push(
        ...inlineExtensions.extensions), resource-loader.js ~lines
        366-372) — T02 re-confirms this independently rather than
        inheriting it from this amendment's prose.
    13. Does Pi perform any deduplication between an extension loaded from
        disk and the same factory supplied inline (by path, by name, by
        factory reference, or otherwise)? — This amendment's research found
        none. T02 must independently verify no such check exists anywhere
        in loadFinalExtensionSet()'s call chain before relying on its
        absence.
    14. What exact identity/path does a named inline factory
        { name: "sce", factory } receive from loadExtensionFromFactory()
        (or equivalent) — the literal Extension.path/source-identity
        value, not merely "something like <inline:sce>"? Cite the exact
        source line that assigns it.
    15. What exact identity/path does the generated disk SCE extension
        (<repo>/.pi/extensions/sce/index.ts) receive from on-disk
        discovery, after whatever canonicalization/realpath behavior the
        loader applies? Cite the exact source line.
    16. Given 14 and 15, which field is safe to use to identify only SCE's
        generated compatibility copy — sufficient to exclude
        .pi/extensions/sce-custom/, a foreign package merely named `sce`,
        and any extension whose path/name only contains "sce"? Record the
        exact predicate, not a name/basename heuristic.
    17. Confirm that extensionsOverride's `base` parameter, for a
        launcher-hosted session with the generated disk copy present, does
        in fact contain both the disk-discovered and the inline SCE
        instances before any normalization runs — i.e. that the duplication
        this amendment describes is not merely theoretical for the pinned
        version.
    18. Confirm that the array extensionsOverride returns is exactly, and
        only, what AgentSession._buildRuntime() passes to ExtensionRunner's
        constructor — no further Pi-internal filtering, deduplication, or
        reordering occurs between extensionsOverride's return and
        ExtensionRunner construction.
    ```
    Answers 12-18 must be recorded before T02 is marked complete, alongside
    the original eleven. This amendment performs no T02 implementation work
    itself — recording these seven questions is a plan-text change only; T02
    remains `todo` and answering them is T02's own task, not this amendment's.
    This record replaces, not supplements, the "Also required before this
    task is done" paragraph an earlier version of this task carried — that
    paragraph characterized Pi's dispatch order (`resourcePrecedenceRank()`,
    CLI-provided extensions unconditionally first, `extensionsOverride`'s
    unconstrained power) as reasons no SCE-controllable ordering guarantee
    could exist; that characterization is **retracted only as a conclusion**,
    not as evidence — the underlying dispatch-order facts remain accurate and
    are exactly why the launcher must own `extensionsOverride` itself rather
    than merely detect what an *adversary's* `extensionsOverride` might do.
    T02 must record: (i) the eleven answers above, with exact citations; (ii)
    that `extensionFactories`/`extensionsOverride` are confirmed, typed,
    root-exported constructor options — not invented — on the pinned
    package; (iii) that a raw `pi` invocation or an SDK-embedded
    `AgentSession` that does not use SCE's launcher's `ResourceLoader`
    construction remains a named, permanent, worktree-unsafe external-mutator
    boundary this plan does not close and must never describe as safe. This
    is evidence-recording, not a protocol or Quint change, and does not
    require touching Rust/TS/Quint code in this task. Out — touching
    Claude/Codex/OpenCode's existing confirmation behavior; adding
    session/model/process fields to protocol state; implementing the Pi Rust
    hook adapter itself (T03) or the external-mutation guard/supervisor
    mechanism (T03); implementing the launcher's `createRuntime`/
    `resourceLoaderOptions` wiring or the doctor detection check itself
    (T05).
  - Dependencies: T01
  - Done when: Rust and Quint agree that Pi needs its own confirming Close and
    all existing Claude/Codex/OpenCode behavior remains unchanged; a
    Pi-actor regression proves the existing external-taint/recover mechanism
    abandons a tainted live Pi scope and allows a later clean Pi scope to
    confirm normally, using only the already-existing
    `database_failure`/`recover`/`abandon` actions; the task record states
    explicitly, after inspecting T03's chosen supervisor design, that no
    `protocol.rs`/Quint change is required for D13's lifetime-guard mechanism;
    the task record also restates both D13 dispositions (platform lifetime =
    Option B; dispatch/array admission = Option A per "Corrected a fifth
    time" — SCE's launcher owns `ResourceLoader` construction via
    `extensionFactories`/`extensionsOverride`, so the array `ExtensionRunner`
    is built from is SCE-governed on every rebuild by construction, with no
    runtime self-check or process-termination step required as the primary
    proof) and confirms neither is an open/upstream-blocked question before
    this task is considered done; the task record additionally restates,
    verbatim or by exact cross-reference, D13's required invariant (the
    safety authority must exist outside/below the replaceable Pi extension
    set; SCE's own extension cannot be the sole watchdog for its own
    continued presence) and the worktree-wide unsafe-mode rule (disabling
    Pi's own attribution does not protect a live Claude/Codex/OpenCode
    scope), and names the raw-`pi`/SDK-embedding path — now defined as any
    session whose `ResourceLoader` was not constructed by SCE's launcher —
    as the one permanent, worktree-unsafe boundary this plan does not close;
    the task record also commits the eleven pinned-evidence answers above,
    plus this amendment's seven added answers (12-18, covering inline/disk
    identity, dedup absence, and the extensionsOverride/ExtensionRunner
    array-consumption guarantee), with exact file/line citations
    re-confirmed against the installed package, as this task's own frozen
    evidence record.
  - Verify: `nix run .#quint -- typecheck spec/mutation_cursor.qnt`;
    `nix run .#quint -- test spec/mutation_cursor.qnt`;
    `nix build .#checks.x86_64-linux.mutation-trace-quint-connect`;
    `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace`.
  - Completed: 2026-09-17
  - Files changed: `cli/src/services/mutation_trace/protocol.rs`;
    `cli/src/services/mutation_trace/tests.rs`;
    `cli/src/services/mutation_trace/mbt/model.rs`;
    `cli/src/services/mutation_trace/mbt/driver.rs`;
    `cli/src/services/mutation_trace/runtime/coordinator.rs`;
    `spec/mutation_cursor.qnt`; `spec/mutation_cursor.md` (7 files; no other
    paths touched).
  - Result: `requires_boundary_confirmation`/`requiresBoundaryConfirmation` now
    return `true` for `Pi` in both `protocol.rs` and `mutation_cursor.qnt`
    (previously grouped with `ClaudeCode` under `false`); `mutation_cursor.md`'s
    three confirmation-required-actor prose spots updated to name Pi
    alongside Codex/OpenCode. A new mock `Scope6 -> Pi -> WT0` was added to the
    Quint model (`ScopeId`, `SCOPES`, `scopeWorktree`, `scopeActor`) and the
    four scenarios the task named were added as `run`s: `testUnconfirmedPiScopeBlocksCrossHarnessAttribution`
    (`Start(Pi A)` + `Start(Claude B)` + mutation + `Advance(Claude B)` =>
    `IneligibleUnscoped`), `testPiCloseConfirmsExclusiveAttribution`
    (`Start(Pi A)` + mutation + `Close(Pi A)` => `AiExclusive(A)`),
    `testPiCloseConfirmsContendedAttribution` (`Start(Pi A)` + `Start(Claude B)`
    + mutation + `Close(Pi A)` => `AiContended`), and
    `testPiAndCodexScopesStayMutuallyUnconfirmedAtEitherClose` (`Start(Pi A)` +
    `Start(Codex B)` + mutation + `Close(Pi A)` => `IneligibleUnscoped`, Codex B
    still unconfirmed) — all four auto-covered by the existing `test.*`
    coverage-backstop `quint_test` in `mbt/tests.rs` with no new hand-written
    Rust wrapper needed, matching that backstop's documented purpose. Adding
    `Scope6` exposed a real pre-existing bug in `singleScope` (line ~259): a
    fixed if/else chain over `Scope0..Scope4` with a bare `else { Scope5 }`
    fallback that silently mis-attributed any live set containing only
    `Scope6` as `Scope5` (`AiExclusive(Scope5)` instead of `AiExclusive(Scope6)`);
    the fallback is now `else if (scopes.contains(Scope5)) { Scope5 } else {
    Scope6 }`. `HasPiConfirmedExclusiveEvidence`/`HasPiConfirmedContendedEvidence`
    `val`s were added alongside the existing Codex/OpenCode ones. In
    `tests.rs`, a `pi_scope` helper was added and a new regression,
    `a_tainted_live_pi_scope_is_abandoned_by_recover_and_a_fresh_pi_scope_can_still_confirm`,
    proves the existing `database_failure`/`external_taint`/`recover`
    mechanism composes correctly with `Pi -> true`: a live Pi scope on an
    externally-tainted worktree is abandoned by `recover` and excluded from
    confirmation, and a fresh Pi scope started afterward on the
    recovered worktree still reaches `AiExclusive` at its own `Close`. The MBT
    driver/model (`mbt/driver.rs`, `mbt/model.rs`) were extended with the
    matching `scope6`/`Scope6`/`Pi` wiring so the Quint-Connect replay harness
    stays in lockstep with the mock scope partition. Making Pi
    confirmation-required broke the *premise*, not the correctness, of two
    pre-existing tests that had used Pi as a stand-in for "a second
    non-confirmation-required actor" (the only such actor now is `ClaudeCode`):
    `tests.rs`'s `two_live_non_codex_scopes_still_attribute_contention` and
    `coordinator.rs`'s `contended_scopes_yield_ai_contended_same_and_different_actor`
    (via its `assert_contended_attribution` helper) both drove attribution
    through a boundary (`Advance` on the *other* scope) that never confirms the
    Pi scope, which is now correctly `IneligibleUnscoped` rather than a bug.
    Both were updated to close the confirmation-required scope directly
    (`Close(pi-b)` / `RuntimeBoundary::Close` on `scope-b`) — the same shape
    already proven for Codex/OpenCode elsewhere in this suite — restoring
    `AiContended` coverage for a live non-required scope overlapping a
    confirmed Pi scope.

    **No `protocol.rs`/Quint change is required for D13's lifetime-guard
    mechanism**, after inspecting the corrected D13 supervisor design: it is
    pure runtime/ingress composition of already-existing
    `ProtectedWorktree`/`coordinate()`/`database_failure`/`recover`/`Flush`
    primitives, the pre-existing `CoordinateError` variants, and ordinary OS
    process/fd mechanics (spawning a child, duplicating a file descriptor
    into it). It adds no new field to `ProtocolState`/`ScopeState`/
    `Attribution`, no new Quint action or state component, and no
    protocol-level behavior the current model does not already represent —
    this task's Pi-actor `database_failure`/`recover` regression already
    covers the load-bearing generic-protocol claim the guard depends on
    (`Pi -> true` composes soundly with the existing recovery mechanism). The
    new long-lived supervisor invocation itself, and Pi's own call site,
    remain T03's and T05's responsibility.

    **D13 dispositions restated (not merely inherited from D13):** platform
    lifetime strategy = Option B (fd-duplication on Unix so the `WorktreeLock`
    flock survives the supervisor's own death via the shell's inherited
    duplicate descriptor; `user_bash` unconditionally refused on Windows,
    tracked-tool attribution otherwise intact there). Dispatch/array admission
    strategy = Option A per D13 "Corrected a fifth time": SCE's own launcher
    owns `ResourceLoader` construction for the session via
    `createRuntime`/`resourceLoaderOptions.extensionFactories`/`extensionsOverride`
    (confirmed first-class, publicly-exported, typed constructor options on the
    pinned package, not invented), so the exact array `ExtensionRunner` is
    built from is SCE-governed, SCE-first, on every rebuild (initial load,
    every `/reload`, every `/new`/`/resume`/`/fork` reusing the same
    `createRuntime` factory) by construction, not by a runtime check that can
    itself be absent when SCE is. Neither disposition is open or
    upstream-blocked.

    **D13's required invariant, restated verbatim:** "the authority... must
    exist outside, or below, the replaceable Pi extension set... SCE's own
    extension cannot be the sole watchdog for whether SCE is still present,
    first, or active after an ExtensionRunner replacement." The
    worktree-wide unsafe-mode rule: disabling Pi's own attribution does not
    protect another harness's live scope on the same worktree — for any live
    Claude/Codex/OpenCode/Pi scope, not only Pi's own. The one permanent,
    worktree-unsafe boundary this plan does not close is any session whose
    `ResourceLoader` was not constructed by SCE's launcher — a raw `pi`
    invocation, or an SDK embedding that bypasses SCE's `ResourceLoader`
    construction.

    **18 pinned-evidence answers, independently re-confirmed against the
    installed `config/lib/node_modules/@earendil-works/pi-coding-agent@0.80.6`
    package (exact file/line citations; some line numbers differ slightly
    from the amendment's approximate ones because this task re-derived them
    directly rather than copying them):**
    1. `dist/core/agent-session.js:132`: `this._resourceLoader =
       config.resourceLoader;` — no `??` fallback, no self-construction;
       the constructor requires the caller to supply it.
    2. `dist/core/agent-session.js:2002`: `const extensionsResult =
       this._resourceLoader.getExtensions();` inside `_buildRuntime()`.
    3. `dist/core/agent-session.js:2008`: `new
       ExtensionRunner(extensionsResult.extensions, extensionsResult.runtime,
       this._cwd, this.sessionManager, this._modelRegistry)` — exactly
       `extensionsResult.extensions`, unmodified.
    4. `dist/core/resource-loader.d.ts:70` (`extensionFactories?:
       InlineExtension[]`) and `:78` (`extensionsOverride?: (base:
       LoadExtensionsResult) => LoadExtensionsResult`); consumed at
       `dist/core/resource-loader.js:369` (inline factories pushed into the
       final array inside `loadFinalExtensionSet`) and `:279`
       (`this.extensionsResult = this.extensionsOverride ?
       this.extensionsOverride(extensionsResult) : extensionsResult;`,
       unconditional at the end of `reload()`).
    5. `noExtensions?: boolean` (`resource-loader.d.ts:71`) only gates
       discovery paths (`resource-loader.js:267`, `:351`); `extensionsOverride`
       still runs unconditionally at `:279` regardless, so freezing discovery
       is unnecessary — `extensionsOverride` governs the final array every
       time.
    6. `dist/core/agent-session.js:2023-2034` (`reload()`): calls
       `this._resourceLoader.reload()` (line 2028) then `this._buildRuntime()`
       (line 2029) — the same `ResourceLoader` instance, same bound options;
       nothing replaces the instance itself.
    7. Not disabled, does not need to be: `extensionsOverride` (bound to the
       one instance at construction) intercepts every `reload()`'s result
       unconditionally at `resource-loader.js:279`, with no extension-level
       hook required.
    8. `dist/index.d.ts:17` root-exports `createAgentSession`,
       `createAgentSessionServices`, `createAgentSessionFromServices`,
       `createAgentSessionRuntime` from `./core/sdk.ts`; `dist/main.js:501`
       (`createAgentSessionServices`), `:570`
       (`createAgentSessionFromServices`), `:593`
       (`createAgentSessionRuntime`) show Pi's own production entry point
       using exactly this composition, handing the result to
       `runRpcMode`/`InteractiveMode`/`runPrintMode` (`main.js:652,655,686`),
       which `dist/modes/index.d.ts:4,5,7` confirm are themselves
       package-root-exported.
    9. The `ResourceLoader` the caller supplies is the only seam:
       `ExtensionRunner` is always and only built from its `getExtensions()`
       result (answers 1-3).
    10. For a launcher-hosted session this cannot happen (answer 1: no
        fallback construction exists). For a session not hosted by SCE's
        launcher, no in-extension mechanism can inspect how its own host
        constructed the `ResourceLoader` it was handed.
    11. None — no runtime mechanism inside `sce-pi-extension.ts` remains
        depended on; the mechanism is the launcher's `ResourceLoader`
        construction itself, external to the extension set.
    12. `resource-loader.js:369`:
        `extensionsResult.extensions.push(...inlineExtensions.extensions);`
        inside `loadFinalExtensionSet` — an unconditional append, confirmed
        not a replace/merge.
    13. `resource-loader.js:401-403`, `addExtensionConflictDiagnostics`'s own
        comment: "Keep all extensions loaded. Conflicts are reported as
        diagnostics, and precedence is handled by load order." No
        deduplication exists anywhere in the call chain.
    14. `resource-loader.js:689`: `extensionPath =
        \`<inline:${isNamed ? input.name : index + 1}>\`` inside
        `loadExtensionFactories`; `dist/core/extensions/loader.js:369`
        (`loadExtensionFromFactory`): `createExtension(extensionPath,
        extensionPath)`; `loader.js:334`: `path: extensionPath` — for `{
        name: "sce", factory }` this is exactly `<inline:sce>` for both
        `Extension.path` and `.resolvedPath`.
    15. `loader.js:345` (`loadExtension`): `resolvedPath =
        resolvePath(extensionPath, cwd, { normalizeUnicodeSpaces: true })`;
        `loader.js:352`: `createExtension(extensionPath, resolvedPath)` —
        `.path` is the literal on-disk path string passed to discovery
        (e.g. `.pi/extensions/sce/index.ts`), `.resolvedPath` is the
        canonicalized absolute path.
    16. The safe predicate is exact equality of `.resolvedPath` against the
        canonical absolute path of the generated file (e.g.
        `path.resolve(repoRoot, ".pi/extensions/sce/index.ts")`), never a
        name/basename/substring heuristic: per answers 14-15, an inline
        factory's `.resolvedPath` is never a filesystem path (`<inline:...>`)
        and a foreign `sce`-named or `sce-custom` on-disk extension resolves
        to a different absolute path, so exact-path equality alone
        distinguishes all cases the plan named.
    17. `resource-loader.js:270` (`loadFinalExtensionSet` populates
        `extensionsResult` with both disk-discovered extensions and, per
        answer 12, the pushed-in inline ones) flows directly into `:279`'s
        `this.extensionsOverride(extensionsResult)` — the identical object,
        not a filtered subset, is `base`.
    18. `resource-loader.js:162-164`: `getExtensions() { return
        this.extensionsResult; }` returns the exact value set at `:279`
        verbatim; `agent-session.js:2002/2008` pass
        `extensionsResult.extensions` straight into `new ExtensionRunner(...)`.
        No further Pi-internal filtering, deduplication, or reordering occurs
        between `extensionsOverride`'s return and `ExtensionRunner`
        construction.
  - Verify outcome: `nix run .#quint -- typecheck spec/mutation_cursor.qnt`
    passed. `nix run .#quint -- test spec/mutation_cursor.qnt` passed all 40
    named scenarios (including the 4 new Pi ones) after the `singleScope` fix
    above — first attempt surfaced the real `singleScope` bug via
    `testPiCloseConfirmsExclusiveAttribution` failing with `AiExclusive(Scope5)`
    instead of `AiExclusive(Scope6)`, confirmed via an `--out-itf` trace dump,
    then fixed and reverified green. `nix build
    .#checks.x86_64-linux.mutation-trace-quint-connect` passed. `nix develop
    -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml
    mutation_trace` passed 370/370 after fixing the two pre-existing tests
    described above (368 passed, 2 failed on the first run for the reason
    given; 370 passed, 0 failed after the fix).
  - Context impact: durable-context classification `pending-review` — this is
    a protocol/Quint semantic change (`Pi -> true`) plus a real spec bug fix
    (`singleScope`'s fallback), but none of the five root context files
    (`architecture.md`, `context-map.md`, `glossary.md`, `overview.md`,
    `spec/mutation_cursor.md`) name Pi's confirmation-required status as a
    fact needing correction beyond `spec/mutation_cursor.md` itself, which
    this task already updated directly (it is the plan's living design
    document, not a completed-task record). No other root context file
    asserts Pi is non-confirmation-required. The Task context synchronization
    phase should confirm this classification and record any residual impact.
  - Context synchronization: synced
  - Repair (post-completion review, 2026-09-17): `spec/mutation_cursor.md`
    incorrectly generalized Codex/OpenCode's "post-tool signal absent on
    denial" reasoning to Pi, implying Pi's terminal event is likewise absent
    when execution is denied. It is not (T01 D5: the terminal event fires
    unconditionally, even when Pi's own `tool_call` gate blocks or throws).
    Corrected the prose to state that Pi's terminal event alone is not
    execution evidence, and that only the `tool_result`-then-terminal-event
    pairing (D6/D7) confirms a Pi scope — matching the implementation
    unchanged. Separately,
    `a_tainted_live_pi_scope_is_abandoned_by_recover_and_a_fresh_pi_scope_can_still_confirm`
    was rewritten: it previously continued past `recover` by constructing a
    fresh `ProtocolState::default()` and inserting the follow-up Pi scope
    directly as `Active`, bypassing the real `Start` transition. It now
    continues directly from `recover`'s own returned state, pre-registers the
    follow-up scope as `NeverSeen` (as production seeds a scope row before
    its first hook boundary), and drives it through a real `prepare`/`commit`
    `Start`, an intermediate `attribution_for_boundary` check proving
    `IneligibleUnscoped` before its own confirmation, and a real `Close`
    reaching `AiExclusive(Pi)`. Neither repair touched
    `requires_boundary_confirmation`/`requiresBoundaryConfirmation` or any
    other `protocol.rs`/Quint semantics. Re-verified: `nix develop -c
    ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml
    mutation_trace` (370 passed, 0 failed).

- [x] T03: `Add the Pi mutation-scope adapter` (status:done)
  - Task ID: T03
  - Scope: In — `cli/src/services/hooks/pi_mutation_scope/` and the hidden
    `sce hooks pi-mutation-scope` route, owning strict Pi wire parsing,
    tracked/read-only/untracked classification, `ScopeId`/`EventId`
    derivation, canonical `pi_` session identity, admission-time model
    provenance, checkout-local attempt state (`PendingStart` -> `Executed` ->
    `Closed`, with `PendingAbandon` for the existing D8 recovery bookkeeping),
    `tool_call` Start, the `tool_result`-keyed `Executed` transition, and
    `tool_execution_end` Close gated on that transition having already
    happened (D5/D6/D7, corrected by T01). `tool_execution_start` is received
    and may be surfaced as telemetry only; it drives no phase transition.
    Also adds the D13 external-mutation supervisor mechanism as new,
    harness-neutral runtime/ingress plumbing — not a Pi-specific route, so any
    future harness with the same `user_bash`-shaped overlap can reuse it:

    * A small `cli/src/services/mutation_trace/runtime/protected_worktree.rs`/
      `coordinator.rs` refactor exposing a way to (a) acquire a
      `ProtectedWorktree` and report its "armed" state to a caller without
      immediately processing a boundary and completing it, and (b) later, on
      that *same already-held* `ProtectedWorktree` (no second acquisition, no
      window where the lock could be lost to a queued foreign waiter), force
      the existing `database_failure` + `recover` composition against a
      freshly observed tree and, only on a durable commit, call
      `ProtectedWorktree::complete()`. Both operations reuse
      `WorktreeLock`/`ExternalTaintMarker`/`database_failure`/`recover`
      unchanged; no `protocol.rs`/Quint edit (D13/AC18).
    * One new long-lived sibling of the existing one-shot
      `start`/`advance`/`close`/`flush`/`abandon` operations in
      `cli/src/services/hooks/mutation_scope.rs` (exact operation/command
      naming per repository convention, not frozen here) that plays the role
      of the D13 supervisor: on invocation it performs the acquire above,
      emits one durable "armed" acknowledgement over its control channel once
      the lock is held and the marker is durably persisted, then **itself
      spawns the actual human shell as its own child process** — replicating
      the relevant parts of pinned Pi's own local-shell contract (the command
      string via a shell, `cwd`, `env`) since this is a Rust process and
      cannot call Pi's TypeScript `createLocalBashOperations()` directly; T03
      must document exactly which parts of that contract it reproduces and
      cite the pinned package's own local-execution behavior as the reference
      being matched. The supervisor streams the shell's stdout/stderr back
      over the control channel (so Pi's `onData` still fires) and accepts
      cancellation/timeout requests from the caller, which it enacts by
      signaling the real shell's process group. On Unix, before spawning the
      shell, the supervisor duplicates the `WorktreeLock`'s underlying file
      descriptor (a `std::fs::File`-backed `flock(2)`, confirmed against
      `worktree_lock.rs`) with `FD_CLOEXEC` cleared on the duplicate, so the
      spawned shell inherits an open descriptor referencing the *same* open
      file description and therefore holds the *same* advisory lock,
      independent of the supervisor's own liveness (D13's supervisor-crash
      safety); this supervisor mechanism, and the shell-spawning it performs,
      is invoked only on Unix — on Windows, D13's `user_bash` handler takes
      the guard-establishment-failure branch unconditionally (see D13's
      corrected Windows disposition), so the supervisor is never invoked and
      this fd-duplication step is simply not reached, not "skipped as
      unnecessary." T03 must gate the supervisor's invocation on platform
      (Unix-only) rather than implementing a no-op/simplified guard path for
      Windows. Lifetime-token creation/configuration precedes the durable
      `Armed` acknowledgement; a token-establishment failure emits no
      acknowledgement and spawns no shell. The normal finish trigger is
      **exclusively the shell's own process termination plus lifetime-token EOF**
      (`wait()`/`waitpid` and the kernel-owned pipe on the spawned child and
      ordinary descendants) — never stdin EOF or any other signal on the
      control channel to Pi/Node, which may close independently of the shell's
      lifetime (D13's chosen Option A: control-channel death from Pi/Node dying
      does not finish the guard). Once both conditions hold, the supervisor
      runs the forced recovery/`complete()` sequence above, after which it
      emits a final result over the control channel (if anything is still
      listening) and exits — success only if the recovery commit and the
      marker clear both durably succeeded. Before spawn, ordinary RAII cleanup
      is safe. After successful spawn and before lifetime EOF, every
      supervision error or panic-adjacent unwind uses a consuming abandonment
      path that leaves the marker armed and closes only the supervisor's own
      lock reference without explicit `flock(LOCK_UN)`; the inherited shell or
      descendant descriptor keeps the flock kernel-held. After lifetime EOF,
      ordinary unlock is safe even if final recovery fails, but the marker
      remains armed. This needs no `scope_id`/`event_id` —
      worktree identity is still derived by the runtime from the invoking
      checkout, and guard identity (for stale-owner detection) is exactly "is
      the `WorktreeLock` still held (by the supervisor, by the shell via its
      duplicated descriptor, or both)," never a Pi-specific field, session
      id, or `ActorKind::Pi`.

    Durable Pi adapter state remains under
    `<git-dir>/sce/pi-mutation-scope-state.json` with a versioned schema,
    persisted with the same lock/write-temp/sync/atomic-rename/
    best-effort-parent-sync discipline as other adapters, never holding the
    adapter-state lock while invoking the generic mutation runtime; this file
    and its discipline are unrelated to, and untouched by, the external-mutation
    supervisor, which is generic runtime state, not Pi adapter state. Out —
    any recovery/stale-process handling beyond the supervisor's own crash
    semantics already specified in D13 (T04 owns the adapter-reconciliation
    tests); wiring into the actual TypeScript extension, including the
    `user_bash` call site that spawns/manages the supervisor process (T05).
    **Superseded text, retained for history only:** an earlier draft of this
    line additionally scoped out "the `user_bash` extension-dispatch-order
    launcher/env-var gate and doctor check (T02 records the disposition, T05
    implements both — AC22)." No such launcher, env-var gate, or
    dispatch-order doctor check exists or is implemented by T05 — D13's later
    retraction of any SCE-hosted Pi launcher or extension-order authority
    mechanism (see D13's "No dispatch-safety self-check, no launcher, no
    extension-order authority" and AC22) makes that sentence stale; T05's
    actual scope is the unconditional `user_bash` handler wiring (arm ->
    `Armed` -> `exec`, Windows refusal) with no dispatch-position self-check
    of any kind. Also out of T03's scope — any Windows-specific
    supervisor/guard code (D13's Windows disposition is unconditional refusal
    at the extension level, T05, not a Rust-side platform branch T03 needs to
    implement).
  - Dependencies: T02
  - Done when: the Rust adapter correctly drives the frozen happy-path Pi
    lifecycle through the generic mutation-scope runtime, with durable
    provenance and no recovery shortcuts, reaching mutation semantics only
    through the existing `hooks::mutation_scope` ingress seam (no second
    direct `coordinate()` path); focused tests cover parser rejection,
    classification, identity stability/replay, terminal `ScopeId` non-reuse,
    session separation, model present/absent, the `tool_result`-keyed
    `Executed` transition (never `tool_execution_start`), successful Close,
    failed-execution-still-Close, `tool_execution_end`-without-`tool_result`
    abandon (D7), untracked zero footprint, and the new external-mutation
    supervisor invocation: armed acknowledgement only after the lock is held
    and the marker is durably persisted; the supervisor — not the caller —
    spawns the real shell as its own child, with the spawned shell's parent
    pid equal to the supervisor's pid; on Unix, the shell inherits a
    duplicated, `FD_CLOEXEC`-cleared copy of the lock's file descriptor
    before any command executes; a concurrent foreign `coordinate()` call
    blocks and then fails closed with `CoordinateError::LockAcquisition`
    while the guard is active, touching no protocol state; killing the
    supervisor process directly (not the shell) while the shell keeps running
    leaves `WorktreeLock` held (a concurrent foreign `coordinate()` call still
    blocks/fails closed) until the shell itself exits, at which point the
    lock frees with `ExternalTaintMarker` still armed and the next
    `coordinate()` call self-heals via the existing inherited-taint path;
    closing the control channel to the caller (simulating Pi/Node death)
    while the shell is still running does not trigger the finish sequence and
    does not signal the shell; finish requires the shell's own process
    termination and lifetime-token EOF, then forces `database_failure`+`recover`
    against the already-held `ProtectedWorktree`, only then clearing the marker;
    a failed finish commit leaves the marker armed and reports failure without
    calling `complete()`; all with no `protocol.rs` involvement.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml pi_mutation_scope`;
    `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml hooks::`.
  - Completed: 2026-09-17
  - Repaired: 2026-09-17 (same-day repair pass — cwd contract, exact pinned
    Pi shell contract citations, final stdout/stderr draining race, and
    context synchronization; see the sections below marked "2026-09-17
    repair pass"). T04 was not started by this repair.
  - Files changed: `cli/src/services/hooks/pi_mutation_scope/{mod.rs,state.rs,os_lock.rs,boundary_lock.rs}`
    (new); `cli/src/services/mutation_trace/runtime/external_mutation_guard.rs` (new);
    `cli/src/cli_schema.rs`; `cli/src/services/hooks/mod.rs`;
    `cli/src/services/hooks/mutation_scope.rs`;
    `cli/src/services/mutation_trace/runtime/coordinator.rs`;
    `cli/src/services/mutation_trace/runtime/mod.rs`;
    `cli/src/services/mutation_trace/runtime/protected_worktree.rs`;
    `cli/src/services/mutation_trace/runtime/worktree_lock.rs`;
    `cli/src/services/parse/command_runtime.rs` (13 files; no other paths touched;
    `fixtures/` and `protocol.rs`/`spec/mutation_cursor.qnt` untouched).
  - **2026-09-17 repair pass files changed** (see "Repair" below):
    `cli/src/services/mutation_trace/runtime/external_mutation_guard.rs`;
    `cli/src/services/mutation_trace/runtime/git_snapshot.rs` (new
    `resolve_worktree_root`); `cli/src/services/hooks/mutation_scope.rs`
    (new `cwd` wire field); `cli/src/services/hooks/pi_mutation_scope/{mod.rs,state.rs}`
    (whitespace-only `cargo fmt` reformatting of pre-existing drift found
    while getting `nix flake check`'s `cli-fmt` gate green, no behavior
    change). `protocol.rs`/`spec/mutation_cursor.qnt` and `fixtures/`
    remain untouched.
  - **2026-09-17 lifetime-token repair files changed:**
    `cli/src/services/mutation_trace/runtime/external_mutation_guard.rs`;
    `context/cli/mutation-trace-external-mutation-guard.md`;
    `context/plans/pi-mutation-scope-integration.md`;
    `context/decisions/2026-09-17-external-mutation-guard-process-supervisor.md`.
    No change to normal `WorktreeLock` drop semantics, protocol/Quint change,
    T04 work, or generated-config change was made.
  - Result: **Adapter** (`pi_mutation_scope/{mod.rs,state.rs,os_lock.rs,boundary_lock.rs}`,
    mirroring `opencode_mutation_scope`'s file shape): a strict wire parser
    accepts `hook_event_name` one of `ToolExecutionStart`/`ToolCall`/`ToolResult`/
    `ToolExecutionEnd` (with `session_id`/`tool_call_id`/`cwd`/`tool_name`, plus
    `model` only on `ToolCall`) over the new hidden `sce hooks pi-mutation-scope`
    route; classification is the closed `bash|edit|write -> TrackedMutation`
    allowlist per D2 (`read`/`grep`/`find`/`ls` and everything else untracked) —
    no OpenCode-style bash/`ToolExecuteBefore` split, since D4/T01's NOTES.md
    establish `tool_call` as Pi's single universal pre-execution gate for every
    tool including `bash`. `ScopeId` is `pi-tool-v1|n=<attempt-seq>|s=<len>:<session>|c=<len>:<tool-call-id>`
    exactly per D1's freeze, with a checkout-local monotonic `next_attempt_seq`
    counter in the adapter state file so a toolCallId reused after terminal
    cleanup gets a fresh `attempt_seq`/`ScopeId` rather than ever reactivating a
    closed/abandoned scope (proved by
    `a_new_attempt_after_terminal_cleanup_gets_a_fresh_attempt_seq_and_scope_id`
    and the mod.rs-level
    `a_reused_tool_call_id_after_terminal_cleanup_gets_a_distinct_scope_id`).
    The attempt-phase machine is `AttemptPhase::{PendingStart, Executed,
    PendingAbandon}` — no `Active` phase — because D5's freeze makes
    `PendingStart` Pi's normal resting state for the tool's *entire* in-flight
    execution window (there is no `tool_execution_start`-keyed commit step);
    `mark_executed` transitions `PendingStart -> Executed` on `tool_result` only
    (D5/D6), and `tool_execution_end` closes an `Executed` attempt (falling back
    to abandon/recover on a Close failure, matching the OpenCode precedent) or
    abandons a still-`PendingStart` attempt outright — D7's exact signal, with
    no reliance on `agent_settled`/timeouts. **Deliberate departure from the
    OpenCode/Codex `admit_tracked_attempt` precedent**, recorded because it is
    not spelled out character-for-character in the plan text but follows
    necessarily from D5+D12: since `PendingStart` is Pi's long-lived steady
    state (not a narrow crash-recovery artifact the way it is for
    OpenCode/Codex, whose admission also serializes under one boundary lock
    per invocation but transitions to `Active` before releasing it), the
    "uncertain attempt" fail-closed admission check for Pi only fires on a
    lingering `PendingAbandon` or non-`Clear` `RecoveryState` — **never** on a
    sibling's `PendingStart` — otherwise every concurrent Pi tool call would
    serialize checkout-wide, contradicting D12
    (`a_pending_start_attempt_never_blocks_a_concurrent_new_admission`,
    `concurrent_bash_calls_in_one_session_stay_separate_live_scopes`). D11
    provenance reuses the existing `prefixed_diff_trace_session_id(PI_TOOL_NAME,
    ...)` (already Pi-aware) plus a new `normalize_pi_model_id` alongside the
    existing Codex/OpenCode normalizers in `hooks/mod.rs`. The adapter reaches
    mutation semantics only through the existing `hooks::mutation_scope`
    ingress seam (no second direct `coordinate()` path) — proved against a
    `RecordingSeam` fake for every lifecycle branch and, separately, against a
    real Git repository and a real Agent Trace DB in a new `runtime_seam_tests`
    module (`a_write_start_result_close_lands_a_real_ai_exclusive_event_with_pi_provenance`,
    `a_start_followed_by_no_execution_abandons_through_the_real_runtime`).

    **D13 external-mutation supervisor**, implemented as new, harness-neutral
    runtime/ingress plumbing (not a Pi-specific route), reusing
    `WorktreeLock`/`ExternalTaintMarker`/`database_failure`/`recover` unchanged
    with zero `protocol.rs`/Quint edit:
    * The "acquire a `ProtectedWorktree` and report its armed state without
      processing a boundary" half of the refactor needed **no code change**:
      `ProtectedWorktree::acquire` already does exactly this (returns the guard
      synchronously, with the lock held and the marker durably armed, before
      any boundary work). The only actual refactor is a new `pub(super)`
      `coordinate_on_held_worktree` in `coordinator.rs` — a thin wrapper around
      the existing private `coordinate_protected` — letting a caller that
      already holds a `ProtectedWorktree` (so cannot safely re-enter
      `coordinate()`/`coordinate_inner`, which acquire their own lock and would
      deadlock against the one already held) force the existing
      `database_failure`+`recover` composition against a freshly observed tree
      by passing `force_recovery: true` (reusing the exact mechanism
      `inherited_external_taint` already drives), then the caller itself calls
      the pre-existing `ProtectedWorktree::complete()` only on a durable commit.
      `WorktreeLock::as_raw_fd`/`ProtectedWorktree::lock_raw_fd` (both
      `#[cfg(unix)]`) expose the lock's raw fd for duplication.
    * New `cli/src/services/mutation_trace/runtime/external_mutation_guard.rs`
      (`run_external_mutation_guard`, Unix-only — `#[cfg(not(unix))]` returns
      `GuardError::UnsupportedPlatform` unconditionally, per D13's Windows
      disposition that T05 refuses `user_bash` guard-establishment outright
      there, with no Windows-specific guard code written here): acquires
      `ProtectedWorktree`, creates/configures the lifetime token, and only then
      emits `GuardEvent::Armed`; a lifetime-token establishment failure emits
      no event and spawns no shell. It then spawns the human shell using the
      pinned Pi-compatible shell contract as its own child (`process_group(0)`,
      its own process group so process-group signaling never reaches the
      supervisor itself), streams stdout/stderr back through a channel-fed
      callback, accepts an explicit cancel signal (a caller-supplied
      `mpsc::Receiver<()>` — **never** channel-close/disconnect, which the
      finish loop explicitly ignores), and enacts it by sending `SIGTERM` to
      the shell's process group via a minimal local `dup`/`kill` FFI shim (no
      new Cargo dependency — both are simple, already-linked libc symbols).
      Normal finish requires the shell's own `wait()` **and** lifetime-token
      EOF, never control-channel/cancel-channel state alone. Before spawn,
      ordinary RAII cleanup is safe. After successful spawn and before
      lifetime EOF, all supervision errors and panic-adjacent unwinds use a
      consuming abandonment path that leaves the marker armed and closes only
      the supervisor's lock reference without explicit `flock(LOCK_UN)`; the
      inherited shell/descendant descriptor keeps the flock kernel-held. Once
      both completion conditions hold, it forces recovery through
      `coordinate_on_held_worktree(..., force_recovery: true)` and calls
      `ProtectedWorktree::complete()` only on that durable commit. A failed
      final recovery leaves the marker armed; because lifetime EOF was already
      proven, ordinary unlock is safe in that case. **On Unix, the fd
      duplication itself happens in the *parent*, immediately before
      `Command::spawn()`** (a plain `dup()` on the lock fd, whose result is
      CLOEXEC-clear by POSIX default with no further flag-clearing needed),
      relying on `fork()`'s atomic, synchronous fd-table copy so the child is
      guaranteed to hold its own independent reference to the lock's open file
      description by the moment `spawn()` returns — the parent's own duplicate
      is then closed via `File::from_raw_fd` + `drop`, leaving the child's copy,
      and only the child's copy, alive.

    **Two genuine correctness bugs found and fixed while writing and running
    the tests below (not merely by inspection) — both exactly the class of
    subtle Unix-semantics defect D13's own plan-text history warns about:**
    (1) an earlier draft duplicated the fd inside a `pre_exec` closure
    (child-side, strictly after `fork()`); since `pre_exec` runs
    *asynchronously* relative to the parent's `spawn()` returning, this opened
    a real race where the parent could close its own reference before the
    child had actually duplicated its own — during that window the kernel
    would see zero referencing descriptors and release the flock early. Fixed
    by moving the `dup()` into the parent, before `spawn()`, as described
    above — `fork()` is atomic, so there is no such window. (2)
    `WorktreeLock::drop` calls `File::unlock()` — an *explicit*
    `flock(fd, LOCK_UN)` — which releases the lock for **every** descriptor
    sharing that open file description immediately, not only when the last
    referencing descriptor closes; this never affects the supervisor's own
    graceful finish path (the guarded shell has already exited, and its own fd
    already closed, before `protected` is ever consumed by `complete()`), but
    it means simulating "the supervisor is killed" in a test via a plain Rust
    `drop(protected)` is **unfaithful** — a real `kill -9` never runs Rust
    destructors, so `unlock()` would never explicitly execute; only the
    kernel's implicit "close this fd" teardown would run. The test was
    corrected to reproduce that exact difference (`std::mem::forget` the guard
    plus a raw `close()` on only the *original* fd, leaving the child's
    inherited duplicate as the sole remaining reference) — see
    `a_supervisor_killed_without_unlocking_leaves_the_flock_held_by_the_spawned_shell`.
    Focused tests in `external_mutation_guard.rs` (6, all passing) cover: a
    foreign `WorktreeLock::acquire` times out while the guard is active; the
    spawned shell's own `$PPID` (read from inside the shell itself, avoiding
    any post-exit `/proc` race) equals the supervisor process's pid; the
    kill-9-simulated case above; closing the cancel channel (sender dropped)
    triggers neither finish nor a signal to the shell; a cancel request sent
    mid-run reaches the shell's process group (proved via a `trap 'exit 9'
    TERM` shell script) and finish still waits for the shell's real exit
    rather than firing immediately; and a failed finish commit (injected
    DB-open failure) leaves the external-taint marker armed and returns
    `GuardError::Finish` without calling `complete()`.

    **CLI wiring — a recorded deviation from the plan text's literal
    phrasing, using the "not frozen here" latitude it explicitly grants.** The
    plan describes the new operation as "one new long-lived sibling of the
    existing one-shot start/advance/close/flush/abandon operations in
    `cli/src/services/hooks/mutation_scope.rs`." The five existing operations
    share one JSON-`operation`-dispatched, single-shot shape: `read_hook_stdin()`
    blocks until STDIN reaches EOF, then the whole payload is parsed and
    exactly one `coordinate()`/`abandon_scope()` call runs, returning one
    string. The guard operation cannot fit that shape: it must read an initial
    JSON run request, then keep STDIN open afterward to receive a *later*,
    optional cancel request while the guarded shell is still running, and
    stream JSON status lines to STDOUT as it goes — genuinely long-lived,
    bidirectional, incompatible with "read all of STDIN to EOF, then respond
    once." The new `run_external_mutation_guard_subcommand` function still
    lives in `mutation_scope.rs` (satisfying the instruction at the file
    level) and uses an explicit two-phase operation-tagged JSON protocol:
    exactly `{"operation":"arm"}` first, then (only after the flushed
    `{"status":"armed"}` acknowledgement) exactly one
    `{"operation":"exec","command":"<string>","cwd":"...","env":{...}}`
    frame. `{"operation":"cancel"}` is accepted while waiting for exec and
    after spawn; it means pre-spawn termination before spawn and process-group
    cancellation after spawn. The hidden CLI verb is `sce hooks
    external-mutation-guard` (`cli_schema.rs`/`command_runtime.rs`/`hooks/mod.rs`
    additions mirroring the existing per-adapter hidden routes), rather than
    the shared `MutationScopePayload` enum/`sce hooks mutation-scope` verb.
    STDOUT emits line-delimited JSON: `{"status":"armed"}`, `{"stream":
    "stdout"|"stderr","data":"<chunk>"}` (lossy UTF-8 — no `base64` crate is
    present in this workspace and the Done-when criteria concern lock/fd/process
    semantics, not byte-fidelity of streamed output; recorded as an explicit
    simplification for T05 to revisit if binary-safe streaming is later
    required), and a final `{"status":"result","exit_code":<n-or-null>}`.
    The explicit state machine is `Starting -> ArmedWaitingForExec -> Running
    -> Finished`; no arm acknowledgement can itself spawn a shell. Focused
    tests in `mutation_scope.rs` cover strict arm/exec/cancel parsing and event
    serialization.

    **Assumptions carried forward for T04/T05:** (a) the admission
    "uncertain-attempt" scoping departure above (`PendingAbandon`/non-`Clear`
    recovery only, never a sibling's `PendingStart`) is load-bearing for D12
    and should inform how T04 detects a genuinely orphaned `PendingStart` (a
    crashed `establish_tracked_start`, not a live in-flight tool call) — this
    task deliberately leaves that cross-invocation detection to T04, per its
    own stated scope. (b) **superseded by the 2026-09-17 repair pass below**
    — the guarded shell was originally spawned via a hardcoded `/bin/sh -c
    <command>` without checking pinned Pi source; T03's own scope text asked
    this task to "document exactly which parts of [Pi's local-shell]
    contract it reproduces and cite the pinned package's own local-execution
    behavior as the reference being matched," and that citation work was not
    done. The repair pass closed it directly against pinned
    `@earendil-works/pi-coding-agent@0.80.6`'s `dist/core/tools/bash.js`
    (`createLocalBashOperations`) and its `dist/utils/shell.js`/
    `dist/utils/child-process.js` helpers: the assumed `/bin/sh` contract was
    wrong (Pi prefers real `/bin/bash`, then `bash` on `PATH`, only then
    plain `sh`), and the guard now reproduces that exact resolution order.
    Full contract citations, and the deliberate differences kept as-is
    (env-merge vs. env-replace; `SIGTERM` vs. `SIGKILL` cancellation), live in
    `context/cli/mutation-trace-external-mutation-guard.md` rather than being
    duplicated here.
  - Verify outcome: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path
    cli/Cargo.toml pi_mutation_scope` passed (59/59). `nix develop -c
    ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml hooks::`
    passed (691/691, 1 pre-existing unrelated ignore). Additionally, given the
    shared `mutation_trace::runtime` files touched (`coordinator.rs`,
    `protected_worktree.rs`, `worktree_lock.rs`, `mod.rs`): `nix develop -c
    ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml
    mutation_trace` passed (376/376, up from T02's 370 — the 6 new
    `external_mutation_guard` tests), the full unscoped `cargo test`
    passed (1586/1586, 1 pre-existing unrelated ignore, 0 filtered), and `cargo
    clippy --all-targets -- -D warnings` (with `clippy::pedantic`/`warnings`
    denied workspace-wide, `SCE_CLI_PACKAGE_FALLBACK=1`) passed with zero
    warnings — clippy caught and this task fixed two real pedantic violations
    along the way (`PiHookEvent`'s four variants originally shared a `Tool`
    prefix; a test-local `const` was declared after statements). `spec/mutation_cursor.qnt`
    and `protocol.rs` are confirmed untouched by `git status`.
  - **2026-09-17 repair pass verify outcome:** `nix develop -c
    ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml
    external_mutation_guard` passed (19/19 matched by that name filter — 17
    of those live in `external_mutation_guard.rs` itself, up from 6, the
    other 2 are pre-existing `command_runtime.rs` hidden-route parser tests
    coincidentally matched by the same substring). 11 new tests in
    `external_mutation_guard.rs`: 8 covering the `cwd` contract, 1
    unit-testing the drain-after-join ordering fix directly and
    deterministically (no subprocess timing dependency), and 2 end-to-end
    multi-chunk stdout/stderr regressions. `pi_mutation_scope` re-verified
    unchanged (59/59). `hooks::` passed (695/695, up from 691 — 4 new
    `cwd`-wire-parsing tests in `mutation_scope.rs`'s `guard_protocol`
    module, 1 pre-existing unrelated ignore). `mutation_trace` passed
    (387/387, up from 376 — the 11 new `external_mutation_guard` tests).
    The full unscoped `cargo test` passed (1601/1601, up from 1586, 1
    pre-existing unrelated ignore, 0 filtered). `cargo clippy --all-targets
    -- -D warnings -D clippy::pedantic` (`SCE_CLI_PACKAGE_FALLBACK=1`)
    passed with zero warnings. `git diff --check` passed (no whitespace
    errors). `nix flake check` — not run by T03's original verification —
    was run for this repair pass and reported **all checks passed**,
    including `cli-fmt`; that check first failed against whitespace-only
    drift already present in `pi_mutation_scope/{mod.rs,state.rs}` before
    this repair began (confirmed via `git diff` — none of the flagged lines
    were touched by this repair's own changes), closed by running the
    already-sanctioned `cargo fmt` autofix (AGENTS.md), not by editing test
    assertions or behavior.
  - **2026-09-17 lifetime-token repair verify outcome:** `nix develop -c
    ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml
    pi_mutation_scope` passed (59/59); `hooks::` passed (695/695, 1 ignored);
    `mutation_trace` passed (388/388); the focused external-mutation-guard
    suite passed (20/20). `cargo clippy --all-targets -- -D warnings -D
    clippy::pedantic` passed, `nix flake check` passed all checks, and
    `git diff --check` passed. The graceful-descendant and output/lifetime
    regressions pass alongside the existing supervisor-death regression.
  - **2026-09-17 lifetime-token repair:** added a CLOEXEC-explicit Unix
    lifetime pipe, positive shell-termination-plus-token-EOF completion, and
    continuous bounded output polling. Added deterministic regressions for a
    graceful background descendant and output while its lifetime token is
    held. The real WorktreeLock and marker remain active until durable
    recovery succeeds; D14's deliberate-close escape remains documented.
  - **2026-09-17 post-spawn failure repair:** added explicit guard ownership
    states for pre-spawn, post-spawn-before-lifetime-EOF, and completed-lifetime
    cleanup. `ProtectedWorktree::abandon_after_spawn_without_unlock` consumes
    the supervisor's lock reference without explicit `LOCK_UN`; normal
    `WorktreeLock` drop behavior is unchanged for all existing callers. The
    `Armed` event now follows lifetime-token establishment. Deterministic
    regressions cover lifetime-establishment failure, injected post-spawn
    observation failure with a live background descendant, and the same
    failure after a shell with no descendants. They assert inherited lock
    exclusion, armed-marker persistence, and the next-boundary inherited-taint
    recovery. Focused external-guard tests passed 23/23; `mutation_trace`
    passed 391/391; the requested `pi_mutation_scope` and `hooks::` suites
    remained green at 59/59 and 695/695. `nix flake check` passed all checks,
    including clippy, format, CLI tests, and Quint-connect; `git diff --check`
    passed. T03 remains `done`; T04 remains `todo` and was not started.
  - **2026-09-17 two-phase admission repair:** replaced the misleading
    single-phase `guard` request carrying command data with
    `arm -> flushed Armed -> explicit exec` and made the runtime expose an
    `ArmedExternalMutationGuard` handle. Arm acquires the worktree and marker,
    establishes the lifetime token, and waits without a shell. Exec performs
    request-specific cwd validation and is the only path that calls
    `spawn_guarded_shell`; a lost Armed write/flush, pre-exec EOF, cancellation,
    malformed/unknown/duplicate exec, blank command, or invalid cwd cannot
    spawn. The pre-spawn handle retains the token writer and ordinary RAII lock
    release; successful spawn still enters the existing descendant-lifetime and
    no-`LOCK_UN` abandonment states. Added deterministic regressions for lost
    Armed delivery (including an absent filesystem side effect), arm-without-
    exec plus inherited-taint self-heal, and the two-phase happy path proving
    no side effect before exec; protocol regressions cover strict arm/exec,
    cwd/env, cancellation, malformed, unknown, blank, and extra-field frames.
    `T03` status remains `done`; `T04` and `T05` remain `todo` and were not
    started.
  - Context impact: durable-context classification `pending-review` — this
    task adds a new adapter directory (`pi_mutation_scope/`) alongside the
    existing Claude/Codex/OpenCode ones and a new generic
    `external_mutation_guard` runtime primitive plus a new hidden CLI route,
    but makes no protocol/Quint semantic change (D13 required none, confirmed
    above) and asserts no new fact about Pi's confirmation-required status
    (already recorded by T02) or the mutation-scope protocol's own shape. The
    Task context synchronization phase should confirm whether
    `architecture.md`/`context-map.md` enumerate the concrete adapter set or
    the external-mutation-supervisor mechanism closely enough that this task's
    additions are a correction rather than an unremarkable extension, and
    record any residual impact.
  - **2026-09-17 repair pass context impact:** resolved. The canonical
    `context/cli/mutation-trace-external-mutation-guard.md` now carries an
    "Execution cwd contract" section and an "Exact pinned Pi `0.80.6`
    local-shell contract" section (shell resolution, command transport,
    cwd, stdio, process-group, exit-code, and final-output-draining
    parity, plus the two deliberate documented differences: env-merge and
    `SIGTERM` cancellation) with exact pinned-source file/line citations,
    and its Lifecycle diagram now names continuous output polling during the
    descendant-lifetime wait plus bounded finalization.
    The `2026-09-17-external-mutation-guard-process-supervisor` ADR's
    Follow-up section is updated to mark the shell-contract confirmation
    resolved (its Decision/Rationale/Alternatives/Consequences are
    historical and were left untouched). `architecture.md`, `context-map.md`,
    `glossary.md`, `overview.md`, and `pi-mutation-scope-integration.md`
    already described the guard only at the same high level this repair
    preserved (spawns the human shell as its own child, harness-neutral,
    Unix-only, unwired) with no incorrect specifics to correct, so per "keep
    one canonical explanation and link to it," they are left as their
    existing links to the guard doc rather than duplicating the new detail.
  - Context synchronization: synced

- [x] T04: `Add sound terminal and stale-process recovery` (status:done)
  - Task ID: T04
  - Scope: In — the conservative recovery obligations frozen by T01: Start
    committed then execution later blocked (`tool_execution_end` with no
    preceding `tool_result`, D7) => abandon/rebaseline, never Close; Start
    committed then process dies before execution; execution begins then
    process dies before terminal event; Close/abandon seam failure;
    first-ambiguity-Flush and rebaseline-Flush failure; duplicate/late terminal
    events; adapter crash during recovery; multiple live Pi siblings; a Pi
    sibling plus another harness. Durable terminal intent (mark -> Flush
    ambiguous interval -> abandon -> remove only after abandon succeeds ->
    Flush rebaseline -> clear recovery), with a failed step leaving recovery
    pending and a new tracked Start remaining fail-closed until pending
    recovery resolves. Positive process-staleness handling based only on exact
    process-death evidence from the frozen Pi lifecycle — no TTL, no broad
    session sweep, no same-session predecessor sweep.

    Note on scope narrowing: D13's guard blocks `user_bash` outright whenever
    it cannot be durably established (see D13), so there is no longer a human
    mutation from a *failed guard-establishment* attempt for recovery to
    protect against, and no in-process "withheld Close" state for this task to
    reconcile against. This task does not need — and must not add — recovery
    cases whose only purpose was handling "`user_bash` executed after an
    initial guard-establishment failure," because that execution is now
    forbidden by construction at the `user_bash` handler itself (T05).

    What this task owns, explicitly, is the guard's own lifecycle recovery —
    the property the earlier one-shot fence could not provide — plus the
    *successful*-guard cross-process case:

    * **Supervisor dies mid-command while the shell keeps running
      (Unix).** The long-lived supervisor process (T03) is `SIGKILL`ed/crashes
      while the shell it spawned is still running. Assert `WorktreeLock` is
      **not** released while the shell (or a descendant holding a duplicate of
      the lock's inherited file descriptor) remains alive — a fresh
      `coordinate()` call from any harness still blocks up to the existing
      timeout and then fails closed with `CoordinateError::LockAcquisition`,
      exactly as if the supervisor were alive. Only once the shell (and every
      fd-holding descendant) exits does the OS release the flock; assert that
      release, not the supervisor's death, is what a fresh `coordinate()` call
      actually waits on. Once released, assert the next `coordinate()` call on
      that worktree — from any harness — observes `ExternalTaintMarker` still
      armed (nobody ran the supervisor's finish sequence) and runs the
      existing unmodified inherited-taint `database_failure`+`recover` path
      before processing its own boundary, exactly as for any other unresolved
      marker. Prove no PID/timestamp/TTL check is involved anywhere in this
      sequence: staleness is proven solely by the lock eventually becoming
      free, and the lock's freedom is itself gated on the shell's own exit,
      not the supervisor's.
    * **Pi/Node control process dies mid-command (chosen Option A).** The
      caller's control-channel process dies (or the connection otherwise
      closes) while the supervisor is still waiting on the shell. Assert the
      supervisor does not treat this as a finish signal: it does not signal
      the shell, does not run the forced recovery sequence, and continues
      holding `WorktreeLock`/`ExternalTaintMarker` exactly as before. Assert
      that when the shell later terminates on its own, the supervisor still
      runs its normal finish sequence (forced recover, `complete()`, release)
      even though nothing is listening on the dead control channel, and that
      the worktree ends up in the same correct state as if Pi/Node had stayed
      alive the whole time.
    * **Guard finalization fails.** The finish-time forced
      `database_failure`+`recover` commit does not durably succeed. Assert
      `ProtectedWorktree::complete()` is never called, the marker remains
      armed, the supervisor reports failure (never success), and the next
      `coordinate()` call on that worktree self-heals via the existing
      inherited-taint path.
    * **Foreign boundary arrives while the guard is active.** A
      Start/Advance/Close/Flush from another live scope (same harness or a
      different one) is attempted while the guard still holds `WorktreeLock`
      (whether the supervisor, the shell via its inherited descriptor, or both
      currently hold it). Assert it blocks up to the existing lock-acquisition
      timeout and then fails closed with `CoordinateError::LockAcquisition`,
      touching no protocol state (no read, no taint, no clear); assert the
      adapter that issued it applies its own existing conservative handling
      for that failure (fail-closed block-the-tool for a Start, D9's existing
      unresolved-terminal retry-later pattern for a Close/Advance).
    * **Foreign process crashes while waiting** on
      `CoordinateError::LockAcquisition` (or otherwise mid-retry). Assert this
      leaves no durable state behind for the guard to reconcile against — the
      foreign process never reached a point where it could mutate protocol
      state, so there is nothing to recover for its sake specifically; the
      guard's own finish sequence still runs normally when the shell ends.
    * **Stale guard recovered from positive shell-death evidence** — the
      same "supervisor dies mid-command" case, explicitly reframed as the
      generic-adapter reconciliation obligation: prove the Pi adapter's own
      local attempt-state (for any Pi scope live during the guarded interval)
      correctly reconciles with a scope the generic runtime abandoned out
      from under it on the adapter's next interaction, the same obligation
      existing adapters already have for any externally tainted worktree —
      not a new mechanism D13 introduces.
    * **Multiple AI scopes overlap the guarded interval.** Pi A and at
      least one other-harness scope B are both live when the guard begins;
      assert the finish-time forced recovery abandons both, regardless of
      which one (if either) is the harness that happened to observe
      `user_bash`.

    The critical end-to-end proof, spanning the whole guarded interval, not
    just its endpoints:

    ```text
    guard begins (durably established); supervisor spawns the shell
    human write #1
    a foreign harness's boundary attempts to run
        => blocks, then fails closed (LockAcquisition); no state touched
    human write #2
    the shell itself terminates
        => (if the supervisor is still alive) supervisor observes the
           termination directly via wait() and runs finish immediately;
           (if the supervisor already died) the lock frees only once the
           shell's own fd closes, and the next coordinate() call anywhere
           self-heals the still-armed marker via the existing
           inherited-taint path
        => either way: forced recover/rebaseline; every live worktree
           scope abandoned
    the deferred foreign boundary is retried
        => now succeeds normally against the recovered worktree
    assert: write #1 and write #2 both remain outside positive AI
    attribution for every scope whose lifetime overlapped the guard,
    on every harness involved, regardless of whether the supervisor or
    Pi/Node died at any point during the interval
    ```

    Also the successful-guard cross-process case at the granularity of a
    single write, unchanged in substance from the original text:

    ```text
    Pi A live
    Claude/Codex/OpenCode B live

    guard begins; human mutation occurs; guard ends
        => forced recover before any later boundary is evaluated
        => no human mutation becomes positive AI attribution
    ```

    This is the existing worktree-wide `recover()` behavior (D13/AC18)
    exercised specifically for the case where the triggering boundary belongs
    to a harness other than the one whose scope observed `user_bash`, proving
    the adapter's own local attempt-state reconciles with a scope the generic
    runtime abandoned out from under it — the same reconciliation obligation
    the existing adapters already have for any externally tainted worktree,
    not a new mechanism D13 introduces. Out — any change to the TypeScript
    extension, including the `user_bash` call site itself and supervisor
    process management (T05).
  - Dependencies: T03
  - Done when: no tested crash, rejection, missing terminal event, guard
    lifecycle event, or transient seam failure can turn an uncertain Pi
    interval — or an interval guarded on behalf of any harness — into positive
    AI attribution, while unrelated surviving scopes retain future attribution
    capability; a guard-triggered abandonment is correctly reflected in the
    adapter's own durable attempt state on its next interaction, including
    when the triggering boundary belongs to a different harness than the
    abandoned scope; a foreign boundary that raced an active guard never
    observes or mutates protocol state until the guard ends, and succeeds
    normally once retried afterward; no recovery path exists for a
    `user_bash` execution that a failed guard-establishment attempt should
    have prevented, because D13 forbids that execution at the source; killing
    the supervisor while the shell keeps running never allows the lock to
    free (and therefore never allows a foreign boundary to proceed) before
    the shell itself exits; and killing the Pi/Node control process never
    causes the guard to finish early or signal the shell.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml pi_mutation_scope`;
    `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace`.
  - Completed: 2026-09-17
  - Repaired: 2026-09-17 (same-day repair pass — the original trigger-point
    decision below keyed stale-owner reconciliation solely to an exact
    `(session_id, tool_call_id)` replay match, which a fresh Pi process (a
    new UUIDv7 session per T01) essentially never produces against an older
    dead process's attempt; a dead `Executed` attempt was also not eligible
    at all. Both gaps are closed by this repair; see "2026-09-17 repair
    pass" below. T05 was not started by this repair.)
  - Files changed: `cli/src/services/hooks/pi_mutation_scope/mod.rs`;
    `cli/src/services/hooks/pi_mutation_scope/state.rs`;
    `cli/src/services/hooks/pi_mutation_scope/process_owner.rs` (new)
    (3 files; no other paths touched — `context/plans/pi-mutation-scope-integration.md`
    itself, `protocol.rs`, `spec/mutation_cursor.qnt`, `fixtures/`, and the
    TypeScript extension confirmed untouched).
  - **2026-09-17 repair pass files changed:** `cli/src/services/hooks/pi_mutation_scope/mod.rs`;
    `cli/src/services/hooks/pi_mutation_scope/state.rs`;
    `context/plans/pi-mutation-scope-integration.md`;
    `context/cli/pi-mutation-scope-integration.md`. `process_owner.rs`
    (the process-death primitive itself), `boundary_lock.rs`, `protocol.rs`,
    `spec/mutation_cursor.qnt`, `fixtures/`, and the TypeScript extension
    remain untouched. No T05 work was started.
  - Result: **D10 (process-staleness)** is implemented as a new sibling
    module `process_owner.rs`, mirroring `os_lock.rs`'s single-purpose style
    and reusing the local `unsafe extern "C"` FFI pattern already established
    in `external_mutation_guard.rs` (no new Cargo dependency). `ProcessOwner
    { pid, instance_token }` is captured via `getppid()` at Start-admission
    time — the invoking `sce hooks pi-mutation-scope` process's parent *is*
    the Pi/Node process for that exact synchronous invocation (D3), so this
    needed no wire-protocol or TypeScript-extension change. `is_definitely_dead`
    proves death via `kill(pid, 0)` returning `ESRCH` (Unix), and additionally
    detects PID reuse on Linux by comparing `/proc/<pid>/stat` field 22
    (starttime) against the recorded value; where that instance evidence
    can't be established, a live pid is always conservatively treated as
    alive, per D10's own explicit fallback. No `Instant`/`SystemTime`/TTL is
    used anywhere, enforced structurally by a test that greps the module's
    own production source. `AdapterAttempt` gained an `owner: ProcessOwner`
    field (`ADAPTER_STATE_VERSION` bumped 1→2, rejected fail-closed by the
    pre-existing version gate for any old-schema state file — no real
    production state exists yet since T05 hasn't shipped).

    **Load-bearing trigger-point decision (the part the plan explicitly left
    open) — superseded by the 2026-09-17 repair pass below.** The original
    implementation keyed stale-owner reconciliation to the single-attempt
    lookup-by-incoming-key that `admit_tracked_attempt` already performs for
    D1's replay handling: when an incoming `ToolCall`'s exact `(session_id,
    tool_call_id)` key matched an existing `PendingStart` attempt *and* that
    attempt's recorded owner was positively dead, `admit_tracked_attempt`
    returned an `AdmitDecision::StaleOwnerAbandon { scope_id }` instead of an
    ordinary replay. This was recorded as load-bearing at the time precisely
    *because* its own reachability was narrow — a Pi `session_id` is a fresh
    UUIDv7 per process (T01's NOTES.md), so the same session_id recurring
    after its owning process died is not how Pi's ordinary lifecycle
    behaves — and that self-reported narrowness is exactly what the repair
    below corrects: the realistic crash lifecycle T04's own Done-when names
    ("Start committed → owning Pi process dies before execution", and
    "execution began → tool_result observed → owning Pi process dies before
    tool_execution_end") produces a *different* process's *different*
    session driving the next `tool_call`, which the exact-key trigger could
    never observe. A dead `Executed` attempt (the second crash shape) was
    also never eligible for this trigger at all, since D1's replay lookup
    only special-cased `PendingStart`. Both gaps left a stale scope live
    indefinitely until an exact-key replay that, by T01's own UUIDv7 design,
    essentially never occurs.

    **2026-09-17 repair pass.** Stale-owner reconciliation is now a new
    `reconcile_stale_owners` step in `mod.rs`'s `admit_or_recover`, run on
    *every* tracked Start admission before the incoming key is looked up —
    not folded into `attempt.matches_key(incoming_key)` at all. It repeatedly
    calls a new read-only `state::find_definitely_dead_attempts`, which scans
    every persisted attempt (any session, any prior process) and returns the
    `scope_id`s of exactly those in a `PendingStart` or `Executed` phase whose
    own recorded `ProcessOwner` satisfies `is_definitely_dead` — `PendingAbandon`
    is never included, since it already carries durable terminal recovery
    intent owned by the pre-existing D8 pending-recovery-resume path
    (`admit_tracked_attempt`'s existing `RecoveryState::Pending` →
    `FlushClaimed` branch continues to own resuming an interrupted recovery
    generation unchanged). Every scope discovered in one scan is retired
    together — `begin_terminal_cleanup` on the whole batch, then the
    existing, unmodified `resolve_recovery` (one ambiguity flush, one
    `abandon` per doomed scope, one rebaseline flush) — before the loop
    rescans and, finding nothing left, falls through to ordinary
    incoming-key admission; a mid-sequence failure leaves `RecoveryState`
    durably `Pending` and denies the triggering Start, exactly as the
    pre-existing D8 machinery already guarantees for any other recovery
    generation. `AdmitDecision::StaleOwnerAbandon` and
    `admit_tracked_attempt`'s narrow exact-key dead-owner special case are
    removed outright: by the time `admit_tracked_attempt` runs, any dead
    attempt matching the incoming key has already been retired by the broad
    scan, so an exact-key match remaining there is, by construction, either
    live or uncertain — an ordinary replay, never a stale-owner case. This is
    still not TTL/age/session/`ActorKind` sweeping: each candidate is
    filtered independently by its own `is_definitely_dead(&attempt.owner)`
    result, computed by the same, unmodified `process_owner.rs` primitive
    T04 originally shipped (D10, untouched — no live-but-uncertain owner is
    ever treated as dead). `is_definitely_dead` and the primitive itself are
    unchanged and were not touched by this repair. Six new
    `lifecycle_tests` regressions cover the corrected rule end to end:
    `a_dead_pending_start_attempt_is_recovered_by_an_unrelated_fresh_session_start`,
    `a_dead_executed_attempt_is_recovered_by_a_fresh_session_start_without_a_synthetic_close`
    (asserts no `close` op is ever emitted for a dead `Executed` attempt, per
    D9), `a_dead_owner_scope_is_recovered_while_a_live_owner_sibling_survives_untouched`,
    `multiple_dead_owner_scopes_are_retired_in_one_recovery_generation_while_a_live_sibling_survives`
    (two independently dead-owned scopes retired in one `flush`/`abandon`/
    `abandon`/`flush` generation, a live third scope untouched),
    `an_owner_that_cannot_be_positively_proven_dead_is_never_abandoned_by_an_unrelated_start`
    (a live pid with no recorded instance token), and
    `an_interrupted_stale_owner_recovery_remains_pending_and_denies_the_triggering_start_until_resumed`
    (a one-shot seam failure on the reconciliation's own `abandon` step
    leaves recovery `Pending` and denies the triggering Start; the next
    invocation resumes and completes it, then admits). The pre-existing
    `a_pending_start_attempt_owned_by_a_dead_process_is_abandoned_not_replayed`
    exact-key regression is unchanged and still passes: the broad scan
    subsumes the exact-key case, producing the identical seam-operation
    sequence (`start, flush, abandon, flush, start`).

    **Adapter/guard reconciliation** (new tests only; zero production changes
    needed — confirmed by inspection that `handle_tool_execution_end`'s
    existing Close-failure→abandon fallback, first proven in T03's
    `a_failed_close_falls_back_to_abandon_recovery`, already handles a scope
    the generic runtime abandoned out from under the adapter, whatever caused
    that abandonment). Three new tests in a new `guard_reconciliation_tests`
    module (`#[cfg(all(unix, test))]`) combine a live Pi scope (via the real
    `hooks::mutation_scope` ingress seam against a real Git repo + Agent Trace
    DB) with `run_external_mutation_guard`:
    `a_guard_triggered_worktree_abandonment_reconciles_with_the_pi_adapters_own_state`
    (two live Pi scopes overlap a guarded interval; the guard's finish-time
    forced recovery abandons both; the adapter's own local JSON state
    converges to empty once it observes each scope's terminal event);
    `a_guard_abandons_a_live_pi_scope_alongside_a_live_scope_from_another_harness`
    (a live Pi scope plus a live `ActorKind::ClaudeCode` scope both overlap
    the guard and are both abandoned; the Pi adapter still reconciles
    cleanly); `a_foreign_pi_start_racing_an_active_guard_fails_closed_touching_no_state_then_succeeds_on_retry`
    (a fresh Pi `ToolCall` racing an active guard blocks on the real
    `WORKTREE_LOCK_TIMEOUT`, fails closed with `FAIL_CLOSED_MESSAGE` surfaced
    from `CoordinateError::LockAcquisition`, leaves no scope row in the DB,
    then succeeds normally once the guard releases).

    **Remaining D7/D8 gaps** (new tests only, satisfied by already-existing
    T03 production code): `duplicate_tool_result_after_close_is_a_safe_no_op`,
    `duplicate_tool_execution_end_after_abandon_is_a_safe_no_op`,
    `abandoning_one_sibling_never_touches_a_concurrent_sibling_in_the_same_session`,
    `a_crash_mid_abandon_loop_is_resumed_and_completed_on_the_next_boundary_lock_acquisition`
    (a transient one-shot seam failure on the "abandon" step simulates a crash
    between durable steps, proving `RecoveryState::Pending` correctly resumes
    the sequence on the next invocation — the same pattern T03 already proved
    for the "flush" step).

    **Deliberate, honestly-reported scope narrowing.** T04's Done-when also
    names "killing the supervisor while the shell keeps running never frees
    the lock before the shell exits" and "killing the Pi/Node control process
    never finishes the guard early," from the Pi-adapter's own angle. No
    literal SIGKILL-the-supervisor-with-a-live-Pi-scope test was added,
    because: (a) that requires the `GuardTestHooks`/`run_external_mutation_guard_with_hooks`
    seam T03 deliberately kept module-private to `external_mutation_guard.rs`,
    and widening that visibility is beyond this task's scope; (b) the Pi
    adapter's JSON state and the guard's `WorktreeLock`/DB state are
    structurally independent, and the adapter can only ever observe the
    *outcome* (a scope transitioning to `abandoned` via forced recovery) —
    byte-for-byte identical in the DB whether the guard finished cleanly or
    self-healed after a supervisor kill, since both paths run the exact same
    `database_failure`+`recover` composition. The
    `a_guard_triggered_worktree_abandonment_reconciles_with_the_pi_adapters_own_state`
    test already exercises the adapter's reaction to that outcome; the
    supervisor-kill/control-death mechanics themselves remain covered,
    unchanged, by T03's own `external_mutation_guard.rs` tests. This
    Done-when item is satisfied substantively, not via a literal duplicate
    test — flagged explicitly rather than silently assumed.
  - Verify outcome: `pi_mutation_scope` filter: 76/76 passed (up from T03's
    59 — 8 new `process_owner` unit tests, 6 new `lifecycle_tests`, 3 new
    `guard_reconciliation_tests`; independently reproduced). `mutation_trace`
    filter: 396/396 passed (unchanged by this task's tests; independently
    reproduced). Full unscoped `cargo test`: 1624/1624 passed, 1 pre-existing
    unrelated ignore. `cargo clippy --all-targets -- -D warnings -D
    clippy::pedantic` (`SCE_CLI_PACKAGE_FALLBACK=1`): zero warnings
    (independently reproduced); fixed 4 `clippy::cast_possible_wrap`
    pedantic violations on `std::process::id() as i32` via `.cast_signed()`.
    `cargo fmt -- --check`: clean (independently reproduced). `git diff
    --check`: clean (independently reproduced). `nix flake check` was not
    run — this task touches only Rust adapter internals with no CLI
    schema/hidden-route/Quint/TS surface change (unlike T03), so the full
    clippy+fmt+full-test matrix above was judged sufficient.
  - **2026-09-17 repair pass verify outcome:** `pi_mutation_scope` filter:
    82/82 passed (up from 76 — 6 new `lifecycle_tests` regressions above;
    `hooks::`/tests unaffected). `mutation_trace` filter: 396/396 passed
    (unchanged). `hooks::` filter: 715/715 passed, 1 pre-existing unrelated
    ignore (unchanged from before this repair). Full unscoped `cargo test`:
    1630/1630 passed (up from 1624 by exactly the 6 new tests), 1
    pre-existing unrelated ignore. `cargo clippy --all-targets -- -D
    warnings -D clippy::pedantic` (`SCE_CLI_PACKAGE_FALLBACK=1`): zero
    warnings (one `clippy::needless_continue` pedantic violation surfaced
    and was fixed during this repair by replacing a loop `continue` arm with
    an `if`/`matches!` early-return). `cargo fmt -- --check`: clean.
    `git diff --check`: clean. `nix flake check` was not run for this
    repair, for the same reason T04's original pass gave: this repair
    touches only the same two Rust adapter files with no CLI
    schema/hidden-route/Quint/TS surface change, so the clippy+fmt+full-test
    matrix above was judged sufficient.
  - Context impact: durable-context classification `no-change` — no new
    adapter directory, generic runtime primitive, or hidden CLI route was
    added, and no protocol/Quint change was made. This task only extended
    the already-documented `pi_mutation_scope` adapter's internal recovery
    logic (a new private sibling module, a new attempt field, a new
    `AdmitDecision` variant) and added regression tests combining
    already-documented, already-covered mechanisms (the Pi adapter and the
    external-mutation guard, both already named in
    `context/cli/mutation-trace-external-mutation-guard.md` and the root
    docs at the level T03's own synced pass already settled). None of the
    five root context files or the guard doc contain incorrect specifics
    this task's changes would contradict. The Task context synchronization
    phase should confirm this classification.
  - **2026-09-17 repair pass context impact:** still `no-change` at the
    five-root-context-file level (still no new adapter directory, runtime
    primitive, hidden CLI route, or protocol/Quint change). The corrected
    trigger-point behavior was, however, wrong to leave undocumented in
    `context/cli/pi-mutation-scope-integration.md`'s own "Stale-process
    recovery (D10)" section, which previously described the exact-key
    trigger as the mechanism without flagging it as insufficient — that
    section is rewritten by this repair pass to describe the broad,
    per-attempt reconciliation scan instead. `AdmitDecision::StaleOwnerAbandon`
    is removed (no longer produced); nothing outside `pi_mutation_scope`
    referenced it.
  - Context synchronization: synced

- [x] T05: `Wire mutation scope into the existing Pi extension` (status:done, completed 2026-09-17)
  - Task ID: T05
  - Scope: In — modifying the canonical Pi extension source
    `config/lib/pi-plugin/sce-pi-extension.ts` (not a second project-local SCE
    extension) to register mutation lifecycle handlers in the frozen T01 order
    (bash policy -> mutation Start -> edit/write diff pre-image) plus the
    frozen execution/terminal lifecycle handlers, forwarding `tool_call`
    (Start), `tool_result` (execution evidence), and `tool_execution_end`
    (Close, gated on a prior `tool_result`) — `tool_execution_start` may still
    be forwarded for telemetry but participates in no attribution state
    (D5/D6/D7). Start, Executed, and Close are established purely from SCE
    actually receiving these Pi events for a given `tool_call`; nothing here
    depends on, checks, or gates on SCE's position in Pi's registered
    extension array — Pi calls every registered extension's `tool_call`
    handler in registration order (D5), so a later extension cannot prevent
    SCE's own handler from running.

    **D9 — terminal transport failure denies subsequent tracked Starts and
    never becomes a delayed Close.** Once a tracked tool has executed, losing
    communication with the Rust adapter must not let later tracked work
    proceed as though the previous terminal state were known:

    ```text
    execution happened
        ↓
    terminal mutation-scope transport (tool_execution_end) fails
        ↓
    that exact attempt becomes unresolved in-process
        ↓
    later tracked-tool Starts are denied (fail-closed) while unresolved
        ↓
    once adapter communication becomes available again, recover the old
    scope through abandon/rebaseline — never a delayed Close as though the
    tool finished just now
    ```

    `tool_result` forwarding is never retried: a `tool_execution_end` that
    later reaches the adapter with no recorded `tool_result` is already
    correctly abandoned by the adapter's own D7 pairing rule, so this case is
    sound with no new wire surface. `tool_execution_end` forwarding failure is
    what this requirement protects: it is retried with backoff, and every
    tracked-tool `tool_call` Start is denied while any `tool_execution_end`
    delivery remains outstanding for this process.

    **Known residual case requiring a narrow Rust adapter addition.** When
    `tool_result` already reached the adapter (the attempt is `Executed`)
    before `tool_execution_end` transport failed, a later retried delivery of
    the same `tool_execution_end` is wire-indistinguishable from an on-time
    Close — the existing four `hook_event_name` values (`ToolExecutionStart`,
    `ToolCall`, `ToolResult`, `ToolExecutionEnd`) give the adapter no way to
    know the terminal event arrived late. Closing this exactly requires the
    adapter to accept an explicit, attempt-scoped abandon request from the
    extension — a fifth `hook_event_name` (e.g. `ToolExecutionAbandon`,
    carrying the same `session_id`/`tool_call_id`/`cwd` identity as the other
    events) that retires the named attempt through the existing D7/D8
    abandon/rebaseline pipeline regardless of whether its owning process is
    still alive, rather than through D10's positive-process-death path. This
    is new Rust surface T05 must add
    (`cli/src/services/hooks/pi_mutation_scope/mod.rs`: parsing, a
    `PiHookEvent::ExecutionAbandon` variant, and dispatch into the existing
    abandon pipeline) — it does not exist today, and T05 is not
    TypeScript-only merely because the rest of the wiring is. If this
    sub-case is not closed, T05 must say so explicitly in its completion
    record rather than describe D9 as fully sound.

    **Closed by the T05 repair pass (2026-09-17).** `PiHookEvent::ExecutionAbandon`
    exists in `mod.rs` exactly as specified above, dispatches into the
    existing D7/D8 abandon/rebaseline pipeline regardless of the attempt's
    `PendingStart`/`Executed` phase, and never produces a Close. The
    TypeScript extension's terminal-delivery tracker
    (`config/lib/pi-plugin/sce-pi-extension.ts`) closes the wire-level
    ambiguity this sub-case describes structurally, not merely by adding the
    Rust route: it keys every in-flight attempt by `(session_id,
    tool_call_id)` and always awaits that exact attempt's own `tool_result`
    delivery outcome before choosing what to send for
    `tool_execution_end` — so `tool_execution_end` can never reach the
    subprocess boundary before its own `tool_result`, and Rust never has to
    disambiguate an on-time Close from a late retried one. When `tool_result`
    delivery is known to have failed, or when an already-successful attempt's
    `tool_execution_end` delivery itself fails, the extension marks that
    exact attempt unresolved, denies further tracked Starts while unresolved,
    and sends `ToolExecutionAbandon` (retried with backoff) instead of a
    (possibly stale) `ToolExecutionEnd`. This sub-case is fully closed, not an
    open residual case.

    Also registers a `pi.on("user_bash", ...)` handler using Pi's **actual**
    pinned `0.80.6` API — established directly from
    `config/lib/node_modules/@earendil-works/pi-coding-agent/dist/core/extensions/types.d.ts`
    and the real consumption logic in
    `dist/modes/interactive/interactive-mode.js`'s `handleBashCommand()`
    (D13), not the `tool_call`-only `{ block, reason }` shape:

    ```ts
    interface UserBashEventResult {
        operations?: BashOperations;
        result?: BashResult;
    }
    ```

    There is no `block`/`reason` member on this result. Pi's own dispatch
    (`handleBashCommand()`) treats a truthy `result` as a full replacement —
    it never calls `session.executeBash()` at all in that case — and
    otherwise passes `operations` through to
    `session.executeBash(command, onChunk, { excludeFromContext, operations })`.
    Under D13's supervisor architecture, `operations` is **never**
    `createLocalBashOperations()` or a wrapper around it — the real shell must
    be spawned by the supervisor (T03), not by Pi/Node, so `wrappedOperations`
    is a thin control-channel client:

    ```text
    pi.on("user_bash", async (event) => {
        if process.platform === "win32":
            // D13's Windows disposition — unconditional refusal, not a
            // transient failure. No supervisor is ever spawned on this
            // platform; this is the same shape as the establishment-failure
            // branch below, taken unconditionally, every time.
            return { result: { output: "SCE does not support guarded
                user_bash execution on Windows in this release; run this
                command outside Pi.", exitCode: 1, cancelled: false,
                truncated: false } }

        // Unconditional: SCE guards whatever user_bash event Pi actually
        // dispatches to it. There is no dispatch-position self-check here —
        // D13's accepted, documented limitation is that a competing
        // extension may consume user_bash before Pi ever calls this
        // handler, in which case there is nothing for SCE to gate.
        spawn the external-mutation supervisor process (T03) with a
            control channel (piped stdio or an equivalent IPC transport)
        await its "armed" acknowledgement, bounded by an establishment
            timeout (an availability bound on this one attempt, never a
            staleness determination — D13); no shell exists yet in this
            window, so the supervisor can be killed cleanly on failure

        on failure, missing-`sce`, spawn error, or an ambiguous/timed-out
        acknowledgement:
            terminate the spawned supervisor process so a lock it may
                already hold is not left orphaned (D13 crash semantics;
                safe here because no shell has been spawned yet)
            return { result: { output: "<reason SCE could not establish
                the worktree external-mutation guard>", exitCode: 1,
                cancelled: false, truncated: false } }
            // Pi never calls session.executeBash(); no shell — supervised
            // or otherwise — is ever invoked.

        on durable "armed" acknowledgement:
            return { operations: wrappedOperations }
            // wrappedOperations.exec(command, cwd, options) does NOT call
            // createLocalBashOperations() or spawn any shell itself. It
            // sends command/cwd/env to the already-armed supervisor over
            // the control channel, relays each streamed output chunk to
            // the caller's onData as it arrives, and forwards
            // signal-driven cancellation and timeout expiry to the
            // supervisor as explicit cancellation requests (the
            // supervisor signals the real shell's process group, since
            // only the supervisor holds its pid). exec() resolves only
            // once the supervisor delivers the shell's real,
            // supervisor-observed exit result (exit code + any final
            // output) — never merely because the control channel closed.
            // If Pi/Node's own process were to die at this point, the
            // supervisor keeps running per D13's chosen Option A; there
            // is nothing left in this process to resolve the promise, and
            // that is an accepted, explicit consequence of Option A, not
            // a bug to route around.
    })
    ```

    **No dispatch-safety self-check, no launcher, no extension-order
    authority.** Earlier drafts of this task specified a `dispatchSafe`
    in-process self-check gating both `user_bash` and tracked-tool Start
    registration, and an SCE-hosted Pi launcher (`sce pi`, or a `PATH`
    wrapper) that would own `ResourceLoader` construction and normalize the
    resolved extension array (`sceEnforceExtensionOrder`) to prove SCE first
    and exactly once. Both are retracted as a deliberate product decision
    (see D13): SCE does not redistribute Pi, embed its SDK, or claim
    authority over Pi's extension ordering. `dispatchSafe`,
    `sceEnforceExtensionOrder`, the launcher host program, and any `sce pi`
    entry point do not exist and must not be (re)built. Tracked-tool
    attribution runs unconditionally whenever SCE's own `tool_call` handler
    is invoked; the `user_bash` guard is established unconditionally whenever
    SCE's own `user_bash` handler is invoked; neither depends on whether SCE
    happens to be first among Pi's registered extensions.

    `sce setup --pi` continues to generate and install
    `.pi/extensions/sce/index.ts` exactly as it does today — no new
    packaging, wrapper, or entry point. `sce doctor`/`sce setup --pi` may
    optionally report, as informational-only diagnostic text, that another
    extension is configured ahead of where SCE would rank under naive
    on-disk resolution; this is not a prerequisite for using the Pi
    integration and must never gate or disable attribution.

    Existing mutation Start ordering remains unchanged. Synchronous
    fail-closed Start transport, D9's terminal-transport handling above, and
    preservation of existing Bash policy, conversation trace, edit/write
    diff trace, message trace, Pi session prefix behavior, and tool-version
    resolution. Using the existing generated Pi extension pipeline
    (`config/lib` / Pkl sources) — no hand-edited generated copies; the
    generated factory function is reused unmodified, exactly as it is today,
    with no second integration path. Also in scope — the narrow Rust
    `ExecutionAbandon` addition described above, if needed to close D9's
    residual case; the unconditional Windows-refusal branch in the
    `user_bash` handler; the external-mutation-guard client (arm, await
    `Armed`, `exec`, cancellation/timeout forwarding, resolve-only-on-the-
    supervisor's-own-result).
    Out — any SCE-hosted Pi launcher, `sce pi` command, or Pi SDK embedding;
    `sceEnforceExtensionOrder` or any other extension-array normalization;
    npm/Nix/Flatpak release packaging changes to bundle Pi, Bun, or Pi's
    `node_modules`; any Rust adapter change beyond T03/T04's existing
    machinery plus the narrow `ExecutionAbandon` addition above if it proves
    necessary; implementing the supervisor's own shell-spawn logic itself
    (T03, already done); any Windows-side Rust/supervisor code (none exists —
    refusal is entirely a TypeScript-extension-level branch).
  - Dependencies: T04
  - Done when:
    1. `sce setup --pi` installs the normal generated Pi extension, unchanged
       from today.
    2. A user launches ordinary `pi` — no wrapper, launcher, or alternate
       entry point.
    3. `bash`, `edit`, and `write` establish mutation scope fail-closed
       whenever SCE's own `tool_call` handler runs, independent of SCE's
       position in the extension array.
    4. `read`, `grep`, `find`, `ls`, custom, and unknown tools remain
       untracked, as specified.
    5. `tool_result` is the sole execution evidence (D6).
    6. An executed, terminal path Closes exactly once.
    7. An admitted-but-not-executed path abandons/rebaselines (D7), never
       Closes.
    8. Terminal transport ambiguity follows D9 above and blocks subsequent
       tracked Starts while unresolved; the `Executed`-then-transport-failed
       sub-case is closed via the `ExecutionAbandon` Rust addition plus the
       extension's per-attempt `(session_id, tool_call_id)`-keyed
       terminal-delivery ordering, which also guarantees
       `tool_execution_end` is never sent to Rust before its own
       `tool_result` regardless of independent subprocess scheduling.
    9. `user_bash`, when Pi actually delivers it to SCE, executes only
       through the T03 supervisor (arm -> `Armed` -> `exec`).
    10. Guard-establishment failure prevents shell execution.
    11. An ambiguous/timed-out `Armed` acknowledgement prevents shell
        execution and terminates the orphaned supervisor.
    12. Cancellation/timeout is forwarded to the supervisor rather than
        killing a local child.
    13. The supervisor's real result drives Pi's Bash result; nothing
        resolves merely because the control channel closed.
    14. Normal Pi tracing/diff/bash-policy behavior remains intact.
    15. Windows: `user_bash` is unconditionally refused; tracked-tool
        attribution is unaffected.
    16. No SCE-hosted Pi launcher or embedded Pi runtime exists.
    17. No `sce pi` command exists.
    18. No release-packaging changes were made.

    Bun tests (mocked subprocess transport) cover: bash-policy-denial-means-
    no-Start, tracked-Start-success, tracked-Start-adapter-failure-blocks,
    missing-`sce`-blocks, read-only/unknown-tool-means-no-adapter-call,
    `tool_result`-keyed execution-evidence forwarding/state (never
    `tool_execution_start`), successful/failed `tool_execution_end` gated on
    a prior `tool_result`, `tool_execution_end`-without-`tool_result`
    abandon, `tool_execution_end` deferred until its own `tool_result`
    delivery settles and then sent strictly after it (never racing two
    independently scheduled subprocesses), a failed `tool_result` delivery
    immediately denying Starts and converting a later `tool_execution_end`
    into `ToolExecutionAbandon` rather than a Close, a `tool_execution_end`
    transport failure after a successful `tool_result` retrying as
    `ToolExecutionAbandon` (never a delayed replayed End) via an injectable
    synchronous retry seam, the same `toolCallId` under two different
    `sessionId`s maintaining fully independent unresolved state, terminal
    transport failure denying subsequent tracked Starts and recovering via
    retry, `user_bash`-returns-`operations`-and-relays-to-the-
    supervisor-rather-than-spawning-a-shell-itself,
    `user_bash`-returns-`result`-full-replacement-and-never-calls-
    `session.executeBash`-on-guard-establishment-failure,
    `user_bash`-returns-`result`-full-replacement-and-terminates-the-
    orphaned-supervisor-process-on-an-ambiguous-acknowledgement,
    wrapped-`exec`-forwards-cancellation/timeout-to-the-supervisor-rather-
    than-killing-a-local-child, wrapped-`exec`-resolves-only-on-the-
    supervisor's-own-delivered-exit-result-never-merely-on-control-channel-
    closure, `user_bash`-unconditionally-refused-on-Windows-with-no-
    supervisor-spawn, model present/absent, session canonicalization, and
    unchanged edit/write-diff and conversation tracing.
  - Verify: `nix run nixpkgs#bun -- test config/lib`; `nix run .#pkl-check-generated`;
    `nix flake check`; focused Rust tests
    (`nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml pi_mutation_scope`)
    if the `ExecutionAbandon` addition is implemented; plus a real scratch
    `sce setup --pi` installation followed by running ordinary `pi` against
    it (not `sce pi`) to exercise the tracked-tool and `user_bash` paths
    end to end.
  - Repair pass (2026-09-17, T05 soundness repair, not T06 work): fixed two
    remaining soundness issues identified at head `3a6e2e9d1`:
    (1) `ToolResult`/`ToolExecutionEnd` transport ordering — the extension's
    terminal-delivery tracker is now keyed by `(session_id, tool_call_id)`
    (never bare `toolCallId`) and `forwardEnd` always awaits the exact same
    attempt's `forwardResult` delivery outcome before choosing what to send,
    so `tool_execution_end` can never reach Rust before its own
    `tool_result` regardless of independent subprocess scheduling; a failed
    `tool_result` delivery now marks that exact attempt unresolved
    immediately and converts any later `tool_execution_end` into
    `ToolExecutionAbandon` rather than silently dropping it (the previous,
    now-removed test explicitly accepted the drop — D9 requires it not be
    best-effort); a `tool_execution_end` transport failure after a
    successful `tool_result` likewise recovers via retried
    `ToolExecutionAbandon`, never a delayed replayed `ToolExecutionEnd`.
    (2) `user_bash` control-channel-close fabrication — `exec()` now rejects
    when the guard's control channel closes before any `status: "result"`
    frame, instead of resolving `{ exitCode: null }`; an explicit supervisor
    `result(exit_code: null)` still resolves normally, confirmed by pinned
    Pi `0.80.6` source (`executeBashWithOperations` in `bash-executor.js`
    awaits `operations.exec()` in a try/catch and rethrows on non-abort
    failure, confirming Promise rejection is the correct, native contract).
    Added a matching Rust regression proving the same `toolCallId` under two
    different `session_id`s never cross-contaminates `ExecutionAbandon`
    resolution, plus a duplicate-`ExecutionAbandon`-is-a-safe-no-op
    regression. Verified: `nix run nixpkgs#bun -- test config/lib` (49
    passed), `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path
    cli/Cargo.toml pi_mutation_scope` (88 passed), `... test hooks::` (721
    passed, 1 ignored), `nix run .#pkl-check-generated` (142 files, parity
    ok), `nix flake check` (all checks passed), `git diff --check` (clean).
    Real scratch smoke: `sce setup --pi` + ordinary `pi` (not `sce pi`)
    against a live model (`opencode-go/gpt-5.6-luna`) drove one real tracked
    `bash` tool call end to end — the adapter's own durable state showed a
    clean `PendingStart(seq=1) -> Executed -> Close -> removed` cycle with
    `recovery: {"phase":"clear"}`, proving ordinary Pi's own subprocess
    scheduling did not trigger the Problem-1 race under this fix. The
    `user_bash` (`!command`) supervisor path could **not** be exercised
    through a real interactive Pi session in this sandboxed environment: it
    requires Pi's interactive TUI mode, which needs a real TTY — a
    `printf '!...' | pi --approve --no-session` attempt hung waiting for a
    terminal and was killed by timeout, and `--print` non-interactive mode
    treats its argument as an LLM prompt, not a literal `!`-prefixed REPL
    command, so it cannot reach `user_bash` at all. This is an environmental
    limitation of this sandbox, not a code defect; the `user_bash` fix is
    still fully covered by the Bun unit tests above (control-channel-close
    rejection, explicit `exit_code: null` result) and by the pre-existing
    Rust `guard_reconciliation_tests`/D13 integration tests, which continue
    to pass unchanged.
  - **2026-09-17 real interactive-TUI smoke evidence:** after reinstalling
    the current branch's Pi integration, ordinary interactive `pi` loaded SCE.
    Running `!sh -c 'printf "guarded\\n" >> human.txt; sleep 60'` visibly held
    the command while an external terminal confirmed that
    `<git-dir>/sce/mutation-cursor-tainted` existed. Cancelling the command
    through Pi was followed by confirmation that the marker was absent. This
    proves the normal `pi -> SCE user_bash -> arm -> Armed -> exec/supervisor`
    path and cancellation/finalization cleanup in the real TUI. The previous
    sandbox limitation is superseded; it no longer blocks T05 completion.
  - Completed: 2026-09-17
  - Files changed: `cli/src/services/hooks/pi_mutation_scope/mod.rs`;
    `config/lib/pi-plugin/sce-pi-extension.ts`;
    `config/lib/pi-plugin/sce-pi-extension.test.ts`;
    `context/cli/pi-mutation-scope-integration.md` (4 files; no generated
    `.pi` copy).
  - Result: The canonical generated Pi extension now fail-closed gates
    tracked `bash`/`edit`/`write` Starts, orders per-attempt terminal delivery
    by `(session_id, tool_call_id)`, abandons unresolved terminal observations,
    and routes human `user_bash` through the two-phase supervisor guard. It
    refuses guarded `user_bash` on Windows without affecting tracked-tool
    attribution, while preserving existing policy, diff, and conversation
    tracing behavior.
  - Verify outcome: All previously recorded T05 verification remains
    satisfied — Bun tests (49 passed), focused Pi adapter tests (88 passed),
    hook tests (721 passed, 1 ignored), generated-output parity (142 files),
    and `nix flake check` all passed with clean `git diff --check`. The real
    interactive-TUI smoke above supplies the previously missing `user_bash`
    supervisor evidence.
  - Context impact: root — the completed extension wiring changes the
    externally observable Pi integration boundary; the five-file root pass
    confirmed the current overview, architecture, glossary, patterns, and
    context map, with stale Pi/guard availability statements corrected in
    root and domain context.
  - Context synchronization: synced

- [x] T06: `Add production-path and live Pi attribution regressions` (status:done, completed 2026-09-18)
  - Task ID: T06
  - Scope: In — extending the existing mutation-provenance production test
    harness with Pi, driving real temporary Git repositories and real
    repository-scoped Agent Trace databases through the Pi adapter, generic
    mutation ingress, snapshot coordinator, scope provenance,
    `mutation_trace_events`, `mutation_ai_patch`, post-commit intersection, and
    Agent Trace JSON, covering: Pi bash/write/edit confirmed mutation with
    `pi_<session>` + model in Agent Trace; missing model preserving session
    with `model` `NULL`; read/grep/find/ls and custom/unknown tools with zero
    scope footprint; later-extension rejection after Start producing no
    `mutation_ai_patch`; mutate-then-error still observing the final Git tree
    through the confirmed Close; two overlapping Pi calls as independent
    scopes with correct contended/confirmation behavior; one overlapping call
    failing while the surviving scope's later confirmed interval remains
    attributable; Pi+Claude, Pi+Codex, and Pi+OpenCode overlap under
    confirmation-safe semantics; stale/dead Pi process recovery discarding the
    old ambiguous interval while later fresh Pi work remains usable;
    `user_bash` creating no Pi AI scope; and the D13 guard end to end: `Start
    -> tool_result -> tool_execution_end` reaches confirmed `AiExclusive`
    attribution; `Start -> no tool_result -> tool_execution_end` abandons and
    never reaches AI attribution.

    Also, explicitly, both sides of the D13 guard, each as an end-to-end
    regression:

    **Guard succeeds — cross-harness recovery, including the mid-command
    race.** This is the regression that proves the lifetime property, not
    merely a precondition:

    ```text
    Pi scope A live
    other-harness scope B live

    user_bash begins; guard durably established; supervisor spawns
    the real shell as its own child
    human write #1

    B's boundary attempts to run while the guard is still active
        =>
    blocks, then fails closed (CoordinateError::LockAcquisition);
    no protocol or taint state is read, mutated, or cleared

    human write #2
    the shell itself terminates (never a control-channel signal)
        =>
    forced database_failure + recover runs against the already-held
    ProtectedWorktree; A and B abandoned; cursor rebaselined to the
    final observed tree; only then is the marker cleared

    B's deferred boundary is retried
        =>
    now succeeds normally against the recovered worktree

    assert:
    both write #1 and write #2 remain excluded from positive AI
    attribution for A and for B, on the harness that owned B, not
    only on Pi's own
    ```

    Test at least one actual cross-harness path (Pi+Claude, Pi+Codex, or
    Pi+OpenCode), and retain the broader Pi+Claude / Pi+Codex / Pi+OpenCode
    overlap coverage already required above. Also test, as a variant of the
    same regression, the two death modes D13 requires an exact policy for:

    ```text
    variant — supervisor dies mid-command (Unix):
    human write #1; SIGKILL the supervisor directly; assert a
    foreign coordinate() attempted at this point still fails closed
    with LockAcquisition (the shell's inherited fd keeps the lock
    held); human write #2; the shell exits on its own; assert the
    lock frees only now, and the next coordinate() on this worktree
    (from either harness) self-heals via the existing
    inherited-taint path; both writes remain excluded from positive
    AI attribution for A and B alike

    variant — Pi/Node dies mid-command (chosen Option A):
    human write #1; kill the Pi/Node control process; assert the
    supervisor does not signal the shell and does not finish early;
    human write #2; the shell exits on its own; assert the
    supervisor still runs its normal finish sequence with nothing
    listening on the dead control channel; both writes remain
    excluded from positive AI attribution for A and B alike
    ```

    Then, in the same regression, verify recovery does not permanently
    poison the checkout:

    ```text
    recover
    abandon/rebaseline
    fresh Start(C) on the same worktree
    clean AI mutation
    tool_result(C)
    tool_execution_end(C)
    => C may reach AiExclusive
    ```

    **Guard fails to establish — command never executes.**

    ```text
    Pi scope A live
    other-harness scope B may be live

    user_bash
    guard-establishment fails (lock-acquisition timeout,
    marker-persistence failure, or spawn/transport failure)

    assert:
    the handler returns { result: ... } full replacement
    no shell — supervised or otherwise — is ever spawned by
        Pi/Node or by the supervisor
    human command did NOT execute
    filesystem mutation did NOT occur
    A/B remain unaffected by a nonexistent human mutation
    no false AI attribution was introduced
    ```

    Also test ambiguous begin acknowledgement:

    ```text
    supervisor process durably acquires the lock and persists the
    marker (no shell spawned yet in this window)
    caller's acknowledgement is lost or times out
    user command blocked; caller terminates the orphaned supervisor
    process

    later boundary
        =>
    conservative recovery occurs, since the killed process's death
    releases the lock immediately in this window — no shell exists
    yet to hold a duplicated fd (positive death evidence, not a
    timeout-based staleness guess)
    ```

    That case is allowed to lose attribution (an accepted false negative);
    it must never lose safety. Also test guard-finalization failure end to
    end: force the finish-time recovery commit to fail; assert the
    supervisor reports failure, the marker remains armed, and the next
    boundary on that worktree self-heals via the existing inherited-taint
    path rather than silently proceeding as if attribution were clean.

    Also test the accepted competing-`user_bash`-extension limitation D13
    documents, so the boundary is proven rather than merely asserted in
    prose:

    ```text
    variant — a competing extension consumes user_bash before SCE:
    register a second extension, ranked ahead of SCE under Pi's normal
    on-disk resolution, whose own user_bash handler returns a truthy
    result or operations; trigger !command;

    assert:
        Pi never invokes SCE's own user_bash handler for this command
            (no supervisor process is spawned, no arm request occurs)
        this is the expected, documented outcome, not a test failure —
            SCE makes no guard claim for an event Pi never dispatched
            to it
        a tracked bash/edit/write tool call in the SAME session still
            establishes mutation scope normally, proving this
            limitation is scoped to user_bash dispatch interception and
            does not weaken tracked-tool attribution
    ```

    Also a pinned real-Pi smoke covering
    bash, write, edit, SCE Start failure, later extension rejection, execution
    error, and model provenance. This smoke, like T01's fixtures, runs on
    Linux; also add a Windows-specific pinned real-Pi smoke (D13's Windows
    disposition) covering: (a) `user_bash` unconditionally refused — the
    handler returns the `result` full-replacement, no supervisor process
    exists on Windows at all, and the human command never executes; (b) a
    tracked `bash`/`edit`/`write` tool call on the same Windows session still
    reaches confirmed `AiExclusive` normally, proving refusal is scoped to
    `user_bash` alone. Do not claim AC1's Windows coverage is equivalent to
    Linux's beyond what this smoke actually exercises — record the
    difference in this task's record rather than asserting parity. Out —
    weakening any regression because the conservative runtime produces less
    positive attribution than expected — fix the expectation or the
    lifecycle design per the formal semantics instead.
  - Dependencies: T05
  - Done when: Pi has the same production-path mutation-attribution confidence
    as Claude, Codex, and OpenCode — confirmed exclusive evidence can reach
    final Agent Trace provenance; uncertain, blocked, failed-to-observe, or
    ambiguous execution cannot.
  - Verify (task-level; the plan-wide **Full validation** section — including
    `nix flake check` and the Quint suite — is deferred to `/validate` and is
    not part of this task's own Verify):
    `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml hooks::`;
    `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml pi_mutation_scope`;
    `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace`;
    `nix run nixpkgs#bun -- test config/lib`; `cargo fmt --check`; `biome check`;
    `cargo clippy --all-targets -D warnings`; `git diff --check`; baseline diff
    over `config/schema/agent-trace.schema.json` and
    `cli/migrations/agent-trace-repository/` against
    `opencode-mutation-scope-integration` (expected empty).
  - Completed: 2026-09-18 (reopened same day for a completion-evidence
    correction, then re-completed once the real Pi-runtime smoke below was
    obtained — see **Real Pi-runtime smoke (2026-09-18 correction)**).
  - Files changed: `cli/src/services/hooks/mod.rs`;
    `cli/src/services/hooks/pi_mutation_scope/mod.rs`;
    `config/lib/pi-plugin/sce-pi-extension.test.ts`;
    `config/lib/pi-plugin/real-pi-runtime-smoke/provider-extension.ts`;
    `config/lib/pi-plugin/real-pi-runtime-smoke/driver.mjs`;
    `config/lib/pi-plugin/real-pi-runtime-smoke/run.sh` (6 files; test-only, no
    production code, schema, or migration changed).
  - Result: Added 12 Pi production-path cases to the existing
    `mutation_provenance_e2e` harness (bash/write/edit confirmed attribution
    with session+model; missing-model NULL; zero-footprint read-only/custom
    tools including a `user_bash`-named-tool proxy; later-extension-rejection
    abandonment; mutate-then-error through confirmed Close; concurrent
    reject/confirm; Pi+Claude, Pi+Codex, Pi+OpenCode overlap; stale-process
    recovery with fresh-work reachability) plus a new `#[cfg(test)]` seam
    (`force_attempt_owner_dead_for_tests`) needed to drive dead-owner recovery
    from that module. Added 2 new D13 guard end-to-end regressions to
    `guard_reconciliation_tests` (mid-command race with a foreign harness scope
    failing closed then succeeding on retry; guard-triggered abandonment not
    poisoning a later fresh Pi scope) — the remaining AC19–AC23 sub-scenarios
    (supervisor-SIGKILL, control-channel death, finalization failure,
    parent-pid, establishment-timeout, ambiguous acknowledgement) were already
    exact-match covered, harness-neutrally, by the five pre-existing
    `external_mutation_guard.rs` runtime tests, since D13's supervisor
    mechanism is generic and Pi is only its current caller — duplicating them
    with a Pi-specific wrapper would not add coverage. Added a pinned **Pi
    capture-replay smoke** (Linux) — also describable as a pinned Pi
    lifecycle-fixture replay; covering bash/write/edit, SCE Start failure,
    later-extension rejection, execution error, model provenance — that
    replays T01's exact captured JSONL fixtures (real Pi 0.80.6 lifecycle
    events) through the extension's real registered handlers against a real
    temporary Git repository. This proves real Pi capture shape -> real SCE
    extension handlers -> expected forwarding behavior. **It does not execute
    the actual Pi 0.80.6 runtime/process**, does not exercise Pi's own
    extension auto-discovery/dispatch, and is not a substitute for a real
    Pi-runtime smoke — this record previously mislabeled it "pinned real-Pi
    smoke"; that label is retracted, the test itself is retained and renamed
    in `sce-pi-extension.test.ts`. A live model-authenticated Pi session could
    not be driven in this sandbox (no `~/.pi/agent/auth.json` credentials),
    matching the environmental limitation T05 already documented for its own
    real-TUI evidence — that explains why a live-model session wasn't used
    for this capture-replay smoke; it does not make capture-replay equivalent
    to a real-Pi-runtime smoke. Added a Windows-specific variant of the same
    capture-replay method (same caveat: not a native Windows run and not a
    real-Pi-runtime smoke) that overrides `process.platform` to `"win32"`
    inside the same Linux test process, proving `user_bash` is unconditionally
    refused there while tracked-tool attribution is unaffected. Added one
    further TypeScript test proving the
    accepted competing-`user_bash`-extension limitation (D13/AC22): a
    synthetic extension ranked ahead of SCE in the dispatch loop consumes
    `user_bash` before SCE's own handler ever runs (no supervisor spawn), while
    a tracked `bash` call in the same session is unaffected — this, too, could
    not be exercised through a live interactive Pi TUI session in this sandbox
    (`user_bash` is TUI-keystroke-only per T01's own NOTES.md evidence), so it
    is proven via Pi's documented first-truthy-wins dispatch order instead.
  - Verify outcome: `cargo test … hooks::` 735 passed, 1 ignored, 0 failed
    (includes the 12 new `pi_*` production-path cases and the 2 new
    cross-harness guard cases); `cargo test … pi_mutation_scope` 90 passed, 0
    failed; `cargo test … mutation_trace` 396 passed, 0 failed; `bun test
    config/lib` 58 passed, 0 failed (57 pre-existing/smoke + 1 new
    competing-extension case); `cargo fmt --check` and `biome check` both
    clean after auto-formatting the new code; `cargo clippy --all-targets -D
    warnings` clean (one new test needed the same
    `#[allow(clippy::too_many_lines)]` precedent already used elsewhere in
    this file); `git diff --check` clean; baseline diff over
    `config/schema/agent-trace.schema.json` and
    `cli/migrations/agent-trace-repository/` against
    `opencode-mutation-scope-integration` is empty (AC17 holds). Full
    `nix flake check` / Quint suite from **Full validation** was not
    separately re-run for this test-only change beyond the targeted
    cargo/bun/fmt/clippy/diff-check commands above; run it as part of
    `/validate`.
  - **Post-completion cleanup:** per repo convention and explicit user
    instruction, all explanatory comments added or touched in
    `cli/src/services/hooks/mod.rs`, `cli/src/services/hooks/pi_mutation_scope/mod.rs`,
    and `config/lib/pi-plugin/sce-pi-extension.test.ts` during this task were
    removed (doc comments, section banners, and inline "why" notes alike).
    `cargo fmt --check`, `biome check`, `cargo clippy --all-targets -D
    warnings`, the `pi_mutation_scope`/`hooks::` cargo suites, and `bun test
    config/lib` were all re-run clean afterward.
  - **Real Pi-runtime smoke (2026-09-18 correction):** this record originally
    mislabeled the capture-replay smoke above "pinned real-Pi smoke." That was
    retracted (see the corrected Result text above); T06 was reopened
    (`status:todo`) until a genuine real-Pi-runtime smoke existed, per the
    task's own original scope. Investigation into pinned Pi `0.80.6`
    (`config/lib/node_modules/@earendil-works/pi-coding-agent`,
    `docs/custom-provider.md` and `docs/sdk.md`) found a supported,
    credential-free path: extensions may call `pi.registerProvider()` with a
    `streamSimple` implementation that fully replaces the network call (no
    HTTP request is made), and `@earendil-works/pi-ai` ships its own official
    scripted-response test harness for exactly this (`createFauxCore` /
    `fauxAssistantMessage` / `fauxToolCall`, from `providers/faux.js`) — a
    supported Pi test mechanism, not an invented one. Separately,
    `createAgentSession()`'s `DefaultResourceLoader` discovers
    `.pi/extensions/` from `cwd`, the same auto-discovery ordinary `pi` uses,
    so a scratch repo's real `sce setup --pi`-installed extension loads
    through Pi's own normal mechanism, not a hand-built API shim. No live
    model call, network access, or `~/.pi/agent/auth.json` credential is
    involved anywhere in this path.

    Built and ran this end to end: `config/lib/pi-plugin/real-pi-runtime-smoke/`
    (`provider-extension.ts` — a throwaway `.pi/extensions/test-provider/`
    scripting one `bash` tool call via `createFauxCore`, then a `done` text
    turn; `driver.mjs` — an SDK driver using `createAgentSession` +
    `DefaultResourceLoader` + `session.setModel()` + `session.prompt()`;
    `run.sh` — builds `sce` from this branch's own source via
    `nix develop -c ./scripts/run-cli-cargo.sh build --manifest-path cli/Cargo.toml`,
    creates a fresh scratch Git repo, runs the real `sce setup --pi` with that
    binary on `PATH`, copies in the test-provider extension, runs the driver,
    then queries the scratch repo's own repository-scoped Agent Trace DB via
    the pinned `nix run .#turso`). Reproduce with:
    `bash config/lib/pi-plugin/real-pi-runtime-smoke/run.sh`.

    First attempt showed zero mutation-trace rows despite the bash mutation
    happening — diagnosed as a test-environment bug, not a production one:
    the generated extension's `spawnSync("sce", ["hooks", "pi-mutation-scope"])`
    resolved to a stale, globally-installed `sce` on the host `PATH`
    (predating this branch, with no `pi-mutation-scope` hook subcommand at
    all) rather than this branch's freshly built binary. Prepending this
    branch's `cli/target/debug` to `PATH` before running `sce setup --pi` and
    the driver fixed it. Observed durable evidence, queried directly from the
    scratch repository-scoped `agent-trace.db`:
    `mutation_trace_scopes`: `actor_kind = pi`, `status = closed`, scope ID
    exactly matching D1's canonical format
    (`pi-tool-v1|n=1|s=36:<real Pi session UUID>|c=16:sce-smoke-bash-1`).
    `mutation_trace_events`: `boundary_kind = close`,
    `attribution_kind = ai_exclusive`, `tainted = 0`, `failure_kind = healthy`.
    `mutation_trace_scope_provenance`: `session_id = pi_<real Pi session UUID>`,
    `model_id = sce-test-provider/sce-test-model`. This satisfies all of: the
    tracked `bash` tool executed through Pi's real tool-dispatch engine; the
    SCE extension loaded via normal Pi discovery, not manual registration;
    real `sce hooks pi-mutation-scope` reached the Rust adapter before
    execution (fail-closed Start honored); `ToolResult`/`ToolExecutionEnd`
    reached it in real Pi order; the attempt closed cleanly; the resulting
    Git mutation reached confirmed `AiExclusive`, not contended or ambiguous;
    and session/model provenance reached the final durable row. Reproduced
    twice independently (once directly, once by a parallel investigation)
    with matching results each time.

    Scope discipline: `bash` only, per this task's own "prefer bash as the
    minimum tracked tool" guidance; `edit`/`write` were not additionally
    exercised through the real runtime, matching the instruction not to
    duplicate the capture-replay matrix just to satisfy the word "smoke."
    T06's original Done-when was not weakened; this closes it as originally
    written. `[x] T06 ... (status:done)` is restored. The capture-replay
    smoke and this real-runtime smoke remain two distinct, separately labeled
    evidence types, per the task record above and
    `context/cli/pi-mutation-scope-integration.md`.
  - Context impact: `context/cli/pi-mutation-scope-integration.md` updated to
    record the real Pi-runtime smoke alongside the existing real-TUI
    (`user_bash`) evidence, distinct from the capture-replay smoke. No other
    root context file, schema, migration, or public interface changed.
  - **Real Pi-runtime smoke self-verification hardening (2026-09-18 repair,
    PR #278):** `run.sh` previously printed the durable evidence rows without
    asserting them, so the smoke could exit 0 having proven nothing — exactly
    the failure mode the first attempt above already hit once (stale global
    `sce` on `PATH` silently produced zero mutation-trace rows). Repaired
    `config/lib/pi-plugin/real-pi-runtime-smoke/run.sh` only (`driver.mjs` and
    `provider-extension.ts` unchanged) to close that gap:
    it now asserts, via the pinned `nix run .#turso -- --experimental-multiprocess-wal
    --readonly -m list -q` (machine-readable pipe-delimited `list` output,
    not the pretty table), exact-cardinality `SELECT COUNT(*)` checks —
    total Pi scopes = 1, closed Pi scopes = 1, one `close`/`ai_exclusive`/
    `tainted=0`/`healthy` event, one `mutation_trace_scope_provenance` row
    with `session_id LIKE 'pi_%'` and the expected `model_id`, and zero
    `mutation_trace_worktrees` rows left `tainted`/`needs_rebaseline`/
    unhealthy — plus that `smoke-output.txt` exists and contains
    `sce-real-pi-smoke`, and that `sce doctor`'s reported
    `agent_trace_db.path` is non-empty, non-null, and exists on disk before
    querying it. Any failed assertion prints a diagnostic and exits non-zero;
    a successful exit now entails a passing assertion, not merely a
    completed process. The stale-`sce` regression is now structurally
    prevented, not just fixed once: `run.sh` builds and calls
    `"$sce_bin" setup --pi --non-interactive` /
    `"$sce_bin" doctor --format json` by absolute path (no `PATH` reliance),
    and, immediately before the real Pi driver runs (inside the same
    `nix develop` invocation used to run it, after re-prepending the
    branch-built `cli/target/debug` onto `PATH` so `nix develop`'s own PATH
    setup cannot let a stale global `sce` win back), asserts
    `command -v sce` resolves, after `realpath`, to
    `$repo_root/cli/target/debug/sce`, failing before Pi starts otherwise.
    `node driver.mjs` now runs through `nix develop -c bash -c '... node
    driver.mjs ...'` instead of bare `node`, per this repo's Nix-only
    tooling rule. Setup is explicit `--pi --non-interactive`, with a scratch
    `.sce/config.json` (`agent_trace.auto_sync: false`) written before setup
    so the automated run can never opt into uploading Agent Traces; no
    control-plane upload or login occurs. Every flake-relative invocation
    (`nix develop`, `nix run .#turso`) now runs inside a `(cd "$repo_root" &&
    ...)` subshell, so `run.sh` behaves identically regardless of the
    caller's working directory (verified by running it from `/tmp`).
    Additionally, `XDG_STATE_HOME` is now scoped to each run's own scratch
    directory: the scratch repo's Agent Trace repository identity is derived
    from a fixed synthetic remote URL, so without this the repository-scoped
    Agent Trace DB would previously have been the same persistent file
    reused (and accumulated into) across every run, which would have made
    the new exact-cardinality assertions fail on any run after the first —
    this was found and fixed during this repair, not merely asserted safe.
    Proved the assertions are load-bearing by temporarily changing the
    "closed Pi mutation scopes" expectation from 1 to 2 and re-running the
    full smoke end to end: it failed with `FAIL: closed Pi mutation scopes:
    expected '2', got '1'` and a non-zero exit, then the expectation was
    reverted and the smoke re-run clean (`PASS: real Pi runtime
    mutation-attribution smoke`, exit 0) — this reproduces the exact same
    durable evidence already recorded above (same scope/event/provenance
    shape, fresh Pi session UUIDs each run), so this repair reproves rather
    than reopens T06's runtime claim. Reproduce with:
    `cd /tmp && <repo>/config/lib/pi-plugin/real-pi-runtime-smoke/run.sh`.
  - Context synchronization: synced

## Open questions

None. T01 owns every lifecycle fact that could invalidate this design, and a contradictory finding there is a re-planning gate, not a deferred guess. The one dependency question this plan started with — whether to wait for PR #276 to merge, redo its protocol generalization here, or stack directly on its branch — was resolved during planning: this plan stacks on PR #276's head per **Stack and base**.

## Validation Report

**Status:** validated  
**Date:** 2026-09-18

### Commands run

- `nix run .#quint -- typecheck spec/mutation_cursor.qnt` -> exit 0 (typecheck clean)
- `nix run .#quint -- test spec/mutation_cursor.qnt` -> exit 0 (`mutation_cursor` suite passed)
- `nix build .#checks.x86_64-linux.mutation-trace-quint-connect` -> exit 0 (build succeeded)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml pi_mutation_scope` -> exit 0 (90 passed, 0 failed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace` -> exit 0 (396 passed, 0 failed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml hooks::` -> exit 0 (735 passed, 1 ignored, 0 failed)
- `nix run nixpkgs#bun -- test config/lib` -> exit 0 (58 passed, 0 failed)
- `nix run .#pkl-check-generated` -> exit 0 (142 files, ephemeral generation matches committed output)
- `nix flake check` -> exit 0 (all checks passed: cli-tests, cli-clippy, cli-fmt, mutation-trace-quint-connect, cli-generated-input, pkl-generated, codex-hook-command, npm/config-lib bun+biome checks, workflow-actionlint, native-portability-audit, flatpak-static-validation, cargo-sources-parity, flatpak-manifest-parity)
- `git diff --check` -> exit 0 (clean, no whitespace/conflict markers)
- `git diff origin/opencode-mutation-scope-integration...HEAD -- config/schema/agent-trace.schema.json cli/migrations/agent-trace-repository/` -> exit 0 (empty diff)
- `git diff origin/opencode-mutation-scope-integration...HEAD -- cli/src/services/mutation_trace/protocol.rs spec/mutation_cursor.qnt spec/mutation_cursor.md` -> exit 0 (104 insertions/11 deletions across exactly these 3 files; every added line is Pi-shaped: `ActorKind::Pi` added to `requires_boundary_confirmation`, one new `Scope6`/Pi Quint scope constant, Pi-specific Quint test scenarios and doc wording; no new protocol fields or structural changes)
- `bash config/lib/pi-plugin/real-pi-runtime-smoke/run.sh` (run from `/tmp`, independent of T06's own run) -> exit 0 (`PASS: real Pi runtime mutation-attribution smoke`; real Pi 0.80.6 SDK session via `createFauxCore`/`DefaultResourceLoader`, real `sce setup --pi`-installed extension via ordinary auto-discovery, real `sce hooks pi-mutation-scope` dispatch; durable evidence: 1 Pi mutation scope, closed; 1 `close`/`ai_exclusive`/`tainted=0`/`healthy` event; 1 provenance row `session_id=pi_<uuid>`, `model_id=sce-test-provider/sce-test-model`; 0 tainted/unresolved worktrees)

### Success-criteria verification

- [x] AC1: exact Pi 0.80.6 lifecycle evidence exists -> T01's committed fixtures/report (`cli/src/services/hooks/pi_mutation_scope/fixtures/`) remain in the tree; re-confirmed against the final implementation by the independent real-Pi-runtime smoke reproduction above, which reproduces the exact `tool_call`/`tool_result`/`tool_execution_end` ordering and produces the expected confirmed-Close outcome.
- [x] AC2: `bash`/`edit`/`write` each establish one scope before execution -> `mutation_provenance_e2e::pi_bash_mutation_persists_model_and_session_in_agent_trace`, `pi_edit_mutation_persists_model_and_session_in_agent_trace`, `pi_write_mutation_persists_model_and_session_in_agent_trace` (all passing in the `hooks::` run), plus the real-runtime smoke's `bash` scope.
- [x] AC3: read-only/`user_bash`/unknown tools create no scope -> `pi_mutation_scope::tests::classification_table` and `mutation_provenance_e2e::pi_read_only_and_unknown_tools_create_no_scope_or_mutation_state` passing.
- [x] AC4: failed Start blocks the tool before execution -> `pi_mutation_scope::tests::run_from_payload_fails_closed_when_a_tracked_start_cannot_resolve_its_checkout` passing.
- [x] AC5: positive attribution requires confirming Close, keyed on `tool_result`-then-`tool_execution_end` -> `runtime_seam_tests::a_write_start_result_close_lands_a_real_ai_exclusive_event_with_pi_provenance` and the Quint `requires_boundary_confirmation`/close-pairing model (typecheck + test suite green) passing.
- [x] AC6: unconfirmed Pi scope suppresses attribution across harnesses -> `mutation_provenance_e2e::pi_and_claude_overlap_produces_ai_contended`, `pi_and_codex_overlap_stays_ineligible_until_codex_confirms`, `pi_and_opencode_overlap_stays_ineligible_until_opencode_confirms` passing; Quint `testUnconfirmedPiScopeBlocksCrossHarnessAttribution` in the passing Quint suite.
- [x] AC7: confirming Close yields `AiExclusive`/`AiContended` correctly -> same `pi_and_*_overlap` tests plus `runtime_seam_tests::a_write_start_result_close_lands_a_real_ai_exclusive_event_with_pi_provenance`, and the real-runtime smoke's single-scope `AiExclusive` result.
- [x] AC8: earlier-extension/bash-policy rejection creates no scope; later-extension rejection abandons, never Closes -> `runtime_seam_tests::a_start_followed_by_no_execution_abandons_through_the_real_runtime`, `lifecycle_tests::tool_execution_end_without_a_preceding_tool_result_abandons_never_closes`, `mutation_provenance_e2e::pi_later_extension_rejection_after_start_produces_no_mutation_ai_patch` all passing.
- [x] AC9: `isError` execution still produces `tool_result` and observes the final tree through the same Close -> `mutation_provenance_e2e::pi_mutate_then_error_still_persists_confirmed_mutation_through_close` passing.
- [x] AC10: overlapping Pi calls stay independent scopes -> `mutation_provenance_e2e::pi_concurrent_reject_and_confirm_keeps_only_the_confirmed_mutation_ai`, `lifecycle_tests::abandoning_one_sibling_never_touches_a_concurrent_sibling_in_the_same_session` passing.
- [x] AC11: lost/failed terminal boundary is never replayed as current -> `lifecycle_tests::a_terminal_recovery_flush_failure_leaves_a_pending_recovery_and_denies_new_admission`, `a_crash_mid_abandon_loop_is_resumed_and_completed_on_the_next_boundary_lock_acquisition` passing.
- [x] AC12: stale-process cleanup requires positive death evidence only -> `lifecycle_tests::a_dead_owner_scope_is_recovered_while_a_live_owner_sibling_survives_untouched`, `multiple_dead_owner_scopes_are_retired_in_one_recovery_generation_while_a_live_sibling_survives`, `a_pending_start_attempt_never_blocks_a_concurrent_new_admission` passing.
- [x] AC13: Start provenance stores canonical `pi_<sessionID>` + model or `NULL` -> `mutation_provenance_e2e::pi_missing_model_preserves_session_with_null_model_in_agent_trace` and the three `pi_*_mutation_persists_model_and_session_in_agent_trace` tests passing; independently reconfirmed by the real-runtime smoke's `mutation_trace_scope_provenance` row (`session_id=pi_<real-session-uuid>`, `model_id=sce-test-provider/sce-test-model`).
- [x] AC14: existing Pi Bash policy/conversation-trace/diff-trace/setup/doctor remain intact -> `nix run nixpkgs#bun -- test config/lib` 58/58 passing (bash-policy-plugin, pi-plugin, mutation-scope-plugin suites), and the real-runtime smoke's `sce setup --pi` / `sce doctor --format json` path succeeding end to end.
- [x] AC15: only confirmed exclusive Pi evidence reaches `mutation_ai_patch` -> `pi_later_extension_rejection_after_start_produces_no_mutation_ai_patch` (negative) plus the `pi_bash/edit/write_mutation_persists_model_and_session_in_agent_trace` tests (positive), all passing against a real Git/DB harness.
- [x] AC16: cross-harness Pi overlap (Pi+Claude, Pi+Codex, Pi+OpenCode) -> `pi_and_claude_overlap_produces_ai_contended`, `pi_and_codex_overlap_stays_ineligible_until_codex_confirms`, `pi_and_opencode_overlap_stays_ineligible_until_opencode_confirms` all passing.
- [x] AC17: no new Agent Trace schema or migration -> targeted baseline diff over `config/schema/agent-trace.schema.json` and `cli/migrations/agent-trace-repository/` against `opencode-mutation-scope-integration` is empty.
- [x] AC18: protocol/Quint change limited to the Pi confirmation-required case -> targeted baseline diff over `protocol.rs`/`spec/mutation_cursor.qnt`/`spec/mutation_cursor.md` shows exactly 3 files, 104 insertions/11 deletions, every added line Pi-shaped (see Commands run); no new `protocol.rs` structures.
- [x] AC19: guarded `user_bash` lifetime spans the real shell process, not the supervisor or control channel; Windows refuses -> `external_mutation_guard::tests::a_supervisor_killed_without_unlocking_leaves_the_flock_held_by_the_spawned_shell`, `closing_the_control_channel_does_not_trigger_finish_or_signal_the_shell`, `graceful_completion_waits_for_an_inherited_background_descendant`, `output_is_consumed_while_a_background_descendant_holds_the_lifetime_token` (all in the passing `mutation_trace` run) plus the TS `describe("Windows-specific pinned Pi capture-replay smoke (D13 disposition)")` / `test("refuses unconditionally on win32")` cases in the passing `bun test config/lib` run.
- [x] AC20: post-recovery fresh scopes still reach `AiExclusive` -> `mutation_provenance_e2e::pi_stale_process_recovery_discards_ambiguous_interval_while_fresh_pi_work_remains_usable` and `external_mutation_guard::tests::a_concurrent_foreign_lock_attempt_times_out_while_the_guard_is_active` passing.
- [x] AC21: guard-establishment failure never spawns a shell; supervisor is the real spawner -> `external_mutation_guard::tests::lost_armed_acknowledgement_cannot_spawn_or_mutate`, `a_failed_finish_commit_leaves_the_marker_armed_and_reports_failure`, `lifetime_token_establishment_failure_cannot_emit_armed`, `the_spawned_shells_parent_is_the_calling_process`, `armed_guard_waits_for_exec_and_drops_without_spawning_on_eof` all passing.
- [x] AC22: `sce setup --pi` + ordinary `pi` auto-discovery drives tracked-tool attribution; competing extension is a documented boundary -> independently reproduced by the real-Pi-runtime smoke (ordinary `sce setup --pi`, real Pi auto-discovery, real `sce hooks pi-mutation-scope` dispatch, `AiExclusive` result) for the tracked-tool path; TS `test("a competing extension consuming user_bash ahead of SCE prevents SCE's handler from ever running, while tracked-tool attribution in the same session is unaffected")` passing for the documented-boundary path. The interactive `!`/`!!` guard-establishment portion of this AC's Validate text remains evidenced only by the unit/integration guard tests and T05's documented environmental limitation (no `~/.pi/agent/auth.json`, no live TUI in this sandbox) — an unchanged, previously accepted limitation, not a new gap.
- [x] AC23: control-process death never truncates a running `user_bash` command or ends the guard early -> `external_mutation_guard::tests::closing_the_control_channel_does_not_trigger_finish_or_signal_the_shell` passing, matching this AC's exact scenario.

### Failed checks and follow-ups

- None.

### Residual risks

- AC19/AC22's interactive `!`/`!!` guard-establishment path against a live, credential-authenticated Pi TUI session was not exercised in this sandbox (no `~/.pi/agent/auth.json`); coverage rests on the `external_mutation_guard` unit/integration suite plus T05's real-TUI evidence for the non-guard path, a limitation already documented at task completion, not introduced by this validation.
- T06's own record notes Windows tracked-tool (`bash`/`edit`/`write`) parity with the Linux captures is inferred from source (plain JS/TS control flow, no OS syscalls) rather than directly captured on Windows; the Windows-specific evidence obtained is a same-process `process.platform` override, not a native Windows run.


