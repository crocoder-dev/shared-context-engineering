# External-mutation guard

`run_external_mutation_guard`, in
`cli/src/services/mutation_trace/runtime/external_mutation_guard.rs`, is a
harness-neutral runtime primitive — **not** Pi-specific, despite being built
alongside the Pi adapter to back Pi's `!`/`!!` `user_bash` — that lets a
human-initiated shell command mutate a worktree for a bounded, durably-guarded
interval without ever creating a mutation scope. It is reachable today through
the hidden, Unix-only `sce hooks external-mutation-guard` command and is called
by the canonical Pi extension for `user_bash`.

## Why a scope is the wrong model for `user_bash`

A human typing `!some-command` in an AI coding session is not an AI mutation,
so it must never become a `Start`/`Close` pair the way a tracked tool call
does. But it can genuinely overlap a live, unconfirmed AI scope — for a
confirmation-required harness, an AI scope's attribution is not settled until
its own `Close`, so a human edit landing mid-interval must never be silently
folded into that scope once it confirms. The guard exists to make that
overlap safe: for as long as it is armed, the worktree is durably marked
external-taint, so any mutation boundary that runs while the marker is set
(inherited or fresh) forces the existing `database_failure` + `recover`
composition rather than attributing the ambiguous interval to AI.

## Lifecycle

```text
caller starts the hidden supervisor with {"operation":"arm"}
    -> acquire ProtectedWorktree (WorktreeLock + ExternalTaintMarker, write-ahead)
    -> create a Unix pipe: supervisor owns the read end; the writer is
       CLOEXEC-clear and remains supervisor-owned while waiting
    -> durably establish the lifetime-token infrastructure
    -> write and flush {"status":"armed"}
    -> WAIT in ArmedWaitingForExec; no shell exists yet
caller positively accepts Armed by sending exactly one {"operation":"exec", ...}
    -> validate command, cwd, and env; cwd is checked against the armed worktree
    -> spawn the human shell as the guard's OWN child, own process group,
       at the validated execution cwd
    -> close the supervisor's writer reference
    -> continuously poll shell stdout/stderr, shell wait status, and the
       lifetime pipe while the lock and marker remain active
    -> [optional: caller sends an explicit cancel request
        -> SIGTERM to the shell's process group]
    -> wait for BOTH the shell's own termination and lifetime-pipe EOF
    -> bounded stdout/stderr finalization: consume available chunks until
       both streams close, 100 ms idle, or a 1 s hard cap
    -> force database_failure + recover on the already-held worktree
    -> on durable commit only: ProtectedWorktree::complete() (clears the marker)
```

Foreground-shell lifetime is not external-mutation lifetime. Finish cannot
start on shell `wait()` alone: EOF on the lifetime pipe is the kernel-owned
positive evidence that the last ordinary inheritor of the writer has closed
it. The supervisor holds the real `WorktreeLock` and marker throughout that
wait. Closing the caller's control channel neither signals the shell nor
triggers finish — only an explicit cancel request does. A failed recovery
commit leaves the marker armed and reports failure without calling
`complete()`, so the next boundary self-heals through the ordinary
inherited-taint path.

A successful shell spawn changes cleanup semantics. Before spawn, ordinary
RAII cleanup is safe because no external mutation producer exists. After spawn
and before lifetime-token EOF, every supervision error or panic-adjacent
unwind uses the consuming `ProtectedWorktree::abandon_after_spawn_without_unlock`
path: the marker remains armed and the supervisor closes only its own lock
reference without explicit `LOCK_UN`. The shell and descendants retain their
inherited lock reference, so the next boundary cannot acquire the worktree
until that inherited ownership naturally ends. Once lifetime EOF has been
observed, ordinary unlock is safe even if final recovery fails; the marker
still remains armed until a later boundary recovers it.

`Armed` is an admission acknowledgement, not a progress event for a command
already committed to execution. It proves that `ProtectedWorktree` is held, the
external-taint marker is durably armed, lifetime-token infrastructure has been
created/configured, and the supervisor is waiting for an explicit exec request.
It never authorizes execution by itself. The later `exec` frame is positive
evidence that the caller received and accepted establishment and still wants the
command run. A command is never carried by the arm frame.

