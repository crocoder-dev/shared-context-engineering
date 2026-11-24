# Plan: Claude mutation-scope stable repository routing

## Change summary

A live concurrent-subagent experiment for PR #358 observed an orphaned Claude mutation scope after the Claude session changed its working directory into a nested Git repository. The Claude adapter currently resolves `checkout::resolve_git_dir(cwd)` independently on every hook event. `PreToolUse` can create an attempt in repository A while `PostToolUse` looks for it in repository B, finds no matching attempt, and returns success. This leaves a scope `Active` in A. It can later distort attribution or trigger recovery.

**Goal:** Bind every tracked Claude tool execution to the repository/worktree selected at its `PreToolUse` boundary. Route its `PostToolUse`, `PostToolUseFailure`, permission denial, and terminal cleanup to that same repository even if subsequent hook `cwd` values change. Never discover the original scope by scanning other repositories.

This is a **Claude adapter routing** change, not a change to the mutation-cursor protocol, protocol event model, or formal semantics. `spec/mutation_cursor.qnt` already maps all Start/Advance/Close boundaries for a scope to its one worktree (`scopeWorktree` / `boundaryWorktree`). The adapter must supply the correct checkout to that model.

## Existing constraints and evidence

- `cli/src/services/hooks/claude_mutation_scope/lifecycle.rs` resolves `git_dir` from the event `cwd` separately in `PreToolUse` and terminal handlers.
- `state.rs` stores adapter attempts under `<git-dir>/sce/claude-mutation-scope-state.json`. Merely adding the start path to an attempt in that file is **insufficient**: a terminal event with a different `cwd` cannot find the file.
- `AttemptKey` consists of `session_id`, optional `agent_id`, and `tool_use_id`. The durable `ScopeId` includes a checkout-local attempt sequence; tool IDs may be reused after terminal completion.
- Generated Claude commands use `$CLAUDE_PROJECT_DIR/.claude/hooks/run-sce-or-show-install-guidance.sh`. `CLAUDE_PROJECT_DIR` is a candidate for a stable **lookup anchor**, not proof that its Git repository is the mutation target.
- A session can run mutation-capable tools against more than one nested repository or linked worktree. The starting `cwd` (or its validated checkout identity) must be stored per tool invocation.
- `Start` is fail-closed; `Close` and cleanup are recovery-aware. Adapter bookkeeping is not attribution evidence.
- PR #358 (Claude subagent attribution) is still a separate open PR. Base this work on `main` and avoid coupling to its model-resolution changes.

## Decisions to validate before implementation

1. **Stable anchor.** Probe real Claude hook processes at `PreToolUse` and `PostToolUse` (including a nested Git repository, subagent, and changed `cwd`). Record whether `CLAUDE_PROJECT_DIR`, the hook's project directory, and `transcript_path` remain stable. Record no prompt, token, or secret contents. Use the established project Git directory only if it can be resolved consistently. If it cannot, choose and document one minimal private per-user session routing location before writing production code.
2. **Routing entry.** Store a durable mapping `(session_id, agent_id, tool_use_id) -> (original checkout path, original git_dir)` in a stable lookup location **outside** the mutable target checkout's adapter-state lookup path. Keep existing per-checkout attempt state and its monotonic scope ID allocation. Do not use `agent_id` or session alone as a route.
3. **Safety and atomicity.** Persist a route **before** allowing an observed `Start`. A failure to persist a route must deny a tracked `PreToolUse`, rather than create unrouteable active work. Lock and atomically replace route storage using existing adapter file/lock conventions. Do not hold routing and checkout-state locks simultaneously or while driving the mutation-scope ingress. Route replays idempotently, and do not silently retarget a live tool invocation.
4. **Use original checkout for lifecycle.** Terminal events load the stable route, then load the original checkout's attempt and verify the full `AttemptKey` before calling the existing `handle_close`/abandonment logic. The corresponding runtime boundary must use the original repository path, not the current event `cwd`. A route alone is never evidence that a scope actually started.
5. **Recovery.** Cover crashes between route persistence, attempt allocation, `Start`, durable `Close`, and route cleanup. On uncertainty, preserve existing fail-closed admission/recovery behavior; do not fabricate `Close` or abandon a potentially live tool. Remove a route only after the owning attempt is safely terminal or a proven unused route has been handled. A duplicate completion must be safe.
6. **Broader terminal events.** `PermissionDenied` must use the route. `Stop`, `StopFailure`, `UserPromptSubmit`, `SubagentStop`, and `SessionEnd` must be able to retire applicable attempts across the **known routes of that session only**, grouped by their recorded originating checkout. Do not scan arbitrary repositories. `WorktreeRemove` retains its event-provided worktree identity and must safely retire matching known routes.
7. **Unavailable anchor.** If the stable route cannot be read or verified, do not silently fall back to the event's possibly changed `cwd` and operate on the wrong repository. Distinguish first-time legacy attempts from attempts created with routing, log the unresolved condition, and document any backward-compatibility limitation.
8. **No scope change.** Leave the generic ingress, `mutation_trace::protocol`, worktree ID semantics, CAS batch, migrations, and `spec/mutation_cursor.qnt` unchanged. If implementation reveals a required protocol change, stop and revise this plan rather than broadening the PR.

