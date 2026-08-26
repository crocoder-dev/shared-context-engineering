# Plan: fix-pr-223-error-classification-regressions

## Change summary

Fix the three semantic regressions introduced by PR #223 at `b57c6a8f7400afb947fc0417abf381ac8a5868db`, without redesigning the typed `CliError`/closed `UserError` architecture. The work preserves technical error sources for observability, keeps classification in typed domain boundaries, and restores existing command output/exit semantics where an unauthenticated state is a successful query.

The fixes are deliberately split into three independently testable atomic commits: propagate credential-storage classification through sync streams; type setup repository-root resolution before the CLI boundary; and restore idempotent `auth logout` plus unauthenticated `auth whoami` success paths while retaining typed mappings for genuine failures.

## Acceptance criteria

- [ ] AC1: Initial control-plane, stream-terminal, and stream-refresh `ControlPlaneError::Storage` failures all classify as `auth.storage_unavailable`; stream authentication remains `auth.not_authenticated`; other control-plane/runtime failures remain `general.unexpected_failure`, with technical `TraceSyncError` sources attached.
  - Validate: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml sync::command` and `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml sync::`; inspect the classifier to confirm it remains unchanged and uses typed predicates rather than human-readable strings.
- [ ] AC2: Setup emits `setup.not_git_repository` only when the setup domain positively identifies a target as outside a Git repository; nonexistent, inaccessible, process, malformed-output, and unrelated filesystem failures classify as `general.unexpected_failure`, and both typed paths preserve technical sources.
  - Validate: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup::`; inspect the setup classifier for typed `GitRepositoryResolutionError` matching with no CLI-layer string matching.
- [ ] AC3: `sce auth logout` with no stored credentials succeeds with the existing text and JSON state-query semantics, including `credentials_removed: false`; deleting stored credentials still succeeds with `credentials_removed: true`.
  - Validate: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml auth_command::` plus focused text/JSON assertions for absent and present credentials.
- [ ] AC4: `sce auth whoami` with no stored credentials succeeds with the existing unauthenticated text guidance and JSON payload (`authentication_state: unauthenticated`, `has_stored_credentials: false`), while authenticated `/me` failures retain typed `NotAuthenticated`, `AuthStorageUnavailable`, or `UnexpectedFailure` mappings and technical sources.
  - Validate: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml auth_command::` plus focused missing-credential and authenticated-failure assertions.
- [ ] AC5: Genuine auth storage failures retain `auth.storage_unavailable`, stored credentials rejected by the Control Plane retain `auth.not_authenticated`, and all genuine failures retain exit code `4`, stdout/stderr routing, and machine-readable JSON contracts.
  - Validate: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml auth_command::` and `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml app_support::`.
- [ ] AC6: The closed `UserError` catalog and typed-error architecture remain intact: no arbitrary message variant, no CLI-boundary human-readable string classification, no rollback to the pre-PR architecture, and no new ADR for this regression repair.
  - Validate: inspect `cli/src/services/error.rs`, `cli/src/services/sync/command.rs`, and `cli/src/services/setup/command.rs`; confirm no `UserError::Message`/`Custom` variant and no CLI-layer error-string matching.
- [ ] AC7: Durable context accurately documents sync storage propagation, positive-only setup repository classification, and successful unauthenticated auth state queries, with no stale claim that missing logout/whoami credentials are `NotAuthenticated` failures.
  - Validate: `nix run .#pkl-check-generated` and targeted inspection of the context files listed under Context sync.

### Full validation

- `./scripts/run-cli-cargo.sh fmt --manifest-path cli/Cargo.toml -- --check`
- `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml -- -D warnings`
- `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml`
- `nix run .#pkl-check-generated`
- `nix flake check`

### Context sync

- `context/cli/sync-command.md` — storage failures classify consistently across initial state resolution and stream execution.
- `context/cli/cli-command-surface.md` — setup repository classification and successful logged-out auth state-query behavior.
- `context/sce/cli-error-code-taxonomy.md` — positive-only `NotGitRepository` semantics and the distinction between unauthenticated state observation and authentication failure.
- `context/architecture.md` — corrected auth/setup boundary behavior where its current summary is stale.

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** `cli/src/services/sync/sync.rs`; `cli/src/services/sync/command.rs` tests; `cli/src/services/agent_trace_sync/mod.rs` comments/predicates as needed; `cli/src/services/setup/mod.rs`; `cli/src/services/setup/command.rs`; `cli/src/services/auth_command/mod.rs` and its focused tests; the listed durable context files.
- **Out of scope:** redesigning `CliError` or `UserError`; adding arbitrary user-message variants; broad typing of unrelated setup errors; changing `classify_sync_error`; changing genuine failure exit codes; changing machine-readable JSON contracts except to restore the documented successful logout/whoami payloads; creating an ADR; unrelated PR #223 cleanup.
- **Constraints:** preserve the closed `UserError` catalog; preserve technical sources; classify by typed variants/predicates at domain boundaries, never by matching human-readable strings at the CLI boundary; use no new dependency; keep each task independently testable and suitable for one atomic commit; retain the existing `4` runtime exit code for genuine failures.
- **Non-goal:** generalize repository-resolution typing to every setup operation or alter the typed-error architecture beyond these three regressions.

## Assumptions

