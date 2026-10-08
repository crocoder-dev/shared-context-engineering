# Plan: mutation-cursor-ref-reconciliation-wiring

## Change summary

Make the existing mutation-cursor snapshot-ref reconciler (`runtime/ref_reconciliation.rs`, async `reconcile_worktree`) operational. Today it is "retained and unwired": nothing in production calls it, so pins left by interrupted `coordinate()` paths accumulate forever.

This plan wires reconciliation in two deliberately different ways:

- **Actual reconciliation (Git and DB I/O) runs only through `sce doctor --fix`.** A new runtime maintenance entrypoint performs exactly one `WorktreeLock` acquisition, verifies and opens the existing repository Agent Trace DB without side effects, runs the existing reconciliation algorithm and atomic deletion transaction, and records a diagnostic result.
- **The mutation-hook completion path (`Close`/`Flush`) performs only a lightweight advisory check.** It never lists refs, opens the DB, or runs reconciliation. At most once per advisory window it records that reconciliation is recommended in a small durable state file; `sce doctor` (read-only) reports the recommendation and `sce doctor --fix` acts on it.

**Fully automatic reclamation is not delivered by this PR.** No lifecycle-owned, noncritical-path execution point exists today (see D2), and inline reconciliation on the tool-completion path cannot be bounded. Automatic out-of-band reconciliation is a documented follow-up PR with stated prerequisites. Nothing in this plan or in the synchronized context may claim that orphan pins are reclaimed automatically.

The change extends existing behavior and preserves the reconciliation algorithm, `GitSnapshotService`, `MutationTraceStore`, `WorktreeLock`, the atomic SHA-conditional ref deletion transaction, the protocol, the Quint model, and mutation-attribution rules. It must never delete a tree required by any repository-wide durable root and must never change the success/failure outcome of a mutation boundary.

Review history (all resolutions are binding on T01–T06):

*Round 1 (six findings)*

