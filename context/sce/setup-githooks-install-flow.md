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

The flow iterates canonical embedded required hooks (`pre-commit`, `commit-msg`, `post-commit`) and, for each, computes the bytes to stage with `cli/src/services/setup/hook_merge.rs::merge_or_create_hook(existing_bytes, canonical_bytes, hook_name)` rather than writing the canonical asset's bytes verbatim — the same content-computation-before-swap seam the two JSON merge targets use (see [setup-no-backup-policy-seam.md](setup-no-backup-policy-seam.md)). No hook existing yields the canonical template unchanged; a hook already carrying the current SCE managed block is left unchanged; a hook carrying a stale block gets the block spliced in place, its surrounding content untouched; a marker-free hook containing the legacy pre-marker guidance URL is recognized as SCE-owned wholesale and replaced entirely with canonical bytes; any other marker-free hook is foreign and is kept as an exact byte prefix with the canonical block appended after it. Deterministic per-hook outcomes are then reported against the merged result:

- `Installed`: hook was absent and is now present.
- `Updated`: hook existed but the merged bytes and/or executable bit did not match the file on disk (covers a foreign-hook append, a stale-block replacement, and a legacy-hook upgrade alike).
- `Skipped`: the file on disk already equals the merged bytes and is executable — including a foreign hook that already carries the current SCE block, not only a hook that is untouched foreign-content-free canonical output.

When a foreign hook is appended to and its last effective line is a zero-indent `exec`/`exit`, so the appended block would never run, `RequiredHookInstallResult.unreachable_block_advisory` is set and rendered as a named advisory line in the hook section of setup output; every other outcome leaves it `false`.

The canonical bytes merged into each hook include the shared non-blocking missing-CLI bootstrap: all three hooks warn to stderr and exit successfully when `sce` is unavailable, while an available `sce` receives unchanged hook arguments and its failures propagate. Only `post-commit` performs origin lookup and remote metadata forwarding; see [setup-githooks-hook-asset-packaging.md](setup-githooks-hook-asset-packaging.md) for the canonical payload contract.

## Staged write and atomic-swap behavior

When installing or replacing a hook, setup always writes the merged bytes to a unique staging file in the hooks directory and enforces executable permissions on the staged payload. It never unlinks an existing hook before swapping: the staged file is renamed directly over the final hook path, so `fs::rename` performs the replacement atomically and a hook is never briefly absent mid-install. This mirrors the config-asset swap in [setup-no-backup-policy-seam.md](setup-no-backup-policy-seam.md).

On swap failure, setup removes the staging artifact and returns deterministic recovery guidance (recover the hook from version control if needed) whenever a prior hook existed. No backup artifacts are created and no backup-based rollback is attempted; a rename failure leaves the pre-existing hook's bytes and executable bit untouched because the old file was never removed.

## Verification coverage

`cli/src/services/setup/mod.rs` includes tests for:

- hook update in the default hooks directory with no backup artifact creation
- hook update in custom `core.hooksPath` with no backup artifact creation
- injected swap failure with staging cleanup, deterministic recovery guidance, and the prior hook's bytes and executable bit surviving the failure
- a foreign hook's content surviving as an exact byte prefix with the SCE block appended and the file executable
- idempotent reruns reporting `Skipped` with unchanged bytes for both a block-only hook and a foreign-plus-block hook
- a legacy pre-marker SCE hook upgrading to the current canonical marker form
- a foreign hook ending in a zero-indent `exec` installing the block and reporting `unreachable_block_advisory`, with a sibling foreign hook ending in an ordinary command not reporting it

`cli/src/services/setup/hook_merge.rs` unit-tests the pure merge computation itself (filesystem-free); the tests above verify the install-time wiring around it.