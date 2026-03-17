# Agent Trace Pre-commit Staged Checkpoint

## Scope

Task `agent-trace-attribution-no-git-wrapper` `T04` adds a pre-commit finalization contract that filters pending attribution to staged content only and preserves index/tree anchors for deterministic commit-time binding.

## Implemented contract

- Code location: `cli/src/services/hooks.rs`.
- Finalization entrypoint: `finalize_pre_commit_checkpoint(runtime, anchors, pending)`.
- Runtime hook entrypoint: `run_pre_commit_subcommand` -> `run_pre_commit_subcommand_in_repo(repository_root)`.
- Runtime no-op guards:
  - `sce_disabled = true` -> `NoOp(Disabled)`.
  - `cli_available = false` -> `NoOp(CliUnavailable)`.
  - `is_bare_repo = true` -> `NoOp(BareRepository)`.
- Runtime state resolution:
  - `SCE_DISABLED` truthy env values (`1`, `true`, `yes`, `on`) set disabled mode.
  - CLI availability checks `git --version` in the repository context.
  - Bare-repository guard uses `git rev-parse --is-bare-repository`.
- Staged-only enforcement:
  - Input keeps separate `staged_ranges` and `unstaged_ranges` per file.
  - Finalized output includes only `staged_ranges`.
  - Files with no staged ranges are dropped from finalized attribution.
- Runtime staged/unstaged extraction:
  - Staged hunks from `git diff --cached --unified=0 --no-color --no-ext-diff`.
  - Unstaged hunks from `git diff --unified=0 --no-color --no-ext-diff`.
  - Unified-diff hunks are parsed into deterministic line ranges per file path.
- Anchors captured in finalized output:
  - required `index_tree`.
  - optional `head_tree`.
- Anchor capture source:
  - `index_tree` from `git write-tree`.
  - `head_tree` from `git rev-parse --verify HEAD^{tree}` (optional for repos without `HEAD`).
- Finalized checkpoint handoff artifact:
  - Persisted as JSON at Git-resolved path `$(git rev-parse --git-path sce/pre-commit-checkpoint.json)`.
  - Payload shape: `version`, `anchors`, and staged-only `files[]` entries with `path`, `has_sce_attribution`, and `ranges[]`.
  - Downstream `commit-msg` gating only treats a file as SCE-attributed when `has_sce_attribution = true` and `ranges[]` is non-empty.
  - Current generic git-diff collection still defaults `has_sce_attribution` to `false`; a separate attribution-aware producer must set the marker when staged ranges are proven to come from SCE contribution.
  - Runtime remains fail-open: checkpoint collection/persist failures return deterministic diagnostics without blocking commit flow.

## Verification coverage

- Mixed staged/unstaged fixture test confirms unstaged ranges are excluded and anchor values are preserved.
- Guard-path tests cover disabled, missing CLI, and bare-repository no-op behavior.
- Runtime fixture test validates persisted pre-commit checkpoint artifact contains staged-only ranges when both staged and unstaged edits exist for the same file.
