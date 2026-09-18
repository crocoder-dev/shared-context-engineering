# Decision: External-mutation guard as a kernel-enforced process supervisor

Date: 2026-09-17
Status: Accepted
Plan: `context/plans/pi-mutation-scope-integration.md`
Task: `T03`

## Context

Pi's `!`/`!!` `user_bash` executes a human-initiated shell command that is
never AI attribution, but it can genuinely overlap a live, unconfirmed AI
scope: a confirmation-required harness's attribution is not settled until
`Close`, so a human edit landing mid-interval must never be silently folded
into that scope once it confirms. Pi's own local Bash child is detached on
Unix, so the mutation-producing process is not, by construction, a normal
child of whichever process happens to be waiting on it — it can outlive
either. The plan's own D13 design section went through five recorded
corrections before implementation began, each closing a real soundness hole:
a marker armed but not durably held for the whole command; a marker held by
a separate coordinator process rather than the actual mutating process;
conflating "holds the lock" with "can still mutate"; and a supervisor-crash
race where the flock would release while the guarded shell kept running.
This decision is not Pi-specific: it is the sanctioned shape for any future
harness with the same `user_bash`-style overlap.

## Decision

The external-mutation guard (`run_external_mutation_guard`,
`cli/src/services/mutation_trace/runtime/external_mutation_guard.rs`, hidden
`sce hooks external-mutation-guard`, Unix-only) is the one sanctioned
mechanism for letting a human shell command mutate a worktree outside the
mutation-scope protocol, without ever creating a mutation scope. Its
soundness rests on two specific, deliberately-chosen kernel-level mechanisms:

1. **The guard's `WorktreeLock` fd is duplicated in the *parent* process,
   immediately before spawning the guarded shell**, relying on `fork()`'s
   atomic, synchronous fd-table copy so the child is guaranteed to hold its
   own independent reference to the lock's open file description by the
   moment `spawn()` returns — never inside a `pre_exec` closure running in
   the already-forked child, which runs asynchronously relative to the
   parent's `spawn()` call returning and therefore cannot give that
   guarantee.
2. **The guard's crash safety depends on the difference between an implicit
   fd-table teardown and an explicit `flock` unlock.** `WorktreeLock::drop`
   calls `File::unlock()` — an explicit `flock(fd, LOCK_UN)` — which releases
   the lock for every fd sharing that open file description immediately, not
   only on last-close. A real `kill -9` on the supervisor process never runs
   that destructor, so the flock survives via the shell's own inherited
   duplicate; only the kernel's implicit "this process's fds are gone"
   teardown runs. Reasoning about, or testing, "supervisor killed while shell
   alive" must reproduce that exact difference, not a plain Rust `drop()`.

The control protocol is deliberately two-phase:

```text
{"operation":"arm"}
    -> acquire + marker, create LifetimeToken, write+flush {"status":"armed"}
    -> ArmedWaitingForExec; no human shell exists
{"operation":"exec","command":...,"cwd":...,"env":...}
    -> validate and spawn exactly once
{"operation":"cancel"}
    -> pre-spawn: terminate without spawning; post-spawn: signal the shell group
```

`Armed` is an admission acknowledgement, not authorization to execute. The
later `exec` frame is positive evidence that the caller received and accepted
establishment and still wants the command run. If writing or flushing `Armed`
fails, the supervisor exits the pre-spawn path and never waits for an exec.
Control EOF before exec is also ordinary pre-spawn cleanup, never implicit
execution. Malformed, unknown, blank, invalid-cwd, and duplicate-exec frames
cannot spawn a shell.

Finish is triggered only after BOTH the guarded shell's own process
termination (`wait()`) and EOF on a dedicated kernel-owned lifetime pipe
inherited by the shell and ordinary descendants. It is never triggered by
elapsed time, the caller's control channel closing, or stdout/stderr EOF.
Cancellation is accepted only as an explicit request over that channel and
is enacted by signaling the shell's own process group; a closed or
disconnected channel after spawn signals nothing and triggers nothing.

## Rationale

