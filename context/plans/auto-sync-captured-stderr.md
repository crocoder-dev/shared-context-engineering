# Plan: auto-sync-captured-stderr

## Change summary

Replace the automatic sync child’s inherited stderr with an explicitly piped and
captured stderr stream while retaining the already-approved synchronous wait at
the post-commit launcher boundary. The launcher will drain the child output while
waiting, then forward the captured bytes through the parent’s stderr so typed
automatic-sync failures remain visible without letting the child inherit or hold
the caller’s stderr descriptor.

This is a focused follow-up to the synchronous launcher implementation. It keeps
the existing command, repository-root working directory, null stdin/stdout,
automatic-invocation marker, fail-open behavior, and no-daemon/no-retry boundary;
only child stderr ownership and the corresponding documentation change.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the
check that proves it. `/validate` runs these checks; no task in the stack
performs final validation.

- [x] AC1: The automatic launcher waits for a child whose stderr is piped rather than inherited, drains that pipe without deadlocking, and never leaves the child holding the post-commit caller’s stderr descriptor.
  - Validate: Focused `auto_sync` tests and inspection of `spawn_command`/child-wait code assert `Stdio::piped()` plus `wait_with_output` (or equivalent concurrent drain), with no `Stdio::inherit()` on the automatic child.
- [x] AC2: Captured child stderr is forwarded to the parent stderr after completion, preserving automatic failure visibility while successful `sync --format json` remains stdout-silent; non-zero child exits, startup failures, and wait failures remain fail-open without duplicate launcher diagnostics.
  - Validate: Focused launcher and post-commit hook tests assert captured stderr forwarding, command/stream configuration, fail-open outcomes, and the existing single typed diagnostic behavior.
- [x] AC3: Durable context consistently describes synchronous automatic synchronization with parent-owned forwarding of captured child stderr, without stale inherited-stderr claims or changed manual-sync/no-daemon semantics.
  - Validate: Review the listed context contracts against the final launcher and run `git diff --check`.

### Full validation

Repository-wide checks `/validate` runs after the last task, regardless of
which criterion they map to.

- `nix flake check`
- `nix run .#pkl-check-generated`

### Context sync

- `context/overview.md`
- `context/architecture.md`
- `context/context-map.md`
- `context/glossary.md`
- `context/patterns.md`
- `context/cli/agent-trace-auto-sync.md`
- `context/cli/sync-command.md`
- `context/cli/agent-trace-sync-command.md`
- `context/sce/cli-stdout-stderr-contract.md`
- `context/sce/agent-trace-hooks-command-routing.md`
- A new dated decision record documenting the captured-stderr transport and its relationship to `2026-08-31-synchronous-automatic-sync-completion.md`.

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** `cli/src/services/sync/auto_sync.rs`, focused automatic-sync and post-commit tests, and the durable context contracts that describe automatic child process and stderr behavior.
- **Out of scope:** manual `sce sync` stream routing, control-plane protocol, sync payloads, Agent Trace storage, hook trigger frequency, automatic-sync error taxonomy, generated configuration, and the accepted decision that the launcher waits for terminal completion.
- **Constraints:** preserve `sync --format json`, repository-root cwd, null stdin/stdout, `SCE_INTERNAL_AUTO_SYNC=1`, fail-open child/startup/wait behavior, typed diagnostic ownership, bounded resource use, and repository Nix verification conventions. Drain piped stderr while waiting rather than calling a blocking wait that can deadlock on a full pipe.
- **Non-goal:** suppressing automatic diagnostics, adding a timeout/retry/queue/daemon, or redesigning the application-wide stdout/stderr contract.

## Assumptions

- The review comment’s “inherited stderr” objection means the child must not inherit the caller’s file descriptor; forwarding captured bytes from the synchronous parent preserves the existing operator-visible diagnostics without retaining that descriptor.
- The already-implemented synchronous completion policy remains selected; only the stderr transport is being corrected.
- Existing local Rust seams may be extended for captured `std::process::Output` and test doubles without adding a dependency, following the repository’s focused service/test pattern.

## Task stack

