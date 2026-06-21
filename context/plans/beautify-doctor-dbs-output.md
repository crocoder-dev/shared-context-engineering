# Beautify `sce doctor dbs` text output

## Change summary

Restyle the human text output for `sce doctor dbs` so it aligns with the CLI's other human-facing text surfaces, especially `sce doctor` and shared styled command output. Keep checkout discovery and JSON output behavior stable.

## Success criteria

- `sce doctor dbs --format text` renders a polished, deterministic human layout instead of raw snake_case field lines.
- The text output uses existing shared CLI styling/table helpers where appropriate and remains plain, stable text when color is disabled or stdout is non-TTY.
- Empty-state text is accurate for the filesystem-discovery implementation (`no discovered checkouts` or equivalent current terminology), not stale registry wording.
- Discovered checkout rows remain sorted by `last_seen` descending with checkout ID tie-break behavior unchanged.
- `sce doctor dbs --format json` keeps its existing field names and semantics unless a test-only formatting adjustment is required.
- Existing `sce doctor` diagnose/fix text and JSON output contracts are unchanged.

## Constraints and non-goals

- Do not change checkout discovery, state-root resolution, DB health checks, migrations, or Agent Trace persistence.
- Do not add a checkout registry or restore removed `path`/`remote_url` fields.
- Do not change `sce doctor --fix` behavior.
- Do not introduce new runtime dependencies; prefer the existing `style` service and `comfy-table` wrapper.
- Preserve deterministic output ordering for tests and automation.

## Task stack

- [x] T01: `Restyle doctor dbs text renderer` (status:done)
  - Task ID: T01
  - Goal: Update the `sce doctor dbs` text renderer and focused tests so the checkout listing matches the CLI's established human-output style while preserving data semantics.
  - Boundaries (in/out of scope): In - `cli/src/services/doctor/` text rendering for `DoctorAction::Dbs`, empty-state wording, focused render/output tests, and any localized current-state context updates needed for the new text contract. Out - JSON schema/field changes, checkout discovery logic, DB scanning behavior, doctor diagnose/fix rendering, setup/hook behavior, and new dependencies.
  - Done when: Text output has a polished header plus readable checkout listing using existing style/table primitives or equivalent shared helpers; empty state no longer references removed registry terminology; tests cover non-empty and empty text output; JSON output tests still pass unchanged.
  - Verification notes (commands or checks): Run the narrow Rust test(s) for doctor DB rendering if available through `nix develop -c sh -c 'cd cli && cargo test doctor'`; otherwise run `nix flake check` as the repo-level verification. Manually inspect `nix run .#sce -- doctor dbs --format text` output when local state allows.
  - Completed: 2026-06-21
  - Files changed: `cli/src/services/doctor/mod.rs`
  - Evidence: `nix develop -c sh -c 'cd cli && cargo fmt'`; targeted `cargo test doctor_dbs` was blocked by repo bash policy preferring flake checks; `nix flake check` passed; `nix run .#sce -- doctor dbs --format text` and `nix run .#sce -- doctor dbs --format json` smoke checks succeeded.
  - Notes: Text renderer now uses a styled `SCE doctor dbs` header, a `Checkout databases` section, aligned checkout rows, summary count, and filesystem-discovery empty wording (`no discovered checkouts`). Per user feedback, generated unit tests were removed; JSON rendering code was left unchanged.

- [x] T02: `Validation and cleanup` (status:done)
  - Task ID: T02
  - Goal: Validate the completed output polish end-to-end and ensure planning/context artifacts accurately reflect the final current state.
  - Boundaries (in/out of scope): In - full repo validation, generated-output parity check, cleanup of temporary debug artifacts, plan status/evidence updates, and context-sync verification for affected CLI output docs. Out - additional output redesign beyond T01, unrelated command text changes, and release/version updates.
  - Done when: Full validation passes or failures are documented with actionable follow-up; no temporary scaffolding remains; context files mention the final `sce doctor dbs` text contract accurately if the implementation changed durable behavior; plan task statuses/evidence are updated.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; `nix flake check`; optionally `nix run .#sce -- doctor dbs --format text` and `nix run .#sce -- doctor dbs --format json` for smoke coverage.
  - Completed: 2026-06-21
  - Files changed: `context/plans/beautify-doctor-dbs-output.md`
  - Evidence: `nix run .#pkl-check-generated` passed (`Generated outputs are up to date.`); `nix flake check` passed (`all checks passed!`); `nix run .#sce -- doctor dbs --format text` smoke check rendered the `SCE doctor dbs` header, `Checkout databases` section, aligned checkout row, and summary; `nix run .#sce -- doctor dbs --format json` smoke check preserved `checkout_id`, `database_path`, and `last_seen` fields.
  - Notes: No tracked temporary debug scaffolding was present in the T02 worktree. Durable CLI output context already reflects the final text contract in `context/cli/cli-command-surface.md`; T02 is a verify-only context-sync pass after the existing localized context update.

## Validation Report

### Commands run

- `nix run .#pkl-check-generated` -> exit 0; key output: `Generated outputs are up to date.`
- `nix flake check` -> exit 0; key output: `all checks passed!`
- `nix run .#sce -- doctor dbs --format text` -> exit 0; smoke output showed `SCE doctor dbs`, `Checkout databases`, aligned checkout columns, and `Summary: 1 discovered checkout(s)` for local state.
- `nix run .#sce -- doctor dbs --format json` -> exit 0; smoke output preserved JSON fields `checkout_id`, `database_path`, and `last_seen`.

### Success-criteria verification

- [x] Text output renders a polished deterministic layout instead of raw snake_case field lines -> confirmed by text smoke output and `cli/src/services/doctor/mod.rs` renderer.
- [x] Shared styling remains plain/stable for non-TTY output -> confirmed by smoke output from non-TTY command capture and shared `supports_color()` policy use.
- [x] Empty-state terminology matches filesystem discovery -> confirmed by renderer string `no discovered checkouts`.
- [x] Checkout rows remain sorted by `last_seen` descending with checkout ID tie-break unchanged -> confirmed by unchanged `sort_checkouts_by_last_seen_desc` comparator.
- [x] JSON field names and semantics remain stable -> confirmed by JSON smoke output and unchanged JSON renderer fields.
- [x] Existing `sce doctor` diagnose/fix contracts remain covered -> confirmed by successful `nix flake check`.

### Failed checks and follow-ups

- None.

### Residual risks

- None identified.

## Open questions

None.
