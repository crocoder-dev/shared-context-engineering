# Agent Trace Pre-commit Staged Checkpoint

## Scope

Task `agent-trace-attribution-no-git-wrapper` `T04` adds a pre-commit finalization contract that filters pending attribution to staged content only and preserves index/tree anchors for deterministic commit-time binding.
Task `agent-trace-prompt-capture` `T03` extends the same checkpoint with prompt-capture handoff data from the Claude JSONL append target.

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
  - Payload shape: `version`, `anchors`, checkpoint-level `harness_type`, optional `git_branch`, optional `model_id`, staged-only `files[]`, and `prompts[]`.
  - Downstream `commit-msg` gating only treats a file as SCE-attributed when `has_sce_attribution = true` and `ranges[]` is non-empty.
  - Current generic git-diff collection still defaults `has_sce_attribution` to `false`; a separate attribution-aware producer must set the marker when staged ranges are proven to come from SCE contribution.
  - Runtime remains fail-open: checkpoint collection/persist failures return deterministic diagnostics without blocking commit flow.
- Prompt handoff ingestion:
  - Prompt source path is Git-resolved `sce/prompts.jsonl`.
  - Current harness marker is fixed to `claude_code`.
  - Commit-time metadata enrichment resolves `git_branch` from `git branch --show-current` and `model_id` from deterministic env precedence (`SCE_MODEL_ID`, `CLAUDE_MODEL`, `CLAUDE_CODE_MODEL`, `ANTHROPIC_MODEL`, `MODEL_ID`).
  - Each prompt entry carried into the checkpoint includes `turn_number`, `prompt_text`, `prompt_length`, `is_truncated`, optional `cwd`, and `captured_at`.
  - Pre-commit prompt ingestion dedupes by `(prompt_text, captured_at)` so recommits do not duplicate identical prompt rows in the checkpoint.
  - When a prompt row omits `cwd`, the loader inherits the last known non-empty prompt cwd so commit-time checkpoint context remains populated without session tracking.
  - Missing prompt-capture files yield an empty `prompts[]` array; malformed JSONL rows are skipped without failing the hook.

## Verification coverage

- Mixed staged/unstaged fixture test confirms unstaged ranges are excluded and anchor values are preserved.
- Guard-path tests cover disabled, missing CLI, and bare-repository no-op behavior.
- Runtime fixture test validates persisted pre-commit checkpoint artifact contains staged-only ranges when both staged and unstaged edits exist for the same file.
- Prompt-loading unit coverage confirms invalid JSONL rows are ignored and duplicate prompt rows collapse to one checkpoint prompt entry.
- Prompt-loading/unit coverage also confirms cwd inheritance from the last known prompt and current-branch resolution for commit-time metadata enrichment.
