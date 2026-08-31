# Plan: auto-sync-failure-guidance

## Change summary

The completed T01-T03 work gives post-commit automatic Agent Trace
synchronization typed, user-facing failure diagnostics. Following the
completion-policy investigation, this revision changes the automatic launcher
to wait for its one-shot `sce sync --format json` child to reach terminal
completion before the post-commit hook returns. Child failures and wait errors
remain fail-open, while the child remains responsible for its typed diagnostic
so the parent does not duplicate it.

The exact command, repository-root working directory, null stdout, inherited
stderr, internal automatic-invocation marker, one-shot architecture, and
no-daemon/no-retry boundaries remain unchanged. Manual `sce sync` semantics also
remain unchanged; only the automatic post-commit completion boundary changes.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the
check that proves it. `/validate` runs these checks; no task in the stack
performs final validation.

- [x] AC1: A sync failure raised by an automatic invocation maps to a payload-bearing typed `UserError`/`CliError` path and renders one runtime diagnostic that clearly says automatic synchronization failed and includes the underlying typed failure reason.
  - Validate: Focused sync/error tests assert the rendered diagnostic for control-plane, stream, and local runtime failures, including the reason and `SCE-ERR-RUNTIME` classification.
- [x] AC2: An automatic authentication failure uses a distinct typed automatic-sync failure kind, tells the user that authentication is required, instructs them to run `sce auth login`, and then explicitly tells them to manually retry with `sce sync`.
  - Validate: Focused authentication classification and app-rendering tests assert the complete login-plus-manual-sync guidance and ensure the technical source remains available only for observability.
- [x] AC3: Non-authentication automatic failures use the same typed payload-bearing error model, provide actionable recovery guidance, and explain that the user can manually retry with `sce sync`, without relying on substring matching or duplicating the default runtime `Try:` guidance.
  - Validate: Focused tests cover representative storage, transport/server, protocol, and stream failures and assert deterministic reason/recovery text with no duplicate remediation.
- [x] AC4: Automatic child failures are visible through the existing stderr diagnostic channel while successful post-commit execution remains JSON-stdout-silent and fail-open to the commit; launcher startup failures retain their reason in structured auto-sync diagnostics without failing the hook.
  - Validate: Launcher and post-commit seam tests assert the internal automatic-invocation marker, inherited failure stderr, unchanged `sync --format json` arguments, and fail-open behavior for executable/spawn failures; the completion policy is governed by AC7.
- [x] AC5: Manual `sce sync` failures retain the existing manual-sync semantics and do not claim that automatic synchronization failed.
  - Validate: Command-level tests execute/classify manual and automatic invocation modes separately and assert mode-specific rendering.
- [x] AC6: Durable context documents the typed automatic-sync failure model, stderr visibility, authentication recovery, manual retry command, and preserved no-daemon/fail-open boundaries.
  - Validate: Review the listed context contracts against the final code, then run the generated-context and repository checks under `Full validation`.

AC1–AC6 are the completed baseline established by T01–T03. The following
criteria govern this revision and replace the prior detached-policy experiment
follow-up.

- [x] AC7: The automatic post-commit launcher waits for the `sync --format json` child to reach terminal completion before returning, while preserving the repository-root cwd, internal marker, null stdout, inherited stderr, and fail-open hook boundary.
  - Validate: Focused auto-sync and hook tests assert that the launcher waits, preserves the command boundary, and still returns successful post-commit results.
- [x] AC8: Automatic child non-zero exits and wait errors remain fail-open; child-rendered failures are not duplicated by the launcher, while launcher startup/wait failures retain actionable typed reasons on stderr.
  - Validate: Focused launcher tests cover successful completion, non-zero child exit, wait failure, and startup failure with one diagnostic path and unchanged manual-sync behavior.
- [x] AC9: Durable context describes the synchronous automatic completion boundary, its commit-latency tradeoff, stderr behavior, manual retry path, and preserved no-daemon/no-retry constraints without stale detached-policy claims.
  - Validate: Review the listed auto-sync, hook-routing, sync-command, sync-architecture, and stdout/stderr contracts against the selected implementation, then run the generated-context and repository checks under `Full validation`.

