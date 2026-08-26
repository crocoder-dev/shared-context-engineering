# Plan: setup-invalid-config-and-git-locale

## Change summary

Make repository setup fail closed when the repo-local `.sce/config.json` is
invalid, before lifecycle setup can initialize databases, install hooks, or
install target assets. The Agent Trace storage runtime resolver must not silently
discard an invalid discovered config layer and substitute a default remote,
because that can select the wrong repository database.

Also make Git subprocess behavior locale-stable by setting `LC_ALL=C` on the
Git commands used by setup and repository-identity resolution, preserving the
existing output/error handling while preventing localized Git diagnostics from
breaking parsing and classification.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the
check that proves it. `/validate` runs these checks; no task in the stack
performs final validation.

- [x] AC1: An existing invalid repo-local `.sce/config.json` makes `sce setup`
  fail before lifecycle database initialization, hook installation, or target
  asset installation, and the diagnostic identifies the invalid config.
  - Validate: Focused setup/config lifecycle tests assert the error and that no
    setup-owned DB, hook, or target asset is created.
- [x] AC2: `resolve_agent_trace_storage_runtime_config()` returns an error when a
  discovered config file is invalid instead of returning a fallback remote or
  other storage identity values from the remaining layers/defaults.
  - Validate: Resolver tests cover invalid local config with a configured
    remote and assert the resolver fails without producing storage config.
- [x] AC3: Git subprocesses used by setup and repository-identity remote lookup
  run with `LC_ALL=C`, while successful output and existing diagnostics remain
  unchanged.
  - Validate: Source-level inspection of the centralized Git command paths plus
    the focused setup/repository-identity test suites.

### Full validation

Repository-wide checks `/validate` runs after the last task, regardless of
which criterion they map to.

- `nix run .#pkl-check-generated`
- `nix flake check`

### Context sync

- `context/cli/config-precedence-contract.md`
- `context/sce/setup-repo-local-config-bootstrap.md`
- `context/cli/repository-identity.md`
- `context/cli/agent-trace-storage.md`

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** Runtime config resolution for Agent Trace storage, setup's
  preflight ordering, repository-identity Git remote lookup, setup/repository-
  identity tests, and the four listed durable context files.
- **Out of scope:** Changing the general startup policy that lets `sce` continue
  with degraded defaults for invalid discovered config, changing config schema
  semantics, changing repository identity canonicalization, or changing Git
  remote selection precedence.
- **Constraints:** Preserve credential-safe diagnostics, deterministic error
  text, existing setup ordering after the new preflight, and the repository's
  Nix-based verification workflow. Use the existing config validation seam and
  avoid adding dependencies.
- **Non-goal:** Do not make unrelated commands or observability-only config
  consumers fail hard merely because a default-discovered config is invalid;
  the fail-closed boundary is setup and Agent Trace storage identity resolution.

## Assumptions

- The invalid-config preflight should run immediately after the Git repository
  root is resolved, before prompts, context/lifecycle setup, or integration
  writes, while an absent local config continues through the existing bootstrap
  path.
- Applying `LC_ALL=C` to the shared setup Git runner and the repository-identity
  remote lookup is sufficient; test-only Git setup helpers need no behavioral
  contract change.

## Task stack

- [x] T01: `Fail closed on invalid repository config before setup and storage identity resolution` (status:complete)
  - Task ID: T01
  - Scope: In — add a setup preflight using the existing config validation
    service, make Agent Trace storage runtime resolution reject invalid
    discovered config layers, and add focused tests proving no later setup work
    or fallback remote occurs. Out — general startup degradation behavior and
    config schema changes.
  - Dependencies: none
  - Done when: Invalid `.sce/config.json` stops setup before lifecycle or asset
    side effects, and storage runtime config reports the validation failure
    instead of returning a potentially wrong remote; valid and absent config
    behavior remains unchanged.
  - Verify: Focused Rust tests for config resolver, setup, and Agent Trace
    storage/config lifecycle behavior.
  - Completed: 2026-08-26
  - Files changed:
    - `cli/src/services/config/resolver.rs`
    - `cli/src/services/setup/command.rs`
    - `cli/src/services/setup/mod.rs`
  - Result: Added a repo-local config preflight immediately after Git-root
    resolution, preventing setup prompts, context bootstrap, lifecycle work,
    hooks, and target asset installation when the existing config is invalid.
    Agent Trace storage runtime resolution now rejects invalid discovered config
    layers instead of using fallback storage identity values.
  - Verify:
    - Passed: `services::config::resolver` Rust tests (19 passed), including
      invalid discovered config storage resolution coverage.
    - Passed: `services::setup` Rust tests (62 passed), including setup
      preflight side-effect coverage.
    - Passed: `services::agent_trace_storage` Rust tests (14 passed).
  - Context impact:
    - Classification: material
    - Affected areas: setup lifecycle ordering, runtime config resolution, and
      Agent Trace repository storage identity.
    - Reason: Invalid repo-local configuration now changes the fail-closed
      boundary for setup and Agent Trace storage consumers.
  - Context synchronization: synced

