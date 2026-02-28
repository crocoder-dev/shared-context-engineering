# Plan: remove-generated-file-markers

## 1) Change summary
Remove generated-file warning markers from Pkl-generated outputs so generated artifacts no longer include the HTML `GENERATED FILE` comment in Markdown files or the leading generated warning header in `lib/drift-collectors.js` outputs.

## 2) Success criteria
- Generated Markdown outputs under `config/.opencode/**` and `config/.claude/**` no longer contain the HTML generated-file warning comment.
- Generated `lib/drift-collectors.js` outputs in both target trees no longer contain a generated-file warning header.
- Generator logic remains deterministic and output path mapping remains unchanged.
- Repository checks that validate generation/parity still pass after regeneration.

## 3) Constraints and non-goals
- In scope: Pkl generation/rendering logic and generated artifacts related to marker insertion/removal.
- In scope: context updates needed to keep architecture/pattern docs aligned with current behavior.
- Out of scope: changing generated file ownership boundaries.
- Out of scope: introducing new warning/enforcement mechanisms.
- Non-goal: modifying application runtime code.

## 4) Task stack (T01..T04)
- [x] T01: Remove Markdown generated marker injection from renderer contract (status:done)
  - Task ID: T01
  - Goal: Stop renderer output from inserting HTML generated-file warning comments into generated Markdown artifacts.
  - Boundaries (in/out of scope):
    - In: marker constants/templates and Markdown assembly in Pkl renderer modules.
    - Out: target metadata/frontmatter schema behavior unrelated to marker insertion.
  - Done when:
    - No renderer path appends the generated HTML warning marker to Markdown outputs.
    - Renderer module evaluations still succeed.
  - Verification notes (commands or checks):
    - Evaluate affected renderer modules and inspect representative generated agent/command/skill outputs for marker absence.

- [x] T02: Remove generated warning header from shared drift-collector library generation source (status:done)
  - Task ID: T02
  - Goal: Remove generated warning header text from the canonical shared library source used to emit both target `lib/drift-collectors.js` files.
  - Boundaries (in/out of scope):
    - In: canonical generated library source content used by Pkl output mapping.
    - Out: functional collector logic and exported API behavior.
  - Done when:
    - Generated `config/.opencode/lib/drift-collectors.js` and `config/.claude/lib/drift-collectors.js` do not include generated warning header lines.
    - Library behavior content remains otherwise unchanged.
  - Verification notes (commands or checks):
    - Regenerate outputs and compare generated library files against prior behavior to confirm only header removal.
  - Evidence:
    - Ran `pkl eval -m . config/pkl/generate.pkl` (exit 0) to regenerate both generated `lib/drift-collectors.js` targets from the canonical shared source.
    - Verified diffs for `config/.opencode/lib/drift-collectors.js` and `config/.claude/lib/drift-collectors.js` show only removal of the two-line generated warning header and adjacent blank line.
    - Ran `nix develop -c ./config/pkl/check-generated.sh` (exit 0): `Generated outputs are up to date.`
    - Ran `nix flake check` (exit 0) as a light build/check; flake app and dev shell outputs evaluated successfully on host platform (with expected incompatible-system warnings).

- [x] T03: Regenerate outputs and sync context documentation references to marker behavior (status:done)
  - Task ID: T03
  - Goal: Regenerate generated-owned artifacts and update context/docs that currently state marker behavior so documentation matches code truth.
  - Boundaries (in/out of scope):
    - In: generated target files and context/docs describing marker strategy.
    - Out: unrelated architecture or workflow changes.
  - Done when:
    - All generated-owned outputs are refreshed without marker text.
    - `context/architecture.md`, `context/patterns.md`, and any contributor docs that mention marker insertion are updated to current state.
  - Verification notes (commands or checks):
    - Run generation command and inspect docs/context references for stale marker descriptions.
  - Evidence:
    - Ran `nix develop -c pkl eval -m . config/pkl/generate.pkl` (exit 0) to refresh all generated-owned outputs under `config/.opencode/**` and `config/.claude/**`.
    - Verified marker absence in generated outputs via searches returning no matches for `GENERATED FILE` in generated Markdown and no `^// GENERATED FILE` header in `config/.opencode/lib/drift-collectors.js` and `config/.claude/lib/drift-collectors.js`.
    - Updated current-state docs to match marker-free behavior in `context/architecture.md`, `context/patterns.md`, and contributor runbook `config/pkl/README.md`.
    - Ran `nix develop -c ./config/pkl/check-generated.sh` (exit 0): `Generated outputs are up to date.`
    - Ran `nix flake check` (exit 0) as a light build/check; flake app and dev shell outputs evaluated successfully on host platform (with expected incompatible-system warnings).

- [x] T04: Validation and cleanup (status:done)
  - Task ID: T04
  - Goal: Run full generation/parity validation, confirm deterministic clean state, and remove temporary artifacts.
  - Boundaries (in/out of scope):
    - In: planned checks, final consistency review, cleanup of temporary generated inspection artifacts.
    - Out: new feature work.
  - Done when:
    - Generation and parity checks pass after marker removal.
    - Updated context/docs and generated outputs are mutually consistent.
    - Temporary artifacts introduced during execution are cleaned.
  - Verification notes (commands or checks):
    - Run generator evaluation, metadata coverage check, stale-output/parity check, and repo validation checks defined by current workflow.
  - Evidence:
    - Ran `nix develop -c pkl eval config/pkl/generate.pkl` (exit 0) to validate the generator entrypoint evaluates cleanly in the dev shell.
    - Ran `nix develop -c pkl eval config/pkl/renderers/metadata-coverage-check.pkl` (exit 0) and confirmed metadata coverage output for agents, commands, and skills.
    - Ran `nix develop -c pkl eval -m . config/pkl/generate.pkl` (exit 0) to regenerate generated-owned outputs in place.
    - Ran `nix develop -c ./config/pkl/check-generated.sh` (exit 0): `Generated outputs are up to date.`
    - Ran `nix flake check` (exit 0) as the planned repo validation check; flake apps/dev shells evaluated successfully with expected incompatible-system warnings.
    - Cleaned temporary task artifacts by removing `context/tmp/t04-generated/`; `context/tmp/` now retains only prior task scratch content.

## 5) Open questions
- None. Scope confirmed: remove both marker types (Markdown HTML marker and JS warning header).
