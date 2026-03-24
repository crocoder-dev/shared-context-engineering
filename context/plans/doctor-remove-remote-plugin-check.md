# Plan: Remove remote OpenCode plugin comparison from `sce doctor`

## Change summary
- Remove the `--opencode-plugin-remote` flag from the CLI surface.
- Remove the remote GitHub fetch/compare logic for `.opencode/plugins/*`.
- Remove remote-check tests and documentation references while keeping local embedded-asset drift checks intact.

## Success criteria
- `sce doctor --opencode-plugin-remote` is no longer available in CLI help/parse.
- `DoctorRequest` no longer includes a remote-plugin flag field.
- All remote-fetch/compare logic and tests are removed.
- Local embedded-asset drift check for `sce-bash-policy.ts` remains unchanged.
- Context docs no longer mention remote plugin comparison.
- `nix flake check` passes.

## Constraints and non-goals
- Do not change existing local plugin registry/file/drift checks.
- Do not add new remote checks or alternate flags.
- Do not modify generated assets or plugin content.

## Task stack
- [x] T01: Remove CLI flag and request plumbing (status:done)
  - Task ID: T01
  - Goal: Delete `--opencode-plugin-remote` from CLI schema and `DoctorRequest`.
  - Boundaries (in/out of scope): In - CLI schema, app dispatch, request structs, parser tests. Out - behavior changes beyond removing the flag.
  - Done when: Flag is removed from help/parse and request models compile without it.
  - Verification notes (commands or checks): `nix flake check`.

- [x] T02: Remove remote fetch/compare logic and tests (status:done)
  - Task ID: T02
  - Goal: Delete remote GitHub fetch/compare logic and associated unit tests.
  - Boundaries (in/out of scope): In - doctor service logic, tests for remote checks. Out - local plugin checks.
  - Done when: No remote comparison code remains; tests referencing remote checks are removed.
  - Verification notes (commands or checks): `nix flake check`.

- [x] T03: Update context docs (status:done)
  - Task ID: T03
  - Goal: Remove documentation references to remote plugin comparison.
  - Boundaries (in/out of scope): In - `context/sce/agent-trace-hook-doctor.md`, `context/cli/placeholder-foundation.md`. Out - unrelated doc edits.
  - Done when: Docs only describe local plugin checks; no remote flag references remain.
  - Verification notes (commands or checks): Manual review.

- [x] T04: Validation and cleanup (status:done)
  - Task ID: T04
  - Goal: Run required checks and update plan status.
  - Boundaries (in/out of scope): In - `nix flake check` (and `nix run .#pkl-check-generated` only if generated outputs changed). Out - extra tests not required.
  - Done when: Validation passes and plan status reflects completion.
  - Verification notes (commands or checks): `nix flake check`.

## Open questions
- None.

## Task log

### T01
- Status: done
- Completed: 2026-03-24
- Files changed: cli/src/cli_schema.rs, cli/src/app.rs, cli/src/services/doctor.rs
- Evidence: `nix flake check`
- Notes: Removed `--opencode-plugin-remote` flag, parser coverage, and request field plumbing.

### T02
- Status: done
- Completed: 2026-03-24
- Files changed: cli/src/services/doctor.rs
- Evidence: `nix flake check`
- Notes: Removed all remote-plugin tests and remaining `opencode_plugin_remote` references; local embedded-asset drift check unchanged.

### T03
- Status: done
- Completed: 2026-03-24
- Files changed: context/sce/agent-trace-hook-doctor.md, context/cli/placeholder-foundation.md
- Evidence: Manual review for alignment with current doctor behavior.
- Notes: Removed remote plugin comparison references from docs.

### T04
- Status: done
- Completed: 2026-03-24
- Files changed: context/plans/doctor-remove-remote-plugin-check.md
- Evidence: `nix flake check`
- Notes: No generated assets changed; skipped `nix run .#pkl-check-generated`.

## Validation Report

### Commands run
- `nix flake check` -> exit 0 (all checks passed; omitted incompatible systems: aarch64-darwin, aarch64-linux, x86_64-darwin)

### Success-criteria verification
- [x] `--opencode-plugin-remote` removed from CLI help/parse -> removed from `cli/src/cli_schema.rs` and parser tests.
- [x] `DoctorRequest` no longer includes remote flag -> field removed from `cli/src/services/doctor.rs` and app dispatch.
- [x] Remote fetch/compare logic and tests removed -> remote helper/test sections deleted from `cli/src/services/doctor.rs`.
- [x] Local embedded-asset drift check remains -> `inspect_opencode_plugin_health` still compares against embedded asset and emits warning.
- [x] Docs no longer mention remote comparison -> removed from `context/sce/agent-trace-hook-doctor.md` and `context/cli/placeholder-foundation.md`.
- [x] `nix flake check` passes -> exit 0 recorded above.

### Residual risks
- None identified.
