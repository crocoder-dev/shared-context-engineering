# Plan: repair-durable-context-drift

## Change summary

Repair code-truth drift in the durable `context/` documentation for the current `sce` CLI surface.

This plan is scoped to durable current-state docs only: root shared context files and focused CLI/domain docs that describe implemented CLI behavior. The goal is to remove stale statements where `context/` no longer matches the current Rust CLI implementation, with code treated as the source of truth.

Known drift already visible during planning includes:

- durable docs that still describe `auth` as `login|logout|status` even though code also implements `auth renew`
- uneven representation of the implemented `sce trace prompts <commit-sha>` surface across shared context files
- durable help/command-surface descriptions that need parity against the current `clap` schema and service modules

## Success criteria

- Root durable context (`context/overview.md`, `context/architecture.md`, `context/glossary.md`) accurately reflects the current implemented CLI command surface, including `auth renew` and `sce trace prompts <commit-sha>`.
- Focused CLI context files under `context/cli/` that describe auth, trace, help, or command-surface behavior match current code truth.
- `context/context-map.md` and related durable discovery references guide future sessions to the corrected current-state docs without stale auth/trace omissions.
- No in-scope durable `context/` file continues to make a direct behavior claim contradicted by `cli/src/command_surface.rs`, `cli/src/cli_schema.rs`, `cli/src/app.rs`, `cli/src/services/auth_command.rs`, or `cli/src/services/trace.rs`.

## Constraints and non-goals

- In scope: durable current-state docs under `context/`, especially root shared files, focused CLI docs, and navigation/discovery files that directly describe implemented CLI behavior.
- In scope: code-drift repairs only; code is the source of truth for this pass.
- Out of scope: application-code changes, README/root-doc changes outside `context/`, grammar-only cleanup, and broad prose polishing that is not required for code-truth parity.
- Out of scope: active plans, handovers, and historical decision records unless a broken reference inside durable navigation makes a targeted repair necessary.
- Each executable task must remain one coherent commit unit.

## Task stack

- [x] T01: `Repair root durable CLI capability docs` (status:done)
  - Task ID: T01
  - Goal: Update the root shared context files so they accurately describe the current implemented CLI command surface and high-level behavior contracts.
  - Boundaries (in/out of scope): In - `context/overview.md`, `context/architecture.md`, and `context/glossary.md`; direct behavior claims about auth, trace, implemented-vs-placeholder status, help coverage, and current command availability. Out - focused `context/cli/` doc rewrites, plans/handovers/decisions, and non-context docs.
  - Done when: Root shared docs no longer omit `auth renew`, correctly represent `sce trace prompts <commit-sha>`, and keep current implemented-vs-placeholder wording aligned with the code-owned command surface.
  - Verification notes (commands or checks): Manual parity review against `cli/src/command_surface.rs`, `cli/src/cli_schema.rs`, `cli/src/app.rs`, `cli/src/services/auth_command.rs`, and `cli/src/services/trace.rs`.
  - Completed: 2026-03-31
  - Files changed: `context/overview.md`, `context/architecture.md`, `context/glossary.md`
  - Evidence: Manual parity review completed against `cli/src/command_surface.rs`, `cli/src/cli_schema.rs`, `cli/src/app.rs`, `cli/src/services/auth_command.rs`, and `cli/src/services/trace.rs`
  - Notes: Root durable docs now include `auth renew`, `sce trace prompts <commit-sha>`, and current command-surface wording.

- [x] T02: `Repair focused CLI context for auth, trace, and help behavior` (status:done)
  - Task ID: T02
  - Goal: Update focused CLI/domain context files so command-local behavior and service descriptions match the implemented auth, trace, and help surfaces.
  - Boundaries (in/out of scope): In - `context/cli/placeholder-foundation.md` and any directly related focused CLI context file that currently makes stale auth/trace/help claims. Out - root shared files already handled in T01, unrelated CLI domains with no observed drift, and non-context docs.
  - Done when: Focused CLI docs accurately document `auth login|renew|logout|status`, the implemented `trace prompts` read path, and relevant help/usage/service-contract behavior without stale omissions.
  - Verification notes (commands or checks): Manual parity review against `cli/src/cli_schema.rs`, `cli/src/command_surface.rs`, `cli/src/services/auth_command.rs`, `cli/src/services/trace.rs`, and any touched focused context file.
  - Completed: 2026-03-31
  - Files changed: `context/cli/placeholder-foundation.md`
  - Evidence: Manual parity review completed against `cli/src/cli_schema.rs`, `cli/src/command_surface.rs`, `cli/src/app.rs`, `cli/src/services/auth_command.rs`, and `cli/src/services/trace.rs`
  - Notes: Focused CLI docs now include `auth renew`, `sce trace prompts --help`, the current classified stderr/exit-code error model, and updated auth service-contract wording.

- [x] T03: `Repair durable context navigation and discovery references` (status:done)
  - Task ID: T03
  - Goal: Update durable navigation/discovery docs so future sessions can find the corrected current-state command documentation without stale capability summaries.
  - Boundaries (in/out of scope): In - `context/context-map.md` and any other in-scope durable discovery/reference file whose command summaries or links become stale after T01-T02. Out - active plan history, handovers, and decision-log rewrites.
  - Done when: The context map and related durable discovery references point to the corrected auth/trace/current-command docs and no longer preserve stale command-surface summaries.
  - Verification notes (commands or checks): Manual cross-reference review across `context/context-map.md`, touched durable docs, and the same code-truth sources used in T01-T02.
  - Completed: 2026-03-31
  - Files changed: `context/context-map.md`, `context/overview.md`
  - Evidence: Manual cross-reference review completed against `context/context-map.md`, `context/overview.md`, `context/cli/placeholder-foundation.md`, `cli/src/command_surface.rs`, `cli/src/cli_schema.rs`, `cli/src/app.rs`, `cli/src/services/auth_command.rs`, and `cli/src/services/trace.rs`
  - Notes: Durable navigation now points future sessions to the corrected command-surface/help docs for `auth renew` and `sce trace prompts <commit-sha>`.

- [ ] T04: `Run durable-doc validation and cleanup` (status:todo)
  - Task ID: T04
  - Goal: Complete a final in-scope parity sweep, confirm no temporary planning/audit scaffolding remains, and leave the plan ready for execution tracking.
  - Boundaries (in/out of scope): In - final review of all touched durable `context/` files, plan status updates, and cleanup of any temporary context-only notes created during execution. Out - new doc expansion beyond the scoped drift repair and any application-code changes.
  - Done when: The touched durable docs have been rechecked against code truth, no remaining in-scope mismatches are known, and no temporary repair scaffolding remains.
  - Verification notes (commands or checks): Manual parity review of all touched files against `cli/src/command_surface.rs`, `cli/src/cli_schema.rs`, `cli/src/app.rs`, `cli/src/services/auth_command.rs`, and `cli/src/services/trace.rs`; confirm current-state discoverability from `context/context-map.md`.

## Open questions

- None. The user confirmed that this pass should be plan-first, code-drift-focused, and limited to durable current-state docs rather than active plans/handovers.
