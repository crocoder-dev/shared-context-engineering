# Plan: automatic-agent-trace-sync

## Change summary

Add opt-in, post-commit Agent Trace synchronization without changing the existing synchronization engine. After the production post-commit flow successfully persists the built Agent Trace in the repository-scoped database, the hook will resolve `agent_trace.auto_sync` through the existing config resolver and, when enabled, launch the current `sce` executable as a detached/best-effort `sync --format json` child in the repository root. The hook will not wait for the child, expose its output, or make child startup/completion/network failures affect a successful commit.

Extend the canonical Pkl schema and Rust config layers with `agent_trace.auto_sync`, defaulting to `false`, including typed parsing, resolution, inspection/validation output, and focused schema/resolution tests. Add a small sync-owned launcher seam and production tests for command construction, working directory, fail-open spawning, and post-commit ordering. Preserve the existing `services::sync` cursor-authoritative four-stream implementation, avoid all high-frequency trace-hook triggers and persistent background machinery, and document the one-shot asynchronous behavior and retry semantics in durable SCE context.

## Acceptance criteria

- [x] AC1: A config file containing `{ "agent_trace": { "auto_sync": true } }` validates and resolves as enabled, an invalid `auto_sync` type is rejected, and an omitted value resolves to `false`.
  - Validate: targeted config schema/resolver tests for valid, invalid-type, and omitted-value cases; `nix run .#pkl-check-generated`.
- [x] AC2: When `agent_trace.auto_sync` is enabled and post-commit Agent Trace persistence succeeds, the hook launches the current executable with exactly `sync --format json`, uses the repository root as child working directory, discards stdin/stdout/stderr, and returns without waiting for the child.
  - Validate: focused launcher and post-commit boundary tests asserting executable/arguments/current directory/stdio configuration and injected launcher invocation ordering.
- [x] AC3: Disabled auto-sync causes no launch; failed Agent Trace validation or persistence causes no launch; and a launcher/current-executable/spawn failure leaves the otherwise successful post-commit result successful.
  - Validate: focused post-commit and launcher failure tests covering each fail-open branch.
- [x] AC4: Automatic synchronization invokes only the existing `sce sync` command and introduces no daemon, watcher, polling loop, local cursor, synchronization database, persistent service, or high-frequency `conversation-trace`/`diff-trace` trigger.
  - Validate: code inspection plus targeted module tests and the existing sync test suite; verify no changes to the existing sync protocol/cursor implementation.
- [x] AC5: Durable SCE context explains manual `sce sync`, opt-in `agent_trace.auto_sync`, one-shot asynchronous execution, no daemon, fail-open behavior, and local retryability through the control-plane cursor authority.
  - Validate: manual review of the updated/new context files against the implemented code.

### Full validation

Repository-wide validation after the last task:

- `nix flake check`
- `nix run .#pkl-check-generated`

### Context sync

