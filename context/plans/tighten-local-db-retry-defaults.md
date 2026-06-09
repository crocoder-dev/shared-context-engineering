# Plan: Tighten Local DB Retry/Backoff Defaults

## Change summary

Reduce hardcoded database retry/backoff defaults in `cli/src/services/db/mod.rs` to values appropriate for a local embedded Turso database with multiprocess WAL, where operations complete in microseconds rather than seconds. The current defaults (5s/3s timeouts, 100ms–1s backoff) are calibrated for remote/network databases and produce worst-case latencies of ~15s for connection open and ~9s for queries — unacceptable for a CLI tool operating on local files.

## Success criteria

- Hardcoded `CONNECTION_OPEN_RETRY_POLICY` and `QUERY_RETRY_POLICY` constants in `cli/src/services/db/mod.rs` are updated to the new tighter defaults.
- All existing tests pass (`nix flake check`).
- Context documentation reflects the new defaults.
- No behavioral change to the config-driven override mechanism — users who need longer timeouts can still set `policies.database_retry` in `sce/config.json`.

## Constraints and non-goals

- **In scope**: Update hardcoded default constants, update context docs, verify tests pass.
- **Out of scope**: Changing the `RetryPolicy` struct, the config override mechanism, the Pkl schema, or the resilience module logic. The config-driven override path remains the escape hatch for unusual environments.
- **Out of scope**: Changing `max_attempts` (3 is appropriate for local DBs).
- **Out of scope**: Adding new per-database differentiated defaults (all three DBs share the same hardcoded fallback today; this plan keeps that pattern).

## New default values

| Constant | Field | Current | New | Rationale |
|---|---|---|---|---|
| `CONNECTION_OPEN_RETRY_POLICY` | `timeout_ms` | 5,000 | 1,000 | Local file open completes in <10ms; 1s is generous |
| `CONNECTION_OPEN_RETRY_POLICY` | `initial_backoff_ms` | 100 | 25 | Local lock contention resolves in µs; 25ms is a brief pause |
| `CONNECTION_OPEN_RETRY_POLICY` | `max_backoff_ms` | 1,000 | 200 | Multiprocess WAL writer lock held for ms at most |
| `QUERY_RETRY_POLICY` | `timeout_ms` | 3,000 | 500 | Local queries complete in <1ms; 500ms is generous |
| `QUERY_RETRY_POLICY` | `initial_backoff_ms` | 100 | 25 | Same rationale as connection open |
| `QUERY_RETRY_POLICY` | `max_backoff_ms` | 500 | 100 | Same rationale as connection open |

New worst-case timings:
- Connection open: ~3.025s (down from ~15.1s)
- Query: ~1.525s (down from ~9.1s)

## Task stack

- [x] T01: `Update hardcoded retry policy constants to local-DB-appropriate defaults` (status:done)
  - Task ID: T01
  - Goal: Change the `CONNECTION_OPEN_RETRY_POLICY` and `QUERY_RETRY_POLICY` constant values in `cli/src/services/db/mod.rs` to the new tighter defaults.
  - Boundaries (in/out of scope): In — the two `const` blocks at lines 27–39. Out — `RetryPolicy` struct, config override resolution, Pkl schema, resilience module.
  - Done when: `CONNECTION_OPEN_RETRY_POLICY` has `timeout_ms: 1_000, initial_backoff_ms: 25, max_backoff_ms: 200` and `QUERY_RETRY_POLICY` has `timeout_ms: 500, initial_backoff_ms: 25, max_backoff_ms: 100`. `max_attempts` stays 3 for both.
  - Verification notes: `nix develop -c sh -c 'cd cli && cargo test'` or `nix flake check`.
  - **Status:** done
  - **Completed:** 2026-06-09
  - **Files changed:** `cli/src/services/db/mod.rs`
  - **Evidence:** `nix flake check` — all checks passed (cli-tests, cli-clippy, cli-fmt, pkl-parity, plus JS checks)
  - **Notes:** Six numeric literal changes across two const blocks; no structural changes; config-driven override path untouched

