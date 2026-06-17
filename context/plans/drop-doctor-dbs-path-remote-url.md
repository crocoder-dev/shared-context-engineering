# drop-doctor-dbs-path-remote-url

## Change summary

Remove the `path` and `remote_url` fields from the `DiscoveredCheckout` struct and the `sce doctor dbs` output (text and JSON). These fields are always hardcoded to `"unknown"` and `None` respectively — they carry zero runtime information for filesystem-discovered checkouts. The prior `remove-checkout-registry` plan already targeted dropping these fields from output; this plan finishes that cleanup by removing them from the struct and both renderers.

## Success criteria

- `sce doctor dbs` text output no longer renders `path: unknown` or `remote_url: none` lines.
- `sce doctor dbs` JSON output no longer includes `"path"` or `"remote_url"` keys.
- `nix flake check` passes (compile, clippy, fmt, tests, pkl-parity).

## Constraints and non-goals

- **In scope:** Remove `path` and `remote_url` from the `DiscoveredCheckout` struct, their hardcoded construction in `discover_checkouts_from_filesystem()`, and their rendering in `render_doctor_dbs_text()` and `render_doctor_dbs_json()`.
- **Out of scope:** The `remote_url` column in the `agent_traces` database table (unrelated — that's active storage). No changes to the main `sce doctor` (diagnose/fix) command. No new tests (no existing test coverage for this command either). No context-file updates needed (context was already refreshed during `remove-checkout-registry` to describe the output as `checkout_id`, `database_path`, `last_seen` only).

## Task stack

- [x] T01: `Remove path and remote_url from DiscoveredCheckout and doctor dbs output` (status:done)
  - **Files changed:** `cli/src/services/doctor/mod.rs`
  - **Evidence:** `nix flake check` passed (cli-tests, cli-clippy, cli-fmt, pkl-parity all green)
  - **Completed:** 2026-06-17
  - **Notes:** Six deletions: struct fields, construction, text renderer lines, JSON keys. `DiscoveredCheckout` now has only `checkout_id`, `database_path`, `last_seen`.
  - Task ID: T01
  - Goal: Remove the `path` and `remote_url` fields from the `DiscoveredCheckout` struct and strip them from both the text and JSON renderers for `sce doctor dbs`.
  - Boundaries (in/out of scope): In — `cli/src/services/doctor/mod.rs` only: the `DiscoveredCheckout` struct definition (lines 80–83), the hardcoded field assignments in `discover_checkouts_from_filesystem()` (lines 181–182), the `path` and `remote_url` lines in `render_doctor_dbs_text()` (lines 208, 211–214), and the `"path"` and `"remote_url"` entries in `render_doctor_dbs_json()` (lines 227, 230). Out — all other files, the main doctor command, database schema, tests.
  - Done when: `DiscoveredCheckout` has only `checkout_id`, `database_path`, `last_seen`; `render_doctor_dbs_text()` no longer emits `path:` or `remote_url:` lines; `render_doctor_dbs_json()` no longer includes `"path"` or `"remote_url"` keys; `nix flake check` passes on compile/clippy/fmt.
  - Verification notes (commands or checks): `nix flake check`; visual inspection of `sce doctor dbs` output shows no `path` or `remote_url` fields.

- [x] T02: `Validation and cleanup` (status:done)
  - **Files changed:** none (verification only)
  - **Evidence:** `nix flake check` passed all derivations; `sce doctor dbs` text output clean (no `path:`/`remote_url:` lines); `sce doctor dbs --format json` clean (no `"path"`/`"remote_url"` keys); grep for `path.*unknown\|remote_url.*none` in `cli/src/services/doctor/mod.rs` returned zero matches
  - **Completed:** 2026-06-17
  - **Notes:** Final validation gate. Plan complete.
  - Task ID: T02
  - Goal: Run full `nix flake check`, perform a smoke test of `sce doctor dbs`, and verify no residual references remain.
  - Boundaries (in/out of scope): In — `nix flake check`, manual smoke test of `sce doctor dbs` text and JSON output, grep for stale references. Out — new features, test additions, context-file changes.
  - Done when: `nix flake check` passes all derivations; `sce doctor dbs` and `sce doctor dbs --format json` produce correct output without `path`/`remote_url`; grep for `path.*unknown|remote_url.*none` in `cli/src/services/doctor/mod.rs` returns zero matches for the `dbs`-specific paths.
   - Verification notes (commands or checks): `nix flake check`; `nix run .#sce -- doctor dbs`; `nix run .#sce -- doctor dbs --format json`; `rg 'path.*unknown|remote_url.*none' cli/src/services/doctor/mod.rs`.

---

## Validation Report

### Commands run

| Command | Exit | Result |
|---|---|---|
| `nix flake check` | 0 | All derivations passed (cli-tests, cli-clippy, cli-fmt, pkl-parity, integrations, npm, config-lib) |
| `nix run .#pkl-check-generated` | 0 | Generated outputs are up to date |
| `nix run .#sce -- doctor dbs` | 0 | Output: `checkout_id`, `database_path`, `last_seen` only — no `path:` or `remote_url:` lines |
| `nix run .#sce -- doctor dbs --format json` | 0 | Output: `checkout_id`, `database_path`, `last_seen` only — no `"path"` or `"remote_url"` keys |
| `rg 'path.*unknown\|remote_url.*none' cli/src/services/doctor/mod.rs` | 1 (no matches) | Zero residual references |

### Success-criteria verification

- [x] `sce doctor dbs` text output no longer renders `path: unknown` or `remote_url: none` lines — confirmed via smoke test output
- [x] `sce doctor dbs` JSON output no longer includes `"path"` or `"remote_url"` keys — confirmed via JSON smoke test output
- [x] `nix flake check` passes (compile, clippy, fmt, tests, pkl-parity) — confirmed, all checks green

### Failed checks and follow-ups

None.

### Temporary scaffolding

None added or removed.

### Context sync

Verify-only — glossary tightened ("no longer stored" → "no longer rendered") for precision.

### Residual risks

None identified. The removed fields were hardcoded to `"unknown"`/`None` and had no downstream consumers.
