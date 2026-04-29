# SCE setup git-hooks install flow

## Scope

Task `sce-setup-githooks-any-repo` `T03` implements the required-hook installation orchestration for `sce setup --hooks` at the setup-service layer.

## Implemented setup-service surface

`cli/src/services/setup/mod.rs` now provides:

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

## Staged write and remove-and-replace behavior

When replacing an existing hook, setup always writes canonical bytes to a unique staging file in the hooks directory, enforces executable permissions on the staged payload, removes the existing hook directly, and swaps the staged content into the final hook path.

On swap failure, setup removes the staging artifact and returns deterministic recovery guidance (recover the hook from version control if needed). No backup artifacts are created and no backup-based rollback is attempted.

## Verification coverage

`cli/src/services/setup/mod.rs` includes T03-focused tests for:

- hook update in the default hooks directory with no backup artifact creation
- hook update in custom `core.hooksPath` with no backup artifact creation
- injected swap failure with staging cleanup and deterministic recovery guidance