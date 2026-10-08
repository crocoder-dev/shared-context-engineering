# Plan: observability-log-lock-striping

## Change summary

`cli/src/services/observability.rs::file_log_lock` keeps a process-global `OnceLock<Mutex<BTreeMap<PathBuf, Arc<Mutex<()>>>>>`. Every distinct log path ever written adds an `Arc<Mutex<()>>` that is never evicted, so synchronization memory grows with the number of historical paths. This plan replaces the registry with a fixed `static LOG_LOCKS: [Mutex<()>; LOG_LOCK_STRIPES]` (64 stripes) selected by a deterministic hash of the path modulo the stripe count. It is a replacement of the lock-selection mechanism only; append, fallback, retention, permission, redaction, and formatting behavior are preserved. Same-process serialization only: no cross-process guarantee exists today and none is added.

Investigation findings (recorded from the code before selecting the design; T01 re-confirms them while implementing):

- Production callers: `append_log_line_with_cleanup` is reached only through `append_log_line`, which is reached only from `Logger::write_log_line` (`observability.rs`). `append_log_line_once_with_cleanup` is reached only from the v2 fallback closure inside `append_log_line_with_cleanup`. `file_log_lock` has exactly those two call sites.
- Ownership: `Logger` is a `Clone + Debug` plain value holding `ObservabilityConfig`, `log_dir`, and retention limit; it owns no shared state. App startup builds one in `app.rs` (`Logger::from_resolved_config`) and stores it in the app context, but `Clone` and `pub` construction let independent instances target the same path, so per-instance mutexes (Option A) would be unsound and would need new shared ownership. Option B (static striping) is selected as the smallest correct design.
- Concurrency: logging is synchronous `std::fs` I/O invoked from the main thread, `tokio::task::spawn_blocking` paths, and plain `std::thread` users, so the critical section stays a synchronous `std::sync::Mutex`; no async mutex.
- Deadlock hazard: today the v2 fallback runs while the primary path's guard is still held, and takes the v2 path's own lock. With striping the primary and v2 paths can hash to the same stripe, and `std::sync::Mutex` is not reentrant, so the fallback must run after the primary guard is released. This is the main correctness risk of the change. A regression here deadlocks instead of failing, so the same-stripe test must run in a bounded child process (T02), never in-process and never via a detached thread plus `recv_timeout` (the deadlocked thread would outlive the failed test).
- Poisoning failure domain: `lock()` is called only at the two sites in `append_log_line_with_cleanup` / `append_log_line_once_with_cleanup`. The guarded section is `persist_log_line` (returns `Result`, uses `?`, no panicking calls expected) plus the retention `cleanup` callback, which is the only caller-supplied code run under the guard and therefore the only realistic poisoning source (a panic inside cleanup). T01 re-confirms this by inspection. Today a poisoned lock affects one path; with 64 stripes it affects every path hashing to that stripe, including unrelated ones. This enlarged failure domain is a documented, accepted consequence; existing poisoned-lock handling is preserved unchanged (no `into_inner` recovery, no `clear_poison`, no new fallback).
- Test-process infrastructure: `cli/src/app.rs::app_output_boundary_uses_isolated_process` already re-executes `std::env::current_exe()` with `--exact <child test> --nocapture` and an env marker to run an isolated child. T02 reuses this pattern (std only, no new dependency).
- Behavior to preserve: lock-acquisition failure (poisoned stripe) is returned as `failed to lock log file '<path>': ...` and does not trigger the fallback (the fallback is only reached from the `persist_log_line` error arm, after the lock was acquired); fallback is a single non-recursive `-v2` attempt whose combined error text is unchanged; retention cleanup runs only after a write that created the file and runs inside the guard of the path that was written; files are opened with owner-only `0o600` on Unix; redaction happens before `append_log_line`.
- Cross-process: no filesystem lock or cross-process guarantee exists or is documented; `context/sce/cli-observability-contract.md` only says "serializes writes independently per path", which becomes inaccurate once paths can share a stripe.

## Acceptance criteria

