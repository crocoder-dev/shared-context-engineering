# SCE setup git-hooks install flow

## Scope

Task `sce-setup-githooks-any-repo` `T03` implements the required-hook installation orchestration for `sce setup --hooks` at the setup-service layer.

## Implemented setup-service surface

`cli/src/services/setup.rs` now provides:

- `install_required_git_hooks(repository_root: &Path)`
- `RequiredHooksInstallOutcome`
- `RequiredHookInstallResult`
- `RequiredHookInstallStatus` (`Installed`, `Updated`, `Skipped`)

This flow is independent from setup target install (`.opencode`/`.claude`) and is scoped to required git hooks.

## Path resolution and repository targeting

For the provided repository path, setup resolves git truth before any writes:

1. `git rev-parse --show-toplevel`
2. `git rev-parse --git-path hooks`

Before those git operations, setup canonicalizes/validates the user-provided repository path (`--repo`) as an existing directory.

If the hooks path is relative, it is resolved against the git toplevel.

Before staged hook writes, setup runs explicit directory write-permission probes for the resolved repository root and effective hooks directory to fail fast on non-writable targets.

This keeps behavior compatible with:

- default `.git/hooks`
- per-repo `core.hooksPath`
- global `core.hooksPath` (when git resolves it for the selected repo)

## Per-hook installation contract

The flow iterates canonical embedded required hooks (`pre-commit`, `commit-msg`, `post-commit`) and applies deterministic per-hook outcomes:

- `Installed`: hook was absent and is now present.
- `Updated`: hook existed but content and/or executable bit did not match canonical state.
- `Skipped`: hook already matched canonical bytes and executable state.

## Backup, staged write, and rollback behavior

When replacing an existing hook, setup always writes canonical bytes to a unique staging file in the hooks directory, enforces executable permissions on the staged payload, and then branches backup behavior through the shared `SetupBackupPolicy` seam.

### Non-git-backed repositories

- setup creates a deterministic backup path via `next_backup_path(...)`
- swaps the staged hook into the final hook path
- if swap fails after backup, setup removes the staging artifact, restores the previous hook from backup when the destination path is absent, and returns explicit rollback context in the error chain

### Git-backed repositories

- setup removes the existing hook directly instead of creating a `.backup` artifact
- swaps the staged hook into the final hook path without backup-based rollback
- if swap fails, setup removes the staging artifact and returns deterministic guidance to recover the hook from git state

## Verification coverage

`cli/src/services/setup.rs` includes T03-focused tests for:

- git-backed update in the default hooks directory with no backup artifact creation
- git-backed update in custom `core.hooksPath` with no backup artifact creation
- injected git-backed swap failure with staging cleanup, no rollback, and deterministic git-recovery guidance
- existing non-git-backed backup behavior remaining covered separately