### Full validation

Repository-wide checks `/validate` runs after the last task, regardless of
which criterion they map to.

- `nix flake check`
- `nix run .#pkl-check-generated`

### Context sync

- `context/overview.md`
- `context/architecture.md`
- `context/glossary.md`
- `context/context-map.md`
- `context/patterns.md`
- `context/cli/agent-trace-auto-sync.md`
- `context/cli/sync-command.md`
- `context/cli/agent-trace-sync-command.md`
- `context/sce/cli-error-code-taxonomy.md`
- `context/sce/cli-stdout-stderr-contract.md`
- `context/sce/agent-trace-hooks-command-routing.md`

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** typed automatic-sync failure/recovery modeling; automatic-versus-manual sync invocation context; sync command/app error rendering; detached launcher stderr and startup-failure reporting; post-commit fail-open integration; focused Rust tests; the durable context files listed under Context sync.
- **Out of scope:** changes to the control-plane protocol, cursor reconciliation, Agent Trace schema/storage, manual sync success output, authentication flow implementation, generated target trees, or unrelated hook failure behavior.
- **Constraints:** preserve the exact child arguments `sync --format json`, current-executable resolution, repository-root working directory, commit fail-open semantics, stdout/stderr separation, typed authentication classification, and shared sensitive-text redaction; wait for the automatic child to reach terminal completion; treat non-zero child exits and wait errors as fail-open; do not duplicate child-rendered diagnostics; add no dependency, timeout policy, or persistent retry state.
- **Non-goal:** making Git wait for network synchronization or adding a daemon, watcher, scheduler, queue, status file, or local retry cursor.

## Assumptions

- T01-T03 remain the recorded baseline. The user has selected synchronous waiting after reviewing the completion-policy evidence, accepting the added commit latency in exchange for deterministic child completion and stream closure.
- The automatic invocation marker is an internal process-boundary detail, not a new user configuration key or public CLI option; manual `sce sync` remains mode-neutral and keeps its existing error wording.
- The typed failure model will preserve the technical source for structured logging while rendering a reviewed, deterministic recovery sentence at the app boundary, following `CliError` and `UserError` ownership patterns.
- “Wait for the child to finish” means waiting for terminal process completion without introducing a new timeout, retry, daemon, or persistent state mechanism.

## Task stack

- [x] T01: `Add payload-bearing typed automatic-sync user errors` (status:complete)
  - Task ID: T01
  - Scope: In — `cli/src/services/error.rs`, sync invocation context/classification, app-level rendering support, and focused tests for automatic authentication, runtime, stream, and control-plane failures. Model the new error after the payload-bearing `UserError` entries added by setup remote preflight: keep a closed catalog, use a typed failure-kind payload, retain the underlying reason, and preserve the technical source through `CliError::user_with_source`. Out — child process stdio changes, hook wiring, and durable context edits.
  - Dependencies: none
  - Done when: automatic failures have a payload-bearing typed user-error representation that distinguishes authentication from other sync failures, includes the underlying reason without adding an arbitrary-message escape hatch, renders deterministic automatic-sync wording plus actionable manual retry guidance, preserves the technical source for logging, and leaves manual sync classification/rendering unchanged.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml error::`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml sync::command`.
  - Completed: 2026-08-27
  - Files changed:
    - `cli/src/services/app_support.rs`
    - `cli/src/services/command_registry.rs`
    - `cli/src/services/error.rs`
    - `cli/src/services/parse/command_runtime.rs`
    - `cli/src/services/sync/command.rs`
    - `cli/src/services/sync/mod.rs`
  - Result: Added typed automatic-sync failure kinds and payload-bearing user errors with preserved technical sources, deterministic authentication and recovery guidance, and an explicit manual-versus-automatic sync invocation context. Manual sync classification remains unchanged.
  - Verify:
    - `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml error::` — passed (6 tests).
    - `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml sync::command` — passed (8 tests).
  - Context impact: root — changed the typed CLI error contract, sync command invocation classification, and app-level diagnostic rendering; durable context synchronization is required before another task starts.
  - Context synchronization: synced

