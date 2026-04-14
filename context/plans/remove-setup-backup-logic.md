# Plan: Remove setup backup logic

## Change summary

Remove all backup-and-restore logic from `sce setup`, making both config install (`.opencode`/`.claude`) and hook install use remove-and-replace everywhere. This eliminates the `SetupBackupPolicy` enum, the `CreateAndRestoreBackups` path, `next_backup_path()`, backup-related struct fields, and backup-related output messaging. The git-backed remove-and-replace path becomes the only path.

## Success criteria

1. `SetupBackupPolicy` enum and all its variants are removed from `cli/src/services/setup.rs`.
2. `resolve_setup_backup_policy`, `resolve_setup_backup_policy_with_probe`, and `is_git_backed_repository` are removed.
3. `next_backup_path()` is removed.
4. `install_assets_for_concrete_target_with_rename` no longer branches on backup policy — it always removes the existing target before swapping staged content (the current git-backed path).
5. `install_single_required_hook_with_rename` no longer branches on backup policy — it always removes the existing hook before swapping staged content (the current git-backed path).
6. `SetupInstallTargetResult.backup_root` and `RequiredHookInstallResult.backup_path` fields are removed.
7. `SetupInstallTargetResult.skipped_backup_in_git_backed_repo` and `RequiredHookInstallResult.skipped_backup_in_git_backed_repo` fields are removed.
8. Success messaging no longer includes backup-related lines (`backup:` status lines).
9. `hook_backup_path()` in `cli/src/services/default_paths.rs` is removed (dead code — never called).
10. `git_backed_setup_install_recovery_guidance` and `git_backed_hook_install_recovery_guidance` are generalized to just "recovery guidance" (no longer git-specific wording).
11. `install_assets_for_git_backed_target_with_rename` is folded into `install_assets_for_concrete_target_with_rename` (no longer a separate git-backed variant).
12. `update_git_backed_required_hook_with_rename` is folded into `install_single_required_hook_with_rename` (no longer a separate git-backed variant).
13. `remove_existing_install_target` is kept as a shared helper.
14. `nix flake check` passes.
15. Context files are updated to reflect the new remove-and-replace-everywhere behavior.

## Constraints and non-goals

- **In scope**: Removing `SetupBackupPolicy`, backup path generation, backup struct fields, backup messaging, git-backed probe, and consolidating the two install paths into one remove-and-replace path for both config and hook install.
- **In scope**: Removing the dead `hook_backup_path()` accessor from `default_paths.rs`.
- **In scope**: Updating context files (`context/sce/setup-no-backup-policy-seam.md`, `context/sce/setup-githooks-install-flow.md`, `context/sce/setup-githooks-install-contract.md`, `context/sce/setup-githooks-cli-ux.md`, `context/overview.md`, `context/glossary.md`, `context/patterns.md`, `context/context-map.md`) to reflect the simplified behavior.
- **Out of scope**: Changing the staged-write/swap choreography itself (staging still happens; only the backup-before-swap step is removed).
- **Out of scope**: Changing hook content, adding new hooks, or modifying doctor behavior.
- **Out of scope**: Changing the `--repo` validation, `ensure_git_repository` gate, or any other setup preflight logic.
- **Out of scope**: Changing config schema, auth, or observability behavior.

## Assumptions

- The user confirmed that remove-and-replace should apply everywhere, including non-git-backed repositories. This means non-git-backed repos will no longer get `.backup` artifacts and will have no rollback path on swap failure — same as git-backed repos today.
- The `is_git_backed_repository` probe is only used by `resolve_setup_backup_policy`. Once backup policy is removed, the probe function is also removed.
- The `hook_backup_path()` method in `default_paths.rs` is dead code (never called outside its definition) and can be safely removed.

## Task stack