The supervisor writes and flushes `Armed` before it can accept `exec`. If that
write or flush fails, it exits the pre-spawn path without waiting for an exec and
without spawning a shell; conservative marker retention is allowed. EOF or an
explicit `cancel` while `ArmedWaitingForExec` likewise exits without spawning.
Thus a lost or timed-out acknowledgement cannot race into execution: a caller
that never sends `exec` can never cause its command to run.

`GuardEvent::Armed` is emitted only after the lifetime-token infrastructure has
been successfully created and configured. Lifetime-token establishment failure
therefore emits no `Armed` event and spawns no shell.

The lifetime pipe is separate from the flock. The lock serializes runtime
access for the whole protected interval; the pipe tells the still-live
supervisor when ordinary shell descendants are gone. The pipe read end is
CLOEXEC, the writer is explicitly CLOEXEC-clear before spawn, the supervisor
closes its writer immediately after successful spawn, and only the shell's
inherited writer references remain. Normal Unix `fork`/`exec` fd inheritance
passes that writer to ordinary descendants, so EOF means no ordinary
inheritor still owns it. A descendant that deliberately closes the writer
while continuing to run remains the documented D14 escape boundary.

## Execution cwd contract

`GuardRequest.cwd` carries Pi's real `user_bash` execution cwd (the tool
call's own cwd, not necessarily the guard process's own launch directory).
It is never trusted verbatim. `resolve_execution_cwd`
(`external_mutation_guard.rs`) validates it after the worktree is armed and
before the explicit exec can spawn anything:

- Worktree identity for the containment check is always derived from
  `repository_root` (the guard runtime's own invoking checkout) via a new
  `resolve_worktree_root` (`git_snapshot.rs`, `git rev-parse
  --path-format=absolute --show-toplevel`, canonicalized) — never from the
  untrusted `cwd` field itself. The repository/worktree being protected is
  determined by the supervisor invocation, not by an exec request.
- Absent `cwd`: the guarded worktree's own top-level directory (the guard's
  original, pre-cwd-carrying default).
- Present `cwd`: must be non-blank and absolute (Pi's own
  `sessionManager.getCwd()`/tool-call cwd is always resolved-absolute by
  construction, matching `resolvePath` in pinned Pi `0.80.6`'s
  `dist/utils/paths.js:60-64`), is canonicalized (resolving `..` and
  symlinks against the real filesystem — this is also where a nonexistent
  path is rejected, matching `createLocalBashOperations`'s own
  `fsAccess(cwd, F_OK)` check in `dist/core/tools/bash.js:47-52`), must
  resolve to an existing directory, and must lie inside the canonicalized
  worktree root. Any failure — blank, relative, nonexistent, not a
  directory, or outside the checkout — is rejected outright before shell
  spawn. Because the arm phase has already established the guard, the
  pre-spawn supervisor releases its ordinary lock reference but retains the
  marker conservatively; there is no silent fallback to the repository root on
  a validation failure (only a genuinely absent `cwd` gets that default).

## Guard wire protocol

The hidden `sce hooks external-mutation-guard` route has an explicit
pre-spawn state machine:

```text
Starting -> ArmedWaitingForExec -> Running -> Finished
```

The first frame is exactly `{"operation":"arm"}`. After and only after a
successfully delivered `{"status":"armed"}` acknowledgement, the caller sends
exactly one `{"operation":"exec","command":...,"cwd":...,"env":...}` frame.
`cancel` is accepted while waiting and means pre-spawn termination; after spawn
it signals the shell process group. Malformed JSON, unknown operations, blank
commands, invalid cwd values, and duplicate exec attempts are rejected without
creating a second shell. Control EOF before exec is not implicit authorization;
it exits the pre-spawn supervisor path. Control EOF after spawn remains
non-authoritative and does not cancel or finish the shell.

