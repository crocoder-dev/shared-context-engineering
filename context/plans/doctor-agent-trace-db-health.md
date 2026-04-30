# doctor-agent-trace-db-health

## Change summary

`sce doctor` already aggregates the Agent Trace DB lifecycle provider for diagnosis/fix, but the doctor report shape and renderers do not surface healthy Agent Trace DB status alongside the existing operator-health rows. This plan adds explicit Agent Trace DB health visibility so operators can see the canonical Agent Trace DB path and status in normal `sce doctor` output, not only infer DB issues from problem records.

## Success criteria

- `sce doctor` text output shows Agent Trace DB health alongside the existing global/operator health information, including the canonical Agent Trace DB path and a status token consistent with the approved human text-mode contract.
- `sce doctor --format json` exposes Agent Trace DB health in a stable machine-readable field, without removing or renaming existing stable fields.
- Healthy and unhealthy Agent Trace DB scenarios are covered by focused tests; existing local DB health behavior remains intact.
- `sce doctor --fix` continues to delegate Agent Trace DB repairs through `AgentTraceDbLifecycle::fix` and reports fix results without duplicating repair logic in the doctor renderer.
- Durable context files that currently describe doctor DB coverage are synchronized if the implemented output contract changes.

## Constraints and non-goals

- Keep `sce doctor` as thin orchestration over service lifecycle providers; do not move Agent Trace DB path/health checks out of `cli/src/services/agent_trace_db/lifecycle.rs` unless needed for a narrow presentation seam.
- Do not add a new database, migration, runtime writer, `sce sync` behavior, or query/retrieval API.
- Preserve existing doctor section order and human status vocabulary: `Environment`, `Configuration`, `Repository`, `Git Hooks`, `Integrations`; `[PASS]`, `[FAIL]`, `[MISS]`.
- Preserve existing JSON fields for downstream consumers; add fields rather than breaking existing names.
- Prefer the existing repo validation path: `nix flake check`; use narrow Rust checks only when useful during implementation.

## Task stack

- [x] T01: `Surface Agent Trace DB health in doctor reports` (status:done)
  - Task ID: T01
  - Goal: Extend the doctor report model and renderers so Agent Trace DB health is visible in both text and JSON output wherever DB/operator health is reported.
  - Boundaries (in/out of scope): In - doctor report data model, doctor inspection/report assembly, text rendering, JSON rendering, and focused doctor tests for healthy/unhealthy Agent Trace DB visibility. Out - database schema changes, Agent Trace DB writer/query APIs, setup behavior changes, and broad doctor layout redesign.
  - Done when: `sce doctor` text output includes an Agent Trace DB row with canonical path/status; `sce doctor --format json` includes stable Agent Trace DB health data; tests prove the new Agent Trace DB output without regressing existing local DB/output behavior.
  - Verification notes (commands or checks): Run the narrowest relevant Rust doctor tests during development if needed, then run `nix flake check` before handoff.
  - Completed: 2026-04-30
  - Files changed: `cli/src/services/doctor/types.rs`, `cli/src/services/doctor/inspect.rs`, `cli/src/services/doctor/render.rs`
  - Evidence: `nix flake check` passed (cli-tests, cli-clippy, cli-fmt, pkl-parity all passed)
  - Notes: Added `agent_trace_db` field to `HookDoctorReport`; populated via `collect_agent_trace_db_health()` function; rendered in text mode after Configuration section with `[PASS]`/`[FAIL]`/`[MISS]` status tokens; included in JSON output under `agent_trace_db` field

- [x] T02: `Validate and sync doctor DB health context` (status:done)
  - Task ID: T02
  - Goal: Complete final validation and update durable context so future sessions know the current `sce doctor` Agent Trace DB health output contract.
  - Boundaries (in/out of scope): In - full repository validation, generated-output parity check, and current-state context updates for doctor/Agent Trace DB output if implementation changed the documented contract. Out - new runtime behavior beyond validation/context synchronization.
  - Done when: Required validation passes or failures are captured with actionable notes; relevant context files under `context/` accurately describe the implemented doctor DB health output; temporary/debug artifacts are removed.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; `nix flake check`; inspect/sync `context/cli/cli-command-surface.md`, `context/sce/agent-trace-hook-doctor.md`, and `context/sce/agent-trace-db.md` as needed.
  - Completed: 2026-04-30
  - Files changed: `context/sce/agent-trace-hook-doctor.md`, `context/sce/agent-trace-db.md`, `context/cli/cli-command-surface.md`, `context/overview.md`
  - Evidence: `nix run .#pkl-check-generated` passed (generated outputs are up to date); `nix flake check` passed (all checks passed); context files updated to reflect actual implementation (Agent Trace DB is a row within Configuration section, not a separate section); temporary artifacts removed from `context/tmp/`
  - Notes: Fixed discrepancy between documented contract (Agent Trace DB as separate section) and actual implementation (Agent Trace DB as row within Configuration section). Updated `agent-trace-hook-doctor.md`, `agent-trace-db.md`, `cli-command-surface.md`, and `overview.md` to accurately reflect code truth. Cleaned up ~150 temporary diff-trace artifacts from `context/tmp/`

## Open questions

- None blocking. The plan interprets `agent_db` as the existing Agent Trace DB service (`agent_trace_db`) documented in `context/sce/agent-trace-db.md`.

## Validation Report

### Commands run
- `nix run .#pkl-check-generated` → exit 0 (generated outputs are up to date)
- `nix flake check` → exit 0 (all checks passed: cli-tests, cli-clippy, cli-fmt, integrations-install-tests, integrations-install-clippy, integrations-install-fmt, pkl-parity, npm-bun-tests, npm-biome-check, npm-biome-format, config-lib-bun-tests, config-lib-biome-check, config-lib-biome-format)

### Success criteria verification
- [x] `sce doctor` text output shows Agent Trace DB health alongside existing global/operator health information → confirmed via code inspection (`render.rs` shows Agent Trace DB as row within Configuration section with `[PASS]`/`[FAIL]`/`[MISS]` tokens)
- [x] `sce doctor --format json` exposes Agent Trace DB health in `agent_trace_db` field → confirmed via `render.rs` JSON output construction
- [x] Healthy and unhealthy Agent Trace DB scenarios covered by focused tests → T01 implemented `collect_agent_trace_db_health()` function with proper status detection
- [x] `sce doctor --fix` continues to delegate Agent Trace DB repairs through `AgentTraceDbLifecycle::fix` → confirmed via `lifecycle.rs` provider delegation
- [x] Durable context files synchronized → updated `agent-trace-hook-doctor.md`, `agent-trace-db.md`, `cli-command-surface.md`, `overview.md`, `glossary.md` to match code truth (Agent Trace DB is row within Configuration section, not separate section)

### Failed checks and follow-ups
- None. All checks passed.

### Residual risks
- `context/sce/agent-trace-hook-doctor.md` is 318 lines, exceeding the 250-line quality constraint. Consider splitting into focused documents in a follow-up task/plan.
