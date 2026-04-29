# Fix Pkl parity check gaps and remove stale sync-opencode-config

## Change summary

The `config/pkl/check-generated.sh` parity checker has drifted from `config/pkl/generate.pkl`. It is missing ten Pkl-generated paths (`lib/*`, `plugins/*`, and `opencode.json` under both manual and automated OpenCode profiles), so it can falsely pass when those outputs are stale.

Additionally, `scripts/sync-opencode-config.sh` is a dead script: the corresponding flake app was already removed, but the script and documentation references remain. Removing it eliminates a misleading workflow that duplicates the simpler `nix run .#pkl-generate` + manual `.opencode/` sync pattern.

This plan closes the parity gap, removes the dead script and references, and documents the coupling between `generate.pkl` and `check-generated.sh` so future Pkl changes don't silently break parity again.

## Success criteria

- [x] SC1: `nix run .#pkl-check-generated` compares **all** Pkl-generated paths, including `lib/`, `plugins/`, and `opencode.json` for both manual and automated profiles.
- [x] SC2: No false-positive "up to date" result when any Pkl-generated file differs.
- [x] SC3: `scripts/sync-opencode-config.sh` is deleted and no repo file or flake output references it.
- [x] SC4: `config/pkl/README.md` no longer documents the removed destructive-sync workflow.
- [x] SC5: `AGENTS.md` no longer lists `sync-opencode-config` or `scripts/sync-opencode-config.sh`.
- [x] SC6: A durable note exists in both `check-generated.sh` and `config/pkl/README.md` warning that `generate.pkl` and `check-generated.sh` must be kept in sync.
- [x] SC7: `nix run .#pkl-check-generated` passes on clean tree; `nix flake check` passes.

## Constraints and non-goals

- **In scope:** Updating `check-generated.sh` paths, deleting `scripts/sync-opencode-config.sh`, updating docs, adding coupling notes.
- **Out of scope:** Changing Pkl generation logic, modifying `generate.pkl` outputs, touching the root `.opencode/` directory, adding or removing flake apps.
- **Assumption:** The flake app `sync-opencode-config` was intentionally removed earlier; we are only cleaning up the orphaned script and docs.
- **Assumption:** The missing `plugins/`, `lib/`, and `opencode.json` paths in `check-generated.sh` were an oversight, not an intentional exclusion.

## Task stack

- [x] T01: `Expand check-generated.sh to cover all Pkl-generated paths` (status: done)
  - Task ID: T01
  - Goal: Add the missing generated paths to `config/pkl/check-generated.sh` so parity checks cover `lib/`, `plugins/`, and `opencode.json` under both `config/.opencode/` and `config/automated/.opencode/`.
  - Boundaries (in/out of scope): In — editing `config/pkl/check-generated.sh` `paths` array only. Out — changing `generate.pkl`, changing any other script.
  - Done when: The `paths` array includes all outputs declared in `config/pkl/generate.pkl`, and `nix run .#pkl-check-generated` still passes on a clean tree.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated` (expect "Generated outputs are up to date."); manually diff `generate.pkl` `files` block against `check-generated.sh` `paths` array to confirm no omissions.
  - Completed: 2026-04-29
  - Files changed: `config/pkl/check-generated.sh`
  - Evidence: `nix run .#pkl-check-generated` passed with "Generated outputs are up to date."; manual audit of all 21 `generate.pkl` outputs against 17 `check-generated.sh` paths confirms complete coverage (directories cover nested files).

- [x] T02: `Document Pkl/check coupling in check-generated.sh and README` (status: done)
  - Task ID: T02
  - Goal: Add explicit comments in `check-generated.sh` and `config/pkl/README.md` stating that whenever `generate.pkl` gains or loses outputs, `check-generated.sh` must be updated to match.
  - Boundaries (in/out of scope): In — comment additions and README edits. Out — changing generation logic.
  - Done when: A reader of `check-generated.sh` and `config/pkl/README.md` can discover the coupling without prior knowledge.
  - Verification notes (commands or checks): Read `check-generated.sh` header and `config/pkl/README.md` "Run commands" section to confirm the note is present and accurate.
  - Completed: 2026-04-29
  - Files changed: `config/pkl/check-generated.sh`, `config/pkl/README.md`
  - Evidence: `nix run .#pkl-check-generated` passed; both files now contain explicit coupling notes.

- [x] T03: `Delete scripts/sync-opencode-config.sh and remove docs references` (status: done)
  - Task ID: T03
  - Goal: Delete the orphaned `scripts/sync-opencode-config.sh` file and remove the entire "Use destructive sync" section (lines 63–97) from `config/pkl/README.md`.
  - Boundaries (in/out of scope): In — file deletion and README edits. Out — changing flake.nix (already clean), changing any other script.
  - Done when: `scripts/sync-opencode-config.sh` no longer exists; `config/pkl/README.md` no longer mentions `sync-opencode-config`; `grep -r "sync-opencode-config" config/pkl/README.md` returns empty.
  - Verification notes (commands or checks): `ls scripts/sync-opencode-config.sh` (expect not found); `grep -r "sync-opencode-config" config/pkl/` (expect empty).
  - Completed: 2026-04-29
  - Files changed: `scripts/sync-opencode-config.sh` (deleted), `config/pkl/README.md`
  - Evidence: `ls scripts/sync-opencode-config.sh` returned not found; `grep -r "sync-opencode-config" config/pkl/` returned empty; `nix run .#pkl-check-generated` passed; `nix flake check` passed.