While waiting for exec, the supervisor owns the lifetime-token writer. A
successful spawn transfers an inherited writer to the shell, then closes the
supervisor's writer; if exec never arrives, both token ends close during
pre-spawn cleanup. Successful `Command::spawn()` remains the exact boundary
for post-spawn abandonment: before it, ordinary RAII unlock is safe; after it
and before lifetime EOF, failure uses the no-`LOCK_UN` abandonment path.

## Exact pinned Pi `0.80.6` local-shell contract

T03 originally assumed `/bin/sh -c <command>` without checking pinned Pi
source, and recorded that gap explicitly for later confirmation. The actual
contract, read from `createLocalBashOperations()` in pinned
`@earendil-works/pi-coding-agent@0.80.6`'s `dist/core/tools/bash.js:39-113`
and its helpers:

- **Shell resolution** (`dist/utils/shell.js#getShellConfig`, no
  `shellPath` override plumbed by T03/T05): prefer `/bin/bash` if it
  exists; else the first `bash` found via `which bash` on `PATH`
  (`findBashOnPath`); else fall back to plain `sh` resolved via `PATH` at
  spawn time. This is **not** always `/bin/sh` — on a typical Linux/macOS
  host with `/bin/bash` present, Pi runs real bash, not POSIX `sh`, so
  bash-only syntax in a command behaves differently under a plain-`sh`
  guard. The guard's `resolve_shell_executable` now reproduces this exact
  order instead of hardcoding `/bin/sh`.
- **Command transport**: the command string is passed as `-c <command>`
  (an argv element), not via stdin — the stdin-transport branch in the same
  function is a legacy-WSL-`bash.exe`-only special case that never applies
  on Unix. The guard matches this (`Command::new(shell).arg("-c").arg(&request.command)`).
- **cwd**: `spawn(..., { cwd, ... })` — Node's `child_process.spawn` cwd
  option, set from the resolved-absolute `cwd` argument. The guard matches
  this via the validated execution cwd above.
- **stdin/stdout/stderr**: `stdio: [ignore, pipe, pipe]` for the non-stdin
  transport (the only branch reached on Unix). The guard matches this
  (`Stdio::null()`, `Stdio::piped()`, `Stdio::piped()`).
- **Process group / detach**: `detached: process.platform !== "win32"` —
  true on Unix, making the child its own process-group leader. The guard's
  `process_group(0)` is the same primitive.
- **Exit-code shape**: Node resolves `exitCode` as a number only on a
  normal exit; a signal-terminated, aborted, or timed-out process reports
  `undefined` (`dist/core/bash-executor.js:75`). Rust's `ExitStatus::code()`
  is `None` exactly when signal-terminated, `Some(n)` otherwise — the same
  shape.
- **Final-output draining**: pinned Pi has its own known bug class here —
  `dist/utils/child-process.js#waitForChildProcess` documents
  (`earendil-works/pi#5303`) that a fixed post-`exit` deadline can drop
  output still arriving from a stream; Pi's fix is an idle-grace timer
  re-armed on every chunk after `exit`, finalizing only once both stdout
  and stderr report `end` (or the timer elapses). The guard continuously
  consumes both streams while the lifetime token is held, so no unattended
  queue grows during descendant execution. After shell exit and lifetime
  EOF it uses the same re-armed 100 ms idle policy, with an explicit 1 s
  hard cap so a deliberately token-closing process cannot hold completion
  forever. Chunks observed before that bounded finalization ends are
  delivered; output after the deliberate-close escape boundary is not a
  lifetime-safety signal.
- **Environment** (deliberate, documented, safety-compatible difference —
  not changed to match exactly): Pi always passes a *full* env snapshot
  (`getShellEnv()`, `dist/utils/shell.js:97-107` — `process.env` plus a
  `PATH` adjustment) as a **replacement**, since Node's `spawn` `env` option
  replaces rather than merges when given explicitly. The guard's
  `.envs(request.env.iter().cloned())` **merges** the wire `env` entries
  onto the guard process's own inherited environment rather than fully
  replacing it. The current Pi client forwards only optional `env` entries supplied through
  the operations contract; it does not construct Pi's full `getShellEnv()`
  snapshot. This remains a deliberate compatibility difference: flipping to a
  hard `.env_clear()` would strip `PATH` (and everything else) from empty-`env`
  calls, breaking ordinary command resolution. Reconcile this only if a future
  caller requires exact full-snapshot environment parity.
