# Plan: claude-background-shell-mutation-scope

## Change summary

Replace the existing explicit-background denial for Claude `Bash` with a
deliberately minimal compatibility path. `Bash` with
`tool_input.run_in_background = true` is allowed immediately, creates no SCE
mutation scope, creates no Claude adapter attempt, calls no generic
mutation-scope `start`, and receives no lifecycle tracking. Ordinary foreground
Claude mutation-capable tools continue through the existing adapter and runtime
unchanged. `PowerShell` remains denied if its existing background predicate
remains unsupported.

This is an explicit product tradeoff, not an attribution-safety feature. SCE
does not attribute or isolate mutations produced by native background Bash, and
those mutations may overlap with and contaminate attribution of later or
concurrent tracked scopes. T01's captured Claude Code `2.1.284` evidence stays
historical evidence: lifecycle completion was not proven, so this change
deliberately avoids lifecycle tracking rather than attempting to recover it.

The product decision is recorded before the behavior changes. The accepted
2026-10-01 decision still requires background execution to stay denied until
lifecycle completion is proven, so the task stack first writes a new Accepted
ADR that `Supersedes:` it (T02) and only then enables native background Bash
(T03). The 2026-10-01 ADR is immutable history and is never edited, including
its `Status: Accepted` line, per
`context/decisions/2026-08-12-decision-gate-semantics.md`.

Decision state and implementation state stay distinct in durable context, so
current-state documentation never gets ahead of the code:

```text
after T02:
    policy:          background Bash permitted by current decision
    implementation:  adapter still denies until T03

after T03:
    policy:          permitted
    implementation:  permitted natively and untracked
```

Resulting architecture:

```text
Claude background Bash
        ↓
allowed natively
        ↓
not represented in mutation-scope adapter
        ↓
no lifecycle tracking
        ↓
attribution correctness not guaranteed
```

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the
check that proves it. `/validate` runs these checks; no task in the stack
performs final validation. Decision and documentation criteria (AC1-AC3) come
first because they must hold before the implementation criteria (AC4-AC9) may
be satisfied.

