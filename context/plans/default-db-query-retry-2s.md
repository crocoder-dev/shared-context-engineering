# Plan: Default DB Query Retry 2s Failure Budget

## Change summary

Ensure the hardcoded fallback database **query** retry policy fails within a maximum of 2 seconds from the first DB operation attempt, including all per-attempt timeouts and retry backoff delays. This applies to the default `QUERY_RETRY_POLICY` in `cli/src/services/db/mod.rs`, which is inherited by local DB, auth DB, and Agent Trace DB operation methods when no `policies.database_retry.<db>.query` override is configured.

## Success criteria

- Default DB query retry behavior has a code-level regression check proving the fallback worst-case wall-clock budget is `<= 2_000ms` from the first attempt through terminal failure.
- The default query retry policy remains configurable through existing `policies.database_retry.<db>.query` overrides; explicit user config may choose a longer budget.
- Connection-open retry defaults are unchanged.
- Context documentation reflects the implemented default query retry values and the 2-second default failure-budget contract.
- Full repo validation passes.

## Constraints and non-goals

- **In scope**: Hardcoded fallback query retry policy in `cli/src/services/db/mod.rs`, focused unit/regression coverage for the fallback budget, and context documentation updates.
- **Out of scope**: `CONNECTION_OPEN_RETRY_POLICY`, retry algorithm semantics in `cli/src/services/resilience.rs`, config schema shape, new dependencies, per-database differentiated hardcoded defaults, and changes to migration retry behavior.
- **Out of scope**: Enforcing a 2-second cap on explicit user-provided `policies.database_retry.<db>.query` config values.
- **Non-goal**: Remove retry support. The CLI should still tolerate short local DB lock contention.

## Assumptions

- “Request” means one synchronous DB operation call (`execute`, `query`, or the query/fetch portion of `query_map`) using the default query retry policy.
- The 2-second budget includes every attempted operation timeout plus all retry backoff sleeps before terminal failure.
- Code truth is authoritative: current context has minor drift around query retry defaults, so implementation should verify constants directly in `cli/src/services/db/mod.rs` and repair context to match.

## Task stack

- [x] T01: `Enforce default query retry failure budget in code` (status:done)
  - Task ID: T01
  - Goal: Make `cli/src/services/db/mod.rs` encode and test that default query retry fallback timing cannot exceed 2 seconds from first attempt through failure.
  - Boundaries (in/out of scope): In — `QUERY_RETRY_POLICY`, helper/test code needed to calculate the worst-case fallback duration, and any local test-only constants/helpers. Out — connection-open policy, config parsing/schema, `run_with_retry_sync` algorithm changes unless an existing calculation helper already requires a small extraction.
  - Done when: The default `QUERY_RETRY_POLICY` values produce a calculated worst-case duration `<= 2_000ms`; a focused regression test fails if future changes increase the fallback query budget above 2 seconds; existing DB operation retry behavior and config override plumbing remain intact.
  - Verification notes (commands or checks): Prefer `nix develop -c sh -c 'cd cli && cargo test db'` for targeted Rust coverage if available, then `nix flake check` before completion.
  - Completed: 2026-06-11
  - Files changed: `cli/src/services/db/mod.rs`
  - Evidence: `nix develop -c sh -c 'cd cli && cargo test db'` was blocked by repo bash policy `use-nix-flake-check-over-cargo-test`; `nix flake check` passed.
  - Notes: Added a focused regression test that computes fallback query retry worst-case failure budget from `QUERY_RETRY_POLICY` (`5 * 200ms` attempts plus `25ms + 50ms + 100ms + 100ms` retry backoffs = `1_275ms`) and asserts it remains `<= 2_000ms`. No connection-open retry, config override, or retry algorithm changes were made.