- **Cancellation/timeout** (deliberate, documented, safety-compatible
  difference — not changed to match exactly): Pi's own abort path
  (`AbortSignal`/timeout) kills with `SIGKILL` to the process group
  (`killProcessTree`, `dist/utils/shell.js:170-189`). The guard's D13
  design (recorded in the ADR below, predating this repair) deliberately
  signals `SIGTERM` instead, on an explicit caller-driven cancel request
  only — there is no guard-level `timeout` parameter at all yet. This was
  an intentional D13 choice (a graceful-shutdown opportunity for the
  guarded command), re-confirmed rather than silently carried forward by
  this repair, and remains T05's responsibility to reconcile against Pi's
  real timeout/abort wiring when `user_bash` is actually connected.

## The two correctness properties this depends on

**The lifetime pipe proves ordinary descendant completion.** The supervisor
creates a pipe before spawning. Its read end stays in the supervisor and its
write end is explicitly made CLOEXEC-clear before `Command::spawn()`. The
supervisor closes its own writer after spawn. Because the shell inherits the
writer across `exec`, and ordinary descendants inherit it under normal Unix
fd inheritance, a readable EOF is equivalent to the kernel having closed the
final ordinary writer reference. No process enumeration, shell parsing, PID,
TTL, or stream EOF is involved in this proof.

**The real lock remains held until that proof and recovery finish.** The
supervisor retains `ProtectedWorktree` throughout shell wait, lifetime-pipe
wait, output finalization, and forced `database_failure + recover`. Only a
durable recovery permits `ProtectedWorktree::complete()`, whose normal
`WorktreeLock::drop()` then explicitly calls `flock(LOCK_UN)`. Thus no
explicit `LOCK_UN` can release the shared flock while an ordinary descendant
still holds the lifetime token. If post-spawn supervision becomes unreliable,
the consuming abandonment path closes only the supervisor's own fd without
calling `flock(LOCK_UN)`; the inherited shell/descendant fd remains tied to the
same open file description and keeps the flock kernel-held. The marker stays
armed, so the next boundary waits until that inherited lock is free and then
runs the existing inherited-taint `database_failure + recover` path before
clearing it. If the supervisor itself is `SIGKILL`ed, Rust destructors do not
run: the shell's inherited lock fd keeps the flock held, as covered by the
existing supervisor-death regression. A descendant that intentionally closes
the lifetime token is outside this guarantee, as D14 documents; retaining a
lock fd after that deliberate close can also be released by the supervisor's
eventual explicit unlock.

## Non-goals of this task's implementation

- No additional harness wiring beyond the canonical Pi extension's `user_bash`
  client described in [`pi-mutation-scope-integration.md`](pi-mutation-scope-integration.md).
- No `protocol.rs`/Quint change: the mechanism is pure composition of
  already-existing `ProtectedWorktree`/`WorktreeLock`/`ExternalTaintMarker`/
  `database_failure`/`recover` primitives via a new `coordinate_on_held_worktree`
  wrapper around the existing (still-private) `coordinate_protected`, plus
  ordinary OS process/fd mechanics.
- No Windows support: the guard is Unix-only; a non-Unix build's
  `run_external_mutation_guard` unconditionally returns
  `GuardError::UnsupportedPlatform`.
- No recovery beyond the guard's own crash semantics above — a stale,
  never-armed marker left by some other failure mode is out of scope here.

See [`pi-mutation-scope-integration.md`](pi-mutation-scope-integration.md) for
the adapter this mechanism was built alongside, and
[`mutation-trace-protected-worktree.md`](mutation-trace-protected-worktree.md)
for the `ProtectedWorktree` prefix it reuses unchanged.
