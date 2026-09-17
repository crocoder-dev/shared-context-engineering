# External-mutation guard

`run_external_mutation_guard`, in
`cli/src/services/mutation_trace/runtime/external_mutation_guard.rs`, is a
harness-neutral runtime primitive — **not** Pi-specific, despite being built
alongside the Pi adapter to eventually back Pi's `!`/`!!` `user_bash` — that
lets a human-initiated shell command mutate a worktree for a bounded,
durably-guarded interval without ever creating a mutation scope. It is
reachable today through the hidden, Unix-only `sce hooks
external-mutation-guard` command, but no harness extension calls it yet.

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
acquire ProtectedWorktree (WorktreeLock + ExternalTaintMarker, write-ahead)
    -> report Armed
    -> spawn the human shell as the guard's OWN child, own process group
    -> stream the shell's stdout/stderr back to the caller
    -> [optional: caller sends an explicit cancel request
        -> SIGTERM to the shell's process group]
    -> wait for the shell's OWN process termination (never a control-channel
       signal, never elapsed time)
    -> force database_failure + recover on the already-held worktree
    -> on durable commit only: ProtectedWorktree::complete() (clears the marker)
```

Finish is triggered **exclusively** by the shell's own `wait()`. Closing the
caller's control channel (simulating the calling process dying) neither
signals the shell nor triggers finish — only an explicit cancel request does,
and even then finish still waits for the shell's real exit before running the
recovery step. A failed recovery commit leaves the external-taint marker
armed and reports failure without calling `complete()`, so the next boundary
on that worktree self-heals through the ordinary inherited-taint path.

## The two correctness properties this depends on

**The child must hold its own independent lock reference before the guard
can safely stop being the sole holder.** The guard's own `WorktreeLock` fd is
`dup()`'d in the *parent*, immediately before `Command::spawn()`, and the
duplicate is CLOEXEC-clear by plain POSIX `dup()` semantics — no separate
flag-clearing step is needed. Doing this in the parent, not inside a
`pre_exec` closure running in the forked child, matters: `fork()` copies the
whole fd table atomically and synchronously as part of `spawn()` returning,
so there is no window where the kernel could see zero descriptors
referencing the lock's open file description. An earlier draft duplicated
the fd inside `pre_exec` instead; since `pre_exec` runs asynchronously
relative to the parent's `spawn()` call returning, the parent could
(rarely) proceed and drop its own reference before the child had actually
duplicated its own, releasing the flock early. This is fixed by the
parent-side `dup()` alone — no kernel-level workaround was needed once the
ordering was corrected.

**An explicit unlock releases the lock for every descriptor that shares it,
not just the caller's own.** `WorktreeLock::drop` calls `File::unlock()` —
an explicit `flock(fd, LOCK_UN)` — which is not equivalent to simply closing
that one fd: it drops the lock immediately for every fd (including the
child's inherited duplicate) that shares the same open file description.
This never affects the guard's own graceful finish path, because by the time
`ProtectedWorktree` is ever consumed there, the shell has already exited and
its own fd is already closed. It matters only for reasoning about what
happens if the *supervisor process itself* is killed (`SIGKILL`): a real
kill never runs Rust destructors, so `unlock()` is never explicitly called —
only the kernel's implicit "this process's fds are gone" teardown runs, and
the flock survives via the shell's still-open, independently-inherited
descriptor. A test that wants to simulate "supervisor killed, shell alive"
must reproduce that exact difference (forget the guard value and `close()`
only the original fd, rather than calling Rust's own `drop`) — a plain
`drop()` is not a faithful stand-in for `kill -9` here.

## Non-goals of this task's implementation

- No wiring into any harness's TypeScript extension or `user_bash` call
  site — this is a Rust-side mechanism only, waiting for a caller.
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
