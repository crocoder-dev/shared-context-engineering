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
  `opencode-mutation-scope-integration`, head commit `281a5290d35750f6952ea5646bd3aaefb08bb36c`;
  based on `mutation-scope-provenance`). **PR #276 is currently open and unmerged.**
- **This branch:** `pi-mutation-scope-integration` (PR #278) is already stacked
  directly on `opencode-mutation-scope-integration`: PR #276's head
  (`281a5290d`) is the merge base and the direct parent commit of this branch,
  which is exactly one plan commit ahead of it. Confirmed at planning time:
  the OpenCode mutation-scope adapter and the generalized per-`ActorKind`
  confirmation-required predicate (covering `ActorKind::Codex` and
  `ActorKind::OpenCode`) are already present in this branch's history via that
  stacked base.
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
supervisor waits for the shell's OWN process to terminate
    (wait()/waitpid — positive termination evidence, never a
    control-channel signal, never elapsed time)
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

When the supervisor *is* still alive at the moment the shell exits (the ordinary, non-crash path), it observes that termination directly via its own `wait()`/`waitpid` on its child and immediately runs the finish sequence itself (forced recover, `complete()`, exit) rather than waiting for some unrelated future `coordinate()` call to stumble into the self-heal path — the fd-duplication mechanism above is specifically the supervisor-crash backstop, not a replacement for the supervisor's normal eager finish.

The fd-duplication mechanism above is Unix-specific by construction: it depends on `flock(2)` semantics attaching to the open file description and surviving `dup()`/inheritance across `fork()`/`exec()`, which is a POSIX guarantee with no Windows equivalent for `std::fs::File`'s locking primitive. T03 must verify Rust's exact spawning API for clearing `FD_CLOEXEC` on a duplicated descriptor before exec (e.g. `dup()` via `std::os::fd::AsRawFd`/`OwnedFd` plus a `pre_exec` hook, or an equivalent `std::process::Command` facility) against the actual `WorktreeLock`/`File` type before committing to an exact implementation; if inspection turns up a simpler or stronger kernel-backed alternative that provides the same "shell lifetime durably implies lock lifetime" guarantee on Unix, T03 should use it instead — the requirement is the property, not this exact API sequence.

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

**Extension-dispatch bypass — `user_bash` is first-handler-wins, so a foreign extension can execute the command before SCE ever sees it.** This is a second, independent soundness hole, not a variant of the lifetime hole above: T01's own evidence (`handleBashCommand()`) establishes that Pi consumes exactly one `user_bash` handler's result — the first one that returns a truthy `operations` or `result` — not a chain where every registered extension's handler runs in sequence the way `tool_execution_start`/`tool_call` do for tool calls (D5). If any other registered Pi extension's `user_bash` handler runs ahead of SCE's and itself returns a result, Pi never invokes SCE's handler at all: no supervisor is spawned, no `WorktreeLock` is acquired, no `ExternalTaintMarker` is armed, and the human command executes with **no guard whatsoever** while it may race a live, later-confirming AI scope — the exact false-positive-attribution shape D13 exists to prevent, reached by a completely different path than the lifetime holes above.

SCE cannot rewrite Pi's own `user_bash` dispatch, and this plan does not attempt to. A prior version of this section proposed relying on `sce doctor`/setup warnings alone to mitigate this hole. **A warning is diagnostic, not a safety boundary, and is retracted as the mitigation.** The required invariant is:

```text
positive Pi mutation attribution enabled
    =>
SCE user_bash interception is proven unavoidable

equivalently:

SCE user_bash interception not provably unavoidable
    =>
positive Pi mutation attribution disabled/fail-closed
```

**Confirmed exact dispatch evidence, against pinned Pi `0.80.6` source (not merely T01's `handleBashCommand()` finding):**

* `emitUserBash()` (`config/lib/node_modules/@earendil-works/pi-coding-agent/dist/core/extensions/runner.js`, ~line 667) iterates `this.extensions` — a flat, pre-resolved array fixed once per session load — in order; for each extension it iterates that extension's own registered `user_bash` handlers in registration order; the first handler whose result is truthy is returned immediately (`return handlerResult`) and no further extension is even reached. This is the exact, complete dispatch loop — there is no separate non-short-circuiting phase.
* `this.extensions`'s order is **not** simply "project-local, then global, then explicitly configured" as an earlier version of this plan assumed from the simpler `discoverAndLoadExtensions()` helper (`loader.js`) — that helper exists but is not what a real session uses. The actual path (`resource-loader.js`, used by both `agent-session.js` and the SDK's `sdk.js`) ranks every resolved extension by `resourcePrecedenceRank()` (`package-manager.js`, ~line 60): `0` = project + explicit settings-entry, `1` = project + auto-discovered, `2` = user/global + explicit settings-entry, `3` = user/global + auto-discovered, `4` = extension supplied by an installed package — then, separately and with strictly higher precedence than all of the above, merges in any **CLI-provided** extension path (`-e`/`additionalExtensionPaths`) as `primary` via `mergePaths(cliEnabledExtensions, enabledExtensions)` (`resource-loader.js`, ~line 269), which places every CLI-provided extension **unconditionally first**, ahead of every ranked entry including rank `0`.
* SCE's generated extension is installed at `.pi/extensions/sce/index.ts` (`cli/build.rs`) with no corresponding entry ever written into `.pi/settings.json`'s extensions list (confirmed: no `"extensions"` write exists in `cli/src/services/setup/mod.rs`), so it is **rank `1` — project + auto-discovered** — never rank `0`.
* Consequently, three independent ways an extension can dispatch `user_bash` ahead of SCE's exist on pinned `0.80.6`, in addition to the readdir-order tie risk noted below:
  1. any extension path passed via Pi's CLI `-e` flag or an SDK caller's `additionalExtensionPaths` — unconditional, no configuration state can prevent it;
  2. any extension the user explicitly lists in `.pi/settings.json`'s project-scope `extensions` array (rank `0`, strictly above SCE's rank `1`);
  3. an **SDK-embedding caller's own `extensionsOverride` hook** (`resource-loader.js`, ~line 279: `this.extensionsResult = this.extensionsOverride ? this.extensionsOverride(extensionsResult) : extensionsResult;`) — a first-class, documented extensibility point that lets an embedder replace or arbitrarily reorder the **entire** resolved extension array after Pi's own ranking is computed, with no constraint whatsoever. This is not a race or an edge case; it is a supported Pi capability specifically for controlling extension composition, and it defeats any purely configuration-based ordering guarantee by construction.
