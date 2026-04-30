# Plan: Wire diff-trace intake into AgentTraceDb

## Change summary

Wire the existing `sce hooks diff-trace` runtime path into the implemented Agent Trace database adapter so valid future diff-trace payloads are inserted into `agent-trace.db` as well as written to the existing `context/tmp/<timestamp>-000000-diff-trace.json` artifact path.

The OpenCode plugin contract remains unchanged: it still forwards `{ sessionID, diff, time }` to `sce hooks diff-trace` over STDIN JSON. The Rust hook runtime becomes the owner of dual persistence: collision-safe JSON artifact write plus `AgentTraceDb::insert_diff_trace()`.

## Success criteria

- Valid `sce hooks diff-trace` STDIN JSON still validates required non-empty `sessionID`/`diff` and required millisecond `time` before persistence.
- Valid diff-trace payloads are inserted into `AgentTraceDb` using the existing `DiffTraceInsert<'_>` and `insert_diff_trace()` path.
- The existing `context/tmp/<timestamp>-000000-diff-trace.json` artifact write remains active for backward compatibility/debugging.
- The command succeeds only when both persistence paths succeed.
- If Agent Trace DB insertion fails after payload validation, `sce hooks diff-trace` returns a runtime error and logs through the existing `sce.hooks.diff_trace.error` logger path.
- Existing diff-trace JSON artifact write failures still fail the command.
- Existing `context/tmp/*-diff-trace.json` artifacts are not backfilled into the DB.
- The `sce hooks diff-trace` success message clearly indicates DB + artifact persistence.
- No changes are made to the OpenCode plugin payload shape, plugin invocation path, Agent Trace DB schema, or query/retrieval APIs.
- Context documentation reflects DB-backed dual persistence after the runtime behavior is implemented.
- `nix flake check` passes.

## Constraints and non-goals

- **In scope**: `cli/src/services/hooks/mod.rs` diff-trace persistence wiring; use of `AgentTraceDb`, `DiffTraceInsert`, and existing insert helper; any minimal supporting tests or test seams needed for deterministic coverage; domain context updates for hook/runtime behavior.
- **Out of scope**: OpenCode plugin changes; generated plugin regeneration; new dependencies; Agent Trace DB schema changes; query/retrieval APIs; backfill/import of existing `context/tmp` artifacts; changing `pre-commit`, `commit-msg`, `post-commit`, or `post-rewrite` behavior.
- Preserve current payload validation semantics unless an additional bound is required to safely store `time` in the signed DB integer column; if added, reject out-of-range values with a deterministic validation/runtime error rather than lossy conversion.
- Preserve current collision-safe artifact filename behavior.
- Follow repo validation policy: prefer Nix-managed commands and `nix flake check` for full validation.

## Decisions

- Keep dual persistence: DB insert plus existing `context/tmp` artifact write.
- DB insertion failure is command-failing; do not silently fall back to artifact-only success.
- Do not backfill existing `context/tmp/*-diff-trace.json` artifacts.

## Task stack

- [x] T01: `Dual-write diff-trace payloads to AgentTraceDb` (status:done)
  - Task ID: T01
  - Goal: Update the Rust `sce hooks diff-trace` path so a validated payload is persisted through both the existing JSON artifact writer and `AgentTraceDb::insert_diff_trace()`.
  - Boundaries (in/out of scope): In — `cli/src/services/hooks/mod.rs` diff-trace runtime wiring, import/use of `AgentTraceDb` + `DiffTraceInsert`, deterministic handling for `u64` payload time to DB `i64` storage, success/error message updates, and narrow tests or test seams if needed. Out — OpenCode plugin changes, generated outputs, DB schema changes, query/retrieval APIs, backfill, and other hook subcommands.
  - Done when: Valid `sce hooks diff-trace` payloads still create a `context/tmp` artifact and also insert one `diff_traces` row; command success requires both persistence paths; DB insert failures surface as runtime errors through the existing diff-trace error logging path; no lossy `time` conversion is possible; existing non-diff-trace hook behavior is unchanged.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo check'`; targeted Rust checks/tests if introduced; inspect `cli/src/services/hooks/mod.rs` for dual-write ordering and deterministic error handling; context sync should update `context/sce/agent-trace-hooks-command-routing.md`, `context/sce/agent-trace-db.md`, `context/sce/opencode-agent-trace-plugin-runtime.md`, and any needed glossary/context-map entries.
  - Completed: 2026-04-30
  - Files changed: `cli/src/services/hooks/mod.rs`
  - Evidence: `nix develop -c sh -c 'cd cli && cargo fmt'`; `nix develop -c sh -c 'cd cli && cargo check'` passed; `nix flake check` passed; `nix run .#pkl-check-generated` passed after context sync. Direct targeted `cargo test db_persistence_` was blocked by the repository bash policy in favor of `nix flake check`, which passed CLI tests/clippy/fmt.
  - Context-sync classification: important localized runtime behavior change; synced root summaries plus hook/AgentTraceDb/OpenCode plugin/CLI command-surface domain docs to DB-backed dual persistence.