- `context/overview.md`
- `context/architecture.md`
- `context/glossary.md`
- `context/patterns.md`
- `context/context-map.md`
- `context/cli/sync-command.md`
- `context/cli/config-precedence-contract.md`
- `context/sce/agent-trace-hooks-command-routing.md`
- A new `context/cli/agent-trace-auto-sync.md` documenting the automatic trigger and its non-goals.

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`. A completed task must be `synced` before another task can start or the plan can finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition** beside the status. Never infer `synced` from conversation history; write every lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** canonical Pkl config schema; Rust config DTO, schema, resolver, render/validation plumbing and tests; `cli/src/services/sync/auto_sync.rs` or equivalent launcher; production post-commit boundary integration and injected launcher tests; relevant durable SCE documentation.
- **Out of scope:** changes to synchronization HTTP/API behavior, cursor reconciliation, export readers, Agent Trace DB schema, local sync state, high-frequency trace hooks, plugin/event-triggered sync, retry queues, daemons, watchers, polling loops, schedulers, PID files, locks/leases, and persistent background services.
- **Constraints:** use `std::env::current_exe()` rather than `$PATH`; spawn `sync --format json` with null stdin/stdout/stderr and repository-root `current_dir`; use `Command::spawn()` without status/wait; treat launcher failures and child non-zero completion as fail-open; preserve server-owned cursor retry behavior; use Nix-managed repository tooling and ephemeral Pkl generation rather than editing generated artifacts.
- **Non-goal:** implementing a second synchronization mechanism or making Git wait for network synchronization.

## Assumptions

- `agent_trace.auto_sync` is a config-file value in the existing global-then-local config merge, with no new environment variable or CLI flag; its omitted/default value is `false`, matching the requested opt-in rollout and the existing repository-identity config pattern.
- The launcher gets a narrow test seam (an injected process-launch closure or equivalent internal adapter) so tests can assert the child specification and spawn failures without starting a real sync process; production still uses `current_exe()` and `Command::spawn()`.
- Existing `sce config show`/`validate` surfaces are part of “support throughout the existing config system,” so the resolved `auto_sync` value and its default/config-file provenance are exposed consistently with the other Agent Trace config values.

## Task stack

- [x] T01: `Add opt-in agent_trace.auto_sync config resolution` (status:done)
  - Task ID: T01
  - Scope: In — `config/pkl/base/sce-config-schema.pkl`, `cli/src/services/config/schema.rs`, `types.rs`, `resolver.rs`, config rendering/validation plumbing, and focused config tests for boolean validation, default false, global/local resolution, and inspection output. Out — process spawning and post-commit behavior.
  - Dependencies: none
  - Done when: the generated schema accepts boolean `agent_trace.auto_sync`, rejects non-boolean values, omitted config resolves to false, configured values resolve through the existing config layers, and config show/validate remain deterministic with the new field.
  - Verify: targeted config tests through `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml config::`; `nix run .#pkl-check-generated`.
  - Context synchronization: synced
  - Completed: 2026-08-19
  - Files changed: `cli/src/services/config/render.rs`, `cli/src/services/config/resolver.rs`, `cli/src/services/config/schema.rs`, `cli/src/services/config/types.rs`, `config/pkl/base/sce-config-schema.pkl`, `context/plans/automatic-agent-trace-sync.md`
  - Result: Added boolean `agent_trace.auto_sync` schema and typed config plumbing, defaulting to false, with global/local precedence, resolved provenance, deterministic config show output, and focused validation/resolution tests.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml config::` — pass (25 tests); `nix run .#pkl-check-generated` — pass (107 generated files, inventory sha256 `5ebbf7a119a7f79e19f65a7c30ee032681ae749279270735b5fbb87b0e1b2658`).
  - Context impact: interface — document the new `agent_trace.auto_sync` config contract, default/provenance behavior, and its future post-commit hook boundary in current-state config and Agent Trace context; review all five root context files for stale configuration summaries.

- [x] T02: `Implement fail-open one-shot sync launcher` (status:done)
  - Task ID: T02
  - Scope: In — `cli/src/services/sync/auto_sync.rs` or equivalent, sync module registration, production `current_exe`/`Command::spawn` construction, null stdio/current-directory configuration, and deterministic launcher tests. Out — invoking the launcher from high-frequency hooks or changing sync internals.
  - Dependencies: T01
  - Done when: the production trigger constructs the current-executable `sync --format json` child in the supplied repository root, detaches without waiting, suppresses all child streams, and ignores current-executable/spawn failures; tests prove exact command construction and fail-open behavior.
  - Verify: targeted Rust tests for `services::sync::auto_sync` through `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml auto_sync`.
  - Completed: 2026-08-19
  - Files changed: `cli/src/services/sync/auto_sync.rs`, `cli/src/services/sync/mod.rs`, `context/cli/sync-command.md`, `context/plans/automatic-agent-trace-sync.md`
  - Result: Added a sync-owned best-effort launcher that resolves `current_exe`, starts `sync --format json` in the supplied repository root with null stdin/stdout/stderr, drops the child without waiting, and ignores executable/spawn failures; added exact command-spec and fail-open tests without changing sync internals or hook call sites.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml auto_sync` — pass (8 tests, including existing auto-sync config tests).
  - Context impact: domain — documented the sync-owned asynchronous launcher boundary and its fail-open/no-wait behavior in `context/cli/sync-command.md`; root context pass verified with no edits.
  - Context synchronization: synced

- [x] T03: `Trigger auto-sync after successful post-commit persistence` (status:done)
  - Task ID: T03
  - Scope: In — `cli/src/services/hooks/mod.rs` production post-commit boundary, config-gate lookup, launcher injection seam for post-commit tests, and focused tests for disabled/enabled/order/persistence-failure/spawn-failure cases. Out — `pre-commit`, `diff-trace`, `conversation-trace`, plugin event changes, and synchronization algorithm changes.
  - Dependencies: T01, T02
  - Done when: a successful post-commit persistence result triggers exactly one best-effort launcher only when resolved auto-sync is enabled; persistence failures do not trigger it; disabled config does not trigger it; launcher failures do not change the successful hook result; the hook path never waits on the child.
  - Verify: targeted Rust hook tests through `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml hooks::`; inspect the post-commit boundary and confirm no high-frequency hook call sites changed.
  - Completed: 2026-08-19
  - Files changed: `cli/src/services/hooks/mod.rs`
  - Result: Integrated the resolved `agent_trace.auto_sync` gate after successful post-commit Agent Trace persistence, invoking the existing sync-owned launcher through an injected fail-open seam; added tests for enabled ordering, disabled behavior, persistence failure, and launcher failure without changing high-frequency hook paths.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml hooks::` — pass (24 tests); post-commit boundary inspection confirmed launch occurs only after persistence and diff/conversation trace call sites were unchanged.
  - Context impact: interface — document the automatic post-commit trigger boundary, resolved config gate, and fail-open launcher behavior in durable SCE context; review all five root context files for stale hook/config descriptions.
  - Context synchronization: synced

