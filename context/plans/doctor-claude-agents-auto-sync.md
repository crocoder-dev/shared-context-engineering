# Plan: doctor-claude-agents-auto-sync

## Change summary

Align `sce doctor` with the generated target inventory by removing the stale Claude `Agents` inspection and output while retaining OpenCode agent inspection. Add explicit doctor coverage for the post-commit automatic Agent Trace sync capability: inspect the installed canonical `post-commit` SCE managed block using the same merge/current semantics as setup, resolve the effective `agent_trace.auto_sync` setting, and report whether automatic sync is enabled, intentionally disabled, or not ready.

This is worth building because the current Claude row reports an asset that setup never generates, while the new post-commit trigger can silently stop being available through hook drift or configuration state even though the existing generic hook checks do not explain that capability. A smaller alternative would only remove the Claude row and rely on the existing hook-content check; that fixes the false Claude failure but does not make the automatic-sync readiness and explicit opt-out observable, so it does not satisfy the operator-health gap.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the
check that proves it. `/validate` runs these checks; no task in the stack
performs final validation.

- [x] AC1: Claude doctor inspection and both doctor renderers expose only the generated Claude areas (`Plugins`, `Commands`, and `Skills`), while OpenCode continues to expose its `Agents` area and existing target-scoped ordering remains deterministic.
  - Validate: focused doctor tests assert no Claude `Agents` group/children or rendered `Claude Code` agent label and assert the OpenCode `Agents` group remains present; inspect text and JSON fixtures for the same target inventory.
- [x] AC2: In a repository with a current installed canonical `post-commit` managed block and resolved `agent_trace.auto_sync: true` (including the omitted default), doctor reports automatic sync as enabled and ready in text and `--format json`; with explicit `false`, it reports a healthy intentional disabled opt-out without marking overall readiness not ready.
  - Validate: focused doctor/config tests assert the text status/label and stable JSON auto-sync state, enabled value, and resolved source for default, configured true, and configured false cases.
- [x] AC3: Doctor reports automatic sync as not ready when the effective `post-commit` managed block is missing, stale, unreadable, or otherwise not current, while preserving the existing hook problem/remediation and readiness behavior; doctor never launches `sce sync` or any background process.
  - Validate: filesystem-backed hook/doctor tests cover current, drifted, missing, and unreadable post-commit states and assert no launcher invocation; existing hook lifecycle tests continue to pass.
- [x] AC4: The post-commit runtime still forwards `origin` metadata, launches the existing detached `sync --format json` only after successful Agent Trace persistence when enabled, and remains fail-open for launcher failures; no canonical hook asset, launcher semantics, setup flow, or high-frequency trigger changes.
  - Validate: focused hook tests and inspection of `cli/assets/hooks/post-commit`, `hooks/mod.rs`, and `sync/auto_sync.rs` confirm the existing ordering, argument forwarding, and fail-open behavior are unchanged.
- [x] AC5: Durable context accurately describes the Claude target inventory and the doctor automatic-sync readiness, text, JSON, default, and opt-out contracts without claiming that Claude generates agents.
  - Validate: manual review of the context files listed under `Context sync` against the implemented report fields and focused tests.

### Full validation

Repository-wide checks `/validate` runs after the last task, regardless of
which criterion they map to.

- `nix flake check`

### Context sync

