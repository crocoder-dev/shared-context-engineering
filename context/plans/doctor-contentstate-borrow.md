# Plan: Doctor content state borrow fixes

## Change summary
- Verify the reported `cli/src/services/doctor.rs` usage of `child.content_state` and, if it still moves non-`Copy` enum variants, update match/matches sites to borrow the field (`&child.content_state`) so `IntegrationContentState::ReadFailed` does not move its inner `String`.

## Success criteria
- All `child.content_state` comparisons in the specified doctor integration block use borrows (`&child.content_state`) instead of moves.
- `matches!` and `match` sites no longer move `IntegrationContentState::ReadFailed` data.
- No behavior changes outside the targeted `child.content_state` comparison logic.
- Changes are applied only if the current code still uses move-based pattern matching.

## Constraints and non-goals
- Do not alter integration logic, messaging, or output structure beyond borrow-vs-move fixes.
- Do not refactor unrelated doctor logic or enums.
- Keep changes localized to `cli/src/services/doctor.rs` where the integration content-state comparisons occur.

## Task stack
- [x] T01: Update doctor integration content-state matches to borrow (status:done)
  - Task ID: T01
  - Goal: Replace move-based `child.content_state` matches with borrow-based comparisons in the integration content-state logic.
  - Boundaries (in/out of scope): In scope: `cli/src/services/doctor.rs` matches/filters around the integration content-state comparisons (missing/ mismatch/ match checks). Out of scope: any changes to enum definitions or other services.
  - Done when: All `matches!` and `match` uses of `child.content_state` in the targeted section are updated to `&child.content_state`, and no move-based pattern matching remains there.
  - Verification notes (commands or checks): Inspect the updated `doctor.rs` block to confirm borrow usage; run `nix flake check` if executing the verification baseline.

- [x] T02: Validation and context sync (status:done)
  - Task ID: T02
  - Goal: Run required checks and verify shared context remains accurate for this localized fix.
  - Boundaries (in/out of scope): In scope: `nix run .#pkl-check-generated`, `nix flake check`, verify-only context sync. Out of scope: additional refactors or unrelated documentation edits.
  - Done when: Validation commands are executed (or explicitly noted if skipped) and context files are confirmed up to date with no edits required.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; `nix flake check`.

## Open questions
- None.

## Task: T01 Update doctor integration content-state matches to borrow
- **Status:** done
- **Completed:** 2026-04-07
- **Files changed:** cli/src/services/doctor.rs
- **Evidence:** `nix run .#pkl-check-generated`; `nix flake check`
- **Notes:** Updated integration content-state comparisons to borrow `child.content_state` to avoid moving non-`Copy` variants.

## Task: T02 Validation and context sync
- **Status:** done
- **Completed:** 2026-04-07
- **Files changed:** None
- **Evidence:** `nix run .#pkl-check-generated`; `nix flake check`
- **Notes:** Verify-only context sync completed; root context files reviewed with no changes required.

## Validation Report

### Commands run
- `nix run .#pkl-check-generated` -> exit 0 (Generated outputs are up to date.)
- `nix flake check` -> exit 0 (all checks passed)

### Success-criteria verification
- [x] `child.content_state` comparisons use borrows in integration block -> `cli/src/services/doctor.rs` filters and matches now use `&child.content_state` (missing/mismatch filters, group status, child status).
- [x] No move-based `matches!`/`match` sites remain in targeted block -> verified in `cli/src/services/doctor.rs` around the integration group helpers.
- [x] No behavior changes beyond comparison semantics -> scope limited to borrow-vs-move updates.

### Failed checks and follow-ups
- None.

### Residual risks
- None identified.
