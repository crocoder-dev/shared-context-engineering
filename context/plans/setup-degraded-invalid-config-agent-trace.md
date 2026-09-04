# Plan: setup-degraded-invalid-config-agent-trace

## Change summary

Align `sce setup` and Agent Trace hook-runtime storage with ordinary startup
configuration behavior. When a default-discovered global or repo-local
`.sce/config.json` is invalid, the shared resolver should skip that layer,
retain the existing `sce.config.invalid_config` warning, and continue with any
valid remaining layer or degraded defaults. An outdated `$schema` URL must not
abort setup.

The same degraded values must reach
`open_agent_trace_db_for_hook_runtime()` so invalid discovered configuration does
not block conversation tracing, diff tracing, commit attribution, post-commit
processing, Claude model-state intake, or other Agent Trace DB-backed hook work.
Explicit `--config` and `SCE_CONFIG_FILE` selections remain fatal, and invalid
repo-local configuration is not repaired or rewritten as a side effect of
continuing setup.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the
check that proves it. `/validate` runs these checks; no task in the stack
performs final validation.

- [x] AC1: `sce setup` completes its normal Git/remote preflight, bootstrap,
  lifecycle, and requested asset-install flow when a default-discovered global
  or repo-local config file is invalid, including an outdated `$schema` URL;
  valid remaining layers and defaults continue to determine setup values, and
  the invalid repo-local file remains byte-for-byte unchanged.
  - Validate: Focused setup tests plus an integration-style setup case with
    invalid global/local discovered config assert successful continuation,
    unchanged invalid-file content, and the existing startup warning event.
- [x] AC2: `resolve_agent_trace_storage_runtime_config()` and
  `open_agent_trace_db_for_hook_runtime()` continue past invalid
  default-discovered config, use the remaining valid layer or default remote,
  and expose an open repository-scoped DB to DB-backed hook flows.
  - Validate: Focused resolver, Agent Trace storage, and hooks tests cover
    invalid global/local layers, default/remaining-layer identity selection,
    successful hook-runtime DB opening, and representative conversation/diff
    persistence paths.
- [x] AC3: Invalid explicit `--config` and `SCE_CONFIG_FILE` selections remain
  fatal, while the existing config schema, repository identity canonicalization,
  remote precedence, hook no-migration behavior, and genuine DB fail-open
  diagnostics remain unchanged.
  - Validate: Focused resolver/setup/hooks tests assert explicit-config failure,
    identity and migration invariants, and unchanged hook diagnostics.

### Full validation

Repository-wide checks `/validate` runs after the last task, regardless of
which criterion they map to.

- `nix run .#pkl-check-generated`
- `nix flake check`

### Context sync

- `context/overview.md`
- `context/architecture.md`
- `context/glossary.md`
- `context/cli/config-precedence-contract.md`
- `context/sce/setup-repo-local-config-bootstrap.md`
- `context/cli/agent-trace-storage.md`
- `context/sce/agent-trace-hooks-command-routing.md`
- A new dated decision record superseding `context/decisions/2026-08-26-setup-storage-fail-closed-on-invalid-config.md`

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** The default-discovered invalid-layer boundary in the shared
  runtime resolver, setup preflight and integration-persistence behavior,
  `open_agent_trace_db_for_hook_runtime()` and its hook callers, focused Rust
  regression tests, and the listed durable context.
- **Out of scope:** Changes to the JSON Schema, startup warning identifier,
  explicit-config strictness, repository identity canonicalization or remote
  precedence, Agent Trace DB schema/migrations, generated integrations, or
  unrelated setup behavior.
- **Constraints:** Reuse the existing resolver and schema seams; preserve
  credential-safe deterministic diagnostics, startup warning behavior, setup Git
  and remote preflights, no-migration hook opening, existing hook fail-open
  handling for genuine DB failures, and Nix-based verification. Add no
  dependencies.
- **Non-goal:** Do not make invalid configuration silently valid, delete or
  repair user files, introduce a new storage fallback database, or make explicit
  config selections degradable.

## Assumptions

- “Follow startup behavior” means only default-discovered invalid layers are
  skipped; explicit `--config` and `SCE_CONFIG_FILE` inputs remain fatal.
- Continuing setup with an invalid repo-local file must not rewrite that file;
  setup may omit integration-target/optional-workflow persistence for that run
  rather than mutating invalid user configuration.