## T01 anchor probe findings

**Probe setup.** Disposable repository `$S/repoA` contained nested Git repository `$S/repoA/nestedB`. `$S` is a session scratchpad directory. A project-only `.claude/settings.json` ran an allow-listed logging hook on `PreToolUse`, `PostToolUse`, `PostToolUseFailure`, `SubagentStart`, `SubagentStop`, `Stop`, and `SessionEnd`. The hook recorded only identity fields, `CLAUDE_PROJECT_DIR`, the hook `$PWD`, and `git rev-parse --absolute-git-dir` for the event `cwd` and for `CLAUDE_PROJECT_DIR`. It did not record tool input, prompts, or transcript contents. The session ran as `claude -p --setting-sources project` (Claude Code 2.1.309) from `$S/repoA` and performed these steps: `pwd`, `cd nestedB`, `pwd`, an `Agent` subagent running `pwd` and `Write`, and a final `pwd`.

**Observations (19 events, one session).**

| Field | Observed behavior |
|---|---|
| `cwd` | Changes within one tool: `PreToolUse` and `PostToolUse` for the `cd nestedB` Bash call have `cwd` = `$S/repoA` and `$S/repoA/nestedB`, respectively. All later events have `cwd` = `$S/repoA/nestedB`. The live defect is reproduced. |
| `git -C <cwd> rev-parse --absolute-git-dir` | Follows `cwd`: `$S/repoA/.git` changes to `$S/repoA/nestedB/.git` within the same `tool_use_id`. **Not stable.** |
| Hook process `$PWD` | Equals `cwd`. **Not stable.** |
| `CLAUDE_PROJECT_DIR` | `$S/repoA` in every event, including subagent tool events, `SubagentStop`, `Stop`, and `SessionEnd`. **Stable.** |
| Project Git dir (`--absolute-git-dir` of `CLAUDE_PROJECT_DIR`) | `$S/repoA/.git` in every event. **Stable.** |
| `session_id` | Same on all events, including subagent events. |
| `agent_id` | Absent on main-agent tool events and on the delegating `Agent` tool's own Pre/Post. Present (`ae2e…`) on the subagent's `Bash`/`Write` events and on `SubagentStart`/`SubagentStop`. |
| `tool_use_id` | Matches exactly between each `PreToolUse` and `PostToolUse` pair. |
| `transcript_path` | Location is stable: the parent session transcript `~/.claude/projects/<encoded-project-dir>/<session_id>.jsonl`, including on subagent tool events. `SubagentStop` adds `agent_transcript_path` under `<session_id>/subagents/`. |

**Decision 1 outcome (selected anchor).** The route lookup anchor is the project Git directory, resolved as `git rev-parse --absolute-git-dir` of `CLAUDE_PROJECT_DIR`. Routes live in their own file, `<project-git-dir>/sce/claude-mutation-scope-routes.json`, separate from `claude-mutation-scope-state.json`. The anchor is only a lookup key. Each route record stores the original checkout path and `git_dir` of its tool invocation, and the target checkout is never inferred from the anchor. The anchor is justified for these reasons:
- It was the only Git-derived value that stayed the same across `cwd` changes, subagents, and terminal events.
- Every generated SCE Claude hook command already depends on `CLAUDE_PROJECT_DIR` (`bash "$CLAUDE_PROJECT_DIR/.claude/hooks/run-sce-or-show-install-guidance.sh" …`, generated by `cli/src/services/setup/config_merge.rs`), and `cli/src/app.rs` already uses it as a Claude caller marker.
- `sce setup` installs these hooks at a repository root, so the project directory resolves to a Git directory in supported installs.
- `transcript_path` was rejected as an anchor: it is a per-user Claude-internal location, not an SCE-owned path.
- No separate per-user routing location is needed, because the project Git dir resolved consistently.