- `context/sce/agent-trace-hook-doctor.md`
- `context/cli/cli-command-surface.md`
- `context/architecture.md`
- `context/overview.md`
- `context/sce/doctor-human-text-contract.md` when the new text row/status shape is implemented

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`. A completed task must be `synced` before another task can start or the plan can finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition** beside the status. Never infer `synced` from conversation history; write every lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** Claude-specific doctor integration grouping and ordering/output claims; OpenCode agent preservation; doctor report types, inspection, rendering, JSON fields, and focused tests; hook lifecycle reuse of canonical managed-block/current semantics; resolved `agent_trace.auto_sync` readiness reporting; the durable context files listed under Context sync.
- **Out of scope:** changing `cli/assets/hooks/{pre-commit,commit-msg,post-commit}`; changing setup installation or merge behavior; changing `cli/src/services/hooks/mod.rs` post-commit execution; changing `cli/src/services/sync/auto_sync.rs`; launching sync from doctor; adding a daemon, watcher, scheduler, retry queue, or new synchronization engine; removing the shared `IntegrationArea::Agents` model required by OpenCode.
- **Constraints:** use the existing canonical hook asset and `hook_merge`/managed-block currency semantics rather than a second parser or shell execution; resolve `agent_trace.auto_sync` through the existing config resolver with default `true` and global-then-local precedence; preserve existing problem categories, remediation, exit/readiness semantics, text status vocabulary, JSON compatibility, and setup/doctor ownership boundaries; use repository wrappers and Nix-managed validation commands.
- **Non-goal:** making an explicit `agent_trace.auto_sync: false` opt-out fail doctor readiness or making doctor prove that a detached sync child completed.

## Assumptions

- This is a new plan: the existing automatic-sync and doctor plans are completed historical plans with different scopes, not revision targets.
- Automatic-sync readiness is a doctor report fact, not a new independent failure class: existing hook/config problems continue to determine overall readiness, while the new fact explains the capability state without duplicating remediation records.
- A current canonical `post-commit` managed block is the readiness proof for the hook side; doctor does not execute the hook, invoke the launcher, or inspect child-process/network outcomes.
- Text reports a ready enabled state as `[PASS] Post-commit Agent Trace auto-sync`, a deliberate opt-out as `[PASS] Post-commit Agent Trace auto-sync (disabled by config)`, and a non-ready repository hook state as `[FAIL] Post-commit Agent Trace auto-sync`; JSON carries a stable `post_commit_auto_sync` object with `state`, `enabled`, and resolved configuration source fields, plus the existing problem records.
- The shared `IntegrationArea::Agents` enum and generic labels remain because OpenCode still owns generated agents; only Claude-specific production, rendering-order, and documentation claims are removed.

## Task stack

- [x] T01: `Remove Claude Agents from doctor integration inspection` (status:done)
  - Task ID: T01
  - Scope: In — Claude integration asset classification, group construction, target-specific area ordering/labels, and focused doctor inventory tests; preserve Claude plugins/commands/skills and the complete OpenCode plugins/agents/commands/skills inventory. Out — shared OpenCode agent types, setup assets, generated config, hook behavior, and auto-sync reporting.
   - Dependencies: none
   - Done when: Claude doctor reports no `Agents` group or agent asset expectation in text or JSON, OpenCode still reports its `Agents` group, deterministic target ordering remains valid, and focused doctor tests cover the regression.
   - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml doctor::` with assertions for Claude group inventory and OpenCode agent preservation; inspect `doctor/inspect.rs`, `doctor/render.rs`, and `doctor/types.rs` for no Claude-specific agent production path.
   - Context synchronization: synced
   - Completed: 2026-08-20
   - Files changed: `cli/src/services/default_paths.rs`, `cli/src/services/doctor/inspect.rs`, `cli/src/services/doctor/render.rs`, `cli/src/services/doctor/types.rs`, `context/plans/doctor-claude-agents-auto-sync.md`
   - Result: Removed Claude agent asset classification and group construction while preserving OpenCode agents, Claude plugins/commands/skills, target-specific ordering, and focused regression coverage.
   - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml doctor::` — pass (10 tests); inspection confirmed the Claude collector has no agent production path and OpenCode retains its Agents group.
   - Context impact: interface — Claude doctor inventory and rendered target-area ordering changed; the listed doctor, CLI surface, architecture, overview, and human-text contract context files require synchronization before the next task.

- [x] T02: `Report post-commit automatic-sync readiness in doctor` (status:done)
  - Task ID: T02
  - Scope: In — `HooksLifecycle`/doctor inspection seam using canonical embedded `post-commit` managed-block currency, resolved `agent_trace.auto_sync` state, doctor report types, human text row, JSON object, and filesystem/config/format regression tests. Out — canonical hook files, setup installation, the post-commit runtime trigger, sync launcher implementation, new daemon/retry behavior, and unrelated hook problem taxonomy changes.
  - Dependencies: T01
  - Done when: doctor deterministically reports enabled/default, explicit disabled, not-ready, and not-applicable states; current hook plus resolved configuration produces ready output; hook drift/missing/read failures preserve existing problem/remediation and readiness behavior; doctor performs no sync launch; text and JSON output are stable and tested.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml doctor::`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml hooks::`; inspect `cli/src/services/config/resolver.rs`, `cli/src/services/hooks/lifecycle.rs`, `cli/src/services/hooks/mod.rs`, and `cli/src/services/sync/auto_sync.rs` to confirm the existing default/opt-out and fail-open runtime contracts remain unchanged.
  - Context synchronization: synced
  - Completed: 2026-08-20
  - Files changed: `cli/src/services/config/mod.rs`, `cli/src/services/config/resolver.rs`, `cli/src/services/doctor/inspect.rs`, `cli/src/services/doctor/render.rs`, `cli/src/services/doctor/types.rs`
  - Result: Added deterministic post-commit Agent Trace auto-sync readiness facts using canonical managed-block currency and resolved configuration, with enabled/disabled/not-ready/not-applicable text and JSON reporting while preserving existing hook problems, readiness, and runtime launcher behavior.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml doctor::` — pass (15 tests); `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml hooks::` — pass (25 tests); inspected `cli/src/services/config/resolver.rs`, `cli/src/services/hooks/lifecycle.rs`, `cli/src/services/hooks/mod.rs`, and `cli/src/services/sync/auto_sync.rs` — existing default/opt-out, managed-block, forwarding, ordering, detached-launch, and fail-open contracts remain unchanged.
  - Context impact: interface — doctor report types, human text, JSON fields, and configuration-resolution consumption changed; `context/sce/agent-trace-hook-doctor.md`, `context/cli/cli-command-surface.md`, `context/architecture.md`, `context/overview.md`, and `context/sce/doctor-human-text-contract.md` require synchronization before the next task.

