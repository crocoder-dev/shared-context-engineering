# SCE setup git-hooks embedded asset packaging

## Scope

Task `sce-setup-githooks-any-repo` `T02` defines how required git-hook templates are packaged for `sce setup --hooks` without runtime reads from `config/`.

## Canonical embedded hook sources

`cli/build.rs` now embeds three canonical hook templates from `cli/assets/hooks/`:

- `pre-commit`
- `commit-msg`
- `post-commit`

These templates are emitted into `OUT_DIR/setup_embedded_assets.rs` as `HOOK_EMBEDDED_ASSETS` with deterministic sorted relative paths.

Current `post-commit` template behavior is:

- resolve `origin` with `git remote get-url origin`; if `sce` is not on `PATH`, print `sce CLI not found. Install it from https://sce.crocoder.dev/docs/getting-started#install-cli` to stderr and exit successfully so missing local CLI installation does not block the commit
- if the remote lookup returns a non-empty URL, invoke `sce hooks post-commit --vcs git --remote-url "$remote_url" "$@"`
- otherwise still invoke `sce hooks post-commit --vcs git "$@"`; Rust-side validation fails this missing-URL path without blocking git commit completion under the hook script policy.

## Setup-service accessor surface

`cli/src/services/setup/mod.rs` exposes hook-template access through:

- `iter_required_hook_assets()` for deterministic full-set iteration
- `get_required_hook_asset(RequiredHookAsset)` for stable per-hook lookup

`RequiredHookAsset` is the canonical hook mapping enum for this packaging layer:

- `PreCommit`
- `CommitMsg`
- `PostCommit`

## Determinism and validation

Packaging determinism is enforced by setup tests in `cli/src/services/setup/mod.rs`:

- `embedded_hook_manifest_is_complete_sorted_and_normalized`
- `required_hook_lookup_resolves_each_canonical_hook`

These tests verify manifest completeness (exactly three required hooks), normalized relative paths, sorted ordering, and stable hook lookup semantics.
