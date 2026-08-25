# Plan: setup-git-remote-preflight

## Change summary

Extend `sce setup`'s existing repository preflight so every setup mode stops
before prompts or writes unless the target is both an initialized Git
repository and has a configured Git remote URL. The remote check will use the
same resolved `agent_trace.repository_remote` name that Agent Trace identity
resolution uses, defaulting to `origin`, rather than hard-coding `origin` or
accepting an unrelated remote.

Add `UserError::NotGitRepository` and `UserError::NotGitRemote` to the typed
CLI error catalog. Preserve technical sources for observability while giving
operators stable, actionable messages explaining `git init` or
`git remote add <name> <url>` remediation. Reuse the existing Git root
resolution and `lookup_remote_url` implementations; this change does not test
remote network reachability or redesign repository identity resolution.

Correction to the completed preflight implementation: narrow typed error
classification so only Git's explicit `not a git repository` failure becomes
`NotGitRepository`, and only an actually missing configured remote URL becomes
`NotGitRemote`. Git process-launch, permission, bare-repository, malformed
repository, and remote-lookup execution failures must remain runtime errors
with their technical sources intact.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the
check that proves it. `/validate` runs these checks; no task in the stack
performs final validation.

- [x] AC1: `sce setup` reports a typed, actionable `NotGitRepository` failure
  when the target path is not inside an initialized Git repository, and does
  not prompt or perform setup writes.
  - Validate: Setup preflight tests exercise a non-repository directory and
    assert the `UserError::NotGitRepository` variant and `git init` guidance.
- [x] AC2: `sce setup` reports a typed, actionable `NotGitRemote` failure when
  the resolved SCE remote has no configured URL, and does not prompt or perform
  setup writes.
  - Validate: Setup preflight tests exercise an initialized repository without
    the selected remote URL and assert the `UserError::NotGitRemote` variant
    and `git remote add <name> <url>` guidance.
- [x] AC3: Remote validation uses the resolved `agent_trace.repository_remote`
  name, including the default `origin`, and setup proceeds past the preflight
  when that named remote has a URL regardless of whether another remote is
  present.
  - Validate: Tests cover both the default `origin` and a configured alternate
    remote such as `upstream`, including rejection when only an unrelated
    remote exists.
- [x] AC4: Existing successful setup behavior and user-error rendering remain
  compatible, while technical error sources remain available to observability
  and remote URLs are not echoed in diagnostics.
  - Validate: Existing CLI error tests plus the new setup error tests pass, and
    the full repository check suite succeeds.
- [x] AC5: Setup classifies only the explicit missing-repository and missing-
  remote conditions as typed user errors; Git/process/configuration failures
  remain runtime errors and retain their technical sources.
  - Validate: Focused setup and repository-identity tests cover a missing Git
    repository, missing remote URL, Git launch/non-repository edge failures,
    and remote lookup execution failures with exact `CliError` classification.

### Full validation

Repository-wide checks `/validate` runs after the last task, regardless of
which criterion they map to.

- `nix flake check`

### Context sync

- `context/overview.md` — update the current setup and typed user-error
  summaries.
- `context/cli/cli-command-surface.md` — document the Git-plus-remote preflight
  and its ownership in `services/setup/command.rs`.
- `context/sce/setup-githooks-cli-ux.md` — document the remote prerequisite for
  all setup modes and actionable failure behavior.
- `context/sce/setup-repo-local-config-bootstrap.md` — record that both
  repository preflights precede context/config/database/bootstrap side effects.
- `context/sce/cli-error-code-taxonomy.md` — add the two setup-specific
  `UserError` catalog entries and preserve-source rendering contract.
- `context/cli/repository-identity.md` — document the strict remote-lookup
  distinction used by setup while preserving repository-identity behavior.

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** `cli/src/services/error.rs`, `cli/src/services/setup/mod.rs`,
  `cli/src/services/setup/command.rs`, focused Rust tests, and the listed
  durable setup/error context files.
- **Out of scope:** remote network connectivity checks, remote URL
  canonicalization changes, changes to Agent Trace identity precedence,
  changes to `doctor`, and changes to setup flags or successful output.