A supervisor that is merely "the process holding the lock" is not sound,
because "holds the lock" and "is capable of mutating the checkout" are not
the same fact once the actual mutating process (Pi's detached local shell)
can outlive whoever is waiting on it. Making the guard itself spawn the
shell as its own direct child closes the ordinary parent-death ambiguity,
but that alone is still not sufficient if the guard process itself can be
killed — an OS-level guarantee, not merely a longer-lived process tree, is
needed for that case. `flock(2)` locks tied to an open file description that
survives across `dup()`+`fork()` into the child are exactly the "smallest
sound Unix mechanism" already available in this codebase's own
`worktree_lock.rs`, chosen over inventing a new fencing primitive.

## Alternatives considered

- **A separate long-lived guard process that only coordinates with, but does
  not itself spawn, the human shell** — rejected: the guard's own liveness
  then proves nothing about whether the actual (potentially detached) shell
  is still running or capable of mutating.
- **Treating control-channel EOF (the caller's process dying) as equivalent
  to "the human command finished"** — rejected: that treats the control
  process's silence as proof the mutation-capable process has stopped, which
  is exactly the class of false inference this decision exists to close.
- **Duplicating the lock fd inside a `pre_exec` closure in the forked
  child** — tried first during T03's implementation and found to race: the
  parent could observe/act on the state before the child had actually
  duplicated its own fd, opening a real window with zero descriptors
  referencing the lock.

## Compatibility and risks

- The mechanism is currently unused: no harness extension calls this route
  yet. Wiring Pi's real `user_bash` call site to it is separate, later work,
  so the risk surface is contained to this Rust-side process/fd/signal
  logic until that wiring lands.
- Getting fd/lock/signal semantics wrong here fails either too permissively
  (an unguarded human mutation gets folded into AI attribution) or too
  conservatively (a legitimate mutation blocks unrelated concurrent work);
  both directions were exercised by the task's own test suite, including a
  faithful kill-9 simulation distinct from a plain Rust `drop()`.
- Unix-only by design; Windows takes the guard-establishment-failure branch
  at the extension level instead (a separate task's responsibility), so this
  decision does not need, and must not grow, a Windows-specific code path.

## Guardrails

- The normal finish trigger is exclusively the guarded shell's own process
  termination plus lifetime-token EOF — never elapsed time, never any
  control-channel signal.
- The parent-side `dup()`-before-`spawn()` ordering is load-bearing; moving
  fd duplication into any child-side (`pre_exec`) hook reintroduces the fixed
  race and must not be reintroduced without re-deriving this same guarantee.
- No `protocol.rs`/Quint change may ride on this mechanism: it is pure
  composition of already-existing `ProtectedWorktree`/`WorktreeLock`/
  `ExternalTaintMarker`/`database_failure`/`recover` primitives.

## Consequences

- Any future harness with a `user_bash`-shaped human-mutation overlap has a
  ready-made, already-reasoned-through mechanism to reuse rather than
  re-deriving the same lock/fd/process semantics from scratch.
- Reasoning about or testing "the supervisor process died" must always
  simulate the kernel's implicit fd teardown (forget the value, close only
  the original fd) rather than Rust's own `Drop`, which is not equivalent
  here.

## Follow-up

- Wire a real harness's `user_bash` call site (starting with Pi's TypeScript
  extension) to this route; until then it remains reachable only via its
  hidden CLI command.
- ~~Confirm the guarded shell's exact local-shell contract (`/bin/sh -c
  <command>`) against Pi's own `createLocalBashOperations()` behavior when
  that wiring lands.~~ **Resolved by a 2026-09-17 T03 repair pass**: the
  assumed `/bin/sh -c <command>` contract was wrong — pinned Pi `0.80.6`
  prefers real `/bin/bash`, falling back to `bash` on `PATH`, only then
  plain `sh` (`dist/utils/shell.js#getShellConfig`); the guard now
  reproduces that exact resolution order instead of hardcoding `/bin/sh`.
  The same pass also gave the guard a validated `cwd` field (Pi's real
  `user_bash` execution cwd, carried and checked against the guarded
  worktree rather than always defaulting to the repository root) and fixed
  a final-output-draining race that could drop a stdout/stderr chunk
  arriving right at shell exit. Full contract citations and the remaining
  deliberate differences (env-merge vs. env-replace; `SIGTERM` vs.
  `SIGKILL` cancellation) are recorded in
  [`mutation-trace-external-mutation-guard.md`](../cli/mutation-trace-external-mutation-guard.md#exact-pinned-pi-0806-local-shell-contract),
  not restated here.
- **2026-09-17 lifetime-token repair:** graceful completion now waits for
  EOF on a dedicated CLOEXEC-explicit pipe inherited by the shell and
  ordinary descendants, not merely for foreground-shell exit. The
  supervisor keeps the real WorktreeLock and marker active through that wait
  and through forced durable recovery. It polls stdout/stderr continuously
  during the wait, uses bounded idle/hard-limit output finalization, and
  retains the D14 deliberate-close escape limitation.
- **2026-09-17 post-spawn failure repair:** successful `Command::spawn()` is
  the cleanup boundary. Before spawn, ordinary RAII unlock is safe. After
  spawn and before lifetime-token EOF, every supervision error or
  panic-adjacent unwind consumes `ProtectedWorktree` through the
  supervisor-specific abandonment path: it leaves the marker armed and
  closes only the supervisor's lock reference without explicit `LOCK_UN`.
  The shell/descendant inherited descriptor therefore remains the
  kernel-authoritative flock reference. Once it becomes free, the next
  boundary observes the still-armed marker and performs inherited-taint
  recovery before proceeding. After lifetime EOF, ordinary unlock remains
  safe even if final recovery fails, but the marker stays armed. `Armed` is
  emitted only after lifetime-token creation/configuration succeeds; failure
  before that point emits no acknowledgement and spawns no shell.
- **2026-09-17 two-phase admission repair:** the old `guard` frame no longer
  carries a command. The hidden route now arms first, writes and flushes
  `Armed`, waits in `ArmedWaitingForExec`, and accepts exactly one later
  `exec` frame containing command/cwd/env. Lost or timed-out acknowledgement,
  pre-exec EOF, cancellation, malformed exec, and invalid cwd all remain
  pre-spawn and cannot create a shell. The lifetime-token writer stays with
  the supervisor until successful spawn, then the supervisor closes its copy;
  the existing descendant-lifetime and no-`LOCK_UN` post-spawn rules are
  unchanged.

## References

- Plan: [`pi-mutation-scope-integration.md`](../plans/pi-mutation-scope-integration.md)
- Task: `T03`
- Current-state context: [`mutation-trace-external-mutation-guard.md`](../cli/mutation-trace-external-mutation-guard.md)
- Current-state context: [`architecture.md`](../architecture.md)
- Current-state context: [`pi-mutation-scope-integration.md`](../cli/pi-mutation-scope-integration.md)
- Evidence: [`external_mutation_guard.rs`](../../cli/src/services/mutation_trace/runtime/external_mutation_guard.rs)