- [x] T02: `Surface detached automatic-sync failures without blocking post-commit` (status:complete)
  - Task ID: T02
  - Scope: In — `cli/src/services/sync/auto_sync.rs`, post-commit launcher seam in `cli/src/services/hooks/mod.rs`, internal child invocation marker/stdio configuration, structured launcher-failure reporting using the same typed automatic-sync error payload, and focused launcher/hook tests. Out — sync protocol behavior, waiting for child completion, retry queues, and high-frequency hook triggers.
  - Dependencies: T01
  - Done when: the detached child keeps the exact `sync --format json` command and repository-root/no-wait behavior, identifies automatic mode, exposes only typed failure diagnostics through stderr, and launcher executable/spawn errors retain actionable reasons through structured auto-sync reporting while remaining fail-open to the successful hook result.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml auto_sync`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml hooks::`.
  - Completed: 2026-08-27
  - Files changed:
    - `cli/src/services/app_support.rs`
    - `cli/src/services/error.rs`
    - `cli/src/services/parse/command_runtime.rs`
    - `cli/src/services/sync/auto_sync.rs`
    - `cli/src/services/sync/command.rs`
    - `cli/src/services/sync/mod.rs`
  - Result: Preserved detached `sync --format json` execution while passing an internal automatic-invocation marker, inheriting child stderr for typed failure diagnostics, and rendering structured fail-open launcher errors with actionable reasons.
  - Verify:
    - `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml auto_sync` — passed (14 tests).
    - `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml hooks::` — passed (163 tests).
  - Context impact: root — changed the automatic sync process boundary, stderr visibility, invocation classification, and fail-open launcher diagnostic contract; durable context synchronization is required before another task starts.
  - Context synchronization: synced

- [x] T03: `Document typed automatic-sync failure recovery contract` (status:complete)
  - Task ID: T03
  - Scope: Update the auto-sync, sync, CLI error, stdout/stderr, hook-routing, and required root context contracts listed under Context sync to describe the final typed error and recovery behavior. Out — generated configuration artifacts, historical plans/decisions, and code/test changes.
  - Dependencies: T02
  - Done when: durable context states the payload-bearing typed error model, automatic-failure prefix, reason preservation, authentication login flow, manual `sce sync` retry, stderr visibility, mode distinction, and unchanged detached/fail-open/no-daemon boundaries without stale null-output claims.
  - Verify: Manual code/context review against `cli/src/services/error.rs`, `cli/src/services/app_support.rs`, `cli/src/services/sync/command.rs`, `cli/src/services/sync/auto_sync.rs`, and `cli/src/services/hooks/mod.rs`.
   - Completed: 2026-08-27
   - Files changed:
     - `context/architecture.md`
     - `context/cli/agent-trace-auto-sync.md`
     - `context/cli/agent-trace-sync-command.md`
     - `context/cli/sync-command.md`
     - `context/context-map.md`
     - `context/glossary.md`
     - `context/overview.md`
     - `context/patterns.md`
     - `context/sce/agent-trace-hooks-command-routing.md`
     - `context/sce/cli-error-code-taxonomy.md`
     - `context/sce/cli-stdout-stderr-contract.md`
   - Result: Updated durable root and domain context to describe the closed typed automatic-sync failure catalog, the stable automatic-failure diagnostic prefix, authentication login-plus-manual-sync recovery, preserved non-authentication reasons, stderr visibility, manual-mode distinction, and unchanged detached/fail-open/no-daemon boundaries.
   - Verify:
     - `Manual code/context review against cli/src/services/error.rs, cli/src/services/app_support.rs, cli/src/services/sync/command.rs, cli/src/services/sync/auto_sync.rs, and cli/src/services/hooks/mod.rs` — passed.
     - `git diff --check` — passed.
   - Context impact: root — clarified the durable CLI error, stream, synchronization, hook-routing, and recovery contracts to match the implemented automatic-sync behavior.
   - Context synchronization: synced

