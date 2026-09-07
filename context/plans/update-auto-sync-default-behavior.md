# Plan: update-auto-sync-default-behavior

## Change summary

Separate the two `agent_trace.auto_sync` defaults that currently share one
configuration concept. When `sce setup` creates a missing repo-local
`.sce/config.json`, the generated file will explicitly contain
`"auto_sync": true`, opting the new repository into post-commit synchronization.
The runtime config resolver will remain conservative: an omitted value resolves
to `false`, while explicit config values continue to control behavior.

The existing schema and post-commit trigger remain unchanged apart from these
default boundaries. Focused setup and resolver tests will make the distinction
regression-safe, and durable context will be corrected where it currently treats
the setup payload and resolver fallback as the same default.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the
check that proves it. `/validate` runs these checks; no task in the stack
performs final validation.

- [x] AC1: A newly generated repo-local `.sce/config.json` explicitly contains `"agent_trace": { "auto_sync": true }` alongside its schema declaration.
  - Validate: setup bootstrap tests assert the generated payload/file contains the explicit `agent_trace.auto_sync` value.
- [x] AC2: When `agent_trace.auto_sync` is absent from all config layers, the resolver returns `false` with default provenance; explicit `true` and `false` values and existing global/local precedence remain unchanged.
  - Validate: focused config resolver tests cover the missing-value fallback, explicit values, and local-over-global resolution.
- [x] AC3: Durable SCE configuration and setup documentation distinguishes setup's explicit `true` bootstrap value from the resolver's `false` fallback without changing the documented post-commit opt-out semantics.
  - Validate: manual review of the affected durable context files against the implemented setup payload and resolver branch.

### Full validation

Repository-wide checks `/validate` runs after the last task, regardless of
which criterion they map to.

- `nix flake check`
- `nix run .#pkl-check-generated`

### Context sync

- `context/overview.md`
- `context/architecture.md`
- `context/patterns.md`
- `context/glossary.md`
- `context/context-map.md`
- `context/cli/config-precedence-contract.md`
- `context/cli/agent-trace-auto-sync.md`
- `context/sce/setup-repo-local-config-bootstrap.md`
- `context/sce/agent-trace-hooks-command-routing.md`

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** repo-local setup config bootstrap payload and tests; config resolver omitted-value fallback and tests; the durable context files listed under Context sync.
- **Out of scope:** changes to the JSON schema's accepted shape, explicit config precedence, post-commit launcher behavior, synchronization protocol, generated target trees, and unrelated setup persistence.
- **Constraints:** preserve existing files when `.sce/config.json` already exists; preserve explicit `agent_trace.auto_sync` values and global-before-local merge behavior; use repository test and validation commands through Nix; do not edit generated artifacts.
- **Non-goal:** making every resolver default or every existing repository opt into auto-sync; only a newly created setup config receives the explicit `true` value.

## Assumptions

- The existing optional `agent_trace.auto_sync` schema field and config inspection surfaces already support the required boolean; this change only separates setup serialization from missing-value resolution.
- The current completed `automatic-agent-trace-sync` plan remains historical context and is not amended; this request is tracked as a new plan as requested.

## Task stack

- [x] T01: `Separate setup bootstrap and resolver auto_sync defaults` (status:complete)
  - Task ID: T01
  - Scope: In — `cli/src/services/setup/mod.rs` bootstrap serialization/tests and `cli/src/services/config/resolver.rs` missing-value fallback/tests. Out — schema changes, hook/launcher behavior, and durable context edits.
  - Dependencies: none
  - Done when: newly created setup config payloads explicitly serialize `agent_trace.auto_sync` as `true`; missing resolver values remain `false` with default provenance; explicit values and global/local precedence still pass their focused tests.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml config::`.
  - Completed: 2026-09-07
  - Files changed:
    - `cli/src/services/setup/mod.rs`
    - `cli/src/services/config/resolver.rs`
  - Result: Setup bootstrap payloads now explicitly write `agent_trace.auto_sync: true`; omitted runtime values resolve to `false` with default provenance; explicit values and local-over-global precedence remain covered by focused tests.
  - Verify:
    - `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup` — passed (69 tests).
    - `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml config::` — passed (59 tests).
  - Context impact: `application_behavior` — setup bootstrap serialization and runtime config resolution changed; durable context remains pending for T02.
  - Context synchronization: synced

- [x] T02: `Document the intentionally split auto_sync defaults` (status:complete)
  - Task ID: T02
  - Scope: In — the durable context files listed under Context sync, updating setup-generation and resolver-fallback statements to match T01. Out — application code, tests, generated outputs, and historical plan/decision records.
  - Dependencies: T01
  - Done when: current context consistently says setup writes explicit `true` for a newly generated config, resolver fallback is `false` when missing, and explicit opt-out/trigger behavior is unchanged.
  - Verify: manual review of the affected context files against `cli/src/services/setup/mod.rs` and `cli/src/services/config/resolver.rs`.
  - Completed: 2026-09-07
  - Files changed:
    - `context/architecture.md`
    - `context/context-map.md`
  - Result: Durable context now distinguishes the explicit `true` setup bootstrap opt-in from the resolver's `false` omitted-value fallback while preserving explicit configuration, precedence, trigger, and fail-open behavior.
  - Verify:
    - Manual review of the affected context files against `cli/src/services/setup/mod.rs` and `cli/src/services/config/resolver.rs` — passed; all listed setup/config/auto-sync statements align with the implementation, and stale current-state default wording was corrected.
  - Context impact: documentation — current setup, config, and Agent Trace auto-sync context is synchronized with T01; the mandatory context synchronization pass remains required.
  - Context synchronization: synced

## Open questions

None. The requested setup value, resolver fallback, separation boundary, and test coverage are explicit; remaining choices follow existing config and setup conventions.

## Validation Report

**Status:** validated  
**Date:** 2026-09-07

### Commands run

- `nix flake check` -> exit 0 (flake evaluation and checks passed)
- `nix run .#pkl-check-generated` -> exit 0 (ephemeral Pkl generation passed with 141 files)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup` -> exit 0 (69 setup tests passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml config::` -> exit 0 (59 config tests passed)

### Success-criteria verification

- [x] AC1: A newly generated repo-local `.sce/config.json` explicitly contains `"agent_trace": { "auto_sync": true }` alongside its schema declaration. -> setup test suite passed, including `repo_local_config_bootstrap_payload_uses_versioned_schema_url` and generated-payload/file bootstrap coverage.
- [x] AC2: When `agent_trace.auto_sync` is absent from all config layers, the resolver returns `false` with default provenance; explicit `true` and `false` values and existing global/local precedence remain unchanged. -> config resolver suite passed, including missing default, explicit true/false, and local-over-global tests.
- [x] AC3: Durable SCE configuration and setup documentation distinguishes setup's explicit `true` bootstrap value from the resolver's `false` fallback without changing the documented post-commit opt-out semantics. -> manually reviewed `context/overview.md`, `context/architecture.md`, `context/patterns.md`, `context/glossary.md`, `context/context-map.md`, `context/cli/config-precedence-contract.md`, `context/cli/agent-trace-auto-sync.md`, `context/sce/setup-repo-local-config-bootstrap.md`, and `context/sce/agent-trace-hooks-command-routing.md` against `cli/src/services/setup/mod.rs` and `cli/src/services/config/resolver.rs`; statements align.

### Failed checks and follow-ups

- None.

### Residual risks

- None identified.