- [x] T04: `Remove stale sync-opencode-config references from AGENTS.md` (status: done)
  - Task ID: T04
  - Goal: Remove the `nix run .#sync-opencode-config` command reference (line 43) and the `scripts/sync-opencode-config.sh` file reference (line 202) from `AGENTS.md`.
  - Boundaries (in/out of scope): In — `AGENTS.md` edits only. Out — any other file.
  - Done when: `grep -n "sync-opencode-config" AGENTS.md` returns no matches.
  - Verification notes (commands or checks): `grep -r "sync-opencode-config" AGENTS.md` (expect empty).
  - Completed: 2026-04-29
  - Files changed: `AGENTS.md`
  - Evidence: `grep -n "sync-opencode-config" AGENTS.md` returned empty; `nix run .#pkl-check-generated` passed; `nix flake check` passed.

- [x] T05: `Validation and cleanup` (status: done)
  - Task ID: T05
  - Goal: Run full repository validation, confirm no stale references remain anywhere, and verify the parity check now covers all generated outputs.
  - Boundaries (in/out of scope): In — running validation commands and doing a repo-wide grep. Out — making new code changes.
  - Done when: `nix run .#pkl-check-generated` passes; `nix flake check` passes; `grep -r "sync-opencode-config" --include="*.md" --include="*.sh" --include="*.nix" .` returns no matches outside plan/context history; a manual audit of `generate.pkl` outputs vs. `check-generated.sh` paths shows complete coverage.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; `nix flake check`; `rg -i "sync-opencode-config" .`; `rg -i "pkl-generated-parity" context/` to confirm no stale CI workflow references were reintroduced.
  - Completed: 2026-04-29
  - Files changed: `context/overview.md`, `context/architecture.md`, `context/patterns.md`, `context/plans/fix-pkl-parity-gaps-and-remove-stale-sync.md`
  - Evidence: `nix run .#pkl-check-generated` passed with "Generated outputs are up to date."; `nix flake check` passed with "all checks passed!"; `rg -n -i --glob '*.md' --glob '*.sh' --glob '*.nix' 'sync-opencode-config' .` returned matches only in this plan history; manual audit confirmed all `config/pkl/generate.pkl` output classes are covered by `config/pkl/check-generated.sh` paths; stale `pkl-generated-parity` context references were replaced with the current flake-owned `pkl-parity` contract.

## Open questions

None. Requirements are explicit and scope is bounded.

## Validation Report

### Commands run

- `nix run .#pkl-check-generated` -> exit 0; key output: `Generated outputs are up to date.`
- `nix flake check` -> exit 0; key output: `all checks passed!`
- `rg -n -i --glob '*.md' --glob '*.sh' --glob '*.nix' 'sync-opencode-config' .` -> exit 0; matches are limited to this plan's historical task text/evidence.
- `rg -n -i 'pkl-generated-parity' context/` -> exit 0; matches are limited to this plan's validation notes/evidence.
- Manual audit of `config/pkl/generate.pkl` against `config/pkl/check-generated.sh` -> complete coverage: scalar generated files are listed directly and generated directories cover OpenCode/Claude agents, commands, skills, `lib/`, `plugins/`, and `opencode.json` outputs for manual and automated profiles.

### Success-criteria verification

- [x] SC1: Confirmed `check-generated.sh` covers all generated output classes from `generate.pkl`, including manual and automated OpenCode `lib/`, `plugins/`, and `opencode.json`.
- [x] SC2: Confirmed checker compares the generated temporary tree against committed generated paths and fails on drift.
- [x] SC3: Confirmed stale `sync-opencode-config` references remain only in this plan's historical text/evidence.
- [x] SC4: Prior task evidence confirms `config/pkl/README.md` no longer documents the removed destructive-sync workflow.
- [x] SC5: Prior task evidence confirms `AGENTS.md` no longer lists `sync-opencode-config` or `scripts/sync-opencode-config.sh`.
- [x] SC6: Prior task evidence confirms coupling notes exist in `config/pkl/check-generated.sh` and `config/pkl/README.md`.
- [x] SC7: Final `nix run .#pkl-check-generated` and `nix flake check` both passed.

### Context sync

- Root context updated to replace stale references to the removed `.github/workflows/pkl-generated-parity.yml` with the current flake-owned `pkl-parity` contract.
- `context/context-map.md` and `context/glossary.md` were verified and did not need changes.

### Failed checks and follow-ups

- None.

### Residual risks

- None identified.