- [x] T04: `Measure Git commit impact of automatic-sync completion policies` (status:complete)
  - Task ID: T04
  - Scope: In — a repository-owned focused experiment/benchmark using a real temporary Git repository and commit, a controlled child that delays and writes stderr, and comparable detached-plus-inherited-stderr, wait-to-exit, and (where useful) stderr-null runs; measure direct commit duration, a pipeline consuming stderr, output ordering, and pipe closure without changing production behavior. Out — choosing or implementing the production policy, network synchronization, persistent benchmark infrastructure, and changes to the completed T01-T03 contract.
  - Dependencies: T03
  - Done when: the experiment runs deterministically enough to compare the policies, asserts the expected child-output and pipe-lifetime behavior, reports the measured commit/pipeline timings, and records a clear recommendation about whether waiting's latency is acceptable for post-commit use.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml auto_sync -- --nocapture`; retain the focused experiment output and measurements in the task completion evidence.
  - Completed: 2026-08-28
  - Files changed:
    - `cli/src/services/sync/auto_sync.rs`
  - Result: Added a Unix-focused repository-owned experiment that creates temporary Git repositories, runs real post-commit hooks, and compares detached inherited-stderr, waiting, and null-output child policies. The experiment captures direct commit latency, stderr pipe closure, stderr-consuming pipeline completion, output presence/order, and recommends retaining detached launch because waiting added roughly 250 ms to the direct commit in the measured run while inherited stderr kept the pipeline observable.
  - Verify:
    - `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml auto_sync -- --nocapture` — passed (14 tests); experiment measurements: detached direct commit 6 ms / pipe close 259 ms / pipeline 266 ms, wait direct commit 258 ms / pipe close 258 ms / pipeline 261 ms, null direct commit 6 ms / pipe close 6 ms / pipeline 7 ms.
    - `nix develop -c sh -c 'cd cli && cargo fmt'` — passed.
  - Context impact: root — established experiment-backed evidence for the automatic-sync completion and inherited-stderr policy; durable context synchronization is required before another task starts.
  - Context synchronization: synced