**Fallback policy (decisions 3 and 7).** If `CLAUDE_PROJECT_DIR` is unset or blank, or does not resolve to a Git directory:
- A tracked `PreToolUse` fails closed with a deny decision and a logged diagnostic. This matches the existing behavior when `resolve_git_dir` fails.
- Terminal events log the unresolved condition and never fall back to the event `cwd` for routed attempts.
- Only legacy attempts created before routing, which have no route-era marker, may use today's `cwd`-local lookup. This is a documented backward-compatibility limitation for scopes started before the upgrade.

**Not probed live.**
- A `CLAUDE_PROJECT_DIR` inside a linked worktree. `--absolute-git-dir` returns the per-worktree directory, which is still stable for one session.
- A `cwd` outside every repository. The anchor does not depend on `cwd`.

T03/AC7 cover both cases with real Git fixtures.

## Acceptance criteria

- [x] **AC1 — Proven anchor:** A real two-repository Claude hook probe confirms a consistent route lookup anchor across `PreToolUse` and `PostToolUse`, and records the selected anchor and fallback policy.
- [x] **AC2 — Original target:** A `Start` in repository A followed by `PostToolUse` with `cwd` in nested repository B closes the exact attempt in A; no attempt or mutation boundary is created in B.
- [x] **AC3 — Multiple target repositories:** One parent session concurrently starts tools in A and B, with distinct keys and routes. Reversed-order completions close only their own attempts. Main-agent/subagent identity collisions do not cross-route.
- [x] **AC4 — Terminal parity:** `PostToolUseFailure`, `PermissionDenied`, session/agent cleanup, and worktree removal use the original route and preserve existing abandonment and recovery semantics.
- [x] **AC5 — Crash safety:** Inject failures before/after route write, attempt allocation, Start, Close, and route removal. No tracked tool is allowed without a recoverable route; no uncertain scope is incorrectly closed or abandoned.
- [x] **AC6 — Idempotency:** Duplicate terminal events, duplicate `PreToolUse`, a reused `tool_use_id` after terminal completion, and concurrent hook processes do not cross-route or reactivate a terminal `ScopeId`.
- [x] **AC7 — Non-repository `cwd`:** If the post-hook `cwd` is outside every Git repository, an existing route still closes the original attempt; a missing route never closes an unrelated scope.
- [x] **AC8 — Protocol unchanged:** No modifications to `spec/mutation_cursor.qnt`, `cli/src/services/mutation_trace/protocol.rs`, generic ingress, or migrations; existing Quint and mutation-scope tests remain green.
- [x] **AC9 — Live rerun:** Run an isolated Claude session with A containing nested B, establish a tool's scope in A, change `cwd` to B before completion, and show the original scope becomes `Closed`. Confirm route cleanup, no unexpected `Active` scopes, and correct worktree-specific mutation events.

## Task stack

- [x] **T01: Verify and document the stable Claude hook lookup anchor** (status:done)
  - Scope: read-only probe of hook `cwd`, `CLAUDE_PROJECT_DIR`, session ID, agent ID, tool-use ID, and transcript-path *location* (not contents); inspect generated hook commands. Record paths and stable identity in this plan.
  - Done when: AC1 holds and the durable routing location is justified.
  - Completed: 2026-11-24
  - Files changed: `context/plans/claude-mutation-scope-stable-repository-routing.md`
  - Result: A real Claude Code 2.1.309 probe (see **T01 anchor probe findings**) reproduced the defect on a single `tool_use_id` (`PreToolUse` `cwd` = A, `PostToolUse` `cwd` = nested B). It showed that `CLAUDE_PROJECT_DIR` and its resolved Git directory stayed the same in all 19 hook events, including the subagent events. Selected anchor: `git rev-parse --absolute-git-dir` of `CLAUDE_PROJECT_DIR`. Route file: `<project-git-dir>/sce/claude-mutation-scope-routes.json`. Fallback: fail closed, with no `cwd` fallback.
  - Verify: Probe log passed: 8 `PreToolUse`/`PostToolUse` pairs matched by `tool_use_id`; `CLAUDE_PROJECT_DIR` = `$S/repoA` and the project Git dir = `$S/repoA/.git` in every event; `cwd` moved A → B on the `cd nestedB` tool. Plan diff reviewed: passed.
  - Context impact: none. The decision is recorded only in this plan. Durable context changes are deferred to T04 (`context/cli/claude-mutation-scope-integration.md`).
  - Context synchronization: synced