- [x] T02: `Update context documentation to reflect new retry defaults` (status:done)
  - Task ID: T02
  - Goal: Update all context files that reference the old hardcoded default values to reflect the new values.
  - Boundaries (in/out of scope): In — `context/glossary.md` (entries for `DB connection-open retry policy` and `DB query retry policy`), `context/sce/shared-turso-db.md` (default values in contract description), `context/overview.md` (if it mentions specific default values). Out — code changes, Pkl schema changes.
  - Done when: All context files that previously referenced `5s`/`3s`/`100ms`/`1000ms`/`500ms` defaults now reference `1s`/`500ms`/`25ms`/`200ms`/`100ms` as appropriate.
  - Verification notes: Grep context files for old values to confirm no stale references remain.
  - **Status:** done
  - **Completed:** 2026-06-09
  - **Files changed:** `context/glossary.md`, `context/sce/shared-turso-db.md`, `context/overview.md`, `context/architecture.md`
  - **Evidence:** `nix flake check` — all checks passed (cli-tests, cli-clippy, cli-fmt, pkl-parity, plus JS checks); grep confirms no stale old default references remain in context documentation files (only the plan file retains historical rationale)
  - **Notes:** Four context files updated with new default values; connection-open: `5s→1s`, `100ms→25ms`, `1000ms→200ms`; query: `3s→500ms`, `100ms→25ms`, `500ms→100ms`; no code changes

- [x] T03: `Validate and clean up` (status:done)
  - Task ID: T03
  - Goal: Run full validation suite and confirm no stale references to old defaults remain anywhere in the repo.
  - Boundaries (in/out of scope): In — `nix flake check`, `nix run .#pkl-check-generated`, grep for old default values in code and docs. Out — any code or doc changes beyond what T01 and T02 already made.
  - Done when: `nix flake check` passes, `nix run .#pkl-check-generated` passes, no stale references to old default values (`5000`/`1000`/`3000`/`500` as retry constants) remain in code or context files.
  - Verification notes: `nix flake check && nix run .#pkl-check-generated`; `grep -r '5_000\|5000\|3_000\|3000' cli/src/services/db/mod.rs` should show no matches for old timeout values; `grep -r '5s.*timeout\|3s.*timeout\|100ms.*backoff\|1000ms.*backoff\|500ms.*backoff' context/` should show no stale references.
  - **Status:** done
  - **Completed:** 2026-06-09
  - **Files changed:** None (validation-only task)
  - **Evidence:** `nix flake check` — all checks passed; `nix run .#pkl-check-generated` — generated outputs up to date; grep confirms no stale old default references in code (`5_000`/`3_000` absent from `db/mod.rs`) or context `.md` files (all reflect new values; only the plan file references old values as historical rationale)
  - **Notes:** Pure validation task — no code or doc changes needed; all three tasks in the plan are now complete

## Validation Report

### Commands run
- `nix flake check` → exit 0 (all checks passed: cli-tests, cli-clippy, cli-fmt, pkl-parity, integrations-install-tests/clippy/fmt, npm-bun-tests, npm-biome-check/format, config-lib-bun-tests, config-lib-biome-check/format)
- `nix run .#pkl-check-generated` → exit 0 (generated outputs up to date)
- `grep -n '5_000\|5000\|3_000\|3000' cli/src/services/db/mod.rs` → no matches (old timeout constants removed)
- `grep -rn '5s.*timeout\|3s.*timeout\|100ms.*backoff\|1000ms.*backoff\|500ms.*backoff' context/ --include='*.md'` → only matches in plan file (historical rationale) and new-value references (`500ms` timeout, `25ms..100ms` backoff); no stale old defaults found

### Success-criteria verification
- [x] Hardcoded `CONNECTION_OPEN_RETRY_POLICY` and `QUERY_RETRY_POLICY` constants updated → confirmed: `timeout_ms: 1_000, initial_backoff_ms: 25, max_backoff_ms: 200` and `timeout_ms: 500, initial_backoff_ms: 25, max_backoff_ms: 100`
- [x] All existing tests pass (`nix flake check`) → confirmed: all checks passed
- [x] Context documentation reflects the new defaults → confirmed: `context/glossary.md`, `context/overview.md`, `context/architecture.md`, `context/sce/shared-turso-db.md` all reflect new values
- [x] No behavioral change to config-driven override mechanism → confirmed: only const values changed; config resolution path untouched

### Temporary scaffolding
- None introduced by this plan.

### Residual risks
- None identified.