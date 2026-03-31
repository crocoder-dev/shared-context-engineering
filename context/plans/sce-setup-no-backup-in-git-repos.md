# Plan: sce-setup-no-backup-in-git-repos

## Change summary

Update `sce setup` so that when the target repository is backed by git, setup does not create backup files for any setup-managed write flow. This applies to both config install targets and required git hook installation. If a write/swap fails in a git-backed repository, setup should not attempt backup-based rollback; instead it should fail with clear guidance that git-backed recovery is available.

## Success criteria

- Running `sce setup` in a git-backed repository does not create `.backup` artifacts for config install targets or required hook files.
- Running `sce setup --hooks` in a git-backed repository does not create hook backup artifacts.
- Failure paths in git-backed repositories do not attempt backup-based restore and instead surface deterministic user guidance that recovery should use git state.
- Existing non-git-backed setup flows keep their current backup-and-rollback behavior.
- Tests cover git-backed no-backup success paths and failure behavior for both config install and hook install flows.

## Constraints and non-goals

- In scope: setup-service behavior, setup CLI messaging, and tests for config install + hook install backup policy branching.
- In scope: treating git-backed repositories as the condition that disables backup creation and backup-based rollback.
- Out of scope: changing repository detection semantics beyond what setup already uses or can reuse from existing git truth resolution.
- Out of scope: changing which assets/hooks are installed.
- Out of scope: introducing a new recovery mechanism beyond deterministic failure messaging.
- Non-goal: preserving backup files in git-backed repos for convenience; the user explicitly wants git to be the recovery path.

## Task stack

- [x] T01: `Add git-backed backup policy decision for setup writes` (status:done)
  - Task ID: T01
  - Goal: Introduce a single setup-layer policy decision that determines whether backup creation/rollback is enabled based on whether the target repository is git-backed, and thread that decision into both config-install and hook-install write flows.
  - Boundaries (in/out of scope): In - policy detection, shared setup-service branching inputs, small supporting types/helpers. Out - changing write messaging, tests for user-facing failure wording, or unrelated setup refactors.
  - Done when: Setup service can deterministically distinguish git-backed vs non-git-backed target repos for backup policy purposes, and both install paths consume the same policy input rather than duplicating branching logic.
  - Verification notes (commands or checks): Targeted Rust unit tests for policy detection/branching inputs; existing setup parser/service tests still compile and pass.

- [x] T02: `Disable config-install backups in git-backed repos` (status:done)
  - Task ID: T02
  - Goal: Update config install backup-and-replace behavior so git-backed repositories skip backup creation and skip backup-based rollback, failing with deterministic git-recovery guidance when swap/write operations fail.
  - Boundaries (in/out of scope): In - `.opencode`/`.claude/` install flow behavior, failure-path messaging, config-install tests. Out - git hook installer behavior, doctor flows, or non-git-backed behavior changes.
  - Done when: Config install creates no backup artifacts in git-backed repos, does not attempt restore from backup on failure, and emits stable failure guidance; non-git-backed behavior remains unchanged.
  - Verification notes (commands or checks): Setup-service unit tests covering git-backed success path with no backup artifacts plus injected failure path asserting no backup restore attempt and expected guidance.

- [x] T03: `Disable hook-install backups in git-backed repos` (status:done)
  - Task ID: T03
  - Goal: Update required git hook installation so git-backed repositories skip hook backup creation and skip backup-based rollback, failing with deterministic git-recovery guidance when hook swap/write operations fail.
  - Boundaries (in/out of scope): In - `install_required_git_hooks` behavior, per-hook outcome/backing metadata as needed, hook-install tests. Out - config install flow, hook asset contents, or hook target resolution semantics.
  - Done when: Hook installation creates no backup artifacts in git-backed repos, does not restore from backup on failure, and emits stable guidance; existing non-git-backed backup behavior remains intact where applicable.
  - Verification notes (commands or checks): Setup-service tests for default/custom hooks-path git-backed installs with no backup artifacts and injected failure coverage for no-rollback guidance.
  - Evidence captured: `nix develop -c sh -c 'cd cli && cargo test setup -- --nocapture'`; `nix develop -c sh -c 'cd cli && cargo fmt --check'`; `nix develop -c sh -c 'cd cli && cargo build'`

- [x] T04: `Update setup output contracts and current-state context` (status:done)
  - Task ID: T04
  - Goal: Align setup CLI/context wording with the new git-backed no-backup policy so operator-facing output and durable context describe the current behavior accurately.
  - Boundaries (in/out of scope): In - setup output text affected by backup reporting, relevant context files under `context/`. Out - unrelated overview/context churn and implementation beyond wording/contract alignment.
  - Done when: Any backup-status text that currently implies backup creation in git-backed repos is corrected, and relevant current-state context files document the git-backed no-backup policy and failure guidance.
  - Verification notes (commands or checks): Review rendered CLI/help/output assertions and context docs for consistent policy wording; ensure text fixtures/tests updated where exact output is asserted.
  - Evidence captured: `nix build .#default`; `nix run .#pkl-check-generated`; `nix flake check`

- [x] T05: `Run final validation and cleanup` (status:done)
  - Task ID: T05
  - Goal: Run the full validation pass for the completed change set, confirm no temporary scaffolding remains, and verify context stays aligned with final code truth.
  - Boundaries (in/out of scope): In - required repo validation, test cleanup, final context sync verification. Out - new behavior changes discovered during validation beyond minimal fixups required to make planned work pass.
  - Done when: Relevant tests/checks pass, no temporary debugging scaffolding remains, and context updates from earlier tasks still match the final implementation.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; `nix flake check`; confirm current-state context files and plan status reflect the finished work.
  - Evidence captured: `nix run .#pkl-check-generated`; `nix flake check`; removed ignored `context/tmp/sce.log`; verified setup context files still match final implementation; retained ignored `context/tmp/release-private-key.pem` by user choice.

## Validation Report

### Commands run
- `nix run .#pkl-check-generated` -> exit 0 (`Generated outputs are up to date.`)
- `nix flake check` -> exit 0 (all x86_64-linux flake checks evaluated and ran successfully; Nix reported other systems as omitted/incompatible for local execution)

### Cleanup
- Removed: `context/tmp/sce.log`
- Retained by user choice: `context/tmp/release-private-key.pem`

### Context verification
- Verified `context/overview.md`, `context/architecture.md`, `context/glossary.md`, `context/patterns.md`, and `context/context-map.md` against final code truth with no further drift requiring edits in this task.
- Verified setup domain files still reflect the final no-backup behavior and output wording:
  - `context/sce/setup-no-backup-policy-seam.md`
  - `context/sce/setup-githooks-install-flow.md`
  - `context/sce/setup-githooks-cli-ux.md`
  - `context/cli/placeholder-foundation.md`

### Success-criteria verification
- [x] `sce setup` in git-backed repositories creates no `.backup` artifacts for config install targets -> covered by earlier implemented tests and retained final context/code alignment
- [x] `sce setup --hooks` in git-backed repositories creates no hook backup artifacts -> covered by earlier implemented tests and retained final context/code alignment
- [x] Git-backed failure paths avoid backup-based restore and guide recovery through git state -> covered by earlier implemented tests and retained final context/code alignment
- [x] Non-git-backed flows retain backup-and-rollback behavior -> retained final context/code alignment
- [x] Tests cover git-backed config + hook success/failure behavior -> preserved by passing final `nix flake check`

### Residual risks
- One ignored local scratch file remains under `context/tmp/` (`release-private-key.pem`) by explicit user choice; it is outside tracked repo state but was not removed during final cleanup.

## Open questions

- None.
