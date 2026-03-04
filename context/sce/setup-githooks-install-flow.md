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

If the hooks path is relative, it is resolved against the git toplevel.

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

When replacing an existing hook:

- setup creates a deterministic backup path via `next_backup_path(...)`
- writes canonical bytes to a unique staging file in the hooks directory
- enforces executable permissions on the staged payload
- swaps staging file into final hook path

If swap fails after backup:

- staging artifact is removed
- previous hook is restored from backup when the destination path is absent
- failure returns explicit rollback context in the error chain

## Verification coverage

`cli/src/services/setup.rs` includes T03-focused tests for:

- fresh install in default hooks directory
- rerun idempotency (`Skipped` outcomes)
- upgrade in custom `core.hooksPath` with backup retention
- injected swap failure with rollback and staging cleanup checks