1. **Single lock acquisition (P1).** `reconcile_worktree_inner` already acquires `WorktreeLock` (`ref_reconciliation.rs:172`). The reconciler is split into a lock-acquiring wrapper and a shared lock-held implementation. Each maintenance operation acquires the lock exactly once. See D1.
2. **Hook latency (P1).** Resolved definitively in round 2: no reconciliation on the hook path. See D2.
3. **Failure state (P2).** Failures are durably recorded as diagnostics with a failure streak. Round 2 removed automatic retry scheduling because no automatic full pass exists. See D3.
4. **Corrected AC3 (P2).** Two invariants are claimed, not "every durable root is pinned": local consistency (fail closed, delete nothing) and repository-wide deletion safety (never remove another worktree's protecting pin). Reconciliation never repairs another worktree's missing pins.
5. **PR metadata (P2).** The real PR is #305, head branch `ref-recon`, base `fix-logging`.
6. **Database-opening contract (P2).** The existing hook opener is not read-only. Reconciliation uses a dedicated verified-existing-DB opener with no create/migrate/repair/initialize side effects. See D4.

*Round 2 (three findings)*

7. **Unbounded inline latency (P1).** A cooperative budget checked between phases cannot bound `git for-each-ref`, DB open, repository-wide root queries, `git update-ref`, or Tokio blocking-worker shutdown, and reconciliation holds the lock that the coordinator waits on for 10 s. Selected: advisory-only hook trigger, reconciliation through `sce doctor --fix`, automatic execution deferred to a follow-up. The 250 ms budget, size/cost gates, and deferral machinery are removed. See D2.
8. **Repeated scans after state-persistence failure (P2).** The reservation-before-expensive-work requirement applies to any *automatic* full pass. Because this PR has none, no reservation infrastructure is built; the requirement is recorded as a binding prerequisite of the follow-up. The advisory path emits a recommendation only after the state write succeeds, and `Completed` vs `CompletedStatePersistFailed` stay distinct for explicit passes. See D2/D3.
9. **Far-future timestamps (P2).** The state file stores only past-event timestamps and never a future deadline; eligibility is computed as `now − timestamp ≥ window`, so rereading cannot extend a deadline. An impossible timestamp is normalized once under `WorktreeLock` and persisted. See D3.

Stacking:

- Repository: `crocoder-dev/shared-context-engineering`
- Parent: PR #304, branch `fix-logging`. PR #304 is not modified by this plan.
- This work: PR #305, head branch `ref-recon`, base `fix-logging`. No other branch or PR is created.
- Verified at authoring time: PR #304 head `0be74aad5bea5070828846d3277041a4649c9842`; PR #305 head is whatever `ref-recon` points to (it advances with every plan or implementation commit, so T01 re-records the then-current heads rather than relying on a value written here).
- If #304 advances, rebase `ref-recon` onto its latest head and record the exact SHA in the T01 audit findings.

## Authoring-time observations (to be re-verified by T01)

Facts the design relies on. T01 must re-read each cited location and either confirm or flag it as invalidating a later task.

- `ref_reconciliation.rs:159-243` `reconcile_worktree_inner` acquires `WorktreeLock` via `acquire_inner_async` with a fixed 10 s timeout (`:172`), resolves the worktree ID, calls `open_db`, lists pins, loads local and repository-wide durable roots, and deletes through `delete_pins(lock.lease(), …)` (`:233`). The `lock` local is dropped on return.
- `worktree_lock.rs`: the lock is a non-reentrant advisory file lock (`File::try_lock`) on `<git-dir>/sce/mutation-cursor.lock`; `acquire_inner` polls every 100 ms until the deadline, so a zero timeout is exactly one `try_lock` attempt. `WorktreeLockLease` is an `Arc` clone of the lock inner; the file unlocks only when the last clone drops.
- `git_snapshot.rs:281-300` `run_ref_mutation_inner` moves the `WorktreeLockLease` into the `spawn_blocking` closure (`let _lease = lease;`). A dropped caller future therefore cannot release the file lock while the blocking `git update-ref` worker runs. `delete_pins_inner` has a post-preflight seam but `run_ref_mutation` passes a no-op `on_worker_entered`.
- `protected_worktree.rs:14` `WORKTREE_LOCK_TIMEOUT` is 10 s: a competing `coordinate()` fails its boundary (before durable completion, with the existing lock-timeout error) if any holder keeps the lock longer than that.
- `protected_worktree.rs:178-184` `abandon_after_spawn_without_unlock` leaves the lock held for an external-guard child; other lock users must skip or wait per their own policy.
- `hooks/mutation_scope.rs:356-378`: `classify_coordinate` maps `Ok(_)` and `MarkerClearAfterCommit` to `Ok(String::new())` and every other `CoordinateError` to a boundary failure. `drive_mutation_scope` returns after `coordinate_boundary(...).await`, so the coordinate-held lock is released before any code placed after it runs.
- `hooks/runtime.rs:53-69` `open_agent_trace_db_for_hook_runtime` → `agent_trace_storage/mod.rs:66-74, 195-207` `open_repository_db_for_hook_runtime`: (a) `TursoDb::open_without_migrations_at` (`db/mod.rs:856-860`) calls `ensure_db_parent_dir` and `turso::Builder::new_local(...).build()`, which can create parent directories and an empty DB file; (b) on schema-not-ready it calls `repair_missing_repository_schema_migration_metadata` (`agent_trace_db/repository.rs:136`), which inserts the baseline migration row; (c) it then calls `verify_or_initialize_repository_metadata` (`repository.rs:171`), which inserts repository metadata when absent. It avoids schema migrations but is **not** read-only.
- Pin count is not bounded by orphan count: every retained historical tree root keeps its pin (`A → B → C → D` retains all four), so inventory and durable-root load cost grow with history. A full pass is O(history), not O(orphans).
- Tokio runtime shutdown joins in-flight `spawn_blocking` workers, so dropping a future (for example through `tokio::time::timeout`) does not shorten process latency for work already in a blocking worker.
- `resolve_git_dir` and `resolve_worktree_id` spawn `git` subprocesses (`runtime/git_snapshot.rs`). Any hook-path advisory that called them would add a new subprocess to the completion path.
- Candidate execution points for out-of-band work found in the tree: the Git `post-commit` subcommand (`hooks/commit_hooks.rs:114`), per-harness session-end/stop events (for example `hooks/claude_mutation_scope/lifecycle.rs`, `hooks/codex/stop.rs`, Codex session-end fixtures), `sce setup` lifecycle, and the doctor command (`doctor/mod.rs`, `doctor/fixes.rs`). None is a lifecycle-owned, noncritical-path executor that is shared across harnesses; see D2.

## Design decisions

- **D1 — Single lock acquisition and ownership (round 1).**
  - `ref_reconciliation.rs` is split into: (a) `reconcile_worktree` / `reconcile_worktree_inner`, kept with unchanged signatures and behavior, which acquire `WorktreeLock` once and delegate; and (b) a new lock-held implementation `reconcile_with_held_lock` that borrows the held `WorktreeLock`, never calls `acquire_inner_async`, and derives deletion leases from the held lock via `lock.lease()`.
  - There are exactly two maintenance operations, each with its own single lock acquisition and neither calling the other:
    - `reconcile_explicit` — acquires the lock once with the existing bounded 10 s wait, then runs (b), then records the diagnostic result in the state file while still holding the same lock. It never calls the lock-acquiring wrapper (a).
    - `advise_if_due` — hook-path advisory; acquires the lock once with a zero timeout (single `try_lock`), reads/normalizes/writes the state file, releases. It never calls (a) or (b) and has no DB or Git parameters.
  - State reads that decide a write and the write itself happen under the same single acquisition. An unlocked read is permitted only as an advisory fast exit (and for read-only `sce doctor` reporting) and is always rechecked under the lock before any write.
  - The `WorktreeLock` stays owned by the explicit pass for its whole duration. Mutating Git work receives `lock.lease()` and, as today, moves it into the blocking worker, so cancellation of the caller future cannot release the lock while a Git ref mutation worker is still running.
  - The hook ingress invokes `advise_if_due` only after `coordinate_boundary(...)` has returned, so the mutation coordinator's own lock is never held during the advisory.
  - No reentrant lock, no lock-passing between processes, no global lock registry.
  - Documented concurrency contract: an explicit pass holds the lock for its whole duration because the pin-then-root ordering that makes deletion safe requires it (inventory, root reads, and deletion must not interleave with `coordinate()`'s pin-then-CAS). A mutation boundary arriving meanwhile waits up to `WORKTREE_LOCK_TIMEOUT` (10 s) and then fails *before durable completion* with the existing lock-timeout error. This is an operator-initiated, documented trade-off of `sce doctor --fix`; the hook-path advisory never waits on the lock.
- **D2 — Hook-path trigger is advisory-only; reconciliation is explicit (round 2, resolves P1).**
  - *Why inline reconciliation is rejected.* The hook completion path cannot bound `git for-each-ref`, DB open, repository-wide durable-root queries (they scale with historical evidence), or a started `git update-ref` (which must complete safely), and Tokio shutdown waits for blocking workers. A size cap, a historical-cost gate, or a cooperative between-phase budget only predicts cost; none gives an upper bound. Wrapping any step in a timeout would either abort unsafe Git work or fail to shorten latency. Additionally, reconciliation holds `WorktreeLock`, so a slow pass can push an unrelated boundary past its 10 s lock timeout. Cooperative budgets, caps, cost ceilings, deferral windows, and any 250 ms promise are therefore **removed from the design**, and no replacement timeout is introduced.
  - *Execution-point investigation.* Alternatives considered and rejected (T01 re-verifies each claim):
    1. Git `post-commit` subcommand — synchronous inside `git commit`, which an agent tool can invoke, so it is the same critical path with the same unbounded work.
    2. Per-harness session-end/stop events — not part of the shared mutation-scope ingress, differ per harness (adapter changes are out of scope), still synchronous hook processes that block session shutdown, and have no uniform lifecycle owner.
    3. `sce setup` / lifecycle service — install-time, not recurring.
    4. Detached Tokio task, fire-and-forget subprocess, background daemon — prohibited (a short-lived hook process cannot own them; no supervision, ownership, or failure recovery exists).
    5. `sce doctor --fix` — operator-owned, foreground, can report its true result. **Selected.**
    No suitable lifecycle-owned noncritical-path mechanism exists in the current architecture.
  - *Selected design.*
    - `Close`/`Flush` (durably completed, including the `MarkerClearAfterCommit` case, after `coordinate_boundary` returned) calls `advise_if_due`, a synchronous function with this complete I/O budget: one capped read (≤ `MAINTENANCE_STATE_MAX_BYTES`) of the state file, at most one zero-wait `try_lock`, at most one atomic write of that small file, all local. It performs no `git` subprocess, no DB open or query, no ref listing, no `spawn_blocking`, no await point after the lock is taken. It is a bounded set of small local file operations of the same class the hook path already performs for the lock and taint marker; it is not claimed to be hard real-time, and any unexpected failure is swallowed into a logged outcome.
    - The advisory must not add a new Git subprocess. It obtains the worktree's Git dir without spawning Git (T01 determines whether it can be derived from the filesystem — `.git` directory, or `gitdir:` file for linked worktrees — and proves equality with `resolve_git_dir` for the main and a linked worktree, or whether `coordinate`'s already-resolved value can be reused without protocol changes). **Decision rule:** if neither is possible without a new Git subprocess, the hook trigger is dropped entirely and `sce doctor` computes the recommendation from the state file alone; AC1/AC5 are then satisfied with zero hook callers and T03 reduces to documenting that outcome. This fallback needs no user confirmation because it only removes a convenience signal.
    - `advise_if_due` states: absent/invalid-version/oversized/corrupt state → write an anchor and return `Anchored` (no advice); reference time (`last_success`, else `anchor`) older than `RECONCILIATION_ADVISORY_AFTER` and no valid advice within the same window → persist `last_advised`, then (only after the write succeeded) return `Advised`; otherwise `NoAction` with no write. Lock busy → `Busy` (skipped, not an error). State read/write failure → `StateUnavailable` (no advice emitted, no retry loop, no scan). It emits at most one advisory per window per worktree.
    - `sce doctor` (read-only) reports whether reconciliation is recommended (last success age, last attempt/failure streak, last advice) computed from the state file as a pure function; `sce doctor --fix` runs `reconcile_explicit` regardless of any state.
  - *Deferred to a follow-up PR (explicitly not in this plan).* Fully automatic out-of-band reconciliation. Binding prerequisites for that PR: (1) a lifecycle-owned execution point with defined ownership, lifetime, scheduling and failure recovery (for example an explicitly invoked `sce` maintenance command run by a user-managed scheduler, or a future shared session-end mechanism) — never a detached task, daemon, or timeout-cancelled Git operation; (2) a **durable next-attempt reservation persisted under `WorktreeLock` before opening the DB, listing pins, or loading durable roots**, bounded, never interpreted as success, superseded by the success cadence after success or exponential failure backoff after failure, and bypassed by explicit repair; if the reservation cannot be persisted the automatic pass is skipped and the maintenance-state failure is reported; (3) distinct outcomes for completed, completed-but-state-not-persisted, failed, and skipped/deferred; (4) bounded or split lock-hold analysis against the 10 s coordinator timeout; (5) the same invalid-timestamp normalization as D3. T06 records this follow-up in context.
  - *Trade-offs.* (+) hook completion path has no unbounded work and no new failure mode; correct under every ordering with `coordinate()`; no new infrastructure without a consumer. (−) orphan pins are reclaimed only when an operator runs `sce doctor --fix`; the recommendation is time-based (staleness since last success), not evidence-based, because detecting orphans needs the Git/DB scan this design keeps off the hook path.
- **D3 — Durable maintenance state: diagnostics, no stored deadlines (rounds 1–2).**
  - One small JSON file per worktree at `<git-dir>/sce/ref-maintenance.json` (versioned, capped at `MAINTENANCE_STATE_MAX_BYTES`, 4 KiB), beside the lock and taint marker. Fields (all optional past-event unix-ms timestamps unless noted): `anchor`, `last_success`, `last_attempt`, `last_advised`; `last_attempt_outcome` (`completed` | `failed`); `last_report` (retained/deleted/local-required counts); `consecutive_failures` (integer); `last_failure` (short classification plus bounded message).
  - **No field is ever a future deadline.** Eligibility is computed at read time as `now − reference ≥ window` (saturating at 0). Re-reading therefore cannot move a deadline, and there is nothing to clamp.
  - Writes happen only under `WorktreeLock`, via atomic staging-and-swap (temp file in the same directory, then rename; pattern identified in T01) so a crash leaves either the old or the new file. No in-memory registry or process-global state.
  - Transitions: advisory — anchor and `last_advised` only. Explicit success — `last_attempt` = `last_success` = now, `last_report` set, `consecutive_failures` = 0, failure fields cleared. Explicit failure (DB unavailable/invalid, malformed ref, missing required pins, durable-root read error, delete-transaction error) — `last_attempt` = now, outcome `failed`, `consecutive_failures` += 1, `last_failure` set, `last_success` untouched. Lock contention (including explicit lock timeout) — **no state change**; never classified as a failure.
  - **Invalid timestamp normalization (deterministic).** A timestamp is invalid when it is more than `FUTURE_SKEW_TOLERANCE` (5 min) after the injected `now` (far-future values, or clock rollback beyond the tolerance). A timestamp within the tolerance is treated as age 0. Under `WorktreeLock`, `advise_if_due` and `reconcile_explicit` normalize once: invalid `last_success`/`last_attempt`/`last_advised`/`anchor` values are cleared; if `last_success` and `anchor` are both unusable the state is **immediately eligible** for advice, so the advisory persists `last_advised = now` and returns `Advised` in the same locked operation, and a fresh `anchor = now` is written. The normalized state is persisted before returning, so a second read sees valid timestamps and does not shift anything. Plain `sce doctor` never writes; it evaluates invalid timestamps as "recommended" with an "invalid maintenance timestamps" note. `sce doctor --fix` runs regardless of timestamp validity and rewrites valid state afterwards.
  - Recovery: a missing, unparsable, wrong-version, or oversized file is treated as default and rewritten by the next recorded operation; it never fails an explicit pass.
  - Result distinction for explicit passes (typed outcome): `Completed(report)`; `CompletedStatePersistFailed(report, warning)` — the cleanup stands and is reported as done, with a visible warning that diagnostics were not recorded; `Failed(error)`; `Skipped(Busy)`. Never merged. Advisory outcomes: `Anchored | NoAction | Advised | Busy | StateUnavailable`.
  - Time comes from an injected wall clock (`Fn() -> i64` unix ms) passed as a generic parameter for deterministic tests.
- **D4 — Verified-existing Agent Trace DB opener (round 1).**
  - Contract for reconciliation: (1) never create a DB file or any parent directory; (2) never run migrations; (3) never call `repair_missing_repository_schema_migration_metadata`; (4) never call `verify_or_initialize_repository_metadata` or otherwise insert/update repository metadata; (5) verify repository identity (stored `repository_id` equals the identity resolved for this checkout, `source_instance_id` valid) and schema readiness (baseline plus the migration providing durable mutation roots) **before** any ref listing or deletion; (6) any failure returns an error with refs untouched, as a maintenance error that never arms `ExternalTaintMarker` and never becomes `CoordinateError::AgentTraceDbUnavailable`.
  - The existing hook opener cannot satisfy (1), (3), or (4), and changing it would alter every hook caller. The smallest satisfying change is a dedicated opener in `agent_trace_storage/mod.rs` (for example `resolve_existing_agent_trace_storage_for_maintenance`) plus one read-only verification method on `RepositoryAgentTraceDb` (for example `verify_existing_repository_metadata(repository_id)`) wrapping the existing `select_repository_metadata_row` without insert. The opener first checks that the resolved DB path exists as a file; if not, it fails before touching Turso. If Turso offers no read-only open, T01 records that and the opener uses only `SELECT`-class statements after open; incidental WAL bookkeeping on an existing DB is an accepted limitation, and tests assert logical tables are unchanged.
  - Typed failure kinds (small enum, converted to `anyhow` at the existing `open_db` seam): `Missing`, `Unreadable`, `IncompatibleSchema`, `MissingMetadata`, `RepositoryMismatch`. The existing hook opener and all its callers are unchanged.
  - Considered and rejected: opening the DB before taking the lock to shorten lock-hold. It would require a second lock acquisition just to record an open failure, violating the one-acquisition-per-operation rule in D1; the lock-first order of the existing reconciler is kept and the D1 concurrency contract documents the consequence.

## Acceptance criteria

How this plan is proven complete. `/validate` runs these checks; no task performs final validation.

- [ ] AC1: Reconciliation has one real non-test **explicit** caller (`sce doctor --fix`) and no automatic reconciliation caller; the mutation-scope ingress has at most an advisory caller that cannot reach reconciliation.
  - Validate: `nix shell nixpkgs#ripgrep -c rg "reconcile|advise_if_due" cli/src --glob '!*test*'` shows the reconciliation entrypoint referenced only from `doctor/` (and its definition module), and `advise_if_due` (if present per the D2 decision rule) referenced from `hooks/mutation_scope.rs`; the hook module imports no reconciliation, `GitSnapshotService`, store, or DB-opener symbol for this purpose.
- [ ] AC2: `sce doctor --fix` deletes an orphan snapshot ref; `Close`/`Flush` never deletes.
  - Validate: T05 S1 drives the doctor fix path against a real Git repo and asserts the orphan `refs/sce/mutation-cursor/<worktree-id>/<tree>` is gone; S14 asserts `Close`/`Flush` leaves all refs untouched. This plan does **not** claim automatic orphan reclamation.
- [ ] AC3: A successful pass establishes two distinct invariants, and claims nothing stronger.
  - **Local consistency:** `durable_roots(W) ⊆ pinned_trees(W)` is checked before any deletion for the target worktree `W`. If it fails, the pass deletes nothing and returns `MissingRequiredPins`.
  - **Repository-wide deletion safety:** `delete(W, T) ⇒ T ∉ durable_roots(repository)`. A pin is removed only when its tree is outside every worktree's durable roots, so reconciliation never removes an existing SCE pin that protects another worktree's required tree.
  - Reconciliation does **not** repair, recreate, or verify missing pins belonging to other worktrees, and does not guarantee that every repository-wide durable root is pinned.
  - Validate: T05 S2, S3, S4, S5 and S13 pass. S2/S3 prove preservation; S4 proves local fail-closed; S13 proves another worktree's missing pin is neither repaired nor treated as a failure of `W`'s pass.
- [ ] AC4: Hook-path maintenance (advisory) cannot change the outcome of an already durably committed mutation boundary.
  - Validate: T05 S17 and S18 pass: every advisory outcome (`Anchored`, `NoAction`, `Advised`, `Busy`, `StateUnavailable`, corrupt/invalid state, seam error) and `MarkerClearAfterCommit` leave the hook result identical to the no-maintenance baseline, refs untouched, and the external-taint marker unarmed.
- [ ] AC5: The mutation-hook completion path performs no reconciliation, ref listing, DB I/O, or new Git subprocess, and writes maintenance state at most once per advisory window.
  - Validate: T05 S14, S15, S16, S19 and S20 pass; `rg` over the advisory module shows no import of `GitSnapshotService`, `MutationTraceStore`, DB openers, `resolve_git_dir`, `resolve_worktree_id`, `spawn_blocking`, or `Command`.
- [ ] AC6: Contention, cancellation, and failed maintenance never violate the pin-to-CAS safety boundary.
  - Validate: T05 S6, S7 and S12 pass using deterministic synchronization (no sleep-based proof): concurrent `coordinate()` cannot race pin deletion; dropping the explicit-pass future during a ref-deletion worker does not release the lock until the worker finishes; each operation completes without self-deadlock.
- [ ] AC7: No new unbounded in-memory registry, dynamic dispatch, detached background worker, timeout-cancelled Git work, or nested runtime.
  - Validate: `git diff fix-logging...HEAD` inspection plus `nix shell nixpkgs#ripgrep -c rg "dyn Future|BoxFuture|tokio::spawn|Runtime::new|block_on|tokio::time::timeout|JoinHandle|abort\(|static .*(Mutex|RwLock|OnceLock|HashMap)" <changed files>` returns no new hits.
- [ ] AC8: Mutation-cursor and Quint model-based tests remain valid without protocol changes.
  - Validate: `git diff fix-logging...HEAD --stat` shows no change to `protocol.rs` or `spec/mutation_cursor.qnt`; the existing `mbt` tests pass.
- [ ] AC9: `sce doctor` stays non-mutating; `sce doctor --fix` reports its actual cleanup result and recovers from every failure, stale, and invalid-state condition.
  - Validate: T05 S9, S25, S26 and S27 pass: counts for deleted/retained/local-required; distinct output for completed, completed-with-state-persist-failure, failed (unavailable/invalid DB, malformed refs, missing pins), and busy; plain `sce doctor` writes no ref and no state; a recommendation is shown when advised or when state is invalid.
- [ ] AC10: Context matches the implementation, the "retained and unwired" status is gone, and context states that automatic reclamation is deferred to a follow-up with its prerequisites.
  - Validate: `nix shell nixpkgs#ripgrep -c rg -i "retained and unwired|unwired" context/cli` returns no stale claims; `rg -i "automatic.*(reclam|reconcil)" context` hits only the deferral/follow-up statements; documented APIs match code.
- [ ] AC11: Exactly one `WorktreeLock` acquisition per maintenance operation, with no self-deadlock.
  - Validate: T02 and T05 S12; `rg "acquire_inner_async|acquire_async|acquire_inner" cli/src/services/mutation_trace/runtime` shows each entrypoint acquires once and `reconcile_with_held_lock` never acquires.
- [ ] AC12: The hook completion path contains no unbounded maintenance work, and the documented concurrency contract for explicit reconciliation holds.
  - Validate: T05 S14–S16 pass: the advisory takes no Git/DB parameters; while an explicit pass is parked mid-inventory or mid-DB holding the lock, the advisory returns `Busy` immediately and the hook result is unchanged; a concurrent `coordinate()` with a short injected lock timeout fails with the existing lock-timeout error before durable completion and succeeds once the pass is released. No timeout, budget, or size cap is claimed to bound hook latency.
- [ ] AC13: Maintenance state is durable, crash-safe, and cannot postpone eligibility indefinitely.
  - Validate: T05 S19–S25 pass: single advice per window with persistence before emission; concurrent processes cannot double-advise; state-persistence failure causes no advice and no scan; interrupted atomic write preserves the previous state; process restart preserves eligibility; far-future and rolled-back timestamps normalize once and recover; no stored timestamp is ever in the future; corrupt/missing state is recovered.
- [ ] AC14: Reconciliation opens only a verified existing DB, with no side effects.
  - Validate: T05 S28–S32 pass: missing DB, incompatible schema, missing metadata, repository identity mismatch, and normal existing-DB operation; failing cases leave refs and the DB logical contents untouched and create no files or directories.
- [ ] AC15: Deferred automatic reclamation is explicit and not implied.
  - Validate: the plan's D2 follow-up prerequisites are copied into the synchronized context by T06; no code path or doc claims automatic reclamation; T05 S14 proves hook completion never reconciles.

### Review-finding traceability

| Finding | Design resolution | Implementing task | Regression scenarios |
| --- | --- | --- | --- |
| R1 P1 double lock | D1 | T02 (T04 consumes) | S6, S7, S12, S17 |
| R1 P1 latency → R2 P1 unbounded inline latency | D2 (advisory only; reconciliation via doctor; automatic deferred) | T01 (investigation), T02, T03, T04 | S14, S15, S16, S17, S18 |
| R1 P2 failure state | D3 (diagnostic streak, no stored deadlines) | T02, T04 | S11, S26, S27 |
| R2 P2 persistence failure / repeated scans | D2/D3 (no automatic full pass; advice only after successful persist; reservation is a follow-up prerequisite) | T02, T03 | S20, S22, S23 |
| R2 P2 far-future timestamps | D3 (past-only fields, normalize once under lock) | T02 | S24, S25 |
| R1 P2 AC3 durability claim | AC3 invariants | T02 (no behavior change), T05, T06 | S2, S3, S4, S5, S13 |
| R1 P2 PR metadata | Stacking section | T01 (re-verify) | none (plan metadata) |
| R1 P2 DB opening side effects | D4 | T01 (audit), T02 | S28–S32 |
| Concurrency / duplicate passes | D1, D3 | T02, T03 | S12, S21 |

Previous acceptance criteria that changed in round 2: AC1 (explicit caller only; no automatic reconciliation caller), AC2 (doctor deletes; hook never does), AC4 (advisory, not automatic maintenance), AC5 (hook path does no I/O beyond state), AC9 (adds recovery conditions), AC10 (adds deferral statement), AC12 (budget/deferral language removed; concurrency contract added), AC13 (backoff replaced by durable-diagnostic and timestamp-recovery guarantees). New: AC15.

### Full validation

- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml`
- `nix develop -c ./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets --all-features -- -D warnings`
- `nix flake check`
- `nix build .#ci-checks`
- `nix run .#pkl-check-generated` (only if generated config inputs are touched)
- `git diff --check`
- `git diff fix-logging...HEAD --stat` (confirms no `protocol.rs` / Quint changes and only intended files)
- `nix shell nixpkgs#ripgrep -c rg "dyn Future|BoxFuture|tokio::spawn|Runtime::new|block_on|tokio::time::timeout|JoinHandle|abort\(" <changed non-test files>` (AC7)
- Focused runs with non-zero match counts, repeated for race scenarios: `… test --manifest-path cli/Cargo.toml ref_reconciliation`, `… maintenance`, `… mutation_scope`, `… doctor`, `… agent_trace_storage`

Record actual outcomes in the validation report; compilation alone does not validate the plan.

### Context sync

- `context/cli/mutation-trace-ref-reconciliation.md`
- `context/cli/mutation-trace-runtime-coordinator.md`
- `context/cli/mutation-trace-snapshot-service.md`
- `context/cli/mutation-trace-store.md` (if its usage contract changed)
- `context/cli/mutation-scope-runtime.md`
- `context/cli/mutation-scope-hook-ingress.md`
- `context/context-map.md`
- Relevant doctor documentation (`context/sce/agent-trace-hook-doctor.md`, `context/sce/doctor-human-text-contract.md`)
- Agent Trace storage documentation covering the new verified-existing-DB opener (located by T06)

## Task context synchronization lifecycle

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** `cli/src/services/mutation_trace/runtime/` (new maintenance module with `reconcile_explicit`, `advise_if_due`, and the state module; `ref_reconciliation.rs` split; `git_snapshot.rs` and `worktree_lock.rs` only for test seams as needed), `cli/src/services/agent_trace_storage/mod.rs` and `cli/src/services/agent_trace_db/repository.rs` (the dedicated verified-existing-DB opener and its read-only metadata check only), `cli/src/services/hooks/mutation_scope.rs` (advisory call only), `cli/src/services/doctor/`, the checkout-local state file `<git-dir>/sce/ref-maintenance.json`, tests, and the context files listed above.
- **Out of scope:** `protocol.rs`, `spec/mutation_cursor.qnt`, mutation-attribution rules, Agent Trace schema migrations, the existing `open_agent_trace_db_for_hook_runtime` behavior for other callers, per-harness (Claude/Codex/OpenCode/Pi) adapter changes, adapter recovery state, any automatic or out-of-band reconciliation execution.
- **Constraints:**
  1. Reuse async reconciliation logic, `GitSnapshotService`, `MutationTraceStore`, `WorktreeLock`; the lock-held implementation is the single shared algorithm.
  2. Static dispatch and existing generic `AsyncFn` patterns only; no `dyn Future`, `BoxFuture`, additional Tokio runtime.
  3. Never delete a ref whose tree is in any repository-wide durable-root set.
  4. Never run any maintenance operation while holding a lock from `coordinate()`; maintenance acquires its own lock after `coordinate()` returned.
  5. Never convert a successful mutation-boundary completion into failure because the advisory was busy, failed, errored, or hit invalid state.
  6. No detached cleanup task, background worker, daemon, fire-and-forget subprocess, or timeout-cancelled Git/DB work; no reconciliation on the hook path.
  7. Do not modify `protocol.rs`, the Quint state machine, or attribution rules.
  8. Never invoke `git gc` or remove Git objects directly; ref deletion goes only through the existing atomic SHA-conditional transaction (`delete_pins`). No independent ref-deletion implementation.
  9. Do not bootstrap, migrate, repair, or initialize an Agent Trace DB to run cleanup (D4).
  10. Exactly one `WorktreeLock` acquisition per maintenance operation; never reacquire inside the lock-held implementation.
  11. Maintenance failures never arm `ExternalTaintMarker`.
  12. The maintenance state file never stores a future deadline; all writes are atomic and under `WorktreeLock`.
  13. No schema migration, new attribution state, or new protocol action.
  14. Follow repository rules: no code comments in touched code, Cargo only via Nix, `nix flake check` for verification.
- **Non-goal (automatic reclamation):** see D2. Follow-up PR with the listed prerequisites, including the durable next-attempt reservation requirement.
- **Non-goal (retired-worktree cleanup):** the reconciler only inventories the namespace of a currently resolvable worktree, so refs owned by removed linked worktrees survive. No global namespace sweep here. Record a follow-up for repository-scoped cleanup with active-worktree inventory, repository-wide durable-root retention, and protection against worktree removal/recreation races.
- **Non-goal (repairing other worktrees' pins):** reconciliation of worktree `W` does not recreate or verify pins that only another worktree requires.
- **Non-goal (evidence-based advice):** the advisory is time-based; detecting actual orphans requires the Git/DB scan kept off the hook path.
- **Non-goal (historical event retention):** trees referenced by retained mutation events are never deleted; bounding history needs a separate event-retention and snapshot-compaction policy.
- **Non-goal (protocol changes):** no new action, state field, attribution category, Quint transition, or schema migration.

## Assumptions

- Constants (T03 records final values): `RECONCILIATION_ADVISORY_AFTER` 24 h; `MAINTENANCE_STATE_MAX_BYTES` 4096; `FUTURE_SKEW_TOLERANCE` 5 min; explicit lock wait 10 s (existing).
- The state file is per-worktree under `<git-dir>/sce/`, matching the existing lock and taint-marker layout; absent or unparsable means default state.
- Worktree identity is already Git-topology-derived (`resolve_worktree_id`: `main` and `worktrees/<name>`); the checkout-ID wording in the context docs is stale, and `SkippedNoCheckoutIdentity` appears unreachable in code (declared, never constructed). T01 confirms and T06 removes or justifies it.
- Doctor output additions are additive fields/rows only.

## Task stack

- [ ] T01: `Audit reconciliation and mutation-scope lifecycle integration points` (status:todo)
  - Task ID: T01
  - Scope: In — read-only audit against PR #304/#305 code of `runtime/ref_reconciliation.rs`, `git_snapshot.rs`, `worktree_lock.rs`, `coordinator.rs`, `protected_worktree.rs`, `hooks/mutation_scope.rs`, `hooks/commit_hooks.rs`, `hooks/` harness lifecycle modules, `doctor/mod.rs`, `doctor/fixes.rs`, `doctor/inspect.rs`, `hooks/runtime.rs`, `agent_trace_storage/mod.rs`, `agent_trace_db/repository.rs`, `db/mod.rs`, and existing reconciliation tests; record findings in this plan under an `Audit findings` subsection. Required findings:
    - production call graph for `Start`/`Advance`/`Close`/`Flush`/`Abandon`; which `CoordinateOutcome`/`CoordinateError` variants are durably completed (including `MarkerClearAfterCommit`); when each lock is released, including the `abandon_after_spawn_without_unlock` path;
    - **lock ownership (D1):** confirm `reconcile_worktree_inner` acquires `WorktreeLock` at its start and the lock is non-reentrant; specify the exact wrapper / lock-held split and the parameter carrying lock ownership; confirm `delete_pins` moves the lease into the `spawn_blocking` worker so dropping the caller future cannot release the file lock early; confirm the 10 s coordinator lock timeout;
    - **execution-point investigation (D2):** re-verify each rejected alternative (post-commit, per-harness session-end/stop, setup lifecycle, detached/daemon) with file references, confirm that no lifecycle-owned noncritical-path executor exists, and confirm runtime shutdown joins blocking workers; confirm the selected design (advisory-only hook trigger plus `sce doctor --fix`) or escalate if a suitable mechanism is found;
    - **advisory Git-dir resolution (D2 decision rule):** determine whether the worktree Git dir can be obtained for the advisory without a new Git subprocess (filesystem derivation proven equal to `resolve_git_dir` for the main and a linked worktree, or reuse of a value `coordinate` already resolves without protocol changes) and record the outcome: advisory kept, or hook trigger dropped;
    - **state (D3):** identify the existing atomic staging-and-swap pattern to reuse for the state file; confirm placement beside the lock and marker; confirm no in-memory registry is needed; confirm the existing clock source and how to inject a wall clock;
    - **DB opening (D4):** inspect `open_agent_trace_db_for_hook_runtime` and its resolver; record exactly which steps can create directories/files, repair migration metadata, or initialize repository metadata; determine whether Turso supports a non-creating/read-only open; define the smallest dedicated verified-existing-DB opener and exact verification queries (schema readiness including the durable-roots migration, metadata row presence, `repository_id` match, valid `source_instance_id`);
    - current worktree identity forms; stale checkout-ID docs and unreachable branches including `SkippedNoCheckoutIdentity`;
    - confirm PR #304 head and PR #305 head/branch (`ref-recon`, base `fix-logging`) and record the SHAs.
    Out — any code or context change.
  - Dependencies: none
  - Done when: the plan records verified call sites with file/line references, lock ownership and the wrapper/lock-held split, error classification, the D2 execution-point conclusion and advisory Git-dir decision, the DB-opening side effects and chosen opener contract, and flags any finding that invalidates a later task's scope.
  - Verify: re-read cited lines to confirm each recorded fact; `gh pr view 304 --json headRefOid` and `gh pr view 305 --json headRefName,baseRefName,headRefOid` match the recorded values.
  - Context synchronization: pending

- [ ] T02: `Add the reusable maintenance entrypoints (explicit reconciliation and hook advisory)` (status:todo)
  - Task ID: T02
  - Scope: In —
    - **Reconciler split (D1):** extract the body of `reconcile_worktree_inner` after lock acquisition into `reconcile_with_held_lock`, which borrows the held `WorktreeLock` and contains no acquire call; keep `reconcile_worktree` and `reconcile_worktree_inner` behavior and signatures, now acquiring once and delegating.
    - **`reconcile_explicit`:** resolves the worktree and Git dir, acquires the lock **once** (existing bounded 10 s wait), opens the DB through the D4 verified opener, runs `reconcile_with_held_lock`, then records the outcome in the state file while still holding the same lock, then drops the lock. Typed outcome `Completed(report) | CompletedStatePersistFailed(report, warning) | Failed(error) | Skipped(Busy)`; lock contention leaves the state untouched and is not a failure; it never calls the lock-acquiring wrapper; it ignores all prior state (stale, failed streak, invalid or corrupt) except to rewrite it.
    - **`advise_if_due`:** synchronous function with no DB, Git, store, or snapshot parameters; zero-wait single `try_lock`; reads, normalizes, and writes the state under that one acquisition; returns `Anchored | NoAction | Advised | Busy | StateUnavailable`; persists `last_advised` before returning `Advised`; takes injected wall clock and Git dir. Subject to the T01 decision rule.
    - **State module (D3):** `ref-maintenance.json` read (capped, default-on-corrupt), invalid-timestamp normalization under lock, atomic write, past-only fields, success/failure transitions, and a pure `evaluate_recommendation(state, now)` function shared by the advisory and read-only doctor.
    - **D4 opener:** the dedicated verified-existing-DB opener and the read-only repository metadata verification method.
    - **Test seams only as required:** a worker-entered seam threaded through `delete_pins` to `run_ref_mutation_inner`'s existing `on_worker_entered`; phase-boundary hooks (post-lock, post-DB-open, post-inventory) used to park a pass deterministically; injectable write-failure and rename-interruption hooks for the atomic writer; injected clock.
    Out — hook and doctor wiring, any new deletion code, any automatic/reservation/backoff scheduling, protocol/store schema changes, changes to the existing hook DB opener, capped inventory or budget logic.
  - Dependencies: T01
  - Done when: both entrypoints are callable from tests; the explicit pass and the advisory each complete without self-deadlock; state never contains a future deadline; invalid timestamps normalize once and persist; lock-busy never touches state; failures leave refs untouched; the DB opener has no side effects; no second deletion path exists.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml ref_reconciliation`; focused maintenance-state and `agent_trace_storage` unit tests (deterministic, no sleeps); `rg` confirms the only ref-deletion call is the existing `delete_pins`, each entrypoint acquires `WorktreeLock` once, `reconcile_with_held_lock` contains no acquire call, and the advisory module imports no Git/DB/store symbol.
  - Context synchronization: pending

- [ ] T03: `Trigger the advisory check from the shared mutation-scope ingress` (status:todo)
  - Task ID: T03
  - Scope: In — in `hooks/mutation_scope.rs`, after `classify_coordinate` yields a durably completed `Close` or `Flush` (including the `MarkerClearAfterCommit` case) and `coordinate_boundary(...)` has fully returned, call `advise_if_due` through an injected seam; the outcome is only logged (`Advised` at warn with the `sce doctor --fix` hint; `Anchored`/`NoAction`/`Busy` at debug; `StateUnavailable` at warn) and is never returned as a boundary error; no trigger from `Start`, `Advance`, `Abandon`, or failed coordinates; a seam error or unexpected condition is contained and logged; the hook module gains no reconciliation, Git, or DB dependency. Apply the T01 decision rule: if the advisory cannot avoid a new Git subprocess, implement no hook call and document that outcome. Record the final constants in the plan and document the deferred follow-up (D2) in the plan's audit findings. Out — per-harness adapter changes, reconciliation on the hook path, detached tasks, background workers, timeouts.
  - Dependencies: T02
  - Done when: a completed `Close`/`Flush` can run the advisory and no other path does; hook output and exit status are identical whether the advisory returns any outcome or errors; the advisory runs only after the coordinator's lock has been released; `MarkerClearAfterCommit` handling is unchanged; no reconciliation, ref listing, or DB I/O is reachable from the hook module's new call.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_scope`; trigger-policy unit tests over every boundary and outcome with a fake advisory seam; a seam that tries to take the real `WorktreeLock` and asserts it is free at invocation; `rg` over the hook module shows no reconciliation/DB-opener/snapshot import added.
  - Context synchronization: pending

- [ ] T04: `Run explicit reconciliation from sce doctor --fix and report recommendation` (status:todo)
  - Task ID: T04
  - Scope: In — extend the existing doctor `--fix` flow to call `reconcile_explicit` (one lock acquisition inside the entrypoint; doctor never holds the worktree lock itself); additive report fields/rows for retained, deleted, and locally-required counts; distinct rows for completed, completed-with-state-persist-warning, failed (unavailable/invalid DB with the D4 failure kind, malformed refs, missing required pins, durable-root errors, delete-transaction error), and skipped-busy; plain `sce doctor` stays read-only but reads the state file (never writes it or any ref) and reports whether reconciliation is recommended (last success age, last attempt and failure streak, last advice, invalid-timestamp note) via the pure `evaluate_recommendation`; skipped or failed cleanup is never reported as success; `--fix` runs regardless of recommendation, failure streak, or timestamp validity; no DB bootstrap/migration/repair; no change to adapter recovery state; existing output contracts backward compatible; the documented concurrency note (a mutation boundary may wait up to 10 s while `--fix` reconciles) appears in help or fix output wording only if existing output contracts allow it additively. Out — new doctor subcommands, changes to mutation-scope repair logic.
  - Dependencies: T02
  - Done when: `sce doctor --fix` always runs a pass and reports its true typed result; a successful pass resets the failure streak and sets `last_success`; `sce doctor` performs no ref or state mutation; existing doctor tests still pass unchanged.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml doctor`
  - Context synchronization: pending

- [ ] T05: `Add production-path and concurrency regression tests` (status:todo)
  - Task ID: T05
  - Scope: In — real Git repos, linked worktrees, and repository Agent Trace DBs. Every race, ordering and timing scenario uses deterministic coordination (barriers, channels, existing `on_lock_contention`/`on_worker_entered`-style seams, parked phase hooks, injected clock); **no timing-based sleeps as correctness proofs**. T02 and T03 add unit-level coverage for their own code; T05 owns the production-path matrix below and each scenario must fail when the guarded behavior is deliberately broken.
    - *Reconciliation core (explicit path):* (S1) `sce doctor --fix` reclaims an orphan pin left by an interrupted coordinate; (S2) `A → B → C → D` retains every historical tree; (S3) worktree A's pin is retained while worktree B needs the tree (**preservation**); (S4) missing required local pin fails closed with zero deletion; (S5) malformed/symbolic refs cause zero deletion; (S6) concurrent `coordinate()` and reconcile cannot race pin deletion — a coordinate attempt parked on the lock via barrier proceeds only after the pass releases it, and a pin it creates is never deleted by the in-flight pass; (S7) dropping the explicit-pass future while the deletion worker is parked at the worker-entered seam leaves the lock held until the worker is released, then free; (S8) idempotent reconciliation; (S9) plain `sce doctor` mutates neither refs nor state; `--fix` reports the true result for every outcome kind (completed, completed-with-persist-warning, failed kinds, busy); (S10) explicit pass against an external holder returns `Skipped(Busy)`, leaves state byte-identical, and does not count as a failure; (S11) explicit pass with DB failure leaves refs untouched, records a failure streak, never arms the external-taint marker; (S12) the explicit pass and the advisory each complete with one lock acquisition and no self-deadlock, and each returns busy (never deadlocks) against an externally held lock; (S13) with worktree B's required pin deliberately missing, reconciling A deletes only A's eligible pins, does not create or repair B's pin, does not fail, and never deletes any pin protecting B's tree.
    - *Hook path and latency (P1):* (S14) the advisory is invoked only after a durably completed `Close`/`Flush` (including `MarkerClearAfterCommit`), never from `Start`/`Advance`/`Abandon`/failed coordinates, and never reaches reconciliation, ref listing, or DB open — its signature takes none, and spy/panic seams on the reconciler, inventory and opener are never called across every boundary and outcome; all refs remain untouched; (S15) slow Git inventory: an explicit pass parked at the post-inventory seam holds the lock, the advisory returns `Busy` through a channel handshake without waiting, and the hook result is unchanged; (S16) slow DB open/root load: the pass is parked at the post-DB-open seam; the advisory returns `Busy` immediately; a concurrent `coordinate()` with a short injected lock timeout fails with the existing lock-timeout error before durable completion and mutates nothing, and the same boundary succeeds once the pass is released (documents the D1 concurrency contract); (S17) the advisory runs only after the coordinator's lock is released — a seam asserts the worktree lock is acquirable at invocation; (S18) every advisory outcome, a state-write failure, a corrupt state file, a seam error, and `MarkerClearAfterCommit` leave the hook result and `Ok` status identical to the no-maintenance baseline, refs untouched, taint marker unarmed.
    - *State, crashes, invalid timestamps (P2):* (S19) absent state → anchor written once, no advice; a second call writes nothing (file byte-identical); (S20) due state → exactly one advice per window with `last_advised` persisted *before* the advice is returned; repeated calls inside the window do not write or advise; after the injected clock passes the window the advice recurs once; (S21) concurrent processes: one advisory parked inside its locked section, a second returns `Busy`, a third after release returns `NoAction` — exactly one advice, and an explicit pass concurrent with an advisory serializes (no duplicate explicit passes, second returns busy/waits per policy); (S22) state-persistence failure (injected write error): the advisory emits no advice and never scans; an explicit pass whose final write fails returns `CompletedStatePersistFailed` — distinct from `Completed` and `Failed` — with its deletions intact; (S23) crash durability: an interrupted atomic write (temp file left, rename not performed) leaves the previous state readable and eligibility preserved, a stale temp file is ignored/overwritten, and a fresh call (simulating process restart) re-reads state and advises at the correct time; (S24) invalid timestamps: far-future `last_success`, `last_advised`, `anchor`, and clock rollback beyond tolerance each normalize once under the lock, persist, and make the advisory fire immediately once; a repeated call leaves the file byte-identical and moves nothing; after the injected clock advances past the window advice recurs (eventual retry after restart); a rollback within tolerance is treated as age 0; an assertion proves no stored timestamp is ever greater than `now`; (S25) missing, unparsable, wrong-version, and oversized state files are recovered without failing any operation and are rewritten valid by the next recorded operation.
    - *Explicit recovery:* (S26) `sce doctor --fix` runs and reports its true result from each of: failed streak, stale advice, invalid timestamps, corrupt state, unwritable state (→ `CompletedStatePersistFailed`); (S27) explicit success resets the failure streak and sets `last_success`; explicit failure increments the streak without touching `last_success`; lock-busy never changes state.
    - *DB opening (D4):* (S28) missing DB — no file or directory created, refs untouched; (S29) incompatible schema — no migration or repair row written; (S30) missing repository metadata — no row inserted; (S31) repository identity mismatch — refs untouched; (S32) normal existing DB — pass completes. All failing cases assert refs unchanged, DB logical contents unchanged, and the external-taint marker unarmed.
    Out — production behavior changes beyond test seams strictly required.
  - Dependencies: T03, T04
  - Done when: all scenarios pass reliably, each fails when the guarded behavior is deliberately broken, no test depends on timing or sleeps, and every review finding in the traceability table maps to at least one passing scenario.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml` focused on the new tests, repeated runs for the race cases; zero-match filters are not accepted as passes; `rg "sleep"` over new test code returns no hits.
  - Context synchronization: pending

- [ ] T06: `Remove dead-code exemptions and synchronize context` (status:todo)
  - Task ID: T06
  - Scope: In — remove declaration-level `#[allow(dead_code)]` from reconciliation declarations (`ref_reconciliation.rs`, `list_pins`/`delete_pins`/`PinnedRef`/`PinInventoryError` in `git_snapshot.rs`, durable-root readers in `store.rs`) that now have production consumers; separately review genuinely unreachable states (e.g. `SkippedNoCheckoutIdentity`, stale checkout-ID wording) and remove or justify each; update the context files in Context sync with the real async APIs, the wrapper/lock-held split and single-acquisition rule, the documented D1 concurrency contract, Git-derived worktree identities, the two maintenance entrypoints, the advisory trigger and why reconciliation is kept off the hook path, the maintenance state file (past-only fields, normalization, atomic writes), the distinct explicit outcomes, the verified-existing-DB opener contract, doctor reporting, and the corrected two-invariant durability statement (no claim that all repository-wide roots are pinned). Record in context, and not as implemented behavior: the deferred automatic out-of-band reclamation follow-up with its binding prerequisites from D2 (lifecycle-owned execution point, durable next-attempt reservation before expensive work, distinct outcomes, lock-hold analysis, timestamp normalization), the retired-worktree cleanup follow-up, and known storage limitations (historical retention, no repair of other worktrees' pins). Out — behavior changes.
  - Dependencies: T05
  - Done when: no reconciliation-related `allow(dead_code)` remains without a stated reason; clippy with `-D warnings` is clean; context no longer says "retained and unwired", describes checkout-ID identity, or claims automatic reclamation; `context/context-map.md` entries match.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets --all-features -- -D warnings`; `nix shell nixpkgs#ripgrep -c rg -n "allow\(dead_code" cli/src/services/mutation_trace`; `rg -i "retained and unwired" context`; `rg -i "automatic.*(reclam|reconcil)" context` hits only deferral statements.
  - Context synchronization: pending

## Open questions

- Is a 24-hour `RECONCILIATION_ADVISORY_AFTER` the right window for the time-based recommendation? It is only a starting value recorded by T03.
- If T01 finds the advisory cannot get the Git dir without a new Git subprocess, the plan drops the hook trigger and relies on `sce doctor` alone (decision rule in D2, no further confirmation needed). Confirm you are happy with that outcome rather than accepting one extra `git rev-parse` on the `Close`/`Flush` path.
- Do you want a follow-up PR scoped now for automatic out-of-band reclamation (D2 prerequisites), or only the documented deferral?
- `SkippedNoCheckoutIdentity` looks unreachable in the current code (the identity is Git-topology-derived and its failure is an `Err`). The plan assumes T06 removes it; confirm you do not want it kept for compatibility.
- D4 adds a dedicated opener and one read-only verification method in `agent_trace_storage` / `agent_trace_db`. T01 may find that Turso cannot open an existing DB without incidental WAL bookkeeping; the plan treats that as an accepted limitation (logical contents unchanged) unless you want a stricter guarantee.