* Within a single precedence rank (e.g. two project-auto-discovered extensions, including a second one alongside SCE's own `.pi/extensions/sce/`), relative order is `fs.readdirSync()` enumeration order (`package-manager.js`/`loader.js`), which Node.js does not specify or guarantee as any particular order; `Array.prototype.sort()`'s stability only preserves whatever that unspecified order happens to be. **Readdir order is not a soundness guarantee** and this plan does not treat it as one.
* `/reload` (`ctx.reload()`, backed by `resourceLoader.reload()` + a fresh `getExtensions()` call, `agent-session.js` ~line 2023-2029) recomputes the entire resolved/ranked extension set and rebuilds the `ExtensionRunner` from scratch; if the project's `.pi/settings.json`, CLI flags, or directory contents changed since the session started, the winner of a future `user_bash` dispatch can change. Neither the load-time `ExtensionAPI` (`createExtensionAPI`, `loader.js`) nor the per-event `ctx` (`createContext()`, `runner.js` ~line 420) exposes any sibling-extension list, position, or registration-order information to an extension's own code — **confirmed by direct inspection of both object constructors, not inferred** — so SCE's extension has no way, from inside itself, to learn at load time or at dispatch time whether it is first, whether a competing handler exists, or whether a `/reload` changed that fact.
* No lower-level, non-short-circuiting hook wraps actual Bash execution: `AgentSession.executeBash()`/`executeBashWithOperations()` (`agent-session.js` ~line 2139) — the only code path that ever actually runs a shell for `!`/`!!` — emits no event and calls no extension hook of any kind; it is reached only when the single `emitUserBash()` winner's result lacked a full-replacement `result`. **Option A (an unavoidable execution seam) does not exist on pinned Pi `0.80.6` and is ruled out by direct source inspection, not assumed absent.**
* No public priority/ordering field exists on `pi.on(event, handler)` or anywhere in `ExtensionAPI`. **Option B (enforceable SCE-first ordering) does not exist**: even disregarding the SDK `extensionsOverride` escape hatch, registering SCE's extension as a rank-`0` project-settings-entry (a real, available improvement T05 could make, since it is currently rank `1`) would still lose to any CLI-provided extension and to any other rank-`0` entry a user lists ahead of it in the same settings array — it narrows exposure, it does not close it, and it does nothing for the `extensionsOverride` case at all.

**Selected mechanism: Option C, a runtime fail-closed compatibility gate — implemented outside Pi's own extension sandbox, since Pi's extension API exposes no introspection an extension could use to gate itself.** Because SCE's own extension code (confirmed above) cannot observe its own dispatch position or detect a competing handler from inside Pi's process, the check cannot live inside `sce-pi-extension.ts`'s handler bodies. T05 must instead:

* Ship a thin `pi` launcher (installed by `sce setup --pi`, invoked by the user or their shell configuration in place of the real `pi` binary — exact packaging, e.g. a wrapper script placed ahead of the real binary on `PATH` or a shell function/alias, is T05's to determine) that, immediately before exec-ing the real `pi` binary with the same arguments, replicates pinned Pi's own `resource-loader`/`package-manager` resolution (importable from the vendored package without executing arbitrary extension code — path/metadata resolution only, not `factory()` invocation) to compute the exact ranked extension order the real invocation is about to use, and checks whether **any** entry precedes SCE's own generated extension path. Re-running this on every launch (not once at `setup`/`doctor` time) is required because the effective order can change between sessions (settings edits, `-e` flags, directory contents) — a cached/stale "doctor said it was fine" result is exactly the failure mode the human's brief prohibits. **Corrected below (see "Corrected a fourth time"): when this check finds a preceding extension, the launcher refuses to exec `pi` at all — it no longer launches Pi with attribution merely withheld.**
* Communicate the result to SCE's own extension via an environment variable the launcher sets before exec-ing the real `pi` process (e.g. `SCE_PI_USER_BASH_GUARD_SAFE=1` only when no preceding extension was found; unset/absent/`0` otherwise). **Corrected below: this variable is no longer the safety proof.** It remains a launch-time optimization/diagnostic signal, but the runtime gate SCE's extension actually trusts before enabling the guarded `user_bash` handler or any Start-capable tracked-tool handling is the fresh, in-process, live-state check described in "Corrected a fourth time" below, re-derived at every factory invocation (initial load and every later `/reload`), never a process-lifetime environment variable read once. Absent/non-`1` (from either signal) is always treated as unsafe, and unsafe always withholds tracked-tool Start handling as well as `user_bash`, per the human's brief.
* **This wording is superseded — see "Corrected a fourth time" below for the actual permanent scope boundary statement, restated for worktree-wide (not merely Pi-attribution) safety.**
* `sce doctor`/`sce setup --pi` remain valuable as **diagnostic-only** reporting (surfacing the same ordering computation for a human to read and act on, e.g. "another extension is configured ahead of SCE's; use the `sce`-provided `pi` launcher, or move/remove the competing extension"), but per the human's brief, doctor output must never be treated, described, or relied upon anywhere in T05/T06 as the actual safety mechanism — the launcher-refusal-plus-in-process-check gate below is the safety mechanism, and it runs on every launch and every runner rebuild unconditionally, not only when a human happens to invoke `doctor`.
* This disposition (the exact dispatch-order facts above, the ruling-out of Options A and B, and the selected Option C mechanism as corrected below, with its named permanent limitations) must be recorded in T02's task record before T02 is considered done, per D2's existing "false negative over false positive, never silent" posture: SCE must never claim D13 protection is active without a fresh, positive check that remains valid for the entire interval attribution is enabled.

**Corrected a fourth time — withholding Pi's own attribution is not worktree protection, and a launch-time attestation does not survive `/reload`.** Two further load-bearing holes were found in the Option C mechanism above, both before any T02 work began.

*Hole 1 — disabling Pi's own attribution does not protect a concurrently live Claude/Codex/OpenCode (or another Pi) scope.* The mechanism as originally stated only withholds SCE's own Start/Close registration and the `user_bash` guard for the current Pi session; it does not prevent Pi from starting, and it does nothing to the worktree itself. If a foreign extension wins `user_bash` dispatch (any of the three configurations in "Confirmed exact dispatch evidence" above), that foreign extension executes the human's command with no SCE guard whatsoever — no supervisor, no `WorktreeLock`, no `ExternalTaintMarker` — while SCE's own extension sits inert. This is not merely a missed-Pi-scope problem: every harness's mutation attribution in this codebase works by attributing whatever tree diff occurred during a scope's live, unconfirmed interval to that scope once it confirms, because nothing else in the generic runtime distinguishes "this diff came from the scope's own tool call" from "this diff came from an unrelated, unguarded shell command that happened to run concurrently." The `WorktreeLock`/`ExternalTaintMarker` guard is the *only* thing in this codebase that prevents an ambient human mutation from being folded into a concurrently-live scope's confirmed attribution — that is D13's entire reason for existing. Therefore:

```text
no Pi scope exists  =>  the worktree is safe
```

is false, and every occurrence of this reasoning anywhere in this plan (including the previous version of AC22's cross-harness bullet and T06's corresponding regression) is retracted. The correct invariant:

```text
An unguarded (unproven-safe) user_bash execution is a worktree-wide
attribution hazard, not merely a Pi-attribution hazard. Disabling
Pi's own Start/Close registration cannot by itself protect a live
Claude/Codex/OpenCode (or another Pi) scope on the same worktree,
because the thing that protects a live scope from an unguarded
human mutation is the WorktreeLock/ExternalTaintMarker guard, not
the presence or absence of a Pi mutation scope.

"Pi attribution disabled" and "worktree attribution protected" are
not equivalent, and no revision of this plan may treat them as
equivalent.
```

*Hole 2 — a launch-time attestation does not survive `/reload`.* `SCE_PI_USER_BASH_GUARD_SAFE` was specified to be read once, at extension factory time, into an in-memory decision. But pinned Pi `0.80.6` can rebuild its entire extension runner in the same process: `ResourceLoader.reload()` (`dist/core/resource-loader.js`, ~line 216-219, inspected directly against the installed package) calls `clearExtensionCache()` whenever `this.loaded` is already true — clearing the module cache so the *next* `loadFinalExtensionSet()` (line 270) re-imports and re-invokes every extension's factory function fresh, including SCE's own `.pi/extensions/sce/index.ts` — and `AgentSession.reload()` (`dist/core/agent-session.js`, ~lines 2023-2044) drives this end to end: it emits `session_shutdown` with `reason: "reload"` on the *old* `ExtensionRunner` (line 2025), then awaits `this._resourceLoader.reload()` (line 2029), then calls `this._buildRuntime(...)` (line 2030), which calls `getExtensions()` (line 2002) and constructs a **brand-new** `ExtensionRunner` instance (line 2008), replacing `this._extensionRunner` outright, and finally (when bindings exist) emits `session_start` with `reason: "reload"` (line 2041) via the *new* runner. `ExtensionRunner.invalidate()` (`dist/core/extensions/runner.js`, line 323) independently confirms the runtime's own model of this: any `ctx` captured before a `ctx.reload()` is declared stale and must not be reused. If the project's `.pi/settings.json`, CLI flags, or directory contents changed since the process launched — the exact same inputs the launcher's pre-exec check read — the *new* incarnation's dispatch order can differ from what the launcher attested, but `process.env.SCE_PI_USER_BASH_GUARD_SAFE` is a process-lifetime value: it is untouched by `reload()`, so a stale `"1"` set at launch remains readable by the newly reinstantiated factory unless something explicitly invalidates it first. This is exactly the hazard the human's brief describes:

```text
launch-time-safe   !=   session-lifetime-safe
```

**Required attestation invariant.** Positive Pi mutation attribution may be enabled only while the exact extension-dispatch configuration actually used by the current Pi `ExtensionRunner` is known to satisfy the `user_bash` interception invariant, for the entire interval attribution remains enabled — not the configuration predicted at process launch. Any operation capable of replacing/reordering that runner invalidates the proof until a new enforceable proof exists. A launch-time-only check does not satisfy this.

**Selected disposition — Option A applied at every point SCE's own factory code runs, not only at initial launch, plus Strategy 3 (fresh attestation on every runner rebuild) grounded in a confirmed Pi seam, not an assumption:**

1. **Launcher refusal (closes Hole 1 at launch).** The `pi` launcher's fresh per-launch check (unchanged computation from "Confirmed exact dispatch evidence" above) no longer merely withholds an environment variable while still exec-ing `pi`. If it finds any extension preceding SCE's own generated extension path, it **refuses to exec the real `pi` binary at all** and exits non-zero with a diagnostic naming the conflicting extension (the same fact `sce doctor` already reports). No Pi process of any kind starts via that invocation, so no `user_bash` dispatch of any shape — guarded or unguarded — can occur through this entry point, and a concurrently live Claude/Codex/OpenCode/Pi scope on that worktree is completely unexposed to this hazard for this launch. This is preferable to launching a session capable of introducing an invisible, unguarded human mutation into a worktree containing scopes from another harness, per the human's brief. This is an attribution-safety *admission* failure the launcher enforces before Pi exists at all, not a Pi-internal Bash-policy decision.
2. **In-process self-check on every factory invocation (closes Hole 2, and narrows — but does not eliminate — the launcher/Pi startup TOCTOU).** SCE's own extension factory (`config/lib/pi-plugin/sce-pi-extension.ts`) independently re-derives the ranked extension order itself, in-process, using the same vendored resolution entry points the launcher already imports (e.g. `DefaultPackageManager.resolve()`, `dist/core/package-manager.js` — exact exported surface to be pinned down at T05 time from the installed package, not invented here), against the *live on-disk* settings/CLI state at the exact moment the factory runs. Because `clearExtensionCache()` forces a genuinely fresh factory invocation on every `reload()` (confirmed above), this in-process check re-executes automatically on every runner rebuild — initial load and every subsequent `/reload`/`AgentSession.reload()` alike — with no separate reload-specific hook required. SCE's extension enables guarded `user_bash` handling and tracked-tool Start registration **only when this fresh, in-process check itself agrees SCE is first**; `SCE_PI_USER_BASH_GUARD_SAFE` is demoted from "the safety proof" (forbidden by the human's brief) to an early, launch-time-only optimization/diagnostic signal — the runtime gate actually enforced inside a running Pi process is always the freshly re-derived, live-state, in-process check, never a trusted environment variable read once.
3. **Terminate, don't merely downgrade, when an already-running launcher-admitted session goes unsafe.** If the in-process check ever fails on a reinstantiation *after* the process has already been running in guarded-attribution mode (i.e., a `/reload` or `AgentSession.reload()` discovers a newly-unsafe configuration), SCE's extension does not merely withhold its own Start/`user_bash` registration and let the now-unsafe process keep running — per Hole 1's invariant, that would leave any concurrently live Claude/Codex/OpenCode scope on the same worktree exposed to exactly the same unguarded-dispatch hazard as an unsafe launch, just discovered later. The extension instead forces the Pi process to terminate (a hard exit after best-effort diagnostic output) so the newly-unsafe configuration can never be exercised via `user_bash` for the remainder of what would otherwise have been this process's life. This is Option A applied uniformly at every point SCE's own code runs — initial launch and every later reinstantiation — not only at the first one. This does not conflict with D13's existing Option A control-death policy (a supervisor already guarding an in-flight human command keeps running that command to completion regardless of Pi/Node's death); terminating Pi/Node here only forecloses *future*, not-yet-started, unguarded `user_bash` dispatch.

**Named residual — retracted below for launcher-admitted sessions, not merely accepted as minimized.** The in-process self-check (item 2 above) was the same deterministic resolution function invoked independently against the same on-disk inputs, at a time much closer to Pi's own real resolution than the launcher's earlier external check — but it was still a *prediction*, not an *authority*: "the safety proof does not claim two independent resolver executions are guaranteed to coincide, only that they use the same code against the same inputs at closely-adjacent times, which is a *minimized*, not *zero*, TOCTOU" is retracted as an acceptable final state by "Corrected a fifth time" immediately below, which replaces prediction with ownership. See that section for why the TOCTOU is eliminated, not minimized, for any session admitted through SCE's launcher, and for the one residual boundary that remains permanent (sessions that bypass the launcher's `ResourceLoader` construction entirely).

**Corrected a fifth time — the safety authority must exist outside the replaceable Pi extension set, not inside its own factory; a concrete bypass proves the prior mechanism unsound as a primary proof, not merely imprecise.** Items 1–3 of "Corrected a fourth time"'s selected disposition — the launcher refusing to exec `pi` at launch, SCE's own extension factory independently re-deriving the ranked order at every invocation, and terminating the process when a *later* factory invocation finds itself unsafe — are retracted as the primary soundness proof. They remain permissible only as diagnostics/defence-in-depth (see below), because all three share one structural flaw: every one of them runs as code *inside* `sce-pi-extension.ts`, which is itself a member of the very extension set the mechanism is supposed to police. This is load-bearing, not a corner case:

```text
Pi launched safely through the old launcher design
SCE extension active, positive attribution enabled

project configuration changes: SCE's own generated extension is
removed / disabled / renamed / excluded from the next resolved set
a foreign extension registering user_bash is added

/reload
    =>
the reinstantiated ExtensionRunner is built from a resolved set that
never includes SCE's extension at all
    =>
SCE's own factory function — the code that was supposed to
independently re-check safety and terminate the process if unsafe —
never executes, because it is not part of the set being loaded
    =>
the "terminate on newly-unsafe reload" logic never runs, because it
lived entirely inside the thing that was removed
    =>
the foreign user_bash handler dispatches unguarded
    =>
unguarded human mutation, on a worktree that may still carry a live
Claude/Codex/OpenCode/Pi scope
```

> The authority deciding whether guarded Pi attribution remains valid must exist outside, or below, the replaceable Pi extension set.
>
> SCE's own extension cannot be the sole watchdog for whether SCE is still present, first, or active after an ExtensionRunner replacement.
>
> If removing SCE also removes the enforcement mechanism, the mechanism is not a sound attribution boundary.

And retain, unchanged:

> The exact ExtensionRunner configuration actually in use must be authoritative.
>
> Independent re-resolution is not equivalent to observing or controlling the runner that Pi actually installed.

**Selected mechanism — Option A: the launcher owns `ResourceLoader` construction and freezes/governs the resolved extension array through two confirmed, first-class, publicly-exported Pi `0.80.6` constructor options, not a second, independently-timed resolver.** Inspecting pinned `0.80.6` directly (every path below is under `config/lib/node_modules/@earendil-works/pi-coding-agent`, the exact vendored copy of the version this plan already pins — confirmed by the installed `package.json`'s `"version": "0.80.6"`):

* `AgentSession` never constructs its own `ResourceLoader`; one is handed to it at construction (`dist/core/agent-session.js` line 132: `this._resourceLoader = config.resourceLoader;`) and reused, unreplaced, for the life of the process, including every `reload()`. `_buildRuntime()` builds `ExtensionRunner` from exactly `this._resourceLoader.getExtensions()` (`agent-session.js` lines 2002/2008: `const extensionsResult = this._resourceLoader.getExtensions(); ... this._extensionRunner = new ExtensionRunner(extensionsResult.extensions, ...)`), and `reload()` (lines 2023-2034) does nothing but `await this._resourceLoader.reload()` followed by the same `_buildRuntime()` call. The array `ExtensionRunner` is actually built from, on every rebuild, is always and only whatever the caller-supplied `ResourceLoader` instance returns — there is no separate, Pi-internal "real" resolution for a launcher to race against once the launcher itself supplies that instance.
* `DefaultResourceLoaderOptions` (`dist/core/resource-loader.d.ts`, an exported type consumed by the root-exported `createAgentSessionServices`/`createAgentSessionRuntime`/`createAgentSession` — `dist/index.d.ts` re-exports `createAgentSession, createAgentSessionFromServices, createAgentSessionRuntime, createAgentSessionServices` from `sdk.ts`, and the package's single `"."` export-map entry serves `dist/index.js`, so none of this requires reaching into an unexported deep path) carries two constructor-bound fields the launcher can set once, at session-construction time, for a session it itself hosts:
  * `extensionFactories?: InlineExtension[]`, where `InlineExtension = ExtensionFactory | { name; factory }` and `ExtensionFactory = (pi: ExtensionAPI) => void | Promise<void>` (`dist/core/extensions/types.d.ts` lines 1059-1065) — **exactly** the signature SCE's existing `.pi/extensions/sce/index.ts` factory already has; it can be passed here unmodified. `resource-loader.js`'s `loadFinalExtensionSet()` — the function `reload()` calls on every single invocation, not only at first load (line 270: `const extensionsResult = await this.loadFinalExtensionSet(extensionPaths, preTrustExtensions);`) — unconditionally calls `this.loadExtensionFactories(...)` and appends its result to `extensionsResult.extensions` (lines 366-372), sourced from `this.extensionFactories`, set once at construction (line 130: `this.extensionFactories = options.extensionFactories ?? [];`) and never affected by `.pi/settings.json`, the `.pi/extensions/` directory, or any other on-disk state. Passing SCE's existing factory function here means SCE's extension is present in the resolved set on every rebuild **because the launcher supplied the function directly, in-process** — there is no on-disk file to delete, rename, or exclude that could remove it, because on-disk discovery was never how it got there.
  * `extensionsOverride?: (base: LoadExtensionsResult) => LoadExtensionsResult` (`resource-loader.d.ts` line 78) — invoked unconditionally at the very end of `reload()` (`resource-loader.js` line 279: `this.extensionsResult = this.extensionsOverride ? this.extensionsOverride(extensionsResult) : extensionsResult;`), **after** CLI-path merging, rank-0..4 resolution, and the inline-factory append above — i.e. against the complete, final, already-assembled array, every single time `reload()` runs, not merely at first load. A launcher-owned override finds SCE's own entry (guaranteed present via `extensionFactories` above), moves it to array index `0`, and returns the reordered array; it fails closed (throws, which propagates out of `reload()`/`_buildRuntime()` as a rejected promise the launcher's own top-level code turns into a hard startup/reload failure) only in the degenerate case that constructing a safe array is impossible at all — i.e. SCE's own inline factory itself throwing when invoked.
* Because `emitUserBash()`, `tool_call`, and every other per-event dispatch (`extensions/runner.js`, confirmed above under "Confirmed exact dispatch evidence") iterate `this.extensions` — the exact array `ExtensionRunner` was constructed from — in array order, forcing SCE's entry to index `0` via `extensionsOverride` makes SCE first for every dispatch, for every launcher-admitted session, by construction, not by predicting Pi's own rank computation and hoping it agrees. This closes the CLI-provided-extension bypass, the rank-`0`-settings-entry bypass, and the same-rank readdir-order tie identically — none of them change whether SCE ends up first, because SCE's launcher, not Pi's own rank computation, has the last word on the returned array.
* `createRuntime` (the factory `createAgentSessionRuntime(createRuntime, options)` stores and re-invokes) is "reused for later `/new`, `/resume`, `/fork`, and import flows" per its own doc comment (`dist/core/agent-session-runtime.d.ts` line 20 area), so a launcher-supplied `createRuntime` closure that always sets the same `resourceLoaderOptions.extensionFactories`/`extensionsOverride` governs every session-replacement path Pi supports, not only `/reload`.
* Pi's own production entry point already uses exactly this composition — `createAgentSessionServices({ resourceLoaderOptions: {...} })` → `createAgentSessionRuntime(createRuntime, {...})` → `new InteractiveMode(runtime, {...})` (`dist/main.js` lines 489-598, 655) — and `InteractiveMode`, `runPrintMode`, `RpcClient`, `runRpcMode` are all exported from the package root (`dist/modes/index.d.ts`, re-exported via `dist/index.d.ts`). The launcher does not need to reimplement Pi's own TUI/print/rpc experience: it hosts the session, `main.js`-style, and hands off to Pi's own unmodified UI layer once the session exists.

This meets Option A's required property exactly: `approved extension set E ↓ Pi ExtensionRunner is created from E ↓ future reload cannot discover an arbitrary new E'` — `E` is whatever the launcher's `resourceLoaderOptions` produces, on every rebuild, and no on-disk mutation or CLI flag determines whether SCE is present or ordered first; those inputs are merely additional entries SCE's `extensionsOverride` may accept and reorder around, never a way to change whether SCE itself remains first. **The launcher determines what Pi is allowed to run; it does not predict what Pi will resolve** — no second resolver, no race, no closely-adjacent-but-not-identical timing window.

**Why this eliminates the startup/reload TOCTOU rather than minimizing it.** The retracted design ran two independent resolver executions — the launcher's pre-exec check and Pi's own real resolution — and, mid-session, the in-process factory check versus whatever `/reload` actually used — and argued they were "closely adjacent" and therefore safe enough. That reasoning is retracted; adjacency in time is not equivalence, and the concrete bypass above shows the gap is exploitable exactly when SCE's own code is what disappears. Under this correction there is exactly **one** array-producing code path per rebuild — `this._resourceLoader.getExtensions()` inside `_buildRuntime()`, fed by the launcher's own constructor-bound `extensionFactories`/`extensionsOverride` — and it is what `ExtensionRunner` is actually, synchronously, constructed from. There is nothing to race, because there is only one resolution, not two independently-timed ones.

**How `/reload` becomes sound rather than merely re-checked.** `reload()` (`agent-session.js` lines 2023-2034) calls `this._resourceLoader.reload()` — the *same* `ResourceLoader` instance, with the *same* constructor-bound `extensionFactories`/`extensionsOverride`, as initial load — then rebuilds `ExtensionRunner` from its result. Nothing about `/reload` replaces the `ResourceLoader` instance or its bound options; it only re-runs the same governed resolution against possibly-changed on-disk inputs, which SCE's `extensionsOverride` sees and re-enforces every single time, unconditionally. `/reload` may pick up legitimate new project content — new skills, new prompts, a newly-added compatible extension — but it cannot make SCE disappear from the array `extensionsOverride` returns, and it cannot place anything ahead of SCE in that array, because SCE's own launcher code is the only thing that ever writes that array's final order. `AgentSession.reload()` is covered identically, since it is the same code path.

**Downgrading the retracted design to defense-in-depth, not deleting it.** The launcher's earlier pre-exec ranked-order computation and SCE's extension's in-process self-check of its own dispatch position (both from "Corrected a fourth time" above) remain permissible as diagnostics — `sce doctor`/`sce setup --pi` may keep reporting a human-readable conflict, and the extension may keep asserting `extensions[0]` is itself as a cheap sanity check — but neither may be described, relied upon, or tested as the reason positive Pi attribution is sound. The soundness proof is the launcher's ownership of `ResourceLoader` construction; a self-check inside `sce-pi-extension.ts` is by definition unable to run once that extension is the thing missing, exactly as the concrete bypass above demonstrates, so it can never again be the primary proof.

**Raw Pi / SDK embedding — the permanent, unenforceable boundary, restated in terms of this mechanism, not weakened.** A user invoking the real `pi` binary directly, or an SDK caller constructing their own `AgentSession`/`ResourceLoader` without going through SCE's launcher's `createRuntime`/`resourceLoaderOptions` wiring, never has SCE's `extensionFactories`/`extensionsOverride` bound in at all — there is no process boundary or API surface that can force an external caller to use them. Such a session can run with a foreign extension anywhere in its own resolved order, unguarded `user_bash` included, regardless of how many diagnostic self-checks SCE's own code performs if it happens to be present at all (it can still correctly disable its own attribution, but per Hole 1's invariant that does not protect any other live scope on the same worktree). This plan does not attempt to close this boundary — doing so would require Pi to expose a way to refuse its own startup from inside extension code, or to force every embedder to use SCE's `ResourceLoader`, and neither exists. The precise, permanent scope statement:

```text
SCE guarantees Pi mutation attribution soundness — including
protection of concurrently live Claude/Codex/OpenCode/Pi scopes
from an unguarded Pi user_bash execution — only for Pi sessions
whose ResourceLoader was constructed by SCE's own launcher (the
extensionFactories/extensionsOverride wiring described above), for
the entire lifetime of that launcher-hosted process (including
every later reload/reinstantiation and every /new, /resume, /fork,
or import that reuses the same createRuntime factory).

A raw pi invocation or an SDK-embedded AgentSession that does not
use SCE's launcher-constructed ResourceLoader is an external mutator
outside that guarantee: it can invalidate concurrent worktree
attribution from any harness, and no AC in this plan may claim
otherwise.
```

Any AC or task text elsewhere in this plan asserting that merely disabling Pi's own scopes prevents contamination of Claude/Codex/OpenCode is retracted by this correction (see the rewritten AC22 and T06 below).

**Exclusivity — multiple `user_bash` invocations reuse the same lock, no new admission logic.** Pi's TUI already serializes `!`/`!!` execution to one in flight at a time (T01). If a mode ever allows a second concurrent `user_bash` on the same worktree, its supervisor's own `ProtectedWorktree::acquire` simply contends on the same `WorktreeLock` as the first supervisor and, on timeout, is treated as an ordinary guard-establishment failure by the mechanism above — command refused, no execution. This requires no reference counting, no tokens, and no admission code beyond what `WorktreeLock` already does.

**No new protocol or Quint semantics.** `database_failure`, `recover`, `Flush`, `CoordinateError::LockAcquisition`, and `CoordinateError::MarkerClearAfterCommit` are all pre-existing, actor-agnostic, and already exercised by Rust tests and MBT. This section changes *which process* spawns the human shell, *how* the OS-level lock's lifetime is made to durably imply the shell's lifetime (fd duplication, a runtime/OS-level mechanism, not a protocol one), and *what triggers* the existing recovery the runtime already performs — not what the protocol model represents. No `protocol.rs` or `spec/mutation_cursor.qnt` edit is required (see AC18). This PR still does not need to solve the separately deferred Bash-policy behavior for `!`/`!!` beyond this guard, nor does it need to solve the extension-dispatch-bypass hole by modifying Pi's own dispatch mechanism — only to refuse, at runtime and on every launch, to enable positive Pi mutation attribution when SCE cannot first prove its own `user_bash` handler is unavoidable (the launcher/env-var gate above), never merely to detect the problem and warn.

**Cross-process safety is explicit, not incidental.**

```text
Attribution safety is worktree-wide, not Pi-process-local, and it holds
for the entire dynamic lifetime of the ACTUAL SHELL PROCESS — not the
supervisor's lifetime, not the control channel's lifetime, and not
merely from the instant the command begins.

No correctness argument for user_bash may depend on an in-memory flag,
a PID check, a timestamp, or a TTL. The only facts any other process
can rely on are: (1) while the OS lock is held — by the supervisor, by
the shell via its inherited duplicate descriptor, or both — no
coordinate() call anywhere can proceed past
ProtectedWorktree::acquire_inner; (2) once the lock is free, either the
supervisor completed its finish sequence cleanly (marker cleared) or it
did not (marker still armed, self-healed by the existing
inherited-taint path); (3) this guarantee exists only for a user_bash
invocation SCE's extension actually observed, which itself exists only
for a Pi process whose ResourceLoader SCE's own launcher constructed —
extensionFactories guaranteeing SCE's extension is present on every
resolution regardless of on-disk state, extensionsOverride guaranteeing
it is placed first on every resolution, both constructor-bound to the
one ResourceLoader instance reused unreplaced for the life of the
process, across every /reload, AgentSession.reload(), /new, /resume,
and /fork. There is no reinstantiation that can "fail a check" and
require terminating the process, because there is no window in which
an unsafe array can be constructed at all for a launcher-hosted
session. See "Corrected a fifth time" above for the exact mechanism,
its required invariant, and its one named permanent limitation (a raw
pi invocation, or SDK embedding that does not use SCE's launcher's
ResourceLoader construction — worktree-wide safety is not guaranteed
for either).
```

### D14 — Detached descendants remain an explicit limitation

A foreground Pi Bash tool can potentially launch a child process that survives the Bash tool's own completion.

If the pinned Pi runtime provides no structured lifecycle proving all descendants are dead, `tool_execution_end` cannot prove that a self-detached descendant has stopped mutating.

Do not attempt to infer this from Bash command text. Do not build a shell parser or static background-process detector in this PR.

Record the exact observed behavior in T01 and document the residual attribution boundary.

**Revisited under D13's corrected lifetime-guard mechanism.** For a `user_bash` execution specifically (not a foreground Pi tool's own Bash), D13's fd-duplication mechanism changes this boundary rather than merely restating it: ordinary Unix fd inheritance means a descendant a shell spawns — including one that backgrounds or otherwise detaches from the shell itself (`cmd &`, `nohup`, `disown`) — inherits the same duplicated lock file descriptor unless it, or the shell, explicitly closes it or marks it close-on-exec. In that ordinary case, protection naturally extends to the descendant: the `WorktreeLock` stays held, by the same kernel guarantee, for as long as that descendant keeps the inherited descriptor open, even after the foreground shell itself has exited.

The exact residual limitation this does **not** cover: a descendant that deliberately closes the inherited descriptor — for example, a fully-daemonizing process that closes all open file descriptors above 2 as part of its own detachment routine (a common pattern for long-running background daemons), or one that execs into a program which itself closes non-standard inherited fds — escapes kernel-enforced tracking at the moment it closes that descriptor, even though it keeps running and can keep mutating the checkout. SCE has no way to distinguish this from genuine termination without parsing process behavior, which this plan does not attempt (same prohibition as the general D14 rule above: no Bash-text parsing, no static background-process detector). This is an accepted, explicitly documented boundary of the fd-inheritance mechanism, not a defect introduced by choosing it — no mechanism available to an extension-based integration without kernel-level process supervision (e.g. a container/cgroup boundary) can close this gap, and building one is out of scope for this PR.

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

- [ ] AC1: Exact lifecycle evidence exists for Pi `0.80.6`, covering `tool_call`, `tool_execution_start`, `tool_execution_end`, `tool_result`, blocking, handler failure, tool failure, interruption, session lifecycle, process death, model observation, extension ordering, and concurrency — including the frozen `tool_execution_start`-before-`tool_call` ordering and the `tool_result`-gates-execution rule (D5/D6/D7).
  - Validate: satisfied by T01's committed fixtures/report (`cli/src/services/hooks/pi_mutation_scope/fixtures/`) with exact Pi version, upstream commit, environment, and event sequences; `/validate` re-confirms this AC against the final implementation, not merely against T01's evidence.
- [ ] AC2: `bash`, `edit`, and `write` each establish one independently identified Pi mutation scope before their mutation-capable execution begins.
  - Validate: adapter tests plus live/runtime fixtures.
- [ ] AC3: `read`, `grep`, `find`, `ls`, `user_bash`, and representative unknown/custom tools create no Pi mutation scope.
  - Validate: zero-footprint classification and runtime tests.
- [ ] AC4: failure to establish a tracked Pi Start blocks the tool before execution.
  - Validate: live probe where the adapter fails and an observable filesystem mutation never occurs.
- [ ] AC5: a Pi scope cannot create positive mutation attribution until its own confirming post-execution Close, where Close is keyed on the `tool_result`-then-`tool_execution_end` pairing (D6), never on raw `tool_execution_end`.
  - Validate: Rust protocol tests plus Quint Pi confirmation-required cases.
- [ ] AC6: an unconfirmed Pi scope suppresses positive attribution at Claude, Codex, OpenCode, Pi, and Flush boundaries.
  - Validate: protocol/MBT/Quint cross-harness tests.
- [ ] AC7: a confirming Pi Close (the D6 `tool_result`-then-`tool_execution_end` pairing) can produce `AiExclusive(Pi)` when it is the only safe live scope and `AiContended` when overlapping confirmation-safe scopes remain.
  - Validate: Rust/Quint reachability tests.
- [ ] AC8: an earlier extension or SCE bash policy rejecting a tool before SCE Start creates no scope; a later extension rejecting after SCE Start cannot create positive attribution and is eventually conservatively recovered. A later-extension rejection after a successful SCE Start produces `tool_execution_end` with no preceding `tool_result` for that `toolCallId` (D7); the adapter must abandon, never Close, on that exact signal.
  - Validate: pinned-runtime ordering fixtures plus adapter/runtime regression; fixtures assert the exact `tool_execution_end`-without-`tool_result` pairing, not a broader heuristic.
- [ ] AC9: a tracked tool that executes and then reports `isError` still produces `tool_result` (proving execution occurred, per D6) and observes its final Git tree through the same `tool_result`-gated terminal boundary as success.
  - Validate: partial-mutation-then-error regression.
- [ ] AC10: simultaneous or overlapping Pi calls remain separate scopes and terminal cleanup of one never implicitly retires another.
  - Validate: concurrency adapter/runtime test.
- [ ] AC11: a lost or failed terminal boundary cannot later be replayed as if its observation happened at recovery time.
  - Validate: injected terminal seam failure followed by another filesystem mutation; recovery must discard/rebaseline the ambiguous interval instead of attributing it.
- [ ] AC12: stale-process cleanup requires positive process-death evidence and never uses TTL, age, session identity, or ActorKind alone.
  - Validate: live-owner vs dead-owner durable-state tests.
- [ ] AC13: Pi Start provenance stores canonical `pi_<sessionID>` plus the exact observed normalized model, or `NULL` when unavailable.
  - Validate: real repository Agent Trace DB and final Agent Trace regressions.
- [ ] AC14: existing Pi Bash policy, conversation tracing, edit/write diff tracing, generated extension installation, and doctor behavior remain intact.
  - Validate: existing Pi/config-lib tests, setup smoke, doctor smoke, and generated-output validation.
- [ ] AC15: only confirmed exclusive Pi evidence reaches `mutation_ai_patch`; blocked, ambiguous, unconfirmed, abandoned, custom/unknown, and recovery intervals do not.
  - Validate: real Git/DB production-path tests.
- [ ] AC16: cross-harness Pi overlap obeys the generalized mutation protocol, at minimum Pi+Claude, Pi+Codex, Pi+OpenCode.
  - Validate: production-path tests against the OpenCode adapter already present in the stacked base (see **Stack and base**), plus Rust/Quint cross-harness tests.
- [ ] AC17: no new Agent Trace schema or mutation-trace SQL migration is introduced.
  - Validate: baseline diff over schema/migration paths is empty.
- [ ] AC18: the protocol/Quint semantic change is limited to adding the Pi case to the already-generalized confirmation-required predicate. This also covers D13's external-mutation guard: it reuses the existing `ProtectedWorktree`/`WorktreeLock`/`ExternalTaintMarker`/`database_failure`/`recover` primitives unchanged, held for a longer, explicitly-terminated interval whose lifetime is anchored to the actual shell process (via the supervisor spawning it directly and, on Unix, fd-duplicating the lock into it), and adds no new protocol.rs or Quint code; the new long-lived supervisor invocation the mechanism requires (T03) lives in the runtime/ingress layer (`cli/src/services/mutation_trace/runtime/`, `cli/src/services/hooks/mutation_scope.rs`), outside this baseline-diff scope entirely.
  - Validate: targeted baseline diff over `protocol.rs`, `spec/mutation_cursor.qnt`, its documentation, and MBT/refinement surface, showing only Pi-shaped additions.
- [ ] AC19: on every platform where guarded Pi `user_bash` attribution is supported, every mutation performed by a `user_bash` execution occurs inside one worktree-wide external-mutation guard whose lifetime spans the entire lifetime of the actual shell process (not the supervisor's, and not the control channel's), and no AI boundary can make any part of that guarded interval positively attributable, for any live harness scope on that worktree (D13's corrected lifetime invariant). On Windows, where guarded `user_bash` attribution is explicitly unsupported for this PR, `user_bash` is unconditionally refused rather than guarded — see the Windows-refusal bullet below — and this AC's guard-lifetime claims apply only to the Unix mechanism.
  - Validate, successful guard, single write: arm the guard while a Pi scope and at least one other-harness scope (Claude, Codex, or OpenCode) are both live and mutating on the same worktree; let the human command execute and mutate; end the guard; then trigger a boundary from the *other* harness's scope (not Pi's own) and assert the forced recovery abandons every live worktree scope before that boundary is evaluated, that neither scope reaches `AiExclusive`/`AiContended` over the guarded interval, and that the interval is excluded from `mutation_ai_patch` (matching AC15).
  - Validate, the mid-command race: with the guard active and a human write already made (write #1), have a foreign harness's boundary attempt to run *while the guard is still active* and assert it does not proceed — it observes `CoordinateError::LockAcquisition` (or the adapter's own conservative retry-later handling of it) and neither reads, mutates, nor clears any protocol or taint state; let a second human write occur (write #2) before the guard ends; end the guard; assert both writes remain excluded from positive AI attribution and no live scope reached `AiExclusive`/`AiContended` for any part of the interval spanning either write.
  - Validate, supervisor dies while the shell is still running (Unix): after the guard is armed and the shell has produced at least one write, `SIGKILL` the supervisor process directly while the shell keeps running; assert `WorktreeLock` remains held (a concurrent foreign `coordinate()` call still blocks/fails closed with `CoordinateError::LockAcquisition`, exactly as if the supervisor were alive) for as long as the shell (or a descendant holding the duplicated fd) is alive; let the shell make a second write and then exit; assert the lock frees only once the shell exits, that `ExternalTaintMarker` is still armed at that point, and that the very next `coordinate()` call on that worktree — from any harness — self-heals via the existing unmodified inherited-taint path before processing its own boundary; assert both writes remain excluded from positive AI attribution.
  - Validate, Pi/Node dies while the shell is still running: kill the Pi/Node control-channel process while the guard is active and the shell is still running; assert the supervisor does not treat this as a finish signal, does not kill the shell, and keeps holding the lock/marker; let the shell make a further write and then exit normally; assert the supervisor still runs its normal finish sequence (forced recover, `complete()`, release) with nothing listening on the dead control channel, and that every write remains excluded from positive AI attribution.
  - Validate, Windows refusal: on Windows, invoke `user_bash`; assert the handler unconditionally returns the `result` full-replacement (never `operations`), that `session.executeBash()` is never called, that no shell — supervised or otherwise — is ever spawned, and that the command's own exit/output is never delivered because it never ran; assert a concurrently live Pi `bash`/`edit`/`write` tracked-tool scope on the same Windows worktree is unaffected and can still separately reach `AiExclusive`/`AiContended` normally, proving the refusal is scoped to `user_bash` alone and does not disable tracked-tool attribution.
- [ ] AC20: after a `user_bash`-guarded interval ends and its forced recovery/rebaseline durably succeeds, a fresh, uninterfered-with Pi scope — and a fresh scope from any other harness whose boundary was deferred by the guard — can still reach `AiExclusive` (D13).
  - Validate: guard, recover (abandoning the live scope(s) and rebaselining), then run a clean tracked Pi tool to completion with no further interference; assert it reaches `AiExclusive` and lands in `mutation_ai_patch`. Also validate that a foreign-harness boundary that was deferred (AC19's mid-command-race case) succeeds normally once retried after the guard ends, and that a boundary deferred by the supervisor-death self-heal case above also succeeds normally once retried.
- [ ] AC21: if the worktree external-mutation guard cannot be durably established, the underlying Bash execution is never invoked, and the supervisor — not Pi/Node — is confirmed to be the process that actually spawns the real shell once the guard is established.
  - Validate: inject a guard-establishment failure (lock-acquisition timeout, marker-persistence failure, or spawn failure) while a Pi scope (and, in at least one variant, an other-harness scope) is live on the worktree; assert the `user_bash` handler returns Pi's `result` full-replacement (never `operations`), that no real shell is ever spawned by either Pi/Node or a supervisor, that an observable shell mutation never occurs, and that the live scope(s) are unaffected because no human mutation was ever introduced.
  - Validate, ambiguous begin acknowledgement: have the supervisor durably acquire the lock and arm the marker while the caller's acknowledgement is lost or delayed past its bound, before any shell has been spawned; assert the command is still blocked (never executed on an uncertain result) and that the caller terminates the orphaned supervisor process so the lock is promptly released (no shell exists yet to hold a duplicated fd in this window); assert a later boundary on that worktree conservatively recovers/rebaselines it anyway — an accepted false negative (unnecessary abandonment), never a safety violation.
  - Validate, guard finalization failure: force the forced-recovery commit at guard-finish time to fail; assert `complete()` is never called, the marker remains armed, and the supervisor reports failure without claiming clean attribution; assert the next boundary on that worktree self-heals via the existing inherited-taint recovery path.
  - Validate, the real shell's parent is the supervisor: assert the spawned shell process's parent pid is the supervisor's pid (not Pi/Node's), confirming Pi/Node's wrapped `exec()` never itself calls `createLocalBashOperations()`/spawns a shell once a guard exists.
- [ ] AC22: positive Pi mutation attribution cannot become active when `user_bash` interception is bypassable, **and** an unsafe `user_bash` dispatch configuration can never introduce an unguarded human mutation into a worktree for which SCE still claims attribution safety — for any live harness scope on that worktree, not only Pi's own. This is stronger than "confirmed and enforced or explicitly documented" — a warning alone never satisfies this AC, and "no Pi scope was established" never satisfies this AC by itself (see D13's Hole 1 correction: disabling Pi's own attribution does not protect another harness's live scope). AC22 proves three separate things — AC22a (a launcher-hosted session's extension array cannot become unsafe at all, because the launcher owns the array's construction, not merely its admission check), AC22b (the safety authority survives every runner rebuild for the process's entire lifetime and cannot be removed by removing SCE's own Pi extension, because that authority was never inside the extension to begin with), and AC22c (SCE present at startup, SCE absent from the naively-resolved set after a requested reload, still cannot produce an operational unsafe runner).
  - **AC22a — a launcher-hosted extension array cannot be unsafe (Hole 1, structurally, not by admission check).**
    - Validate, launcher-hosted session, positive path, no competing extension configured: construct a session via SCE's launcher (`createRuntime`, T05) with no CLI `-e`/`additionalExtensionPaths`, no rank-`0` project-settings entry, and no other project-auto-discovered extension; assert `resourceLoader.getExtensions().extensions[0]` is SCE's own extension (via `extensionFactories` + `extensionsOverride`, D13 "Corrected a fifth time"), and that the guarded `user_bash` handler and tracked-tool positive attribution are both enabled for that session.
    - Validate, a competing extension present at construction time never changes the outcome: register a second, foreign `user_bash`-hooking extension — separately as (a) a CLI/`-e`-provided path, (b) a rank-`0` project-settings-entry, and (c) a same-rank project-auto-discovered sibling whose readdir position would precede SCE's under Pi's own on-disk ranking — and construct the session via SCE's launcher in each configuration; assert in every case `resourceLoader.getExtensions().extensions[0]` is still SCE's own extension (the foreign extension is present in the array, just not first), the guarded `user_bash` handler still wins dispatch, and no unguarded execution occurs. This must not be argued from "the launcher detected and refused the launch"; there is no refusal step to argue from — argue instead from "the returned array was never anything other than SCE-first, because SCE's own launcher code produced it."
    - Validate, the cross-harness case: with a Claude/Codex/OpenCode scope already live on a worktree, construct a Pi session via SCE's launcher with a foreign extension configured in any of the three ways above; assert the resulting session still has SCE first, `user_bash` is never dispatched to the foreign extension, no human mutation is introduced, and the live Claude/Codex/OpenCode scope's eventual Close reaches its own correct, unrelated attribution outcome — prove this by asserting no unguarded `user_bash` dispatch occurred, not merely by asserting the scope's own outcome looked normal.
    - Validate, the one remaining fail-closed case is a construction failure, not a runtime detection: force SCE's own inline extension factory to throw when the launcher invokes it as part of `extensionFactories`; assert `resourceLoader.getExtensions()`/`reload()` rejects, the launcher's own top-level code treats this as a hard startup failure, and no `AgentSession`/`ExtensionRunner` is ever constructed or left running in a state where SCE is absent from the array.
    - Validate, bypassing the launcher is a named, permanent, worktree-unsafe boundary, not a safe fallback: construct a session directly against Pi's own `AgentSession`/`DefaultResourceLoader` (or invoke the real `pi` binary), bypassing SCE's launcher's `createRuntime`/`resourceLoaderOptions` wiring entirely, with the identical unsafe configuration from the prior bullets and a Claude/Codex/OpenCode scope live on the same worktree; assert the foreign extension can execute `user_bash` unguarded in this configuration, that this is **not** prevented by anything in this plan, and that this is recorded in T02's and T06's task records as a permanent, worktree-unsafe scope boundary (per D13's "Raw Pi / SDK embedding" disposition), never described as safe.
    - Validate, the SDK-embedding residual limitation is the same boundary, not a separate one: construct an `AgentSession`/SDK caller using its own `extensionsOverride` (on a `ResourceLoader` SCE's launcher did not build) to place a foreign `user_bash` handler first, entirely outside any construction path SCE's launcher governs; assert the same worktree-unsafe boundary applies as in the direct-invocation case above, recorded identically as permanent.
  - **AC22b — the authority survives every runner rebuild and cannot be removed by removing SCE's own Pi extension (Hole 2, generalized).**
    - Validate, safe startup then a `/reload` that changes nothing: construct a session via SCE's launcher; assert positive Pi attribution is available; trigger `/reload` with no configuration change; assert `resourceLoader.getExtensions().extensions[0]` is still SCE's own extension after the reload (the same `ResourceLoader` instance, same constructor-bound `extensionFactories`/`extensionsOverride`, per D13); assert positive attribution remains available and a subsequent clean tool call still reaches `AiExclusive`.
    - Validate, an on-disk reorder attempt across `/reload` never changes the outcome: construct a session via SCE's launcher; introduce a foreign `user_bash`-hooking extension ahead of where SCE would otherwise rank (any configuration from AC22a) without restarting the process; trigger `/reload`; assert `resourceLoader.getExtensions().extensions[0]` is still SCE's own extension after the reload, the foreign `user_bash` handler never executes unguarded, positive attribution remains enabled throughout, and the Pi process is never terminated merely because an on-disk reorder was attempted — there is nothing to terminate for, since the reorder never reached the array `ExtensionRunner` was rebuilt from.
    - Validate, the same reorder attempt with a concurrently live other-harness scope: repeat the prior bullet with a Claude/Codex/OpenCode scope live on the same worktree; assert that scope's eventual Close reaches its own correct attribution outcome, unaffected, because no unguarded `user_bash` dispatch from the reload ever became possible.
  - **AC22c — removing SCE's own Pi extension from the naively-resolved configuration cannot make an unsafe runner operational (the concrete bypass this correction exists to close).**
    - Validate: construct a session via SCE's launcher with a live Claude/Codex/OpenCode scope on the same worktree; assert SCE is present, first, and positive Pi attribution is available; then mutate the project's on-disk configuration so that Pi's own *naive* on-disk resolution — the array `DefaultResourceLoader` would compute from `.pi/settings.json`/`.pi/extensions/` alone, with no `extensionFactories`/`extensionsOverride` applied — excludes SCE's generated extension entirely (remove/rename/disable it) and adds a foreign extension that registers `user_bash`; trigger `/reload`; assert `resourceLoader.getExtensions().extensions` still contains SCE's own extension at index `0` (reinserted by `extensionFactories`, reordered by `extensionsOverride`, per D13), that the foreign `user_bash` handler is never dispatched ahead of SCE's, that no unguarded execution occurs, and that this holds specifically **because SCE's own extension code never had to run to detect or react to its own on-disk removal** — the assertion must be phrased as "the authoritative array-producing code (the launcher's `ResourceLoader`) is not itself a member of the resolved-from-disk set and therefore cannot be excluded by that set's mutation," not as "SCE's factory detected the removal."
    - Validate, Claude B live across the whole sequence: repeat the above with a live Claude scope (`B`) on the same worktree throughout; attempt `user_bash` after the reload and assert the foreign handler never executes unguarded and no human worktree mutation occurs; then `Close(B)`; assert `B`'s attribution outcome is correct and uncontaminated by any Pi-origin human mutation. Repeat/parameterize for Codex and OpenCode in place of Claude.
  - Validate, doctor is diagnostic only: run `sce doctor`/`sce setup --pi` against a configuration where naive on-disk resolution would place a foreign extension ahead of SCE, and assert it reports the same ordering fact for a human to read, but assert no code path anywhere treats a clean doctor run, or the presence/absence of a diagnostic self-check inside `sce-pi-extension.ts`, as the reason positive attribution is enabled for that session — the reason is always and only that the launcher's own `ResourceLoader` construction is what produced the array in use.
- [ ] AC23: control-process death (Pi/Node) never terminates or truncates a running human `user_bash` command, and never causes the guard to end before the actual shell terminates (D13's chosen Option A policy).
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
  reimplements.
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
  Bash-text detached-process detector.

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

- [ ] T02: `Make Pi a confirmation-required protocol actor` (status:todo)
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
    with exact file/line citations re-confirmed against the installed
    package, as this task's own frozen evidence record.
  - Verify: `nix run .#quint -- typecheck spec/mutation_cursor.qnt`;
    `nix run .#quint -- test spec/mutation_cursor.qnt`;
    `nix build .#checks.x86_64-linux.mutation-trace-quint-connect`;
    `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace`.
  - Context synchronization: pending

- [ ] T03: `Add the Pi mutation-scope adapter` (status:todo)
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
      Windows. The finish trigger is **exclusively the
      shell's own process termination** (`wait()`/`waitpid` on the spawned
      child) — never stdin EOF or any other signal on the control channel to
      Pi/Node, which may close independently of the shell's lifetime (D13's
      chosen Option A: control-channel death from Pi/Node dying does not
      finish the guard). Once the shell terminates, the supervisor runs the
      forced recovery/`complete()` sequence above, after which it emits a
      final result over the control channel (if anything is still listening)
      and exits — success only if the recovery commit and the marker clear
      both durably succeeded. A failure or ambiguous outcome at any point
      before the "armed" acknowledgement (i.e., before any shell has been
      spawned) must leave no lock held by this process (the process either
      never acquired it or exits, releasing it — no shell exists yet to hold
      a duplicated fd in that window). This needs no `scope_id`/`event_id` —
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
    `user_bash` call site that spawns/manages the supervisor process (T05);
    the `user_bash` extension-dispatch-order launcher/env-var gate and doctor
    check (T02 records the disposition, T05 implements both — AC22); any
    Windows-specific supervisor/guard code (D13's Windows disposition is
    unconditional refusal at the extension level, T05, not a Rust-side
    platform branch T03 needs to implement).
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
    does not signal the shell; finish is triggered exclusively by the shell's
    own process termination and forces `database_failure`+`recover` against
    the already-held `ProtectedWorktree`, only then clearing the marker; a
    failed finish commit leaves the marker armed and reports failure without
    calling `complete()`; all with no `protocol.rs` involvement.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml pi_mutation_scope`;
    `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml hooks::`.
  - Context synchronization: pending

- [ ] T04: `Add sound terminal and stale-process recovery` (status:todo)
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
  - Context synchronization: pending

- [ ] T05: `Wire mutation scope into the existing Pi extension` (status:todo)
  - Task ID: T05
  - Scope: In — modifying the canonical Pi extension source
    `config/lib/pi-plugin/sce-pi-extension.ts` (not a second project-local SCE
    extension) to register mutation lifecycle handlers in the frozen T01 order
    (bash policy -> mutation Start -> edit/write diff pre-image) plus the
    frozen execution/terminal lifecycle handlers, forwarding `tool_call`
    (Start), `tool_result` (execution evidence), and `tool_execution_end`
    (Close, gated on a prior `tool_result`) — `tool_execution_start` may still
    be forwarded for telemetry but participates in no attribution state
    (D5/D6/D7). Also registers a `pi.on("user_bash", ...)` handler using Pi's
    **actual** pinned `0.80.6` API — established directly from
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
    Under the corrected D13 architecture, `operations` is **never**
    `createLocalBashOperations()` or a wrapper around it — the real shell must
    be spawned by the supervisor (T03), not by Pi/Node, so `wrappedOperations`
    is a thin control-channel client:

    ```text
    pi.on("user_bash", async (event) => {
        if process.platform === "win32":
            // D13's corrected Windows disposition — unconditional refusal,
            // not a transient failure. No supervisor is ever spawned; this
            // is the same code path as the establishment-failure branch
            // below, taken unconditionally, every time, on this platform.
            return { result: { output: "SCE does not support guarded
                user_bash execution on Windows in this release; run this
                command outside Pi.", exitCode: 1, cancelled: false,
                truncated: false } }

        if not dispatchSafe:
            // dispatchSafe only matters for the legacy/bypass path: a
            // session that discovered this factory from disk without going
            // through SCE's launcher (D13 "Corrected a fifth time"). For a
            // launcher-hosted session (this factory supplied via
            // extensionFactories and reordered first via extensionsOverride
            // — see T05's primary mechanism below), dispatchSafe is always
            // true by construction and this branch is dead code reached
            // only diagnostically; it is not what makes a launcher-hosted
            // session safe. Also disables tracked-tool (bash/edit/write)
            // Start handling for this session — not merely user_bash — per
            // AC22, but per D13's Hole 1 invariant this withholds only
            // THIS session's own attribution and must never be described as
            // protecting a concurrently live Claude/Codex/OpenCode scope.
            return { result: { output: "SCE could not confirm its
                user_bash handler is dispatch-safe for this session; run
                Pi through the sce-provided launcher, or see `sce doctor`
                for the detected conflict.", exitCode: 1, cancelled: false,
                truncated: false } }

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

    **`dispatchSafe` above is retired as an env-var/self-check gate for the
    launcher-hosted path (D13 "Corrected a fifth time" — Option A supersedes
    Option C as the primary proof).** For a session constructed through
    SCE's own launcher (below), SCE's extension is supplied via
    `extensionFactories` and placed first via `extensionsOverride`
    unconditionally, so by the time the factory function runs at all it is
    already known to be first, by construction, for this invocation and
    every future one on the same `ResourceLoader` instance — there is
    nothing left for the extension itself to check or race. The `user_bash`
    handler's guard-establishment branch (spawn supervisor, await armed
    acknowledgement, etc.) therefore runs unconditionally in the
    launcher-hosted factory, with no `dispatchSafe`/environment-variable
    branch guarding it, and tracked-tool (`bash`/`edit`/`write`) Start
    registration is unconditional in the same factory invocation for the
    same reason.

    A second, legacy code path remains for compatibility: `.pi/extensions/sce/index.ts`
    continues to exist on disk (generated exactly as today) so that a
    session constructed **without** SCE's launcher — a raw `pi` invocation,
    or an SDK caller using its own `ResourceLoader`/`DefaultResourceLoader`
    — that happens to discover SCE's extension from disk still gets it
    loaded and can still run its **existing** in-process, per-factory-invocation
    self-check (reusing the same vendored `resource-loader`/`package-manager`
    resolution entry points) as a best-effort, diagnostic-grade fallback:
    if that self-check finds SCE is not first, it withholds its own
    `user_bash` guard and Start registration for that session. This fallback
    is **explicitly named as not a safety boundary** (per D13's Hole 1
    invariant: disabling Pi's own attribution does not protect a
    concurrently live Claude/Codex/OpenCode scope) — it only avoids a
    worse outcome (Pi falsely claiming AI attribution for itself) for
    sessions this plan already cannot make worktree-safe. T05 must not
    describe this fallback as closing the raw-`pi`/SDK-embedding boundary.

    Primary mechanism this task owns (per T02's recorded disposition — AC22,
    D13 "Corrected a fifth time"), replacing the exec-a-real-`pi`-binary
    launcher design an earlier version of this plan specified (retracted —
    a launcher that merely decides whether to exec `pi` cannot survive
    SCE's own extension being removed from what gets exec'd):

    * **SCE's launcher becomes the Pi host process itself**, built on Pi
      `0.80.6`'s own exported SDK — it does not exec a separate `pi` binary
      for a guarded session. It mirrors Pi's own production wiring
      (`dist/main.js` lines 489-598, 655: `createAgentSessionServices(...)`
      → `createAgentSessionRuntime(createRuntime, {...})` →
      `new InteractiveMode(runtime, {...})`, all root-exported per
      `dist/index.d.ts`/`dist/modes/index.d.ts`), substituting SCE's own
      `resourceLoaderOptions` into the `createRuntime` closure:
      ```text
      resourceLoaderOptions: {
          ...(the launcher's own CLI-flag/settings passthrough, unchanged),
          extensionFactories: [
              ...(any factories Pi's own CLI wiring already supplies),
              sceExtensionFactory,   // the SAME factory function already
                                     // exported by config/lib/pi-plugin/
                                     // sce-pi-extension.ts — unmodified
          ],
          extensionsOverride: (base) => sceEnforceExtensionOrder(base),
      }
      ```
      `sceEnforceExtensionOrder` locates SCE's own entry in `base.extensions`
      (guaranteed present because it was supplied via `extensionFactories`,
      independent of on-disk discovery — D13), moves it to index `0`, and
      returns the reordered array; it throws only in the degenerate case
      that SCE's own factory itself failed (propagating as a hard
      startup/reload failure, never a silent unsafe continuation). Print
      mode (`runPrintMode`) and RPC mode (`runRpcMode`) use the identical
      `resourceLoaderOptions`, since the seam is at `ResourceLoader`
      construction, not at the interactive UI layer — a guarded session is
      guarded regardless of which of Pi's own exported entry points renders
      it.
    * Packaging (a new `sce-pi`/`sce pi` entry point vs. a `PATH`-shadowing
      `pi` wrapper vs. a shell function SCE asks the user to source) is
      T05's to determine and document; whichever is chosen, it must be the
      thing that actually constructs the session (per the above), not a
      thing that decides whether to launch a separately-resolving `pi`
      process.
    * `sce doctor`/`sce setup --pi` continue to run the existing
      naive-on-disk-resolution check **standalone, for human-readable
      reporting only** ("another extension is configured ahead of where SCE
      would rank at `<path>` under naive on-disk resolution; this is
      informational only — sessions started through the sce-provided
      launcher are unaffected because the launcher constructs the extension
      array directly") — this output remains explicitly diagnostic; no code
      path may treat a clean doctor run, on its own, as the reason positive
      attribution is enabled for an actual session.
    * **Permanent, named scope boundary (not a defect to close later):** the
      launcher-owned `ResourceLoader` construction protects the worktree
      only for sessions whose `AgentSession` was actually built by SCE's
      launcher's `createRuntime` closure, for the entire lifetime of that
      process (including every later `/reload`/`AgentSession.reload()`/
      `/new`/`/resume`/`/fork`, since they all reuse the same closure). A
      user invoking the real `pi` binary directly, or any SDK-embedding
      caller constructing its own `AgentSession`/`ResourceLoader` without
      this wiring, bypasses it entirely: the legacy on-disk-discovered
      extension's own diagnostic self-check may still withhold Pi's own
      attribution, but per D13's Hole 1 invariant this does **not** make
      the worktree safe — a foreign extension in that configuration can
      still execute `user_bash` unguarded and can still contaminate a
      concurrently live Claude/Codex/OpenCode scope. T05's setup output and
      documentation must state this plainly, as a named, permanent,
      worktree-unsafe boundary, never as "safe, but with no Pi attribution."

    Existing mutation Start ordering remains unchanged. Synchronous
    fail-closed Start transport, terminal transport that never pretends the
    tool did not run on post-execution transport failure (D9's
    unresolved-terminal guard), and preservation of existing Bash policy,
    conversation trace, edit/write diff trace, message trace, Pi session
    prefix behavior, and tool-version resolution. Using the existing
    generated Pi extension pipeline (`config/lib` / Pkl sources) — no
    hand-edited generated copies; the generated factory function is reused
    unmodified as both the disk-discovered extension (legacy/compatibility
    path) and the in-process `extensionFactories` entry (primary,
    launcher-hosted path) — it is one function, registered two ways. Also
    in scope — the launcher host program itself (packaging;
    `createAgentSessionServices`/`createAgentSessionRuntime`/`InteractiveMode`/
    `runPrintMode`/`runRpcMode` wiring; the `resourceLoaderOptions.extensionFactories`/
    `extensionsOverride` composition and its fail-closed-on-factory-failure
    behavior); retaining the existing in-process self-check inside
    `sce-pi-extension.ts` as an explicitly-diagnostic fallback for the
    legacy/bypass discovery path only; wiring the naive-resolution check
    into `sce doctor`/`sce setup --pi` as diagnostic-only reporting; the
    unconditional Windows-refusal branch in the `user_bash` handler.
    Out — any Rust adapter change beyond what T03/T04 already produced;
    implementing the supervisor's own shell-spawn logic itself (T03); any
    Windows-side Rust/supervisor code (none exists — refusal is entirely a
    TypeScript-extension-level branch); reimplementing Pi's own TUI/print/rpc
    rendering (reused unmodified via `InteractiveMode`/`runPrintMode`/`runRpcMode`).
  - Dependencies: T04
  - Done when: a real `sce setup --pi` installation routes Pi's mutation
    lifecycle through the Rust adapter while all existing Pi integration
    behavior remains intact; Bun tests (mocked subprocess transport) cover
    bash-policy-denial-means-no-Start, tracked-Start-success,
    tracked-Start-adapter-failure-blocks, missing-`sce`-blocks,
    read-only/unknown-tool-means-no-adapter-call, `tool_result`-keyed
    execution-evidence forwarding/state (never `tool_execution_start`),
    successful/failed `tool_execution_end` gated on a prior `tool_result`,
    `tool_execution_end`-without-`tool_result` abandon, terminal transport
    failure,
    `user_bash`-returns-`operations`-and-relays-to-the-supervisor-rather-than-spawning-a-shell-itself,
    `user_bash`-returns-`result`-full-replacement-and-never-calls-`session.executeBash`-on-guard-establishment-failure,
    `user_bash`-returns-`result`-full-replacement-and-terminates-the-orphaned-supervisor-process-on-an-ambiguous-acknowledgement,
    wrapped-`exec`-forwards-cancellation/timeout-to-the-supervisor-rather-than-killing-a-local-child,
    wrapped-`exec`-resolves-only-on-the-supervisor's-own-delivered-exit-result-never-merely-on-control-channel-closure,
    `user_bash`-unconditionally-refused-on-Windows-with-no-supervisor-spawn,
    the-launcher-hosted-session's-`resourceLoader.getExtensions().extensions[0]`-is-SCE's-own-extension-in-every-tested-competing-configuration-(CLI-provided,-rank-0-settings,-same-rank-readdir-tie,-and-SCE-entirely-removed-from-on-disk-configuration),
    the-same-holds-after-`/reload`-with-the-on-disk-configuration-mutated-to-try-to-exclude-or-outrank-SCE-between-construction-and-reload,
    a-factory-failure-inside-`extensionFactories`-fails-the-session-construction/reload-closed-rather-than-continuing-with-SCE-absent,
    the-legacy-disk-discovered-extension's-self-check-withholds-only-its-own-attribution-and-is-never-asserted-to-protect-another-harness's-scope,
    `sce doctor`/`sce setup --pi`-report-the-same-conflict-for-humans-but-are-never-consulted-by-the-runtime-mechanism-itself,
    model present/absent, session canonicalization, and unchanged edit/write-diff
    and conversation tracing.
  - Verify: `nix run nixpkgs#bun -- test config/lib`; `nix run .#pkl-check-generated`;
    `nix flake check`; plus scratch setup/doctor smoke; plus a scratch smoke
    constructing a session through the new launcher host against both a
    clean and a deliberately-competing extension configuration, asserting
    `extensions[0]` is SCE's own extension in both cases; plus a scratch
    smoke that starts safely, mutates the on-disk extension configuration to
    exclude SCE entirely and add a foreign `user_bash`-registering
    extension, triggers `/reload`, and asserts `extensions[0]` is still
    SCE's own extension and the foreign handler never dispatches.
  - Context synchronization: pending

- [ ] T06: `Add production-path and live Pi attribution regressions` (status:todo)
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

    Also test the extension-array-authority fail-closed guarantee this
    plan's D13 correction requires (AC22a/AC22b), replacing both the
    doctor-warning-only test and the launcher-refuses-to-exec-`pi` test
    earlier versions of this plan specified. **No variant below may be
    argued from "no Pi Start was ever established" (retracted, D13's Hole 1
    invariant) or from "the launcher refused to launch `pi`" (retracted,
    D13's Option C is no longer the mechanism). Every variant must instead
    assert directly on the array `ExtensionRunner` was actually built from
    — `resourceLoader.getExtensions().extensions` — never on a separately
    predicted array, per the "exact-runner authority" requirement below.**

    ```text
    variant — launcher-hosted session, no competing extension:
    construct a session via SCE's launcher (createRuntime +
    resourceLoaderOptions.extensionFactories/extensionsOverride, T05);
    assert resourceLoader.getExtensions().extensions[0] is SCE's own
    extension; assert the guarded user_bash handler and tracked-Start
    handling for bash/edit/write are both enabled; a clean tool call
    reaches AiExclusive normally

    variant — a competing extension present at construction never wins,
    each configuration separately: (a) a CLI/-e-provided competing
    extension, (b) a rank-0 project-settings-entry competing extension,
    (c) a same-rank project-auto-discovered sibling positioned ahead of
    SCE's by readdir order:
    construct a launcher-hosted session in each configuration; assert
    resourceLoader.getExtensions().extensions[0] is still SCE's own
    extension in every case (the competing extension is present in the
    array, just not first); attempt user_bash; assert SCE's guarded
    handler dispatches, not the foreign one; assert sce doctor/setup,
    run separately, still reports the naive-on-disk-resolution conflict
    for a human to read, unrelated to the actual (safe) outcome

    variant — cross-harness, no contamination because the array was
    never unsafe, not because no Pi Start was established: repeat the
    prior variant with a Claude/Codex/OpenCode scope already live on
    the same worktree; assert that scope's eventual Close reaches its
    own correct, unrelated attribution outcome — prove this by
    asserting the foreign extension's user_bash handler was never
    dispatched, not merely by asserting mutation_ai_patch excludes some
    interval

    variant — the one remaining fail-closed case is construction
    failure, not runtime detection: force SCE's own extensionFactories
    entry to throw when the launcher invokes it; assert session
    construction (or reload) rejects, the launcher's top-level code
    treats this as a hard failure, and no AgentSession/ExtensionRunner
    is ever left running with SCE absent from the array

    variant — launcher bypassed entirely, a named worktree-unsafe
    boundary, not a safe fallback: invoke the real pi binary directly,
    or construct an AgentSession/DefaultResourceLoader directly, with a
    conflicting configuration and a Claude/Codex/OpenCode scope live on
    the same worktree; if SCE's legacy on-disk-discovered extension
    happens to load, assert its own diagnostic self-check correctly
    withholds its own Start/user_bash registration; but assert — and
    record in this task's task record, do not treat as fixed — that the
    foreign extension DOES execute user_bash unguarded in this
    configuration, and that this plan does not prevent that mutation
    from potentially being folded into the live other-harness scope's
    eventual attribution; this is the named, permanent, worktree-unsafe
    residual (D13's "Raw Pi / SDK embedding" disposition), asserted as
    a documented boundary, never as "safe with reduced coverage"

    variant — SDK extensionsOverride on a non-launcher-hosted session,
    the same permanent residual limitation, not a separate one:
    construct an AgentSession/SDK caller using its own extensionsOverride
    (on a ResourceLoader SCE's launcher did not build) to place a
    foreign user_bash handler first; assert the same worktree-unsafe
    boundary applies as in the direct-invocation case above, recorded
    identically as permanent
    ```

    Also test the reload/rebuild-lifetime guarantee D13's Hole 2 invariant
    requires (AC22b) — a regression the previous version of this plan did
    not have, since it treated a launch-time attestation as valid for the
    whole session:

    ```text
    variant — safe startup, reload with no configuration change:
    construct a launcher-hosted session; assert positive Pi attribution
    is available; trigger /reload with nothing on disk changed; assert
    resourceLoader.getExtensions().extensions[0] is still SCE's own
    extension after the reload (the same ResourceLoader instance, same
    bound extensionFactories/extensionsOverride); assert positive
    attribution remains available and a subsequent clean tool call
    still reaches AiExclusive

    variant — safe startup, an on-disk reorder attempt, reload,
    attempted mutation, with a live other-harness scope:
    construct a launcher-hosted session with a Claude/Codex/OpenCode
    scope already live on the same worktree; introduce a foreign
    user_bash-hooking extension ahead of where SCE would naively rank
    on disk (any configuration from the array-authority variants above)
    without restarting the Pi process; trigger /reload; assert
    resourceLoader.getExtensions().extensions[0] is still SCE's own
    extension after the reload; assert the foreign user_bash handler
    never dispatches, the Pi process is never terminated merely because
    an on-disk reorder was attempted (there is nothing unsafe to
    terminate for — the reorder never reached the array
    ExtensionRunner was built from), and positive attribution remains
    enabled throughout; assert the live other-harness scope's eventual
    Close reaches its own correct, unaffected attribution outcome
    ```

    Also test the SCE-removal regression (AC22c) — the concrete bypass this
    correction exists to close, and the specific class the human's brief
    requires as load-bearing, not merely a variant of the reorder case
    above, because it proves the array-producing mechanism does not depend
    on SCE's own extension code running at all:

    ```text
    safe guarded Pi startup through SCE's launcher
        => positive attribution available

    Claude B live on the same worktree

    modify Pi's on-disk extension configuration so that Pi's own naive
    on-disk resolution for the next runner:
        - excludes SCE's generated extension entirely (removed, renamed,
          disabled — not merely reordered)
        - includes a foreign extension that registers user_bash

    trigger /reload

    assert:
        resourceLoader.getExtensions().extensions still contains SCE's
            own extension, at index 0 (reinserted by extensionFactories,
            reordered by extensionsOverride — neither of which read the
            on-disk extension list to decide SCE's presence)
        the authoritative external mechanism (the launcher's
            ResourceLoader construction) prevented the unsafe
            naively-resolved runner from ever becoming operational —
            there is no window in which ExtensionRunner was built from
            an array missing SCE

    attempt user_bash

    assert:
        the foreign handler never executes unguarded
        no human worktree mutation occurs

    Claude Close(B)

    assert:
        no Pi-origin human mutation contaminates B's attribution outcome
    ```

    Repeat/parameterize this exact regression for Claude, Codex, and
    OpenCode in place of B. The test must remove SCE from the on-disk
    configuration completely, not merely reorder another extension ahead
    of it — reordering alone is already covered by the prior variants, and
    this class is what the earlier launcher-refusal/in-process-check design
    could not survive (its own termination logic lived inside the extension
    being removed).

    Also test the exact-runner-authority requirement directly (AC22a/AC22b,
    "the extension-set safety authority survives every runner rebuild and
    cannot be removed by removing SCE's own Pi extension"): every assertion
    in every variant above about "SCE is first"/"the foreign handler never
    dispatches" must read `resourceLoader.getExtensions().extensions` (or
    equivalently instrument `ExtensionRunner`'s actual constructor
    arguments) at the moment `_buildRuntime()` runs, not a separately
    computed prediction of what the array should be. A test that instead
    asserts "the launcher's precomputed order was E" and separately assumes
    "therefore the runner used E" does not satisfy this requirement — mutate
    the on-disk extension directory and `.pi/settings.json` between
    constructing the session and triggering `/reload` in at least one
    variant, and assert the post-reload array is still read from the actual
    `ExtensionRunner` construction, not from an earlier snapshot.

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
  - Verify: the complete **Full validation** section, run after context
    synchronization for this task.
  - Context synchronization: pending

## Open questions

None. T01 owns every lifecycle fact that could invalidate this design, and a contradictory finding there is a re-planning gate, not a deferred guess. The one dependency question this plan started with — whether to wait for PR #276 to merge, redo its protocol generalization here, or stack directly on its branch — was resolved during planning: this plan stacks on PR #276's head per **Stack and base**.