- The existing repository-scoped identity fallback (`agent_trace.repository_id`,
  configured remote, then default `origin`) and no-migration hook DB path are
  sufficient; no new identity or database fallback is needed.

## Task stack

- [x] T01: `Align setup and Agent Trace hook storage with degraded discovered config` (status:done)
  - Task ID: T01
  - Scope: In — remove setup's default-discovered invalid-config hard-fail, make Agent Trace storage runtime configuration use the shared skipped-layer result, keep invalid repo-local config untouched when setup continues, and add focused resolver/setup/storage/hooks regressions for `open_agent_trace_db_for_hook_runtime()` and representative DB-backed hook writes. Preserve explicit-config failures, startup warning emission, Git/remote preflight order, repository identity precedence, no-migration opening, and genuine DB failure diagnostics. Out — schema changes, config repair or migration, new storage fallbacks, repository identity canonicalization, generated assets, and unrelated setup/hook behavior.
   - Dependencies: none
   - Done when: Invalid default-discovered global or local config no longer aborts setup or blocks repository-scoped Agent Trace hook DB opening; setup completes without rewriting the invalid local file; remaining-layer/default precedence and explicit-config failure behavior are covered by passing focused tests.
   - Verify: `nix develop -c sh -c './scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::config::resolver && ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::setup && ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::agent_trace_storage && ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::hooks'`.
   - Completed: 2026-09-04
   - Files changed: `cli/src/services/config/resolver.rs`; `cli/src/services/setup/command.rs`; `cli/src/services/setup/mod.rs`; `cli/src/services/hooks/mod.rs`
   - Result: Agent Trace storage now consumes the shared degraded resolver result instead of failing on invalid default-discovered layers. Setup no longer rejects invalid repo-local config during preflight, and target persistence skips invalid local files without rewriting them. Added resolver precedence regressions, byte-preserving setup persistence coverage, and hook-runtime DB opening/conversation-write coverage with invalid discovered local config. Explicit configuration failures and existing DB/hook behavior remain unchanged.
   - Verify: `nix develop -c sh -c './scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::config::resolver && ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::setup && ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::agent_trace_storage && ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::hooks'` — passed (20 resolver, 67 setup, 14 Agent Trace storage, and 181 hooks tests).
   - Context impact: Material cross-cutting behavior change to shared config resolution, setup persistence, and Agent Trace hook storage; durable context synchronization is required for the listed config/setup/Agent Trace contracts and a superseding decision record.
   - Context synchronization: synced

## Open questions

None. The requested boundary is explicit, the repository already owns the
degraded resolver and hook-runtime opener, and the non-destructive persistence
choice follows the existing setup asset/config safety rules.

## Validation Report

**Status:** validated  
**Date:** 2026-09-04

### Commands run

- `nix run .#pkl-check-generated` -> exit 0 (ephemeral Pkl generation passed: 141 files; inventory parity matched)
- `nix flake check` -> exit 0 (all flake checks passed)
- `nix develop -c sh -c './scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::config::resolver && ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::setup && ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::agent_trace_storage && ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::hooks'` -> exit 0 (20 resolver, 67 setup, 14 Agent Trace storage, and 181 hooks tests passed)

### Success-criteria verification

- [x] AC1: `sce setup` completes its normal Git/remote preflight, bootstrap, lifecycle, and requested asset-install flow when a default-discovered global or repo-local config file is invalid, including an outdated `$schema` URL; valid remaining layers and defaults continue to determine setup values, and the invalid repo-local file remains byte-for-byte unchanged. -> Resolver and setup regression tests passed, including invalid discovered-layer continuation and byte-preserving invalid repo-local persistence.
- [x] AC2: `resolve_agent_trace_storage_runtime_config()` and `open_agent_trace_db_for_hook_runtime()` continue past invalid default-discovered config, use the remaining valid layer or default remote, and expose an open repository-scoped DB to DB-backed hook flows. -> Resolver, Agent Trace storage, and hooks suites passed, including invalid-layer precedence, hook-runtime DB opening, conversation writes, diff persistence, and post-commit flows.
- [x] AC3: Invalid explicit `--config` and `SCE_CONFIG_FILE` selections remain fatal, while the existing config schema, repository identity canonicalization, remote precedence, hook no-migration behavior, and genuine DB fail-open diagnostics remain unchanged. -> Focused resolver, setup, Agent Trace storage, and hooks suites passed, including explicit-config, identity, remote, migration, and fail-open regression coverage.

### Failed checks and follow-ups

- None.

### Residual risks

- None identified.
