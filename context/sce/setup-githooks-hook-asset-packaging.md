# SCE setup git-hooks embedded asset packaging

## Scope

Task `sce-setup-githooks-any-repo` `T02` defines how required git-hook templates are packaged for `sce setup --hooks` without runtime reads from `config/`.

## Canonical embedded hook sources

`cli/build.rs` now embeds three canonical hook templates from `cli/assets/hooks/`:

- `pre-commit`
- `commit-msg`
- `post-commit`

These templates are emitted into `OUT_DIR/setup_embedded_assets.rs` as `HOOK_EMBEDDED_ASSETS` with deterministic sorted relative paths.

All three templates are POSIX `sh` scripts with `set -eu`. Before invoking `sce`, each checks `command -v sce`; when the CLI is unavailable, it prints branded, multiline installation guidance to stderr and exits successfully so Git operations are not blocked solely by a missing local CLI installation. ANSI styling is emitted only when stderr is a terminal; redirected output remains plain text. Failures from an available `sce` command continue to propagate through `exec`.

Available-CLI behavior remains hook-specific:

- `pre-commit` invokes `sce hooks pre-commit "$@"`.
- `commit-msg` invokes `sce hooks commit-msg "$@"`.
- `post-commit` resolves `origin` with `git remote get-url origin`; when the lookup returns a non-empty URL, it invokes `sce hooks post-commit --vcs git --remote-url "$remote_url" "$@"`, otherwise it invokes `sce hooks post-commit --vcs git "$@"`. Remote metadata forwarding is exclusive to `post-commit`.

## Setup-service accessor surface

`cli/src/services/setup/mod.rs` exposes hook-template access through:

- `iter_required_hook_assets()` for deterministic full-set iteration
- `get_required_hook_asset(RequiredHookAsset)` for stable per-hook lookup

`RequiredHookAsset` is the canonical hook mapping enum for this packaging layer:

- `PreCommit`
- `CommitMsg`
- `PostCommit`

## Determinism and validation

Generated-output parity and repository validation verify that the embedded asset manifest remains buildable and synchronized with its canonical source inputs. The hook scripts can also be checked directly with POSIX `sh -n`; behavioral test coverage for these shell assets is not currently retained in the Rust test suite.
