# Setup no-backup policy seam

`cli/src/services/setup.rs` now resolves one shared internal backup-policy decision before setup-managed write flows run.

## Current state

- `resolve_setup_backup_policy(repository_root)` is the canonical setup-layer decision point.
- The runtime probe treats a repository as git-backed when `git rev-parse --show-toplevel` succeeds from the target repository root.
- The decision is represented as `SetupBackupPolicy` with two current variants:
  - `CreateAndRestoreBackups`
  - `GitBackedRepository`
- Both setup-managed write surfaces now receive that same policy input:
  - config install via `install_embedded_setup_assets_with_rename(...)`
  - required hook install via `install_required_git_hooks_in_resolved_repository(...)`

## Implemented behavior

- Config install now branches on `SetupBackupPolicy`.
- In non-git-backed repositories, `.opencode` / `.claude` installs keep the existing backup-and-restore flow.
- In git-backed repositories, config install removes the existing target, skips `.backup` creation, and does not attempt backup-based rollback if the staged swap fails.
- Git-backed config-install failures append deterministic recovery guidance: recover the setup target from git state instead of expecting an installer-created backup.
- Required hook install now also branches on `SetupBackupPolicy`.
- In non-git-backed repositories, required hook replacement keeps the existing backup-and-restore flow.
- In git-backed repositories, hook install removes the existing hook, skips `.backup` creation, and does not attempt backup-based rollback if the staged swap fails.
- Git-backed hook-install failures append deterministic recovery guidance: recover the hook from git state instead of expecting an installer-created backup.
- Success output now distinguishes `backup: not created (git-backed repository)` from ordinary no-backup cases so operator-facing setup text no longer implies that git-backed replacements and first-time installs share the same reason.

## Scope boundary

- This file captures the shared setup-layer backup-policy seam and its current use by both config-install and required-hook install flows.
- Future setup-managed write flows should continue to branch from the same shared seam instead of re-detecting git-backed state independently.

See also: [../overview.md](../overview.md), [../context-map.md](../context-map.md), [setup-githooks-install-flow.md](setup-githooks-install-flow.md)