- [ ] AC1: No path-keyed lock registry remains in `observability.rs`; synchronization storage is a fixed-size `[Mutex<()>; N]` static independent of the number of distinct log paths, with no per-path `Arc`/`Mutex` allocation, no `dyn`, no boxed callbacks, no `unsafe`, and no new dependency.
  - Validate: `nix shell nixpkgs#ripgrep -c rg -n "BTreeMap|OnceLock|Arc<Mutex|dyn |unsafe|Box<" cli/src/services/observability.rs` shows no registry/lock-related hits, and `git diff main -- cli/Cargo.toml cli/Cargo.lock` is empty.
- [ ] AC2: Concurrent writers (threads and independent `Logger` instances) to the same path produce exactly the expected number of complete, non-interleaved, non-duplicated lines.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml observability` passes.
- [ ] AC3: Writing to thousands of distinct paths leaves synchronization storage constant and identical paths always select the same stripe.
  - Validate: the same `observability` test run passes the stripe-selection and many-distinct-paths tests, which assert on the real stripe selection and never on elapsed time.
- [ ] AC4: Primary/v2 fallback works, including when the primary and v2 paths map to the same stripe, without deadlock; the fallback error text and the no-fallback-on-lock-failure semantics are unchanged.
  - Validate: the same-stripe fallback test (child process, bounded timeout) and the existing fallback tests pass under `observability`.
- [ ] AC4a: Lock ordering contract holds: (1) acquire the primary path's stripe; (2) run primary persistence and its retention cleanup; (3) release the primary guard on both success and failure; (4) only then, if primary persistence failed, attempt the v2 fallback; (5) acquire the fallback stripe separately; (6) if both writes fail, the combined `primary log file persistence failed for ...; v2 fallback log file persistence failed for ...` message is byte-identical to today; (7) primary lock acquisition failure returns `failed to lock log file '<path>': ...` and does not attempt the fallback.
  - Validate: code inspection of the guard scope in `append_log_line_with_cleanup` (guard dropped before `attempt_v2_log_fallback` is called, no guard live across it); the same-stripe fallback child test (steps 3-5); the existing combined-error test (step 6); the poisoned-lock test (step 7).
- [ ] AC4b: Same-stripe fallback regression is deterministic and bounded: the test builds a primary path whose `v2_log_path` maps to the same stripe using the production `log_lock_stripe` selection, forces primary failure, and runs in an isolated child process with a finite parent-enforced timeout; the child is killed and reaped on timeout; timeout, unexpected termination, or wrong output fails the test; success requires the fallback file to exist with the expected content.
  - Validate: the test passes under `observability`; temporarily holding the primary guard across the fallback makes it fail by timeout (not hang) and leaves no orphan child.
- [ ] AC4c: Poisoning semantics are preserved and documented: a poisoned stripe still yields `failed to lock log file '<path>': ...` with no fallback and no silent recovery; no new recovery or fallback code is introduced; the enlarged (per-stripe rather than per-path) failure domain is stated in the plan assumptions and durable context.
  - Validate: focused poisoned-stripe test (if practical, see T02); `rg -n "into_inner|clear_poison|PoisonError" cli/src/services/observability.rs` shows no new recovery logic; inspection of `context/sce/cli-observability-contract.md`.
- [ ] AC5: Retention-after-creation, owner-only Unix permissions, and redaction-before-persistence behave as before under the new locking.
  - Validate: `observability` tests covering retention, `0o600` permissions, and redaction pass; no existing test is removed or weakened (`git diff main` shows only additions to the test module).
- [ ] AC6: Durable context states the ownership model, constant memory bound, same-process guarantee, absence of cross-process coordination, and stripe-collision contention trade-off.
  - Validate: inspection of `context/sce/cli-observability-contract.md` (and `context/overview.md`/`context/glossary.md` only if they already describe per-path locking).
- [ ] AC7: The concurrency tests are not flaky.
  - Validate: the targeted logging concurrency tests pass on 20 consecutive runs.

### Full validation

- All existing observability tests remain unchanged and pass (`git diff main` shows only additions to the test module).
- The same-stripe fallback regression executes in a subprocess with a bounded timeout (AC4b); the targeted concurrency tests are repeated 20 times (AC7).
- Review the diff for new dynamic dispatch, heap allocations on the lock-selection path, `unsafe`, dependencies, or unnecessary abstractions: `nix shell nixpkgs#ripgrep -c rg -n "dyn |Box<|Arc<|unsafe|Vec<" ` over the changed lock-selection code, and `git diff main -- cli/Cargo.toml cli/Cargo.lock` empty.
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml`
- `nix develop -c ./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets --all-features -- -D warnings`
- `nix flake check`
- `nix build .#ci-checks`
- `git diff --check`