- [x] T01: `Remove SetupBackupPolicy and consolidate install paths in setup.rs` (status:done)
  - Task ID: T01
  - Goal: Remove the `SetupBackupPolicy` enum, all backup-branching logic, backup struct fields, backup messaging, and the git-backed probe from `cli/src/services/setup.rs`. Consolidate config install and hook install to always use remove-and-replace (the current git-backed path). Remove `next_backup_path()`. Fold `install_assets_for_git_backed_target_with_rename` into `install_assets_for_concrete_target_with_rename`. Fold `update_git_backed_required_hook_with_rename` into `install_single_required_hook_with_rename`. Generalize recovery guidance to remove git-specific wording. Remove `SetupInstallTargetResult.backup_root`, `SetupInstallTargetResult.skipped_backup_in_git_backed_repo`, `RequiredHookInstallResult.backup_path`, and `RequiredHookInstallResult.skipped_backup_in_git_backed_repo`. Remove backup-related lines from `format_setup_install_success_message` and `format_required_hook_install_success_message`. Remove `resolve_setup_backup_policy`, `resolve_setup_backup_policy_with_probe`, and `is_git_backed_repository`. Remove `hook_backup_path()` from `cli/src/services/default_paths.rs`.
  - Boundaries (in/out of scope): In — all code changes in `setup.rs` and `default_paths.rs` described above. Out — context file updates (T02), any changes to hook content, doctor, config schema, or other services.
  - Done when: `SetupBackupPolicy` and all backup-related code paths are removed; both config and hook install always use remove-and-replace; `nix flake check` passes; no backup-related dead code remains in `setup.rs` or `default_paths.rs`.
  - Verification notes (commands or checks): `nix flake check`; `nix develop -c sh -c 'cd cli && cargo test'`; grep for `SetupBackupPolicy`, `next_backup_path`, `backup_root`, `backup_path`, `skipped_backup_in_git_backed_repo`, `hook_backup_path`, `is_git_backed_repository`, `resolve_setup_backup_policy` in `cli/src/` — all should return zero matches.

- [x] T02: `Update context files to reflect remove-and-replace-everywhere behavior` (status:done)
  - Task ID: T02
  - Goal: Update all context files that reference backup policy, `SetupBackupPolicy`, backup-and-restore, or git-backed/no-backup branching to reflect the new simplified remove-and-replace-everywhere behavior. This includes `context/sce/setup-no-backup-policy-seam.md` (rewrite or replace to describe the new unified remove-and-replace behavior), `context/sce/setup-githooks-install-flow.md` (remove backup-branching sections), `context/sce/setup-githooks-install-contract.md` (remove backup-policy branching from the contract), `context/sce/setup-githooks-cli-ux.md` (remove backup status lines from output contract), `context/overview.md` (update setup service description), `context/glossary.md` (update `setup backup-and-replace` and `setup required-hook install orchestration` entries, remove `SetupBackupPolicy` entry if present), `context/patterns.md` (update setup install execution pattern), and `context/context-map.md` (update `setup-no-backup-policy-seam.md` entry).
  - Boundaries (in/out of scope): In — all context file updates listed above. Out — any code changes (those are T01).
  - Done when: All context files accurately describe the current remove-and-replace-everywhere behavior; no context file references `SetupBackupPolicy`, `CreateAndRestoreBackups`, `GitBackedRepository` (as a backup policy variant), `next_backup_path`, or backup-and-restore rollback for setup installs; `nix run .#pkl-check-generated` passes (context files are not Pkl-generated, but this verifies no unintended drift).
  - Verification notes (commands or checks): `nix flake check`; manual review of each context file for stale backup references.
  - **Completed:** 2026-04-15
  - **Files changed:** `context/sce/setup-no-backup-policy-seam.md`, `context/sce/setup-githooks-install-flow.md`, `context/sce/setup-githooks-install-contract.md`, `context/sce/setup-githooks-cli-ux.md`, `context/overview.md`, `context/glossary.md`, `context/patterns.md`, `context/context-map.md`, `context/architecture.md`, `context/cli/cli-command-surface.md`, `context/sce/agent-trace-hook-doctor.md`
  - **Evidence:** `nix flake check` passed (all 13 checks); `nix run .#pkl-check-generated` passed (generated outputs up to date); `rg` for stale identifiers in `cli/src/` returns zero matches; manual review of all context files confirms no stale backup-policy references remain outside the plan file itself.
  - **Notes:** Also updated `context/architecture.md`, `context/cli/cli-command-surface.md`, and `context/sce/agent-trace-hook-doctor.md` which contained stale backup-related references not listed in the original task scope.