- [x] **T02: Add minimal durable route storage with atomic/idempotent semantics** (status:done)
  - Scope: one Claude-only routing module; a bounded route record keyed by the exact attempt identity; existing lock/rename conventions; no generic routing framework.
  - Dependencies: T01
  - Done when: route recovery and failure-injection tests cover AC5/AC6.
  - Completed: 2026-11-24
  - Files changed: `cli/src/services/hooks/claude_mutation_scope/routes.rs` (new), `cli/src/services/hooks/claude_mutation_scope/mod.rs`, `context/plans/claude-mutation-scope-stable-repository-routing.md`
  - Result: Added the Claude-only `routes` module. It stores a versioned route table at `<anchor-git-dir>/sce/claude-mutation-scope-routes.json`, protected by its own `claude-mutation-scope-routes.lock` (`AdapterLockSpec::state`). Writes use the `state.rs` temp-file, `sync_data`, rename, and directory-sync convention. Each `RouteRecord` is keyed by the full `AttemptKey` and stores a `RouteTarget` (`checkout_path`, `git_dir`). The module provides these operations:
    - `persist_route` returns `Inserted`, `AlreadyRouted`, or `Conflict { existing }`. It never retargets a live route.
    - `load_route` looks up one route by exact key.
    - `routes_for_session` lists the routes of one session.
    - `remove_route` returns `Removed`, `Absent`, or `TargetMismatch`. It removes a route only while the route still points at the expected target.
    - `resolve_route_anchor` resolves `CLAUDE_PROJECT_DIR` to its Git directory. It returns a typed `RouteAnchorError` (`ProjectDirUnset`, `Unresolvable`) and never falls back to `cwd`.

    A malformed table or an unsupported version fails closed and is never overwritten. The module is not wired into `lifecycle.rs` yet (T03), so `mod routes` carries `#[allow(dead_code)]`. Assumption: I/O errors use `anyhow`, matching the sibling `state.rs`.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_mutation_scope` passed (21 tests, including 17 new route tests). The new tests cover:
    - recovery by a fresh read from disk
    - an injected failure before rename on persist, which leaves no route, keeps prior routes, and still allows a retry over the leftover `.tmp` file
    - an injected failure on removal, which keeps the route recoverable
    - idempotent duplicate persist and duplicate removal
    - refused retargeting
    - a reused `tool_use_id` after removal, plus a stale removal reporting `TargetMismatch`
    - main-agent, subagent, and other-session key isolation
    - 16 concurrent distinct writers with no lost updates, and 16 concurrent duplicate writers inserting exactly once
    - a malformed or future-version table failing closed
    - anchor errors, and a real nested-Git anchor resolving to the project repository, not the nested one

    `nix develop -c ./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings` passed. `cargo fmt --check` passed.
  - Context impact: local. The new module is internal to the Claude adapter and has no runtime behavior yet. The durable context update is deferred to T04 (`context/cli/claude-mutation-scope-integration.md`) after T03 wires the module in.
  - Context synchronization: synced

- [x] **T03: Route Claude mutation lifecycle through the original checkout** (status:done)
  - Scope: integrate PreToolUse, PostToolUse/Failure, PermissionDenied and session/agent/worktree cleanup into the existing lifecycle. Keep the local adapter's `AttemptKey` and existing runtime seam.
  - Dependencies: T02
  - Done when: AC2/AC3/AC4/AC7 hold with real nested Git repositories.
  - Completed: 2026-11-24
  - Files changed: `cli/src/services/hooks/claude_mutation_scope/lifecycle.rs`, `cli/src/services/hooks/claude_mutation_scope/lifecycle_tests.rs` (new), `cli/src/services/hooks/claude_mutation_scope/state.rs`, `cli/src/services/hooks/claude_mutation_scope/mod.rs`, `context/plans/claude-mutation-scope-stable-repository-routing.md`
  - Result: `lifecycle.rs` now routes every tracked tool execution through the T02 route store:
    - `PreToolUse` resolves the anchor from `CLAUDE_PROJECT_DIR` and denies (fail closed) if the anchor cannot be resolved. It then resolves the target from `cwd` and runs the recovery barrier. It persists the route after the barrier and before attempt allocation. A route `Conflict` or a persist failure denies the tool.
    - `PostToolUse`, `PostToolUseFailure`, and `PermissionDenied` load the route by the full `AttemptKey` and finish the attempt in the route's `git_dir` and checkout path. The route is retired only after the attempt is gone from the original state. A route without an attempt is retired without any seam call.
    - `Stop`, `StopFailure`, `UserPromptSubmit`, `SubagentStop`, and `SessionEnd` group the session's known routes, filtered by agent, by target. They then run the existing `cleanup_attempts_matching` once per target and retire routes whose attempts are gone. Errors on one target do not stop the other targets.
    - `WorktreeRemove` keeps its event-provided worktree cleanup. It also retires the session's routes into that worktree, either by `git_dir` or by a checkout path under the worktree, and only when no attempt remains.
    - `AdapterAttempt` gains `routed` (`#[serde(default)]`). New attempts are `routed: true`. Older state files load as `routed: false`, and only those attempts may still be found through the event `cwd`. An unresolvable anchor or route table on a terminal event is logged as `sce.hooks.claude_mutation_scope.route_unresolved` and never falls back to `cwd` for routed attempts.
    - In the legacy path, a `cwd` outside every repository is now a no-op instead of an error.
    - `handle_close` and `handle_permission_denied` are folded into `close_attempt`/`finish_attempt`, and the `routes` module no longer needs `#[allow(dead_code)]`. Protocol, ingress, migrations, and Quint are unchanged.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_mutation_scope` passed: 30 tests, including 9 new lifecycle tests on real `git init` repositories A, nested B, and a linked worktree, plus an outside-of-repository directory. The new tests cover:
    - AC2: close in A from B, with no B state and the route retired
    - AC3: A/B plus a subagent that reuses the same `tool_use_id`, completed in reverse order with `cwd`s swapped
    - AC4: `PostToolUseFailure`/`PermissionDenied` through the route with `recovery_pending` preserved; `Stop` retiring only the main agent and `SubagentStop` from outside the repository retiring the subagent; a failed abandon keeping the route until `SessionEnd` retries; `WorktreeRemove` retiring only worktree routes
    - AC7: close from an outside `cwd`; a lost route never closing a routed attempt while a seeded legacy attempt (no `routed` field) still closes through `cwd`
    - anchor unset or outside a repository denying before any start

    `nix develop -c ./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings` passed. `cargo fmt --check` passed.
  - Assumptions and limits:
    - The route stores the raw `PreToolUse` `cwd` as the checkout path, matching the repository root the seam received before this change.
    - A same-key race is accepted: a `PreToolUse` for the same `tool_use_id` arriving between a terminal event's attempt check and its route removal could leave that attempt unrouted. Claude does not reuse a live `tool_use_id` concurrently.
    - A route left by a start that failed after persistence is retired by the next terminal or cleanup event for that key.
  - Context impact: important. Claude adapter routing behavior changed: the anchor requirement for `PreToolUse`, route-based terminal and cleanup handling, the legacy `routed` marker, and the new `route_unresolved` log event. The durable update to `context/cli/claude-mutation-scope-integration.md` and `context/cli/claude-mutation-scope-routes.md` is needed.
  - Context synchronization: synced

- [x] **T04: Validate, synchronize context, and rerun the live experiment** (status:done)
  - Scope: focused Rust tests through `nix develop -c ./scripts/run-cli-cargo.sh`, existing mutation-scope and Quint checks, and read-only validation of the isolated Agent Trace database. Update `context/cli/claude-mutation-scope-integration.md` and any directly affected current-state docs.
  - Dependencies: T03
  - Done when: AC8/AC9 hold; report unresolved recovery limits and measured hook overhead.
  - Completed: 2026-11-24
  - Files changed: `cli/src/services/hooks/claude_mutation_scope/lifecycle_tests.rs`, `context/plans/claude-mutation-scope-stable-repository-routing.md`
  - Result: Validation and the live rerun passed. One test-only fix was needed: `worktree_remove_retires_routes_into_the_removed_worktree_only` committed in a fixture repository without a commit identity, so it failed in the `nix flake check` sandbox (no global Git config). The test now sets a local `user.name`/`user.email`, following the convention in `mutation_trace/runtime` tests. No production code changed.
    - **AC8.** `git diff 6cc73fe3..HEAD` (merge base with `main`) touches no file in `spec/`, `cli/src/services/mutation_trace/`, `cli/src/services/hooks/mutation_scope*.rs`, or `cli/migrations/`. All changed code is under `cli/src/services/hooks/claude_mutation_scope/`.
    - **AC9 live rerun.** Disposable repository A contained nested Git repository B (ignored by A). Both were set up with `sce setup --claude --hooks` from a `sce` binary built from this branch, with an isolated `XDG_STATE_HOME`, `.invalid` remotes, and no sync. One `claude -p --setting-sources project` session (Claude Code 2.1.309) ran from A and made four Bash calls: (1) `echo one > a2.txt`; (2) `cd nestedB && echo two > b2.txt && cp ../.git/sce/claude-mutation-scope-routes.json ../route-snapshot.json`; (3) `echo three > b3.txt`; (4) `cd .. && echo four > a3.txt`. Tool 2 had `PreToolUse` `cwd` = A and `PostToolUse` `cwd` = B. Tool 4 had the reverse. The copy made while tool 2 was running showed its route (key `toolu_…WVtwKNA9` → `checkout_path` A, `git_dir` A/.git). After the session, A's Agent Trace DB had scopes `…rv8bBvvd` and `…WVtwKNA9`, both `closed`, with `close` events at revisions 2 and 4 attributed `ai_exclusive` to their own scope. B's DB had scopes `…6m1u2oib` and `…68wwUZNi`, both `closed`, with one `close` event attributed to `…6m1u2oib`. No scope was `active` in either DB. Both worktrees were untainted, with `needs_rebaseline = 0`. The route table in A was `{"routes": []}`. Both adapter state files had `attempts: []` and `recovery_pending: false`. Every hook exited 0 and returned empty stdout.
    - **Pre-routing contrast.** An earlier attempt in the same setup by mistake used a stale `sce` binary built before T03 (2026-11-20). It reproduced the defect: tool 4's attempt stayed `active` in B's adapter state after `Stop` and `SessionEnd`, A was left with `recovery_pending: true`, and no route file existed.
    - **Hook overhead.** A synthetic benchmark measured a debug build: 15 `PreToolUse`/`PostToolUse` pairs per round, each with a one-file mutation, in two rounds, comparing the pre-T03 binary with this branch. Median `PreToolUse` rose from 174–182 ms to 189–197 ms, about +15 ms. Median `PostToolUse` rose from 125–136 ms to 148–165 ms, about +23 to +29 ms. The extra cost is the anchor `git rev-parse` plus one locked, fsynced route-table write on each side. In the benchmark, all 15 scopes ended `closed` and no routes remained. Live-session timings (Pre median 891 ms) were inflated by cold page-in of the 560 MB debug binary and are not used as the overhead figure.
  - Verify:
    - `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml` passed: 880 tests, including the `mutation_trace::mbt` Quint connect tests and 51 `mutation_scope` tests.
    - `nix develop -c ./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings` passed.
    - `cargo fmt --check` passed.
    - `nix flake check` failed on the first run (the sandboxed `cli-tests` fixture identity failure described above). It passed after the fix ("all checks passed!").
    - `nix run .#pkl-check-generated` passed (157 files).
    - Live rerun and benchmark evidence as above.
  - Unresolved recovery limits:
    - A legacy attempt (no `routed` marker) whose `cwd` changed before completion is still orphaned.
    - The T03 same-key race between a terminal event's attempt check and its route removal is accepted.
    - A route left by a start that failed after the route was persisted is retired only by a later terminal or cleanup event for that session. If Claude exits without any such event, the route stays as inert bookkeeping.
    - A tool started in B that mutates A, like live tool 4, is attributed to B's scope boundary only. A's change is unscoped in A until A's next boundary. This is by design: a scope binds to its `PreToolUse` checkout (non-goal: no retargeting).
    - Observed outside the task scope: commit `a0918726` tracks a stray log file, `.claude/skills/sce-next-task/context/tmp/sce-24_11_2026.log`.
  - Context impact: important. Facts to record in current-state context: live validation of the routing behavior, the measured hook overhead, and the remaining recovery limits.
  - Context synchronization: synced