- The suggested `GitRepositoryResolutionError` name and exact internal helper names are flexible; the repository's existing Rust naming and error conventions decide those local details.
- Auth tests may add a narrow pure/injected orchestration seam analogous to the existing auth dispatch test seam so missing-credential branches can be tested deterministically without relying on process-global encrypted storage; production storage behavior remains unchanged.
- The requested text and JSON outputs are the existing `main` semantics described in the request; successful logout with credentials retains its current success output while absent credentials render the existing no-user state.

## Task stack

- [x] T01: `Propagate credential-storage classification through stream sync errors` (status:done)
  - Task ID: T01
  - Scope: In — change `TraceSyncError::is_storage_failure()` to traverse `StreamSyncError`, update its comment, and replace the regression test with terminal and refresh stream-storage cases that classify as `auth.storage_unavailable`; retain authentication, runtime, and other control-plane cases plus source-preservation assertions; update `context/cli/sync-command.md` to document storage classification across initial state, batch execution, and refresh. Out — changing `classify_sync_error()` or sync error architecture.
  - Dependencies: none
  - Done when: initial, terminal-stream, and refresh-stream `ControlPlaneError::Storage` all reach `UserError::AuthStorageUnavailable`, authentication classification is unchanged, technical sources remain attached, and the sync context rule is truthful.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml sync::command` — pass (9 tests); `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml sync::` — pass (66 tests).
  - Context synchronization: synced
  - Completed: 2026-08-26
  - Files changed: `cli/src/services/sync/sync.rs`, `cli/src/services/sync/command.rs`, `context/cli/sync-command.md`
  - Result: Stream credential-storage failures now propagate through the typed sync error predicate and classify as `auth.storage_unavailable`; terminal and refresh cases are covered by focused tests with preserved technical sources.
  - Context impact: domain — `context/cli/sync-command.md` now accurately documents typed credential-storage classification across all sync failure paths; no root context files require changes.

- [x] T02: `Type setup repository-root resolution before the CLI boundary` (status:done)
  - Task ID: T02
  - Scope: In — introduce a narrow setup-owned `GitRepositoryResolutionError` distinguishing positively identified non-Git directories from unexpected resolution failures; preserve the original technical source through `Display`/`Error`; return it from `ensure_git_repository`; classify it in `setup/command.rs` as `NotGitRepository` or `UnexpectedFailure`; add real non-Git-directory, nonexistent-path, and source-preservation tests; update setup taxonomy/context wording. Out — typing every later setup operation, changing setup success behavior, or matching strings in the command layer.
  - Dependencies: none
  - Done when: a valid temporary non-Git directory maps to `setup.not_git_repository`, a definitely nonexistent path maps to `general.unexpected_failure`, both `CliError::User` variants contain technical sources, and only the setup domain recognizes Git's diagnostic.
  - Verify: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup::` — passed (64 tests); `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml -- -D warnings` — passed.
  - Context synchronization: synced
  - Completed: 2026-08-26
  - Files changed: `cli/src/services/setup/mod.rs`, `cli/src/services/setup/command.rs`, `context/cli/cli-command-surface.md`, `context/sce/cli-error-code-taxonomy.md`, `context/architecture.md`
  - Result: Setup repository-root resolution now returns a typed classification, mapping only Git-confirmed non-repository directories to `NotGitRepository` and preserving technical sources while mapping other resolution failures to `UnexpectedFailure`; focused tests cover real non-Git and nonexistent paths plus both sourced CLI mappings.
  - Context impact: domain — `context/cli/cli-command-surface.md`, `context/sce/cli-error-code-taxonomy.md`, and `context/architecture.md` now document positive-only setup repository classification and technical-source preservation; the five root context files require verification during synchronization.

- [x] T03: `Restore idempotent auth state-query semantics` (status:done)
  - Task ID: T03
  - Scope: In — restore `render_logout_result(deleted, format)` and make absent-token logout a successful result; add `render_unauthenticated_whoami(format)` and make missing credentials a successful unauthenticated-state result; retain typed storage and authenticated Control Plane mappings, technical sources, existing successful JSON fields, and genuine failure behavior; add focused text/JSON tests for missing and removed credentials plus authenticated failure tests; update auth command surface, taxonomy, and architecture context wording. Out — changing login renewal/device flow, adding a new user-error catalog entry, or creating an ADR.
  - Dependencies: none
  - Done when: missing-token logout and whoami return `Ok(...)` with their existing text/JSON contracts, token deletion still reports success, authenticated `/me` and storage failures retain their typed errors and sources, and context no longer claims that observing logged-out state is `NotAuthenticated`.
  - Verify: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml auth_command::` — passed (5 tests); `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml app_support::` — passed (5 tests).
  - Completed: 2026-08-26
  - Files changed: `cli/src/services/auth_command/mod.rs`, `context/architecture.md`, `context/cli/cli-command-surface.md`, `context/sce/cli-error-code-taxonomy.md`
  - Result: Logout now succeeds idempotently and reports whether credentials were removed; unauthenticated whoami now returns its documented text/JSON state report, while authenticated and storage failures retain typed mappings and technical sources. Focused regression tests cover both output formats and authenticated failure classification.
  - Context impact: domain — auth command state-query behavior, CLI error taxonomy, and architecture documentation; these context files now distinguish successful unauthenticated observation from genuine authentication failures.
  - Context synchronization: synced

## Open questions

None. The request specifies the three regressions, the required typed boundaries, preserved contracts, tests, context updates, atomic commit messages, and final validation commands. The code inspection confirms the regressions are present at the stated PR head; no smaller change covers all three independent user-visible failures.