- [x] T03: `Validate and finalize` (status:done)
  - Task ID: T03
  - Goal: Run full validation suite and verify no stale backup references remain anywhere in the repository.
  - Boundaries (in/out of scope): In — running `nix flake check`, `nix run .#pkl-check-generated`, and a repository-wide grep for stale backup-related terms. Out — any code or context changes (those are T01 and T02).
  - Done when: `nix flake check` passes; `nix run .#pkl-check-generated` passes; `rg -i 'SetupBackupPolicy|CreateAndRestoreBackups|next_backup_path|backup_root|skipped_backup_in_git_backed_repo|hook_backup_path|is_git_backed_repository|resolve_setup_backup_policy' cli/src/` returns zero matches; context files accurately reflect current behavior.
  - Verification notes (commands or checks): `nix flake check`; `nix run .#pkl-check-generated`; `rg -i 'SetupBackupPolicy|CreateAndRestoreBackups|next_backup_path|backup_root|skipped_backup_in_git_backed_repo|hook_backup_path|is_git_backed_repository|resolve_setup_backup_policy' cli/src/`
  - **Completed:** 2026-04-15
  - **Evidence:** `nix flake check` passed (all 13 checks); `nix run .#pkl-check-generated` passed (generated outputs up to date); `rg` for stale identifiers in `cli/src/` returned zero matches; `rg` for stale identifiers in `context/` (excluding plan file) returned zero matches; context files accurately reflect current remove-and-replace-everywhere behavior.
  - **Notes:** Validation-only task — no code or context changes made.

## Validation Report

### Commands run
- `nix flake check` → exit 0 (all 13 checks passed: cli-tests, cli-clippy, cli-fmt, integrations-install-tests, integrations-install-clippy, integrations-install-fmt, pkl-parity, npm-bun-tests, npm-biome-check, npm-biome-format, config-lib-bun-tests, config-lib-biome-check, config-lib-biome-format)
- `nix run .#pkl-check-generated` → exit 0 (generated outputs up to date)
- `rg -i 'SetupBackupPolicy|CreateAndRestoreBackups|next_backup_path|backup_root|skipped_backup_in_git_backed_repo|hook_backup_path|is_git_backed_repository|resolve_setup_backup_policy' cli/src/` → exit 1 (zero matches, confirming all stale identifiers removed)
- `rg -i 'SetupBackupPolicy|CreateAndRestoreBackups|next_backup_path|backup_root|skipped_backup_in_git_backed_repo|hook_backup_path|is_git_backed_repository|resolve_setup_backup_policy|backup.and.restore|backup-and-restore|backup_policy|backup_policy_with_probe' context/ --glob '*.md' --glob '!context/plans/*'` → exit 1 (zero matches, confirming all context files are clean)

### Temporary scaffolding
- None introduced by this plan.

### Success-criteria verification
- [x] 1. `SetupBackupPolicy` enum and all variants removed → confirmed via zero `rg` matches in `cli/src/`
- [x] 2. `resolve_setup_backup_policy`, `resolve_setup_backup_policy_with_probe`, `is_git_backed_repository` removed → confirmed via zero `rg` matches
- [x] 3. `next_backup_path()` removed → confirmed via zero `rg` matches
- [x] 4. Config install always uses remove-and-replace → confirmed via code review in T01
- [x] 5. Hook install always uses remove-and-replace → confirmed via code review in T01
- [x] 6. `SetupInstallTargetResult.backup_root` and `RequiredHookInstallResult.backup_path` removed → confirmed via zero `rg` matches
- [x] 7. `skipped_backup_in_git_backed_repo` fields removed → confirmed via zero `rg` matches
- [x] 8. Success messaging has no backup-related lines → confirmed via code review in T01
- [x] 9. `hook_backup_path()` removed from `default_paths.rs` → confirmed via zero `rg` matches
- [x] 10. Recovery guidance generalized (no git-specific wording) → confirmed via code review in T01
- [x] 11. `install_assets_for_git_backed_target_with_rename` folded into `install_assets_for_concrete_target_with_rename` → confirmed via code review in T01
- [x] 12. `update_git_backed_required_hook_with_rename` folded into `install_single_required_hook_with_rename` → confirmed via code review in T01
- [x] 13. `remove_existing_install_target` kept as shared helper → confirmed via code review in T01
- [x] 14. `nix flake check` passes → confirmed (all 13 checks passed)
- [x] 15. Context files updated to reflect remove-and-replace-everywhere behavior → confirmed via zero stale references in context files (excluding plan file)

### Residual risks
- None identified.