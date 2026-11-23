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

## Acceptance criteria

- [ ] **AC1 — Proven anchor:** A real two-repository Claude hook probe confirms a consistent route lookup anchor across `PreToolUse` and `PostToolUse`, and records the selected anchor and fallback policy.
- [ ] **AC2 — Original target:** A `Start` in repository A followed by `PostToolUse` with `cwd` in nested repository B closes the exact attempt in A; no attempt or mutation boundary is created in B.
- [ ] **AC3 — Multiple target repositories:** One parent session concurrently starts tools in A and B, with distinct keys and routes. Reversed-order completions close only their own attempts. Main-agent/subagent identity collisions do not cross-route.
- [ ] **AC4 — Terminal parity:** `PostToolUseFailure`, `PermissionDenied`, session/agent cleanup, and worktree removal use the original route and preserve existing abandonment and recovery semantics.
- [ ] **AC5 — Crash safety:** Inject failures before/after route write, attempt allocation, Start, Close, and route removal. No tracked tool is allowed without a recoverable route; no uncertain scope is incorrectly closed or abandoned.
- [ ] **AC6 — Idempotency:** Duplicate terminal events, duplicate `PreToolUse`, a reused `tool_use_id` after terminal completion, and concurrent hook processes do not cross-route or reactivate a terminal `ScopeId`.
- [ ] **AC7 — Non-repository `cwd`:** If the post-hook `cwd` is outside every Git repository, an existing route still closes the original attempt; a missing route never closes an unrelated scope.
- [ ] **AC8 — Protocol unchanged:** No modifications to `spec/mutation_cursor.qnt`, `cli/src/services/mutation_trace/protocol.rs`, generic ingress, or migrations; existing Quint and mutation-scope tests remain green.
- [ ] **AC9 — Live rerun:** Run an isolated Claude session with A containing nested B, establish a tool's scope in A, change `cwd` to B before completion, and show the original scope becomes `Closed`. Confirm route cleanup, no unexpected `Active` scopes, and correct worktree-specific mutation events.

## Task stack

- [ ] **T01: Verify and document the stable Claude hook lookup anchor** (status:todo)
  - Scope: read-only probe of hook `cwd`, `CLAUDE_PROJECT_DIR`, session ID, agent ID, tool-use ID, and transcript-path *location* (not contents); inspect generated hook commands. Record paths and stable identity in this plan.
  - Done when: AC1 holds and the durable routing location is justified.
  - Context synchronization: pending

- [ ] **T02: Add minimal durable route storage with atomic/idempotent semantics** (status:todo)
  - Scope: one Claude-only routing module; a bounded route record keyed by the exact attempt identity; existing lock/rename conventions; no generic routing framework.
  - Dependencies: T01
  - Done when: route recovery and failure-injection tests cover AC5/AC6.
  - Context synchronization: pending

- [ ] **T03: Route Claude mutation lifecycle through the original checkout** (status:todo)
  - Scope: integrate PreToolUse, PostToolUse/Failure, PermissionDenied and session/agent/worktree cleanup into the existing lifecycle. Keep the local adapter's `AttemptKey` and existing runtime seam.
  - Dependencies: T02
  - Done when: AC2/AC3/AC4/AC7 hold with real nested Git repositories.
  - Context synchronization: pending

- [ ] **T04: Validate, synchronize context, and rerun the live experiment** (status:todo)
  - Scope: focused Rust tests through `nix develop -c ./scripts/run-cli-cargo.sh`, existing mutation-scope and Quint checks, and read-only validation of the isolated Agent Trace database. Update `context/cli/claude-mutation-scope-integration.md` and any directly affected current-state docs.
  - Dependencies: T03
  - Done when: AC8/AC9 hold; report unresolved recovery limits and measured hook overhead.
  - Context synchronization: pending

## Non-goals

- General cross-harness repository router, registry service, background sweeper, or repository scan.
- Changes to ClickHouse, control-plane, S3, event-DWH, commit-time model attribution, or PR #358.
- Reassigning an existing scope to a new repository when the current working directory changes.
- Guessing a missing route from the newest transcript entry or whichever checkout contains a similarly named tool.
- Modifying the Quint state machine simply to represent an adapter lookup bug.

## Validation and release

A correct route must survive separate hook processes and process restart. Synthetic tests without real Git boundaries are insufficient. Run focused adapter, state, and runtime integration tests plus `nix flake check` and `nix run .#pkl-check-generated` after implementation; test only through the repository's Cargo/Nix wrapper. The final live experiment must use disposable repositories and must not sync production data.

This PR starts as a plan-only draft. Do not claim that the dangling-scope defect is repaired until T02–T04 have landed and AC9 is observed.