- [x] T01: `Capture and forward automatic-sync child stderr` (status:done)
  - Task ID: T01
  - Scope: In — automatic launcher process configuration and child-wait seam in `cli/src/services/sync/auto_sync.rs`; focused tests for piped stderr, concurrent draining/capture, forwarding, command preservation, and fail-open success/non-zero/startup/wait outcomes. Out — manual sync behavior, hook trigger policy, sync protocol, and durable context edits.
  - Dependencies: none
  - Done when: the launcher uses a non-inherited stderr pipe, drains it as part of terminal child completion, forwards captured bytes through the parent stderr path, and preserves all existing automatic-sync command, diagnostic, and fail-open contracts; focused tests pass.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml auto_sync`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml hooks::`
  - Context synchronization: synced
  - Completed: 2026-09-01
  - Files changed: `cli/src/services/sync/auto_sync.rs`
  - Result: Replaced inherited automatic-sync stderr with `Stdio::piped()`, switched the child seam to `wait_with_output()` so stderr is drained without deadlock, and forwarded captured bytes through the parent stderr path while preserving non-zero child, startup, and wait fail-open behavior. Added focused capture, non-zero-exit, wait-failure, and command-configuration coverage.
  - Evidence: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml auto_sync` passed (16 tests); `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml hooks::` passed (163 tests); `nix develop -c sh -c 'cd cli && cargo fmt -- --check'` passed; `git diff --check` passed.
  - Context impact: material — automatic synchronization process ownership and stderr forwarding behavior changed; T02 must update the listed durable context contracts.

- [x] T02: `Document parent-owned automatic-sync stderr forwarding` (status:complete)
  - Task ID: T02
  - Scope: In — the listed root, CLI, stream, hook-routing, glossary, pattern, and new decision records. Out — historical plan/decision rewriting, generated target trees, and unrelated CLI stream documentation.
  - Dependencies: T01
  - Done when: durable context states that automatic sync waits synchronously, the child stderr is piped/drained and forwarded by the parent, diagnostics remain visible and fail-open, and no stale inherited-stderr claim remains in current-state contracts.
  - Verify: Manual code/context review against `cli/src/services/sync/auto_sync.rs`, `cli/src/services/hooks/mod.rs`, `cli/src/services/sync/command.rs`, and `cli/src/services/app_support.rs`; `git diff --check`
  - Context synchronization: synced
  - Completed: 2026-09-01
  - Files changed: `context/overview.md`
  - Result: Corrected the root overview's stale asynchronous and detached automatic-sync descriptions; current-state contracts now consistently describe synchronous completion, piped/drained child stderr, parent forwarding, visible fail-open diagnostics, and preserved manual/no-daemon/no-retry boundaries.
  - Verify: Manual code/context review — passed; `git diff --check` — passed.
  - Context impact: material — current root and domain contracts describe cross-cutting automatic-sync process ownership and stderr transport; the mandatory context synchronization pass remains required.

## Open questions

None. The review supplies the required transport correction; capturing and
forwarding through the already-synchronous launcher is the smaller compatible
alternative to either inheriting stderr or suppressing automatic diagnostics.

## Validation Report

**Status:** validated  
**Date:** 2026-09-01

### Commands run

- `nix develop -c sh -c './scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml auto_sync && ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml hooks::'` -> exit 0 (16 automatic-sync tests and 163 hook tests passed)
- `nix shell nixpkgs#ripgrep -c rg -n 'Stdio::inherit|Stdio::piped|wait_with_output|wait\\(' cli/src/services/sync/auto_sync.rs` -> exit 0 (automatic child uses piped stderr and `wait_with_output`; no inherited stderr)
- `git diff --check` -> exit 0 (no whitespace errors)
- `nix flake check` -> exit 0 (all flake checks passed)
- `nix run .#pkl-check-generated` -> exit 0 (ephemeral generated-config parity passed)

### Success-criteria verification

- [x] AC1: The automatic launcher waits for a child whose stderr is piped rather than inherited, drains that pipe without deadlocking, and never leaves the child holding the post-commit caller’s stderr descriptor. -> Source inspection confirmed `Stdio::piped()` and `wait_with_output()` with no `Stdio::inherit()`; focused automatic-sync tests passed.
- [x] AC2: Captured child stderr is forwarded to the parent stderr after completion, preserving automatic failure visibility while successful `sync --format json` remains stdout-silent; non-zero child exits, startup failures, and wait failures remain fail-open without duplicate launcher diagnostics. -> Focused automatic-sync and post-commit hook tests passed, covering captured forwarding, command configuration, successful/non-zero/startup/wait outcomes, and one-diagnostic behavior.
- [x] AC3: Durable context consistently describes synchronous automatic synchronization with parent-owned forwarding of captured child stderr, without stale inherited-stderr claims or changed manual-sync/no-daemon semantics. -> Reviewed the listed current-state context contracts against the launcher and `git diff --check` passed.

### Failed checks and follow-ups

- None.

### Residual risks

- None identified.