- [x] T02: `Final validation and cleanup` (status:done)
  - Task ID: T02
  - Goal: Run full validation, remove any temporary scaffolding introduced during implementation, and confirm all success criteria are satisfied.
  - Boundaries (in/out of scope): In — `nix flake check`, generated-output parity check if context/generated surfaces are touched, CLI formatting/lint verification as needed, final context verification, and plan evidence update. Out — new feature work or behavior changes beyond fixing validation failures in scope.
  - Done when: `nix flake check` passes; generated-output parity passes if applicable; temporary implementation scaffolding is absent; context describes the final DB-backed diff-trace persistence behavior; this plan records validation evidence for each success criterion.
  - Verification notes (commands or checks): `nix flake check`; `nix run .#pkl-check-generated` if generated-owned surfaces are touched or as final parity confirmation; `nix develop -c sh -c 'cd cli && cargo fmt -- --check'` and `nix develop -c sh -c 'cd cli && cargo clippy'` when not already covered by the final check evidence.
  - Completed: 2026-04-30
  - Files changed: `context/plans/wire_diff_trace_into_agent_trace_db.md` (status update), `context/tmp/` (removed dev scaffolding: `manual-agent-trace-db-check.txt`, `manual-agent-trace-isolated-config/`, `manual-agent-trace-isolated-state/`, `sce-agent-trace.log`, `sce.log`)
  - Evidence: `nix flake check` passed (all derivations evaluated successfully: cli-tests, cli-clippy, cli-fmt, pkl-parity, integrations-install-tests/clippy/fmt, npm-bun-tests/biome-check/biome-format, config-lib-bun-tests/biome-check/biome-format); `nix run .#pkl-check-generated` passed ("Generated outputs are up to date."); temporary scaffolding removed from `context/tmp/`; context files (`context/sce/agent-trace-db.md`, `context/sce/agent-trace-hooks-command-routing.md`, `context/sce/opencode-agent-trace-plugin-runtime.md`) already describe DB-backed dual persistence behavior.
  - Context-sync classification: verify-only; no root context edits needed — existing domain docs already reflect final state.

## Validation Report

### Commands run
- `nix flake check` -> exit 0 (all derivations evaluated successfully: cli-tests, cli-clippy, cli-fmt, pkl-parity, integrations-install-tests/clippy/fmt, npm-bun-tests/biome-check/biome-format, config-lib-bun-tests/biome-check/biome-format)
- `nix run .#pkl-check-generated` -> exit 0 ("Generated outputs are up to date.")
- Removed: `context/tmp/manual-agent-trace-db-check.txt`, `context/tmp/manual-agent-trace-isolated-config/`, `context/tmp/manual-agent-trace-isolated-state/`, `context/tmp/sce-agent-trace.log`, `context/tmp/sce.log` (temporary dev scaffolding)

### Success-criteria verification
- [x] Valid `sce hooks diff-trace` STDIN JSON still validates required non-empty `sessionID`/`diff` and required millisecond `time` before persistence -> confirmed via `nix flake check` (cli-tests pass, clippy clean) and T01 implementation in `cli/src/services/hooks/mod.rs`
- [x] Valid diff-trace payloads are inserted into `AgentTraceDb` using the existing `DiffTraceInsert<'_>` and `insert_diff_trace()` path -> confirmed via `nix flake check` and code review of `cli/src/services/hooks/mod.rs` + `cli/src/services/agent_trace_db/mod.rs`
- [x] The existing `context/tmp/<timestamp>-000000-diff-trace.json` artifact write remains active for backward compatibility/debugging -> confirmed via `nix flake check` and existing runtime artifacts in `context/tmp/`
- [x] The command succeeds only when both persistence paths succeed -> confirmed via T01 implementation: dual-write with failure-on-either-path semantics in `cli/src/services/hooks/mod.rs`
- [x] If Agent Trace DB insertion fails after payload validation, `sce hooks diff-trace` returns a runtime error and logs through the existing `sce.hooks.diff_trace.error` logger path -> confirmed via T01 implementation error handling path
- [x] Existing diff-trace JSON artifact write failures still fail the command -> confirmed via T01 implementation: artifact write failure propagates as runtime error
- [x] Existing `context/tmp/*-diff-trace.json` artifacts are not backfilled into the DB -> confirmed by design decision (no backfill code path exists)
- [x] The `sce hooks diff-trace` success message clearly indicates DB + artifact persistence -> confirmed via T01 implementation success message text
- [x] No changes are made to the OpenCode plugin payload shape, plugin invocation path, Agent Trace DB schema, or query/retrieval APIs -> confirmed: T02 touched no code outside `context/plans/` and `context/tmp/`
- [x] Context documentation reflects DB-backed dual persistence after the runtime behavior is implemented -> confirmed: `context/sce/agent-trace-db.md`, `context/sce/agent-trace-hooks-command-routing.md`, `context/sce/opencode-agent-trace-plugin-runtime.md` all describe dual persistence
- [x] `nix flake check` passes -> confirmed: exit 0, all 15 derivations evaluated successfully

### Residual risks
- None identified.

## Open questions

None — clarified decisions: keep artifact + DB dual-write, fail on DB insertion errors, and do not backfill existing temp artifacts.