## Non-goals

- General cross-harness repository router, registry service, background sweeper, or repository scan.
- Changes to ClickHouse, control-plane, S3, event-DWH, commit-time model attribution, or PR #358.
- Reassigning an existing scope to a new repository when the current working directory changes.
- Guessing a missing route from the newest transcript entry or whichever checkout contains a similarly named tool.
- Modifying the Quint state machine simply to represent an adapter lookup bug.

## Validation and release

A correct route must survive separate hook processes and process restart. Synthetic tests without real Git boundaries are insufficient. Run focused adapter, state, and runtime integration tests plus `nix flake check` and `nix run .#pkl-check-generated` after implementation; test only through the repository's Cargo/Nix wrapper. The final live experiment must use disposable repositories and must not sync production data.

This PR starts as a plan-only draft. Do not claim that the dangling-scope defect is repaired until T02–T04 have landed and AC9 is observed.

## Validation Report

**Status:** validated  
**Date:** 2026-11-24

### Commands run

- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml` -> exit 0 (880 passed, 0 failed; includes `mutation_trace::mbt` Quint connect tests and all `claude_mutation_scope` tests)
- `nix develop -c ./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings` -> exit 0 (no warnings)
- `nix develop -c ./scripts/run-cli-cargo.sh fmt --manifest-path cli/Cargo.toml --check` -> exit 0 (formatted)
- `nix flake check` -> exit 0 ("all checks passed!")
- `nix run .#pkl-check-generated` -> exit 0 (157 files)
- `git diff --stat 6cc73fe3 -- spec cli/src/services/mutation_trace cli/migrations 'cli/src/services/hooks/mutation_scope*'` -> exit 0 (empty diff)
- `git ls-files --cached | grep -E 'context/tmp|\.log$'` -> exit 0 (only `context/tmp/.gitignore`; stray log removed from the index)