- [ ] AC1: A new Accepted ADR exists that explicitly `Supersedes:`
  `context/decisions/2026-10-01-claude-background-admission-requires-proven-lifecycle.md`
  and records native background Bash as an accepted unsupported-attribution
  boundary. It states that SCE permits Claude-managed
  `Bash(run_in_background=true)` natively without mutation-scope tracking, that
  lifecycle completion remains unproven, and that SCE deliberately makes no
  mutation-attribution guarantee for mutations produced by that background
  process. It explicitly accepts attribution contamination (untracked
  background A mutating while tracked foreground B is live may be observed at
  B's boundary and attributed to B) as a compatibility tradeoff outside SCE's
  guarantees, rather than claiming fail-closed or attribution-safe behavior.
  - Validate: inspect the new `context/decisions/2026-10-02-*.md` record for
    `Status: Accepted`, the `Supersedes:` line naming the 2026-10-01 ADR, the
    decision statement, and the A/B consequence block.
- [ ] AC2: The 2026-10-01 fail-closed ADR and the T01 evidence fixture are
  byte-unchanged; the old ADR still reads `Status: Accepted` and carries no
  backlink.
  - Validate: for each of
    `context/decisions/2026-10-01-claude-background-admission-requires-proven-lifecycle.md`
    and
    `cli/src/services/hooks/claude_mutation_scope/fixtures/probe18-t01-current-version.evidence.json`,
    `git log --format=%H -- <path>` lists exactly one commit (the one that
    added the file) and `git status --short -- <path>` is empty.
- [ ] AC3: The behavior-enabling task did not land before a new Accepted ADR
  existed that explicitly `Supersedes:` the 2026-10-01 fail-closed
  background-admission ADR and records native background Bash as an accepted
  unsupported-attribution boundary. Once the plan is complete, durable
  current-state context states that native Claude background Bash is
  operationally supported but unsupported for mutation attribution, points to
  the new effective decision, carries no leftover pending-implementation
  wording, and preserves the T01 evidence, the probe14/probe17 evidence, the
  D20 self-detached descendant boundary, and the fact that Claude lifecycle
  completion remains unproven.
  - Validate: the plan shows T02 `(status:done)` with
    `Context synchronization: synced` and a `Completed` date no later than
    T03's; `git log --format=%H -- context/decisions/2026-10-02-*.md` resolves
    to a commit that is an ancestor of the commit changing
    `cli/src/services/hooks/claude_mutation_scope/lifecycle.rs`; inspect
    `context/cli/claude-mutation-scope-background-execution.md`,
    `context/cli/claude-mutation-scope-integration.md`, and the
    `context/context-map.md` entries for the final contract sentence and for
    the absence of any "still denies until T03" statement.
- [ ] AC4: Claude `Bash` with `run_in_background=true` returns the existing
  allow shape (`String::new()`), before repository resolution, the recovery
  barrier, model-state resolution, and normal mutation-scope establishment; no
  generic `start` is invoked and no adapter attempt is persisted.
  - Validate: focused Claude adapter regressions assert empty `PreToolUse`
    output, zero git-dir resolver calls, zero seam `start` calls, an unchanged
    adapter-state file, and zero `mutation_trace_scopes` /
    `mutation_trace_events` rows.
- [ ] AC5: A later `PostToolUse` or `PostToolUseFailure` for that intentionally
  untracked background Bash is harmless and does not transition another
  attempt.
  - Validate: focused terminal-hook regressions drive each event after an
    untracked background admission, with and without an unrelated live
    foreground attempt, and assert no close/abandon seam call or state change.
- [ ] AC6: Background `PowerShell` remains denied.
  - Validate: a focused `PowerShell` background `PreToolUse` regression asserts
    the existing deny payload and no scope/attempt creation.
- [ ] AC7: The accepted attribution limitation is executable: an untracked
  background A can mutate while tracked foreground B is live, and SCE observes
  that mutation at B's boundary under the existing protocol; no test or
  protocol machinery attempts to distinguish A from B.
  - Validate: a regression records the background-A/foreground-B sequence
    against a real Git repository and Agent Trace DB, asserts the result the
    existing protocol actually produces, and names in its test name or
    assertion message that the result carries no attribution guarantee.
- [ ] AC8: Foreground Claude `Bash` and other existing mutation-capable tools
  retain their current lifecycle, attribution, failure, read-only, and
  delegation behavior byte-for-byte or semantically unchanged; the probe17
  detached-descendant fixtures and D20 boundary are unchanged.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_mutation_scope`
    passes with no changed expected output for foreground tests;
    `git diff main -- cli/src/services/hooks/claude_mutation_scope/fixtures/`
    shows no change to any `probe17-*` or `probe14-*` file.
- [ ] AC9: The change introduces no background lifecycle or persistence design:
  no suppressor, safety latch, worktree taint, unknown epoch, attribution
  filter, adapter-state version, background correlation, new terminal hook,
  capability gate, process supervision, mutation-protocol or Quint change,
  migration, doctor state, or explicit recovery/reset command.
  - Validate: the cumulative plan diff `git diff --name-only main` (a
    plan-wide check, deliberately spanning T01-T04) shows only the Claude
    adapter sources/tests under `cli/src/services/hooks/claude_mutation_scope/`,
    the T01 fixture and notes, this plan, and the planned durable-context and
    decision records; `spec/mutation_cursor.qnt`, `cli/src/services/mutation_trace/`,
    `cli/migrations/`, doctor sources, and the adapter state format are
    untouched.

### Full validation

Repository-wide checks `/validate` runs after the last task, regardless of
which criterion they map to.

- `nix flake check`

### Context sync

- `context/cli/claude-mutation-scope-background-execution.md`
- `context/cli/claude-mutation-scope-integration.md`
- `context/context-map.md`
- A new dated decision record under `context/decisions/` carrying
  `Supersedes: context/decisions/2026-10-01-claude-background-admission-requires-proven-lifecycle.md`.
  The 2026-10-01 record itself is not modified.

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** the durable context and decision records listed under Context
  sync; the Claude `PreToolUse` background Bash admission branch in
  `cli/src/services/hooks/claude_mutation_scope/lifecycle.rs`; focused Claude
  adapter and mutation-scope regressions in
  `cli/src/services/hooks/claude_mutation_scope/tests.rs`.
- **Out of scope:** `ManagedBackgroundSuppressor`,
  `background_safety_taint`, adapter state v2, background task correlation,
  new terminal hooks, lifecycle capability detection, process supervision,
  attribution filtering, worktree taint, suppressors, unknown epochs,
  mutation-protocol changes, Quint/spec changes (`spec/mutation_cursor.qnt`),
  Agent Trace DB migrations, doctor state, explicit resume/reset commands, and
  background PowerShell admission.
- **Constraints:** T03 must not start until T02 is `(status:done)` and
  `synced` with the superseding ADR written. No document may imply the adapter
  admits background Bash before T03 lands: after T02, current-state context
  records the decision and states that the adapter still denies
  `Bash(run_in_background=true)` until T03; T03's context synchronization
  replaces that with the final contract. Task-scope checks compare a task's
  own start and end revisions (the `/next-task` Git baseline and its
  baseline-relative `Files changed` record), never the cumulative branch
  diff. No task edits
  `context/decisions/2026-10-01-claude-background-admission-requires-proven-lifecycle.md`
  (status, backlink, or body) or
  `cli/src/services/hooks/claude_mutation_scope/fixtures/probe18-t01-current-version.evidence.json`.
  Place the Bash background early return before repository resolution,
  recovery-barrier handling, model-state resolution, and normal
  mutation-scope establishment. Keep foreground behavior, read-only/delegation
  behavior, and tracked-attempt cleanup unchanged. Use the existing missing
  attempt behavior for terminal hooks unless a focused regression proves an
  adapter-local no-op fix is necessary.
- **Non-goal:** making native background Bash attribution-safe or fail-closed.
  Managed background Bash is an explicit unsupported attribution boundary.
  The A/B contamination regression documents the accepted behavior; it does
  not prevent it.

## Assumptions

- `Bash` is the only background tool enabled by this change; the existing
  `is_explicit_background_shell` denial remains for `PowerShell`.
- The existing `handle_close` missing-attempt branch is the intended safe no-op
  for terminal hooks whose `PreToolUse` was deliberately untracked.
- ADR supersession follows the repository convention in
  `context/decisions/2026-08-12-decision-gate-semantics.md`: the new record
  carries `Supersedes:`, the old record stays byte-unchanged with
  `Status: Accepted`, and readers find the effective decision through
  `context/context-map.md` and the current-state context.
- The superseding ADR is written by `sce-decision` through the decision gate
  of T02's context synchronization, not hand-written during task execution;
  its filename is the skill's dated slug (expected form
  `2026-10-02-claude-background-bash-untracked.md`).
- `probe17` is fixture-and-documentation evidence only; no Rust test references
  it today. "probe17 behavior unchanged" therefore means the fixtures and the
  D20 documentation stay unchanged and a self-detaching `run_in_background =
  false` command keeps the ordinary tracked foreground path.
- Repository-wide `nix flake check` and the no-prohibited-files inspection are
  run by `/validate` from the acceptance criteria rather than by a task; T04
  runs only the focused Claude adapter selection.
- A task's start revision is the Git baseline `/next-task` captures before
  editing (recorded `HEAD` plus pre-existing worktree state), and its end
  revision is the worktree at task completion after context synchronization;
  the baseline-relative `Files changed` record is the task-local change list.
  No commit SHA for these revisions is written into the plan ahead of time.

## Task stack

- [x] T01: `Probe and record the Claude managed-background shell lifecycle` (status:done)
  - Task ID: T01
  - Scope: In — on the installed Claude Code version, capture raw hook payloads
    with wall-clock capture timestamps, and make the background command write
    its own timestamps to a tracked, non-ignored path (including a write as its
    last action and a repeating write for kill cases) so ordering is observed,
    for: (1) successful background Bash; (2) non-zero exit; (3)
    cancellation/kill; (4) another tool running while it remains active,
    including a foreground `Edit`/`Bash` that opens and closes during it; (5)
    `Stop`; (6) `StopFailure`; (7) `UserPromptSubmit`; (8) `SubagentStop`,
    with a background Bash started by that subagent; (9) `SessionEnd`; (10) a
    repository mutation after the initial `PostToolUse`; (11) process behavior
    when Claude itself exits; (12) whether the terminal event arrives while the
    session is otherwise idle; (13) a foreground command moved to the
    background mid-flight, if producible; (14) whether hook registrations
    edited mid-session take effect and whether a session's hooks come from one
    settings snapshot. Register the capture hook for `PreToolUse`,
    `PostToolUse`, `PostToolUseFailure`, `Notification`, `Stop`, `StopFailure`,
    `SubagentStop`, `SessionEnd`, `UserPromptSubmit`, `PostToolBatch`, and every
    other hook event the installed version exposes that could report
    background-task completion. Preserve per event: timestamp, `session_id`,
    `agent_id`, `tool_use_id`, `tool_name`, `tool_input`, `tool_response`, any
    background/task/process identifier, and the raw payload. Store fixtures
    under `fixtures/`, add a `NOTES.md` addendum answering G1–G7 with the
    per-event table and the decision-gate outcome, and update
    `context/cli/claude-mutation-scope-background-execution.md` with observed
    behavior and the tested version. Out — any Rust change; any change to the
    denial; committing the scratch hook or marker files.
  - Dependencies: none
  - Done when: every probe case is captured or recorded as not capturable with
    its concrete reason; G4 is shown as
    `last child write < process termination <= terminal event` from timestamps
    the child wrote; each listed lifecycle event has a `yes`/`no`/`unknown`
    answer for "process cannot still mutate after this event"; case 10 is shown
    Git-observable the way `probe17` was; the decision-gate outcome (proceed,
    or stop and revise toward supervision) is written in `NOTES.md`;
    `.claude/settings.json` is back to its pre-task content.
  - Verify: `git status --short` shows only new fixture files, `NOTES.md`, and
    the context document; `git diff -- .claude/settings.json` is empty;
    `claude --version` matches the version recorded in `NOTES.md`.
  - Completed: 2026-10-01
  - Files changed: `cli/src/services/hooks/claude_mutation_scope/fixtures/probe18-t01-current-version.evidence.json`, `cli/src/services/hooks/claude_mutation_scope/fixtures/NOTES.md`, `context/cli/claude-mutation-scope-background-execution.md`
  - Result: Captured the current Claude Code version and the isolated probe attempt. Claude 2.1.284 reached SessionStart but produced no model response or tool/lifecycle event within the probe timeout; all 14 cases were recorded as not capturable for this run, with prior 2.1.258 fixtures retained as prior-version evidence. G1-G7 remain unknown, so the decision gate is stop and revise toward SCE-owned process supervision; the existing background denial was unchanged.
  - Verify: Passed before the lifecycle write: `git status --short --untracked-files=all` listed only the new fixture, `fixtures/NOTES.md`, and the context document; `git diff -- .claude/settings.json` was empty; `claude --version` reported `2.1.284`, matching `NOTES.md`. The evidence fixture also passed JSON parsing and `git diff --check` passed.
  - Context impact: repository-wide behavior and Claude adapter lifecycle boundary; updated the background-execution context document and fixture notes with the current-version probe limitation and the stop decision so T02 cannot assume an unproven terminal contract.
  - Context synchronization: synced
  - Historical status (clarification added by the 2026-10-02 plan revision; the
    record above is unchanged): The T01 lifecycle gate remains the historical
    technical conclusion: SCE cannot implement attribution-safe background
    tracking from the observed Claude lifecycle. A subsequent explicit product
    decision supersedes the associated admission policy by permitting native
    background Bash as an unsupported attribution boundary. The two do not
    conflict: T01 failed to establish safe lifecycle tracking (G1-G7 not
    proven, so the hook-driven attribution-safe lifecycle design cannot
    proceed), and the later product decision accepts untracked execution
    anyway. The `stop_and_revise_toward_sce_owned_process_supervision` outcome
    in `probe18-t01-current-version.evidence.json` is historical evidence from
    the plan state that existed when T01 ran and is not modified. The task
    numbers referenced in the record above predate the 2026-10-02 renumbering.

- [ ] T02: `Record the untracked background Bash product decision and supersede the fail-closed ADR` (status:todo)
  - Task ID: T02
  - Scope: In — update `context/cli/claude-mutation-scope-background-execution.md` so it records the decision and the pending implementation as two distinct facts, in wording equivalent to: "A new product decision permits native Claude background Bash as an unsupported mutation-attribution boundary. The current adapter still denies `Bash(run_in_background=true)` until T03 of `claude-background-shell-mutation-scope` implements that decision." The document keeps the existing deny text as the adapter's present behavior and describes the accepted behavior the decision commits to: no mutation scope, no adapter attempt, no lifecycle tracking, background mutations may contaminate later or concurrent tracked attribution, lifecycle completion remains unproven. It includes the accepted A/B consequence (background A starts untracked; foreground B starts tracked; A mutates the repository while B is live; B closes; SCE may observe A's mutation at B's boundary and may attribute it to B; this is outside SCE's guarantees) stated as an accepted compatibility tradeoff rather than a bug. Apply the same decision-versus-implementation distinction to the background section of `context/cli/claude-mutation-scope-integration.md` and the two `context/context-map.md` annotations, which point to the new effective decision while stating the adapter still denies until T03. Have the T02 context-synchronization decision gate write, through `sce-decision`, one new Accepted ADR with `Supersedes: context/decisions/2026-10-01-claude-background-admission-requires-proven-lifecycle.md` and add its context-map entry. Preserve the T01 evidence section, the probe14/probe17 evidence, the D20 self-detached descendant boundary, and the statement that Claude lifecycle completion remains unproven. Out — any wording that says or implies the adapter already admits background Bash; the final "operationally supported" current-state contract (T03); any edit to the 2026-10-01 ADR (status, backlink, or body); any edit to `probe18-t01-current-version.evidence.json` or `fixtures/NOTES.md`; any Rust or test change; the code-order description of `handle_pre_tool_use` in the integration document (T03).
  - Dependencies: T01
  - Done when: a new ADR exists and is `Accepted`; the new ADR has `Supersedes:` naming the 2026-10-01 ADR and records native background Bash as an accepted unsupported-attribution boundary with the A/B consequence; the 2026-10-01 ADR is byte-unchanged; the background-execution context, the integration context, and the context-map annotations each state the decision, state that the adapter still denies `Bash(run_in_background=true)` until T03, and name the new ADR rather than the 2026-10-01 ADR as the effective decision; no document implies the code already admits background Bash; PowerShell background is still documented as denied; T02 is `synced`. Only after that may T03 enable native background Bash.
  - Verify: comparing T02's start revision (the `/next-task` Git baseline) with T02's end revision, the task-local change list — the baseline-relative `Files changed` record, equivalent to `git diff --name-only <T02-start-revision>..<T02-end-revision>` — contains only `context/cli/claude-mutation-scope-background-execution.md`, `context/cli/claude-mutation-scope-integration.md`, `context/context-map.md`, the new `context/decisions/<new-superseding-adr>.md`, and `context/plans/claude-background-shell-mutation-scope.md`; the 2026-10-01 ADR and `probe18-t01-current-version.evidence.json` are absent from that list; inspect the new ADR for `Status: Accepted`, the `Supersedes:` line, the decision statement, and the A/B block; inspect the three context files for the decision statement, the "adapter still denies until T03" statement, and the retained T01, probe14/probe17, and D20 material.
  - Context synchronization: pending

- [ ] T03: `Allow native background Bash without entering the mutation-scope adapter` (status:todo)
  - Task ID: T03
  - Scope: In — `cli/src/services/hooks/claude_mutation_scope/lifecycle.rs` and its focused adapter tests in `tests.rs`. In `handle_pre_tool_use`, `Bash` with `run_in_background=true` returns `String::new()` before repository resolution, the recovery barrier, model-state resolution, and `establish_start`, creating no adapter attempt and no mutation scope; `PowerShell` with `run_in_background=true` keeps the existing deny; foreground tools keep existing behavior. Replace the existing background-Bash denial tests with admission regressions; keep the PowerShell denial regression; add terminal-hook regressions showing a later `PostToolUse` and `PostToolUseFailure` for the untracked background tool are no-ops because `handle_close()` finds no matching `AttemptKey`, both with no live attempt and with an unrelated foreground attempt live whose adapter state must not be closed or mutated; add the A/B contamination regression (A = native background Bash, intentionally untracked; B = tracked foreground mutation scope; A mutates while B is live; B closes) asserting the actual existing protocol result and documenting that it carries no attribution guarantee. T03's context synchronization moves the three context files T02 touched to their final current-state contract — "Native Claude background Bash is operationally supported but unsupported for mutation attribution" — removing the "adapter still denies until T03" statement and the Bash deny text while keeping the PowerShell denial. Out — attribution filtering, worktree taint, suppressors, unknown epochs, process supervision, lifecycle correlation, reset commands, any state/protocol/schema change, generic ingress change, or background PowerShell support.
  - Dependencies: T02
  - Done when: background Bash returns empty output without resolving the repo, invoking the generic seam, creating adapter state, or creating a mutation scope/attempt; background PowerShell still returns the existing deny payload; later `PostToolUse` and `PostToolUseFailure` with no matching attempt remain harmless no-ops, including when an unrelated foreground attempt is live; the A/B regression pins the existing protocol result for a mutation from untracked A observed at tracked B's boundary; after T03's context synchronization, `context/cli/claude-mutation-scope-background-execution.md`, `context/cli/claude-mutation-scope-integration.md`, and the `context/context-map.md` annotations state the final contract with no pending-implementation wording, and the integration context's `handle_pre_tool_use` order description matches the code.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_mutation_scope` passes, covering empty allow output, zero resolver and `start` calls, zero attempt/scope rows, unchanged state, the PowerShell deny, both terminal-hook no-op paths, and the A/B regression; the existing missing-attempt branch remains sufficient or receives only the smallest adapter-local correction; comparing T03's start revision (the `/next-task` Git baseline) with T03's end revision, the task-local change list contains only `cli/src/services/hooks/claude_mutation_scope/lifecycle.rs`, `cli/src/services/hooks/claude_mutation_scope/tests.rs`, the three context files named above, and this plan.
  - Context synchronization: pending

- [ ] T04: `Lock the unchanged foreground Claude contract` (status:todo)
  - Task ID: T04
  - Scope: In — preserve or add narrow foreground assertions in the Claude adapter tests: foreground `Bash` (`run_in_background` false or absent) still establishes, closes, and fails through the existing tracked path; other mutation-capable Claude tools (including unknown tool names) are unchanged, including when they carry `run_in_background=true`; read-only and delegation tools are unchanged; a self-detaching foreground command keeps the ordinary tracked path with the probe17 fixtures and D20 documentation untouched. Out — changing foreground lifecycle semantics, broad test rewrites, fixture edits, or a validation-only repository-wide task.
  - Dependencies: T03
  - Done when: foreground behavior and stable output remain byte-for-byte or semantically unchanged, the focused foreground assertions pass alongside the existing Claude adapter tests, and the cumulative diff contains only the intentional Bash background admission plus focused coverage and the planned context/decision records.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_mutation_scope` passes; comparing T04's start revision (the `/next-task` Git baseline) with T04's end revision, the task-local change list contains only `cli/src/services/hooks/claude_mutation_scope/tests.rs`, this plan, and any context file its synchronization touches; as deliberately plan-wide checks, `git diff main -- cli/src/services/hooks/claude_mutation_scope/fixtures/` shows no `probe14-*` or `probe17-*` change and `git diff --name-only main` shows no change to `spec/mutation_cursor.qnt`, `cli/src/services/mutation_trace/`, `cli/migrations/`, doctor sources, or the adapter state format.
  - Context synchronization: pending

## Open questions

None. The product direction, ADR supersession convention, task ordering, and
the decision-versus-implementation wording for the T02→T03 window are all
specified by the user.