### Context sync

- `context/sce/cli-observability-contract.md` must replace "serializes writes independently per path" with the striped same-process model and the explicit no-cross-process statement.
- `context/context-map.md` annotation for that file is updated only if its summary text becomes inaccurate.

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** `cli/src/services/observability.rs` (lock selection, the two append functions, inline tests) and the durable context named under Context sync.
- **Out of scope:** OpenTelemetry integration, other observability refactors, log formats/filenames/schema, retention policy, public configuration, logging queues, background workers, new runtimes, filesystem/cross-process locks.
- **Constraints:** std-only; static dispatch; keep generic `F: FnOnce(&Path) -> Result<()>` cleanup callbacks; no `dyn`, boxed callbacks, `unsafe`, new dependencies, or lint suppressions; the hash must be deterministic and independent of mutable global state (e.g. `std::hash::DefaultHasher::new()` with its fixed keys, not `RandomState`); `Cargo` runs go through `nix develop -c ./scripts/run-cli-cargo.sh`; tests stay a minimal focused set using the real implementation and no sleeps or timing assertions.
- **Non-goal:** guaranteeing cross-process serialization, per-instance lock ownership, or eviction caches.

## Assumptions

- 64 stripes (`LOG_LOCK_STRIPES`) is adequate; the exact count only affects collision contention, not correctness.
- Poisoned-lock behavior stays as today (error `failed to lock log file '<path>': ...`, no fallback, no recovery). The failure domain changes from one path to every path on the poisoned stripe (~1/64 of paths per poisoned stripe). Poisoning requires a panic while the guard is held; the only caller-supplied code under the guard is the retention `cleanup` callback, so poisoning is not expected in normal logging. This is accepted, not mitigated, in this plan.
- Releasing the primary guard before the v2 fallback is acceptable: the fallback writes a different file, and the only previously-held-lock effect was ordering against other primary writers, which the fallback never touched. The fallback attempt is no longer atomic with the primary failure with respect to other writers; this is not observable beyond line ordering across two different files.
- Two stripes are never held at once by one call, so there is no lock-order inversion between stripes.
- The child-process timeout is bounded (a few seconds, generous for CI) and is a failure bound only, never a correctness assertion on timing.
- The existing contract doc, not a new ADR, is the right durable home for the model; no decision record is required.

## Task stack

