# Plan: Doctor OpenCode embedded asset presence

## Change summary
Extend `sce doctor` to verify that repo-local `.opencode/{agent,command,skills}` contains all embedded OpenCode assets expected by the CLI (presence only, no content validation). Missing embedded files must raise a manual-only `repo_assets` error and surface in the OpenCode section detail lines; extra files remain allowed.

## Success criteria
- `sce doctor` checks `.opencode/{agent,command,skills}` for all embedded assets expected by the CLI (presence only).
- Any missing embedded asset produces a manual-only `repo_assets` error and causes readiness `not_ready`.
- Text output OpenCode sections list missing embedded asset paths under the relevant section (agent/command/skills).
- JSON output includes the missing embedded asset issues under the corresponding OpenCode section entries.
- Extra files under `.opencode/{agent,command,skills}` do **not** trigger errors.
- Scope remains repo `.opencode` only (no `config/.opencode` or automated profile checks).

## Constraints and non-goals
- Do not validate file contents; only check presence.
- Do not change setup/install behavior or add auto-fix.
- Do not alter OpenCode plugin/runtime/preset checks outside the new presence check.
- Keep existing doctor output fields intact; add only new issues where applicable.

## Task stack
- [x] T01: Add embedded-asset presence checks for agent/command/skills (status:done)
  - Completed: 2026-03-31
  - Files changed: `cli/src/services/doctor.rs`
  - Evidence: `nix run .#pkl-check-generated`; `nix flake check`
  - Notes: Added embedded OpenCode asset presence checks and coverage for missing assets.
  - Task ID: T01
  - Goal: Compute expected embedded asset paths for `agent`, `command`, and `skills`, then verify they exist under repo `.opencode/`.
  - Boundaries (in/out of scope):
    - In: doctor OpenCode health collection, mapping expected embedded asset list to repo paths, issue + problem generation.
    - Out: content validation, setup/install flows, non-repo `.opencode` scopes.
  - Done when:
    - Missing embedded assets create `repo_assets` manual-only problems and OpenCode section issues for the correct section.
    - Extra files under `.opencode/{agent,command,skills}` are ignored.
  - Verification notes (commands or checks): Add/adjust unit tests to assert missing embedded asset detection and problem severity.

- [x] T02: Surface missing embedded assets in text output and JSON (status:done)
  - Completed: 2026-03-31
  - Files changed: `cli/src/services/doctor.rs`
  - Evidence: `nix run .#pkl-check-generated`; `nix flake check`
  - Notes: Added text-output coverage that asserts missing embedded assets surface under the correct OpenCode section.
  - Task ID: T02
  - Goal: Ensure missing embedded asset issues appear under the correct OpenCode section in text output and JSON.
  - Boundaries (in/out of scope):
    - In: doctor output tests for text + JSON; use existing OpenCode issue rendering.
    - Out: output format redesign.
  - Done when:
    - Text output lists missing asset paths under the matching OpenCode section detail lines.
    - JSON output includes missing asset issues under the corresponding section entry.
  - Verification notes (commands or checks): Run targeted doctor tests covering text + JSON outputs.

- [x] T03: Update doctor contract context (status:done)
  - Completed: 2026-03-31
  - Files changed: `context/plans/doctor-opencode-embedded-asset-presence.md`
  - Evidence: Manual review (contract already captured).
  - Notes: Embedded asset presence checks already documented in `context/sce/agent-trace-hook-doctor.md`.
  - Task ID: T03
  - Goal: Document the new embedded-asset presence checks and manual-only error behavior.
  - Boundaries (in/out of scope):
    - In: `context/sce/agent-trace-hook-doctor.md` contract update.
    - Out: unrelated context edits.
  - Done when:
    - Contract states `.opencode/{agent,command,skills}` embedded asset presence checks and manual-only `repo_assets` error on missing files.
  - Verification notes (commands or checks): Manual review of contract update.

- [x] T04: Validation and cleanup (status:done)
  - Completed: 2026-03-31
  - Evidence: `nix run .#pkl-check-generated`; `nix flake check`
  - Notes: Required validation commands succeeded; context sync completed.
  - Task ID: T04
  - Goal: Run full validation and confirm context alignment.
  - Boundaries (in/out of scope):
    - In: repo validation and any required cleanup.
    - Out: additional feature changes.
  - Done when:
    - `nix run .#pkl-check-generated` and `nix flake check` succeed.
    - Plan tasks updated with completion status and context sync confirmed.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; `nix flake check`.

## Open questions
- None.