- [x] T02: `Pin setup and repository remote Git commands to the C locale` (status:complete)
  - Task ID: T02
  - Scope: In — set `LC_ALL=C` on the shared setup Git command runner and
    repository-identity remote lookup, and cover the affected command paths in
    focused tests or inspection. Out — changing Git arguments, remote
    precedence, canonicalization rules, or unrelated non-Git subprocesses.
  - Dependencies: T01
  - Done when: Every production Git subprocess in the affected setup and remote
    lookup paths explicitly sets `LC_ALL` to `C`, and existing success/error
    behavior remains stable.
  - Verify: Focused setup/repository-identity tests and source inspection of the
    affected `Command::new("git")` call sites.
  - Completed: 2026-08-26
  - Files changed:
    - `cli/src/services/repository_identity/resolve.rs`
    - `cli/src/services/setup/mod.rs`
  - Result: Pinned the production setup Git runner and repository-identity remote
    lookup to `LC_ALL=C`, preserving their existing arguments, output handling,
    and diagnostics.
  - Verify:
    - Passed: setup Rust tests (65 passed).
    - Passed: repository-identity Rust tests (24 passed).
    - Passed: Source inspection confirmed the affected production Git runners
      explicitly set `LC_ALL` to `C`; test-only Git initialization helpers were
      left unchanged.
  - Context impact:
    - Classification: material
    - Affected areas: setup Git repository/hooks resolution and repository-identity
      remote lookup.
    - Reason: Production Git parsing and diagnostics in these repository-wide
      identity/setup paths are now explicitly locale-stable.
  - Context synchronization: synced

## Open questions

None. The requested fail-closed boundary, fallback-remote risk, and locale
requirement are specific enough to implement using existing seams.

## Validation Report

**Status:** validated  
**Date:** 2026-08-26

### Commands run

- `nix run .#pkl-check-generated` -> exit 0 (ephemeral Pkl generation passed for 141 files)
- `nix flake check` -> exit 0 (all flake checks passed)
- `nix develop -c sh -c './scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::config::resolver && ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::setup && ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::agent_trace_storage && ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::repository_identity'` -> exit 0 (19 resolver, 62 setup, 14 Agent Trace storage, and 23 repository-identity tests passed)
- Manual `sce setup --claude --non-interactive --hooks --repo <invalid-config-repo>` -> exit 4 (invalid repo-local config reported; no context, hook, Claude assets, or local DB were created)
- Manual `sce sync --format json` in a repo with invalid config and `agent_trace.repository_remote=upstream` -> exit 4 (storage resolution rejected the invalid discovered config; no remote fallback or storage directory was produced)
- Manual setup with absent repo-local config and a valid `origin` remote -> exit 0 (bootstrap, repository-scoped storage, hooks, and Claude assets completed)
- Manual setup with valid `agent_trace.repository_remote=upstream` and distinct `origin`/`upstream` remotes -> exit 0 (the configured `upstream` remote was selected)

### Success-criteria verification

- [x] AC1: An existing invalid repo-local `.sce/config.json` makes `sce setup` fail before lifecycle database initialization, hook installation, or target asset installation, and the diagnostic identifies the invalid config. -> Focused setup/config suites passed; manual setup exited 4 before creating context, hooks, Claude assets, or the local DB. Absent-config setup still completed successfully.
- [x] AC2: `resolve_agent_trace_storage_runtime_config()` returns an error when a discovered config file is invalid instead of returning a fallback remote or other storage identity values from the remaining layers/defaults. -> Resolver and Agent Trace storage suites passed; manual sync exited 4 for invalid config with `upstream`, without selecting a fallback remote or creating storage.
- [x] AC3: Git subprocesses used by setup and repository-identity remote lookup run with `LC_ALL=C`, while successful output and existing diagnostics remain unchanged. -> Source inspection confirmed `LC_ALL=C` on the production setup Git runner and repository-identity remote lookup; test-only Git helpers are excluded. Focused setup and repository-identity suites passed, and valid remote selection remained unchanged.

### Failed checks and follow-ups

- None.

### Residual risks

- None identified.
