# SCE setup git-hooks embedded asset packaging

## Scope

Task `sce-setup-githooks-any-repo` `T02` defines how required git-hook templates are packaged for `sce setup --hooks` without runtime reads from `config/`.

## Canonical embedded hook sources

`cli/build.rs` now embeds three canonical hook templates from `cli/assets/hooks/`:

- `pre-commit`
- `commit-msg`
- `post-commit`

The build script copies these templates to `OUT_DIR/static/hooks/`, then emits them into `OUT_DIR/setup_embedded_assets.rs` as `HOOK_EMBEDDED_ASSETS` with deterministic sorted relative paths. Production Rust includes therefore resolve through `OUT_DIR` rather than back into `cli/assets/`.

All three templates are POSIX `sh` scripts with `set -eu`. The CLI-presence check and the `sce hooks` invocation are wrapped in an SCE managed block delimited by `# >>> sce managed block (do not edit) >>>` / `# <<< sce managed block <<<` comment markers, so the same block content can later be embedded inside a foreign hook without disturbing content around it. Before invoking `sce`, each checks `command -v sce`; when the CLI is unavailable, it prints branded, multiline installation guidance to stderr and exits successfully so Git operations are not blocked solely by a missing local CLI installation. ANSI styling is emitted only when stderr is a terminal; redirected output remains plain text. Failures from an available `sce` command propagate by capturing `$?` and calling `exit` explicitly, not by `exec`, so the block terminates the script deterministically even when it is not the file's only content.

Available-CLI behavior remains hook-specific:

- `pre-commit` invokes `sce hooks pre-commit "$@"`.
- `commit-msg` invokes `sce hooks commit-msg "$@"`.
- `post-commit` resolves `origin` with `git remote get-url origin` inside the managed block, after the CLI-presence check; when the lookup returns a non-empty URL, it invokes `sce hooks post-commit --vcs git --remote-url "$remote_url" "$@"`, otherwise it invokes `sce hooks post-commit --vcs git "$@"`. Remote metadata forwarding is exclusive to `post-commit`, and computing it inside the block keeps that behavior intact wherever the block is embedded.

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
