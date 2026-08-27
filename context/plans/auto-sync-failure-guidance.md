# Plan: auto-sync-failure-guidance

## Change summary

Improve the existing post-commit automatic Agent Trace synchronization path so
that a failed detached sync produces a typed, user-facing diagnostic instead of
an opaque or invisible failure. Following the payload-bearing `UserError`
pattern used by the setup Git preflight (`NotGitRepository`/
`NotGitRemote`), the automatic-sync error will carry its typed failure kind and
underlying reason while rendering reviewed recovery guidance; authentication
failures will explicitly direct the user to log in and then manually run
`sce sync`.

The existing one-shot architecture remains intact: automatic sync still reuses
the `sce sync` command, does not delay the commit, and fails open. The detached
child will identify itself as an automatic invocation and expose only its
failure diagnostics through the existing stderr contract; it will not introduce
local retry state, a daemon, or a second synchronization implementation.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the
check that proves it. `/validate` runs these checks; no task in the stack
performs final validation.

- [ ] AC1: A sync failure raised by an automatic invocation maps to a payload-bearing typed `UserError`/`CliError` path and renders one runtime diagnostic that clearly says automatic synchronization failed and includes the underlying typed failure reason.
  - Validate: Focused sync/error tests assert the rendered diagnostic for control-plane, stream, and local runtime failures, including the reason and `SCE-ERR-RUNTIME` classification.
- [ ] AC2: An automatic authentication failure uses a distinct typed automatic-sync failure kind, tells the user that authentication is required, instructs them to run `sce auth login`, and then explicitly tells them to manually retry with `sce sync`.
  - Validate: Focused authentication classification and app-rendering tests assert the complete login-plus-manual-sync guidance and ensure the technical source remains available only for observability.
- [ ] AC3: Non-authentication automatic failures use the same typed payload-bearing error model, provide actionable recovery guidance, and explain that the user can manually retry with `sce sync`, without relying on substring matching or duplicating the default runtime `Try:` guidance.
  - Validate: Focused tests cover representative storage, transport/server, protocol, and stream failures and assert deterministic reason/recovery text with no duplicate remediation.
- [ ] AC4: Automatic child failures are visible through the existing stderr diagnostic channel while successful post-commit execution remains detached, non-blocking, JSON-stdout-silent, and fail-open to the commit; launcher startup failures retain their reason in structured auto-sync diagnostics without failing the hook.
  - Validate: Launcher and post-commit seam tests assert the internal automatic-invocation marker, inherited failure stderr, unchanged `sync --format json` arguments, no wait, and fail-open behavior for executable/spawn failures.
- [ ] AC5: Manual `sce sync` failures retain the existing manual-sync semantics and do not claim that automatic synchronization failed.
  - Validate: Command-level tests execute/classify manual and automatic invocation modes separately and assert mode-specific rendering.
- [ ] AC6: Durable context documents the typed automatic-sync failure model, stderr visibility, authentication recovery, manual retry command, and preserved no-daemon/fail-open boundaries.
  - Validate: Review the listed context contracts against the final code, then run the generated-context and repository checks under `Full validation`.

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
- **Constraints:** preserve the exact child arguments `sync --format json`, current-executable resolution, repository-root working directory, no wait, commit fail-open semantics, stdout/stderr separation, typed authentication classification, and shared sensitive-text redaction; add no dependency or persistent retry state.
- **Non-goal:** making Git wait for network synchronization or adding a daemon, watcher, scheduler, queue, status file, or local retry cursor.

## Assumptions

- Automatic sync remains a detached child and reports completion failures through its inherited stderr rather than waiting for the child or persisting a new failure record; this preserves the existing one-shot/fail-open contract while making the diagnostic observable.
- The automatic invocation marker is an internal process-boundary detail, not a new user configuration key or public CLI option; manual `sce sync` remains mode-neutral and keeps its existing error wording.
- The typed failure model will preserve the technical source for structured logging while rendering a reviewed, deterministic recovery sentence at the app boundary, following `CliError` and `UserError` ownership patterns.

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

- [ ] T03: `Document typed automatic-sync failure recovery contract` (status:todo)
  - Task ID: T03
  - Scope: Update the auto-sync, sync, CLI error, stdout/stderr, hook-routing, and required root context contracts listed under Context sync to describe the final typed error and recovery behavior. Out — generated configuration artifacts, historical plans/decisions, and code/test changes.
  - Dependencies: T02
  - Done when: durable context states the payload-bearing typed error model, automatic-failure prefix, reason preservation, authentication login flow, manual `sce sync` retry, stderr visibility, mode distinction, and unchanged detached/fail-open/no-daemon boundaries without stale null-output claims.
  - Verify: Manual code/context review against `cli/src/services/error.rs`, `cli/src/services/app_support.rs`, `cli/src/services/sync/command.rs`, `cli/src/services/sync/auto_sync.rs`, and `cli/src/services/hooks/mod.rs`.
  - Context synchronization: pending

## Open questions

None. The existing detached/fail-open contract determines that reporting must
travel through the child diagnostic stream rather than a completion wait or new
persistent retry mechanism, and the setup preflight change establishes the
payload-bearing `UserError` pattern to reuse; the remaining wording and
internal marker choices are local implementation details covered by the existing
CLI error/rendering patterns.