- [x] T05: `Remove the T04 experiment and detached-policy context additions` (status:complete)
  - Task ID: T05
  - Scope: In — remove the T04-only repository experiment and its test support from `cli/src/services/sync/auto_sync.rs`; remove the T04-only experiment findings and detached-policy recommendation from the durable context files changed by T04 synchronization. Out — completed T01-T03 implementation and context records, the existing detached launcher behavior, and the new waiting implementation.
  - Dependencies: T04
  - Done when: no T04 experiment code or experiment-only context claims remain, the completed T04 record remains intact as historical plan evidence, and the context again describes the pre-T04 detached implementation without changing T01-T03 behavior.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml auto_sync`; `git diff --check`; focused review confirms only T04 experiment additions and their context references were removed.
  - Completed: 2026-08-31
  - Files changed:
    - `cli/src/services/sync/auto_sync.rs`
    - `context/architecture.md`
    - `context/cli/agent-trace-auto-sync.md`
    - `context/cli/sync-command.md`
    - `context/glossary.md`
    - `context/overview.md`
    - `context/patterns.md`
    - `context/sce/agent-trace-hooks-command-routing.md`
  - Result: Removed the Unix Git completion-policy experiment and all experiment-only helpers while preserving the detached launcher implementation; removed the T04 experiment findings and detached-policy references from the durable context contracts, leaving the completed T04 plan record intact.
  - Verify:
    - `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml auto_sync` — passed (13 tests).
    - `git diff --check` — passed.
    - Focused review of the affected code and context files — passed; no T04 experiment code or experiment-only context claims remain, and the detached launcher contract is unchanged.
  - Context impact: root — restored the automatic-sync implementation and durable context to the pre-T04 detached-policy baseline; durable context synchronization is required before another task starts.
  - Context synchronization: synced

- [x] T06: `Wait for automatic sync child completion at the launcher boundary` (status:complete)
  - Task ID: T06
  - Scope: In — `cli/src/services/sync/auto_sync.rs`, the launcher process boundary, and focused auto-sync/post-commit tests proving terminal wait, command preservation, output routing, and fail-open behavior. Wait for the spawned child to finish before returning; ignore a non-zero child exit after the child has rendered its own diagnostic; surface wait errors through one typed launcher diagnostic without failing the post-commit hook. Out — manual `sce sync`, control-plane behavior, timeout policy, retries, daemons, queues, and persistent failure state.
  - Dependencies: T05
  - Done when: automatic post-commit execution waits for child termination, successful and failed child exits remain fail-open, wait errors are handled without duplicate diagnostics, and the exact `sync --format json` arguments, repository-root cwd, marker, null stdout, and inherited stderr remain intact.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml auto_sync`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml hooks::`.
  - Completed: 2026-08-31
  - Files changed:
    - `cli/src/services/sync/auto_sync.rs`
  - Result: Changed the automatic launcher to retain and wait for its one-shot sync child, ignoring non-zero child exits while routing wait failures through the existing typed fail-open launcher diagnostic; preserved the command arguments, repository-root cwd, automatic marker, null stdin/stdout, and inherited stderr.
  - Verify:
    - `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml auto_sync` — passed (16 tests).
    - `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml hooks::` — passed (163 tests).
  - Context impact: root — changed the automatic sync process completion boundary and wait-failure diagnostic behavior while preserving the existing hook and stderr fail-open contracts; durable context synchronization is required before another task starts.
  - Context synchronization: synced

- [x] T07: `Document synchronous automatic-sync completion semantics` (status:complete)
  - Task ID: T07
  - Scope: Update `context/overview.md`, `context/architecture.md`, `context/glossary.md`, `context/patterns.md`, `context/cli/agent-trace-auto-sync.md`, `context/cli/sync-command.md`, `context/cli/agent-trace-sync-command.md`, `context/sce/cli-stdout-stderr-contract.md`, and `context/sce/agent-trace-hooks-command-routing.md` to describe the waiting boundary, deterministic stream closure, commit-latency tradeoff, fail-open child/wait handling, and unchanged manual retry/no-daemon constraints. Out — application code, tests, generated target trees, and changes to T01-T03 error semantics.
  - Dependencies: T06
  - Done when: durable context consistently describes automatic sync as a waited one-shot child, does not claim detached/no-wait behavior, preserves the typed stderr and manual `sce sync` recovery contracts, and keeps the no-daemon/no-retry boundaries explicit.
  - Verify: Manual code/context review against `cli/src/services/sync/auto_sync.rs`, `cli/src/services/hooks/mod.rs`, `cli/src/services/sync/command.rs`, and `cli/src/services/app_support.rs`; `git diff --check`.
  - Completed: 2026-08-31
  - Files changed:
    - `context/overview.md`
    - `context/architecture.md`
    - `context/glossary.md`
    - `context/patterns.md`
    - `context/cli/agent-trace-auto-sync.md`
    - `context/cli/sync-command.md`
    - `context/cli/agent-trace-sync-command.md`
    - `context/sce/cli-stdout-stderr-contract.md`
    - `context/sce/agent-trace-hooks-command-routing.md`
  - Result: Updated the durable root and domain context to describe the waited one-shot automatic sync child, deterministic child and inherited-stderr completion, the intentional post-commit latency trade-off, fail-open child/startup/wait handling, typed stderr recovery, and preserved manual retry/no-daemon/no-retry boundaries.
  - Verify:
    - `Manual code/context review against cli/src/services/sync/auto_sync.rs, cli/src/services/hooks/mod.rs, cli/src/services/sync/command.rs, and cli/src/services/app_support.rs` — passed; the launcher waits for terminal completion, preserves the exact command/working-directory/stdio/marker boundary, and the listed contracts match the implementation without stale detached/no-wait claims.
    - `git diff --check` — passed.
  - Context impact: root — changed durable synchronization, hook-routing, stream, CLI, architecture, glossary, and pattern contracts to match the synchronous automatic completion boundary; the mandatory task context synchronization pass is required.
  - Context synchronization: synced

## Open questions

Waiting makes automatic post-commit execution visibly pay the child and network
latency. The user has selected that tradeoff for deterministic completion and
stream closure; the implementation should not add a timeout unless a later
change request explicitly establishes one.

## Validation Report

**Status:** validated  
**Date:** 2026-08-31

### Commands run

- `nix flake check` -> exit 0 (flake evaluation passed and reported all checks passed)
- `nix run .#pkl-check-generated` -> exit 0 (ephemeral Pkl generation passed: 141 files)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml auto_sync` -> exit 0 (16 focused auto-sync tests passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml hooks::` -> exit 0 (163 focused hook tests passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml error::` -> exit 0 (6 focused error tests passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml sync::command` -> exit 0 (6 focused sync classification tests passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml app_support` -> exit 0 (5 focused app-rendering tests passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml parse::command_runtime` -> exit 0 (5 focused command-runtime tests passed)
- `git diff --check` -> failed (the previous failed Validation Report contained trailing whitespace; the report was replaced and the check was rerun successfully)
- `git diff --check` -> exit 0 (no whitespace errors)