- [x] T01: `Replace the path registry with fixed lock striping and release the guard before fallback` (status:done)
  - Completed: 2026-10-08
  - Files changed: `cli/src/services/observability.rs`
  - Result: Replaced `file_log_lock` registry with `LOG_LOCK_STRIPES = 64` / `static LOG_LOCKS` and `log_lock_stripe_index`/`log_lock_stripe` (deterministic `DefaultHasher::new()`); `append_log_line_with_cleanup` scopes the primary guard in an inner block that returns on success and yields the persist error, so `attempt_v2_log_fallback` runs only after the guard is dropped and never after lock-acquisition failure; `append_log_line_once_with_cleanup` uses the stripe directly. Poisoned-lock error text unchanged; only the retention `cleanup` callback runs caller code under the guard.
  - Verify: `observability` tests passed (8/8); `rg "BTreeMap|OnceLock|Arc<Mutex|dyn |unsafe|Box<|into_inner|clear_poison|PoisonError"` over `observability.rs` returned no hits.
  - Context impact: important — `context/sce/cli-observability-contract.md` "serializes writes independently per path" is now inaccurate (handled by T03).
  - Deviation: the plan's doc comment on same-process/poisoning semantics was not added in code, per the user's no-comments-in-code preference; the content is deferred to T03's contract doc.
  - Task ID: T01
  - Scope: In — in `observability.rs`, add `const LOG_LOCK_STRIPES: usize = 64` and `static LOG_LOCKS: [Mutex<()>; LOG_LOCK_STRIPES] = [const { Mutex::new(()) }; LOG_LOCK_STRIPES]`; a small `log_lock_stripe(path) -> &'static Mutex<()>` using a deterministic std hash modulo the stripe count; remove `file_log_lock`, the `BTreeMap`/`Arc`/`OnceLock` imports, and the registry; update `append_log_line_with_cleanup` so the primary guard is dropped before the v2 fallback is attempted on both success and failure paths (scope the guard in an inner block/helper covering lock + persist + retention, returning the persist result; call `attempt_v2_log_fallback` only after that scope ends, and never when lock acquisition itself failed) and `append_log_line_once_with_cleanup` takes the stripe directly; keep error strings, retention-inside-guard, and lock-failure-skips-fallback behavior; keep existing poisoned-lock error handling unchanged (no recovery, no new fallback) and confirm by inspection that only the cleanup callback can panic under the guard; add a doc comment stating same-process-only synchronization, no cross-process guarantee, that stripe collisions only add contention, and that a poisoned stripe now affects all paths on that stripe. Out — tests beyond keeping existing ones green, context docs, any other observability change.
  - Dependencies: none
  - Done when: the registry is gone, both append functions use the static stripes, the fallback cannot nest a lock on the same stripe (AC4a ordering holds), poisoned-lock handling is unchanged, no `dyn`/`Box`/`unsafe`/new dependency is introduced, and all existing `observability` tests pass unchanged.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml observability`; `nix shell nixpkgs#ripgrep -c rg -n "BTreeMap|OnceLock|Arc<Mutex" cli/src/services/observability.rs`; `rg -n "into_inner|clear_poison|PoisonError"` shows no new hits.
  - Context synchronization: synced

