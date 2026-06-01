# SCE setup git-hooks embedded asset packaging

## Scope

Task `sce-setup-githooks-any-repo` `T02` defines how required git-hook templates are packaged for `sce setup --hooks` without runtime reads from `config/`.

## Canonical embedded hook sources

`cli/build.rs` now embeds four canonical hook templates from `cli/assets/hooks/`:

- `pre-commit`
- `commit-msg`
- `post-commit`
- `post-rewrite`

These templates are emitted into `OUT_DIR/setup_embedded_assets.rs` as `HOOK_EMBEDDED_ASSETS` with deterministic sorted relative paths.

Current hook template behavior is:

- `post-commit` resolves `origin` with `git remote get-url origin`; when a non-empty URL is available it invokes `sce hooks post-commit --vcs git --remote-url "$remote_url" "$@"`.
- `post-commit` otherwise invokes `sce hooks post-commit --vcs git "$@"`; Rust-side validation fails this missing-URL path without blocking git commit completion under the post-commit hook policy.
- `post-rewrite` invokes `sce hooks post-rewrite "$@"` so the runtime can capture rebase evidence while keeping non-rebase methods as deterministic no-ops.

## Setup-service accessor surface

`cli/src/services/setup/mod.rs` exposes hook-template access through:

- `iter_required_hook_assets()` for deterministic full-set iteration
- `get_required_hook_asset(RequiredHookAsset)` for stable per-hook lookup

`RequiredHookAsset` is the canonical hook mapping enum for this packaging layer:

- `PreCommit`
- `CommitMsg`
- `PostCommit`
- `PostRewrite`

## Determinism and validation

Packaging determinism is enforced by setup tests in `cli/src/services/setup/mod.rs`:

- `embedded_hook_manifest_is_complete_sorted_and_normalized`
- `required_hook_lookup_resolves_each_canonical_hook`

These tests verify manifest completeness (exactly four required hooks), normalized relative paths, sorted ordering, and stable hook lookup semantics.