- [x] T02: `Sync context docs for query retry defaults` (status:done)
  - Task ID: T02
  - Goal: Update durable context to state the implemented default DB query retry values and the 2-second default failure-budget contract.
  - Boundaries (in/out of scope): In — `context/sce/shared-turso-db.md`, `context/glossary.md`, and any directly stale query-retry mentions in `context/overview.md` or `context/architecture.md`. Out — historical completed plan files unless they are being incorrectly treated as current-state documentation.
  - Done when: Current-state context consistently describes the default query retry policy, notes that explicit config overrides can exceed the default budget, and no current-state docs contain conflicting fallback query defaults.
  - Verification notes (commands or checks): Search context for `DB query retry policy`, `QUERY_RETRY_POLICY`, and specific fallback values to confirm only current-state values remain in durable docs.
  - Completed: 2026-06-11
  - Files changed: `context/overview.md`, `context/architecture.md`, `context/glossary.md`, `context/context-map.md`, `context/sce/shared-turso-db.md`
  - Evidence: `grep` checks for stale current-state fallback query defaults found no `3 attempts / 500ms` durable current-state references; remaining old-value matches are historical plan rationale in `context/plans/tighten-local-db-retry-defaults.md`.
  - Notes: User approved repairing the documentation drift during the T01 context-sync gate. Current-state context now describes fallback query retry defaults as `5` attempts, `200ms` timeout, `25ms..100ms` backoff, with a `<= 2_000ms` default failure-budget contract. Explicit `policies.database_retry.<db>.query` overrides remain configurable and may exceed that default budget.

- [x] T03: `Validate and clean up` (status:done)
  - Task ID: T03
  - Goal: Run final repo validation, confirm generated outputs are unchanged/in sync, and remove any temporary scaffolding.
  - Boundaries (in/out of scope): In — full validation, generated-output parity, stale-reference checks, and context plan status/evidence updates. Out — new behavior changes beyond T01/T02.
  - Done when: `nix flake check` and `nix run .#pkl-check-generated` pass; any temporary files are removed; plan evidence records the exact validation commands and outcomes.
  - Verification notes (commands or checks): `nix flake check`; `nix run .#pkl-check-generated`; targeted grep/search for stale query retry defaults in current-state context.
  - Completed: 2026-06-11
  - Files changed: `context/plans/default-db-query-retry-2s.md`
  - Evidence: `nix flake check` passed; `nix run .#pkl-check-generated` passed; targeted current-state context search for `500ms` found no stale current-state references, with old-value matches limited to historical plan files.
  - Notes: No task-created temporary scaffolding was found or removed. Full validation and generated-output parity are clean.

## Open questions

- None. User clarified that the 2-second budget applies only to query retry policy and includes the full wall-clock time from first request through terminal failure.

## Validation Report

### Commands run

- `nix flake check` -> exit 0; key output: `all checks passed!`
- `nix run .#pkl-check-generated` -> exit 0; key output: `Generated outputs are up to date.`
- Current-state context stale-reference check for `500ms` query retry defaults -> no stale current-state references; remaining matches are historical plan files only.

### Success-criteria verification

- [x] Default DB query retry behavior has a code-level regression check proving the fallback worst-case wall-clock budget is `<= 2_000ms` from first attempt through terminal failure -> confirmed in `cli/src/services/db/mod.rs` by `default_query_retry_policy_stays_within_two_second_failure_budget`.
- [x] Default query retry policy remains configurable through existing `policies.database_retry.<db>.query` overrides; explicit user config may choose a longer budget -> confirmed by unchanged config-driven query retry resolution and current context docs.
- [x] Connection-open retry defaults are unchanged -> confirmed in `cli/src/services/db/mod.rs` as `3` attempts, `1s` timeout, `25ms..200ms` backoff.
- [x] Context documentation reflects the implemented default query retry values and the 2-second default failure-budget contract -> confirmed in `context/overview.md`, `context/architecture.md`, `context/glossary.md`, `context/context-map.md`, and `context/sce/shared-turso-db.md`.
- [x] Full repo validation passes -> `nix flake check` passed.

### Failed checks and follow-ups

- None.

### Residual risks

- None identified.