### Success-criteria verification

- [x] AC1: A sync failure raised by an automatic invocation maps to a payload-bearing typed `UserError`/`CliError` path and renders one runtime diagnostic that clearly says automatic synchronization failed and includes the underlying typed failure reason. -> Focused error, sync-classification, app-rendering, and command-runtime tests passed; the typed catalog, reason-preserving classifier, and single renderer were inspected.
- [x] AC2: An automatic authentication failure uses a distinct typed automatic-sync failure kind, tells the user that authentication is required, instructs them to run `sce auth login`, and then explicitly tells them to manually retry with `sce sync`. -> Focused classification and app-rendering tests passed; the authentication template and preserved observability source were inspected.
- [x] AC3: Non-authentication automatic failures use the same typed payload-bearing error model, provide actionable recovery guidance, and explain that the user can manually retry with `sce sync`, without relying on substring matching or duplicating the default runtime `Try:` guidance. -> Focused sync, error, and app-rendering tests passed; typed matching, deterministic recovery text, and the absence of duplicate generic remediation were inspected.
- [x] AC4: Automatic child failures are visible through the existing stderr diagnostic channel while successful post-commit execution remains JSON-stdout-silent and fail-open to the commit; launcher startup failures retain their reason in structured auto-sync diagnostics without failing the hook. -> Auto-sync and hook tests passed; command arguments, marker, null stdout, inherited stderr, typed startup diagnostics, and fail-open behavior were inspected.
- [x] AC5: Manual `sce sync` failures retain the existing manual-sync semantics and do not claim that automatic synchronization failed. -> Manual and automatic classifier branches were inspected; sync classification and command-runtime tests passed.
- [x] AC6: Durable context documents the typed automatic-sync failure model, stderr visibility, authentication recovery, manual retry command, and preserved no-daemon/fail-open boundaries. -> The listed context contracts were reviewed against the final error, sync, launcher, hook, and app code; generated-context and repository validation passed.
- [x] AC7: The automatic post-commit launcher waits for the `sync --format json` child to reach terminal completion before returning, while preserving the repository-root cwd, internal marker, null stdout, inherited stderr, and fail-open hook boundary. -> Auto-sync and hook tests passed; `launch_with` waits for terminal completion and preserves the command boundary while post-commit remains fail-open.
- [x] AC8: Automatic child non-zero exits and wait errors remain fail-open; child-rendered failures are not duplicated by the launcher, while launcher startup/wait failures retain actionable typed reasons on stderr. -> Auto-sync tests passed for success, non-zero exit, startup, and wait outcomes; the typed launcher diagnostic path and non-duplication behavior were inspected.
- [x] AC9: Durable context describes the synchronous automatic completion boundary, its commit-latency tradeoff, stderr behavior, manual retry path, and preserved no-daemon/no-retry constraints without stale detached-policy claims. -> The listed root, CLI, sync, hook-routing, and stdout/stderr contracts were reviewed; they describe waited completion, latency, inherited stderr, manual retry, and no-daemon/no-retry boundaries without stale detached/no-wait claims.

### Failed checks and follow-ups

- None.

### Residual risks

- None identified.