- [x] T02: `Add focused logging synchronization regression tests` (status:done)
  - Completed: 2026-10-08
  - Files changed: `cli/src/services/observability.rs`
  - Result: Added 12 focused tests to the inline test module: same-path multi-thread exact-line integrity, independent `Logger` instances on one path, stripe selection stable/in-range/constant over 5000 paths, in-process different-stripe fallback, combined primary+v2 error text, same-stripe fallback in a bounded child process (`run_bounded_child`: piped+drained stdout/stderr, `try_wait` polling to a 30s deadline, kill+wait on timeout, sentinel + v2 content + blocking-directory assertions), poisoned-stripe test in an isolated child (error `failed to lock log file '<path>': ...`, no v2 file, no recovery), retention-callback-runs-once under concurrent creation, `0o600` on Unix, redaction before persistence. No existing test changed. Also fixed a pedantic-clippy `cast_possible_truncation` in T01's `log_lock_stripe_index` (`usize::try_from(...).unwrap_or_default()`, same behavior on 64-bit).
  - Verify: `observability` tests passed (20/20); 20 consecutive runs all passed; mutation holding the primary guard across the fallback made the same-stripe child test fail by 30s bounded timeout with the child killed and reaped (no orphan), then reverted; `clippy --all-targets --all-features -D warnings` passed; `git diff --check` clean.
  - Context impact: none beyond T03 (tests only; the small stripe-index cast fix does not change documented behavior).
  - Deviation: poisoned-stripe case runs in an isolated child so the shared static cannot be poisoned for other tests; the T01 cast fix was required for the clippy gate and is behavior-neutral.
  - Task ID: T02
  - Scope: In — inline tests in `observability.rs` that call the real `append_log_line_with_cleanup`/`Logger` paths: (a) same-path multi-thread writes with exact complete-line count and per-line integrity; (b) multiple independent `Logger` clones/instances writing one path; (c) many (thousands of) distinct paths asserting stripe selection is stable, always in range, and the stripe array length is the constant; (d) fallback coverage: (d1) in-process, primary failure forcing the v2 fallback (e.g. a directory occupying the primary file name) with paths on different stripes, plus the combined error text when both fail; (d2) **same-stripe fallback regression in an isolated child process** (see below); (d2a) a focused poisoned-stripe test, if practical: poison the stripe of a temp path by panicking inside the `cleanup` callback of a write that creates the file (caught with `std::panic::catch_unwind` or a joined thread), then assert the next write to that path returns `failed to lock log file '<path>': ...`, no `-v2` file is created, and nothing recovers the lock; use a path/stripe unique to the test so other tests are unaffected, and skip with a recorded rationale in the plan only if it cannot be isolated from the shared static; (e) retention under concurrent creation and `0o600` permissions on Unix (`#[cfg(unix)]`); (f) a secret in a logged record is redacted in the persisted file. Out — production behavior changes, correctness assertions on elapsed time, test-only copies of the locking logic, new dependencies, detached-thread-plus-`recv_timeout` harnesses, background workers.
  - Same-stripe child-process test (d2) requirements:
    1. Build a primary path in a temp dir whose `v2_log_path(primary)` maps to the same stripe, using the production `log_lock_stripe` (compare by index or `std::ptr::eq` on the returned stripe); search deterministically over candidate file names (bounded loop) until one collides, and assert the collision before running.
    2. Force primary persistence to fail (e.g. a directory occupying the primary file name) so the fallback is triggered.
    3. Run the scenario in a child: parent re-executes `std::env::current_exe()` with `--exact <child test name> --nocapture` and an env marker, mirroring `cli/src/app.rs::app_output_boundary_uses_isolated_process`; without the marker the child test is a no-op that passes. The child performs the real `append_log_line_with_cleanup` call, then prints a fixed success sentinel.
    4. Parent uses `Command::spawn` + a bounded `try_wait` polling loop against a finite deadline (std only; sleeps are only poll intervals, never assertions).
    5. On deadline expiry the parent calls `kill()` then `wait()` so the child is reaped, then fails the test with captured output.
    6. Timeout, non-success exit status or signal termination, or missing/incorrect sentinel output fails the test.
    7. On success the parent asserts the `-v2` fallback file exists with the exact expected line content and that the primary path is still the blocking directory.
    8. Child state (temp dir, env such as `HOME`/`XDG_*`) is isolated like the existing pattern; stdout/stderr are captured via pipes and drained so a full pipe cannot itself cause a hang.
  - Dependencies: T01
  - Done when: each listed case has a deterministic test against the real implementation, all pass, the same-stripe child test fails by bounded timeout (not by hanging) if the guard were held across the fallback, no orphan child remains, and no existing test is removed or weakened.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml observability`; repeat the targeted concurrency and fallback tests 20 times in a loop to detect flakiness; temporarily mutate T01 to hold the primary guard across the fallback and confirm the child test fails by timeout, then revert.
  - Context synchronization: synced

- [ ] T03: `Document the striped log-lock ownership model` (status:todo)
  - Task ID: T03
  - Scope: In — update `context/sce/cli-observability-contract.md` (and its `context/context-map.md` annotation only if inaccurate) to state: static 64-stripe `Mutex<()>` ownership, constant memory independent of path count, same-process serialization of identical paths across threads and independent `Logger` instances, stripe-collision contention trade-off, guard released before the v2 fallback (lock-ordering contract), the enlarged poisoning failure domain (a poisoned stripe fails every path on it, unchanged error text, no recovery), and the explicit absence of cross-process coordination. Out — broad documentation refactoring, other context files.
  - Dependencies: T01
  - Done when: the contract no longer says writes are serialized "independently per path" and contains the accurate model, the bound, the guarantee, the non-guarantee, and the trade-off, in current-state wording.
  - Verify: inspect the changed lines in `context/sce/cli-observability-contract.md`; `git diff --check`.
  - Context synchronization: pending

## Open questions

None. The request fully specifies scope, design, and constraints; the one design hazard (same-stripe fallback deadlock) is handled inside T01 and proven by the bounded child-process test in T02.