- [x] T04: `Document asynchronous post-commit Agent Trace synchronization` (status:done)
  - Task ID: T04
  - Scope: In — the new auto-sync context document and the listed overview, architecture, glossary, patterns, context-map, sync-command, config-precedence, and hook-routing updates, reflecting the final implemented names and behavior. Out — code changes, generated target trees, generated schema artifacts, and historical decision records.
  - Dependencies: T03
  - Done when: durable context distinguishes explicit/manual `sce sync` from opt-in asynchronous post-commit triggering, states that there is no daemon and failures are fail-open, explains that pending rows remain local for later retry, and accurately names the config and hook boundaries.
  - Verify: manual code/context review; `nix run .#pkl-check-generated`; `nix flake check`.
  - Completed: 2026-08-19
  - Files changed: `context/plans/automatic-agent-trace-sync.md` (lifecycle/evidence record; scoped durable context was already current at the Git baseline)
  - Result: Verified the scoped durable context against the implemented config resolver, sync-owned launcher, and post-commit hook boundary; all required auto-sync behavior, fail-open semantics, retryability, and non-goals are accurately documented, so no additional context text changes were necessary.
  - Verify: manual code/context review — pass; `nix run .#pkl-check-generated` — pass (107 generated files, inventory sha256 `5ebbf7a119a7f79e19f65a7c30ee032681ae749279270735b5fbb87b0e1b2658`); `nix flake check` — pass (all checks passed; incompatible systems omitted).
  - Context impact: interface — verified the new `agent_trace.auto_sync` config contract, asynchronous post-commit trigger boundary, fail-open launcher semantics, manual/cursor-authoritative retry path, and no-daemon/high-frequency-trigger non-goals across the listed durable context files.
  - Context synchronization: synced

## Open questions

None. The request fixes the trigger boundary, command shape, opt-in default, fail-open semantics, prohibited architectures, test expectations, and documentation requirements; the plan records only local implementation choices that follow existing repository patterns.

## Validation Report

**Status:** validated
**Plan:** `context/plans/automatic-agent-trace-sync.md`
**Name:** `automatic-agent-trace-sync`
**Tasks:** `4/4 complete`
**Date:** `2026-08-19`

## Commands run

- `nix flake check` -> passed — all flake checks passed; incompatible systems omitted.
- `nix run .#pkl-check-generated` -> passed — ephemeral Pkl generation passed for 107 files.
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml config::` -> passed — 25 targeted config tests passed.
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml auto_sync` -> passed — 13 launcher/config/post-commit auto-sync tests passed.
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml hooks::` -> passed — 25 focused hook tests passed.
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml sync::` -> passed — 57 existing sync tests passed.

## Acceptance criteria

- [x] AC1: A config file containing `{ "agent_trace": { "auto_sync": true } }` validates and resolves as enabled, an invalid `auto_sync` type is rejected, and an omitted value resolves to `false` — targeted config tests and generated Pkl validation passed.
- [x] AC2: When enabled, post-commit launches the exact detached command in the repository root with null stdio — launcher and post-commit ordering tests passed.
- [x] AC3: Disabled, validation-failure, persistence-failure, launcher/current-executable/spawn-failure paths are fail-open — focused tests passed, including `post_commit_validation_failure_does_not_resolve_or_launch_auto_sync`.
- [x] AC4: Automatic synchronization reuses only `sce sync` without prohibited daemon, cursor, persistence, or high-frequency trigger behavior — targeted sync tests and code inspection passed.
- [x] AC5: Durable context accurately documents opt-in asynchronous sync, manual retryability, cursor authority, fail-open behavior, and no daemon — manual review passed.

## Residual risks

- None identified.