### Success-criteria verification

- [x] AC1: Proven anchor -> T01 live probe evidence recorded in **T01 anchor probe findings** (`CLAUDE_PROJECT_DIR` and its Git dir stable across all 19 events); anchor and fallback policy documented.
- [x] AC2: Original target -> `post_tool_use_from_a_nested_repository_closes_the_attempt_in_the_original_repository` passed on real nested Git repositories.
- [x] AC3: Multiple target repositories -> `concurrent_tools_in_two_repositories_close_only_their_own_attempts_in_reverse_order` and `main_agent_and_subagent_with_the_same_tool_use_id_do_not_share_a_route` passed.
- [x] AC4: Terminal parity -> `failure_and_permission_denial_terminate_through_the_original_route`, `stop_and_subagent_stop_retire_only_their_own_routed_attempts`, `failed_abandon_keeps_the_route_so_session_end_can_retry`, `worktree_remove_retires_routes_into_the_removed_worktree_only` passed.
- [x] AC5: Crash safety -> Accepted by user decision (2026-11-24) without lifecycle-level evidence. Route-store failure injection (route write, route removal) and failed abandon are tested; no lifecycle test injects failure after attempt allocation, on seam `Start`, on `Close`, or on route persist during `PreToolUse`.
- [x] AC6: Idempotency -> Accepted by user decision (2026-11-24) without lifecycle-level evidence. Duplicate persist/removal, reused `tool_use_id`, and concurrent writers are tested at the route-store level only.
- [x] AC7: Non-repository `cwd` -> `post_tool_use_outside_every_repository_still_closes_through_the_route` and `a_missing_route_never_closes_a_routed_attempt_but_legacy_attempts_close_by_cwd` passed.
- [x] AC8: Protocol unchanged -> protected-path diff against merge base `6cc73fe3` is empty; full test suite including Quint MBT tests passed.
- [x] AC9: Live rerun -> T04 recorded live Claude Code 2.1.309 rerun evidence: both scopes in A `closed`, no `active` scopes in A or B, route table empty, adapter states clean.

### Failed checks and follow-ups

- None.

### Residual risks

- AC5/AC6 lifecycle behavior is accepted by user decision without direct lifecycle test evidence.
- `.claude/skills/sce-next-task/context/tmp/` is not ignored, so the removed log remains untracked on disk and could be re-added by a broad `git add`.
- A legacy attempt (no `routed` marker) whose `cwd` changed before completion is still orphaned.
- Accepted same-key race between a terminal event's attempt check and its route removal.
- A route left by a start that failed after persistence stays as inert bookkeeping if Claude exits without a later terminal or cleanup event.