- **Constraints:** Reuse `setup::ensure_git_repository`, the existing config
  resolver for `agent_trace.repository_remote`, and the repository-identity
  remote lookup seam; preserve stdout/stderr and stable `SCE-ERR-RUNTIME`
  behavior for non-user failures; do not expose raw credential-bearing remote
  URLs.
- **Non-goal:** Accepting any arbitrary Git remote when the configured SCE
  remote is missing; setup must validate the remote Agent Trace will use.

## Assumptions

- The remote preflight applies to `sce setup --bootstrap-context` as well as
  normal, hooks-only, combined, and interactive setup because the requirement
  is that `sce setup` validates both prerequisites before any setup path.
- An explicit `agent_trace.repository_id` does not waive the remote preflight;
  the requested setup contract requires a Git remote independently of identity
  fallback behavior.
- `UserError` messages remain fixed catalog sentences; the configured remote
  name is retained in the technical source for diagnostics/tests rather than
  being added as a payload field to the enum.

## Task stack

- [x] T01: `Add typed setup preflight errors and remote validation primitive` (status:done)
  - Task ID: T01
  - Scope: In — add `NotGitRepository` and `NotGitRemote` to the `UserError` catalog with keys, runtime classification, and actionable fixed messages; add a setup-owned remote preflight helper that delegates to `lookup_remote_url`; add focused tests for catalog behavior, missing remotes, alternate remote names, and credential-safe source text. Out — command wiring and durable context edits.
  - Dependencies: none
  - Done when: The setup service exposes a reusable remote-URL preflight, both typed errors render their reviewed remediation messages, missing/configured remote cases are covered, and no raw remote URL is included in the generated error source.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml error setup repository_identity`
  - Completed: 2026-08-24
  - Files changed: `cli/src/services/error.rs`, `cli/src/services/setup/mod.rs`
  - Result: Added runtime-classified `setup.not_git_repository` and `setup.not_git_remote` catalog entries with fixed `git init` and `git remote add <name> <url>` remediation messages. Added `setup::ensure_git_remote`, which delegates named-remote URL lookup to `repository_identity::resolve::lookup_remote_url` and retains only the configured remote name in its technical error. Added catalog, source-preservation, missing-origin, configured-origin, alternate-remote, and unrelated-remote tests.
  - Verify: The combined filter form was rejected because Cargo accepts one test filter at a time; equivalent separate runs passed: `error` (35 passed), `setup` (63 passed), and `repository_identity` (24 passed).
   - Context impact: Material CLI behavior change in the typed error catalog and setup service; durable context documentation is deferred to T02's context synchronization scope.
   - Context synchronization: synced

- [x] T02: `Enforce Git and configured-remote preflights at setup dispatch` (status:done)
  - Task ID: T02
  - Scope: In — resolve the effective `agent_trace.repository_remote` in `setup/command.rs`, run the Git-root and named-remote preflights before prompts/context bootstrap/lifecycle setup, map failures through `CliError::user_with_source`, add command-level regression coverage for no-Git/no-remote/default/alternate-remote paths, and update the listed durable context files including the existing command-ownership drift. Out — network reachability, remote canonicalization, doctor behavior, and unrelated setup output changes.
  - Dependencies: T01
   - Done when: Every setup mode fails early and actionably for either missing prerequisite, valid default and configured alternate remotes pass the gate, existing setup success/cancellation behavior remains intact, and durable context describes the implemented gate and error catalog accurately.
   - Verify: `nix flake check`
   - Completed: 2026-08-24
   - Files changed: `cli/src/services/error.rs`, `cli/src/services/setup/command.rs`, `context/overview.md`, `context/cli/cli-command-surface.md`, `context/sce/setup-githooks-cli-ux.md`, `context/sce/setup-repo-local-config-bootstrap.md`, `context/sce/cli-error-code-taxonomy.md`, `context/plans/setup-git-remote-preflight.md`
   - Result: Wired setup dispatch through a pre-prompt Git-root and configured-remote gate. The effective `agent_trace.repository_remote` is resolved with the existing config resolver and defaults to `origin`; missing prerequisites map through `CliError::user_with_source` to typed `NotGitRepository` or `NotGitRemote` diagnostics while preserving technical sources without rendering remote URLs. Added command-level coverage for missing Git, missing origin, configured origin, configured alternate remotes, and unrelated remotes, and documented the ordering and ownership across durable setup/error context.
   - Verify: Focused setup preflight tests passed (6 tests). `nix flake check` passed all repository checks; an earlier full-check failure was an unrelated flaky Agent Trace row-count test and passed on rerun.
    - Context impact: Root — setup is now governed by a repository-wide Git-plus-configured-remote preflight and the typed CLI error catalog; updated root setup/error summaries and the authoritative setup/bootstrap/error domain contracts.
     - Context synchronization: synced

- [x] T03: `Narrow setup preflight error classification` (status:done)
  - Task ID: T03
  - Scope: In — distinguish the exact `not a git repository` Git failure from other `rev-parse` execution failures; distinguish a missing/empty configured remote URL from failures running the remote lookup; preserve technical `CliError::Internal` runtime classification for the latter cases; add focused regression coverage and update the setup/error/repository-identity context contracts. Out — remote network reachability, repository identity precedence, doctor behavior, and changes to successful setup output.
  - Dependencies: T02
  - Done when: A missing Git repository still renders `NotGitRepository`, a missing configured remote URL still renders `NotGitRemote`, and Git launch/permission/bare/malformed-repository plus remote lookup execution failures render as runtime errors with their original sources; credential-bearing remote URLs remain absent from user-facing diagnostics.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml repository_identity`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml error`
  - Completed: 2026-08-25
  - Files changed: `cli/src/services/error.rs`, `cli/src/services/repository_identity/resolve.rs`, `cli/src/services/setup/command.rs`, `cli/src/services/setup/mod.rs`, `context/cli/repository-identity.md`, `context/overview.md`, `context/plans/setup-git-remote-preflight.md`, `context/sce/cli-error-code-taxonomy.md`, `context/sce/setup-githooks-cli-ux.md`, `context/sce/setup-repo-local-config-bootstrap.md`
  - Result: Added strict remote URL lookup that preserves execution/configuration failures while retaining compatibility fail-to-missing behavior for repository identity resolution. Setup now emits typed errors only for Git's explicit missing-repository failure and missing configured remote URL, mapping all other preflight failures to runtime `CliError::Internal` errors with technical sources preserved. Added command, setup, repository-identity, and error regression coverage plus updated durable setup/error/identity contracts.
  - Verify: `setup` passed with 69 tests; `repository_identity` passed with 25 tests; `error` passed with 46 tests.
  - Context impact: Root — setup preflight error classification and the repository-identity remote lookup boundary now distinguish expected missing prerequisites from runtime execution failures; durable root setup/error/identity contracts were updated.
  - Context synchronization: synced

## Open questions

None. The remote selection rule, mandatory scope, error classification, and
non-network validation boundary were resolved during discussion.

## Validation Report

**Status:** validated  
**Date:** 2026-08-25

### Commands run

- `nix flake check` -> exit 0 (all repository checks passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup` -> exit 0 (69 tests passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml repository_identity` -> exit 0 (25 tests passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml error` -> exit 0 (46 tests passed)

### Success-criteria verification

- [x] AC1: `sce setup` reports a typed, actionable `NotGitRepository` failure and performs no setup writes -> setup preflight tests passed for a non-repository directory and preserved the `git init` guidance.
- [x] AC2: `sce setup` reports a typed, actionable `NotGitRemote` failure and performs no setup writes -> setup and error tests passed for missing configured remotes and preserved `git remote add <name> <url>` guidance.
- [x] AC3: Remote validation uses the resolved `agent_trace.repository_remote` name -> setup and repository-identity tests passed for default `origin`, configured alternate remotes, and rejection of unrelated remotes.
- [x] AC4: Existing setup behavior and user-error rendering remain compatible without exposing remote URLs -> full repository checks and focused setup/error tests passed, including source preservation and credential-safe diagnostics.
- [x] AC5: Setup classifies only explicit missing-repository and missing-remote conditions as typed user errors -> setup, repository-identity, and error tests passed for missing prerequisites, Git/runtime edge failures, strict remote lookup failures, preserved sources, and safe diagnostics.

### Failed checks and follow-ups

- None.

### Residual risks

- None identified.
