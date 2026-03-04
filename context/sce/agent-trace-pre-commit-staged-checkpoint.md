# Agent Trace Pre-commit Staged Checkpoint

## Scope

Task `agent-trace-attribution-no-git-wrapper` `T04` adds a pre-commit finalization contract that filters pending attribution to staged content only and preserves index/tree anchors for deterministic commit-time binding.

## Implemented contract

- Code location: `cli/src/services/hooks.rs`.
- Finalization entrypoint: `finalize_pre_commit_checkpoint(runtime, anchors, pending)`.
- Runtime no-op guards:
  - `sce_disabled = true` -> `NoOp(Disabled)`.
  - `cli_available = false` -> `NoOp(CliUnavailable)`.
  - `is_bare_repo = true` -> `NoOp(BareRepository)`.
- Staged-only enforcement:
  - Input keeps separate `staged_ranges` and `unstaged_ranges` per file.
  - Finalized output includes only `staged_ranges`.
  - Files with no staged ranges are dropped from finalized attribution.
- Anchors captured in finalized output:
  - required `index_tree`.
  - optional `head_tree`.

## Verification coverage

- Mixed staged/unstaged fixture test confirms unstaged ranges are excluded and anchor values are preserved.
- Guard-path tests cover disabled, missing CLI, and bare-repository no-op behavior.