- [x] T03: `Synchronize doctor and automatic-sync context contracts` (status:done)
  - Task ID: T03
  - Scope: In — update the durable doctor operator contract, CLI command surface, architecture/overview summaries, and human text contract as required by the implemented state/field names and Claude inventory. Out — application code, tests, generated outputs, historical plan files, and any change to the runtime behavior.
  - Dependencies: T02
  - Done when: the named context files no longer claim Claude has generated agents and document the doctor auto-sync readiness proof, default-enabled setting, explicit opt-out, text/JSON observables, preserved existing hook remediation, and no-launch/fail-open boundaries.
  - Verify: manual code-to-context review against T01/T02 output and the focused doctor/hook test results; confirm no context file outside the listed sync set was changed.
  - Context synchronization: synced
  - Completed: 2026-08-20
  - Files changed: `context/sce/agent-trace-hook-doctor.md`, `context/cli/cli-command-surface.md`, `context/architecture.md`, `context/overview.md`, `context/sce/doctor-human-text-contract.md`
  - Result: Synchronized the five durable doctor context contracts with the implemented Claude inventory and post-commit auto-sync readiness states, fields, configuration sources, remediation boundaries, and fail-open/no-launch behavior.
  - Verify: manual code-to-context review against T01/T02 implementation and recorded focused doctor/hook results — pass; `git diff --check` — pass; changed paths are limited to the five listed sync files before the required plan-state write.
  - Context impact: documentation — current doctor inventory, text/JSON output, configuration-resolution, and hook-runtime boundaries are now synchronized; no application behavior changed.

## Open questions

None. The supplied brief and repository conventions determine the readiness proof, opt-out semantics, output shape, ownership boundary, and non-goals; the remaining choices are recorded assumptions.

## Validation Report

**Status:** validated
**Date:** 2026-08-20

### Commands run

- `nix flake check` -> exit 0 (flake evaluation passed; the command reported no checks executed for the current system)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml doctor::` -> exit 0 (15 focused doctor tests passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml hooks::` -> exit 0 (25 focused hook tests passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml config::` -> exit 0 (26 focused config tests passed)
- `git status --short && git diff --check` -> exit 0 (only intended plan/application/context paths are modified; no whitespace errors)

### Success-criteria verification

- [x] AC1: Claude doctor inspection and both doctor renderers expose only the generated Claude areas (`Plugins`, `Commands`, and `Skills`), while OpenCode continues to expose its `Agents` area and existing target-scoped ordering remains deterministic. -> 15 focused doctor tests passed, including exact Claude/OpenCode inventory, Claude agent exclusion, OpenCode agent preservation, and deterministic text ordering; source and renderer tests confirm the text/JSON inventory contract.
- [x] AC2: In a repository with a current installed canonical `post-commit` managed block and resolved `agent_trace.auto_sync: true` (including the omitted default), doctor reports automatic sync as enabled and ready in text and `--format json`; with explicit `false`, it reports a healthy intentional disabled opt-out without marking overall readiness not ready. -> Doctor tests passed for default-enabled, configured true, and explicit disabled text/JSON states; 26 config tests passed for default, configured true, local precedence, and explicit false resolution.
- [x] AC3: Doctor reports automatic sync as not ready when the effective `post-commit` managed block is missing, stale, unreadable, or otherwise not current, while preserving the existing hook problem/remediation and readiness behavior; doctor never launches `sce sync` or any background process. -> Doctor tests passed for current, missing, stale, and unreadable states plus hook diagnostics; focused hook tests passed and source inspection confirmed doctor has no sync-launch path.
- [x] AC4: The post-commit runtime still forwards `origin` metadata, launches the existing detached `sync --format json` only after successful Agent Trace persistence when enabled, and remains fail-open for launcher failures; no canonical hook asset, launcher semantics, setup flow, or high-frequency trigger changes. -> 25 focused hook tests passed; inspection of `cli/assets/hooks/post-commit`, `cli/src/services/hooks/mod.rs`, and `cli/src/services/sync/auto_sync.rs` confirmed origin forwarding, persistence-before-launch ordering, detached JSON sync, and fail-open behavior.
- [x] AC5: Durable context accurately describes the Claude target inventory and the doctor automatic-sync readiness, text, JSON, default, and opt-out contracts without claiming that Claude generates agents. -> Manual review of the five listed context files against the implemented report fields and focused tests confirmed the Claude inventory, readiness states, source fields, opt-out, no-launch, and fail-open contracts; the worktree contains no untracked artifacts and `git diff --check` passed.

### Failed checks and follow-ups

- None.

### Residual risks

- None identified.
