# Agent Trace Hooks Command Routing

## Scope
- Current trace-removal baseline for `cli/src/services/hooks/mod.rs`
- Focus: concrete `sce hooks` subcommand routing plus current minimal runtime behavior

## Implemented command surface
- `sce hooks pre-commit`
- `sce hooks commit-msg <message-file>`
- `sce hooks post-commit`
- `sce hooks post-rewrite <amend|rebase|other>`
- `sce hooks diff-trace`

## Parser and dispatch behavior
- `cli/src/app.rs` routes `hooks` through dedicated hook-subcommand parsing.
- `cli/src/services/hooks/mod.rs` owns deterministic runtime dispatch through `HookSubcommand` + `run_hooks_subcommand`.
- Invalid and ambiguous invocations return deterministic actionable errors pointing to `sce hooks --help`.

## Current runtime behavior
- Shared enablement gate:
  - env `SCE_ATTRIBUTION_HOOKS_ENABLED`
  - config `policies.attribution_hooks.enabled`
  - precedence: env over config file
  - default: disabled
- `commit-msg` is the only active attribution path.
  - Reads the message file as UTF-8.
  - Applies exactly one canonical trailer: `Co-authored-by: SCE <sce@crocoder.dev>`.
  - Writes back only when the attribution gate is enabled, `SCE_DISABLED` is false, and the transformed content differs.
- `pre-commit` is a deterministic no-op entrypoint.
- `post-commit` preserves the `SCE_DISABLED` deterministic no-op; otherwise it attempts latest-session patch-intersection persistence.
  - Opens AgentTraceDb and selects the latest available `session_id` from `diff_traces` by `time_ms DESC, id DESC`.
  - If no diff-trace rows exist, returns `post-commit hook found no diff-trace rows; patch intersection persistence skipped.` without capturing `HEAD`.
  - Loads all selected-session raw patches by `time_ms ASC, id ASC`, preserving that row order as source provenance.
  - Captures `HEAD` SHA with `git rev-parse --verify HEAD` and the canonical post-commit patch with `git show --format= --patch --no-ext-diff HEAD` only after source rows exist.
  - Calls `build_patch_intersection_json(...)` and inserts one `patch_intersections` row with `commit_sha`, JSON-serialized ordered source diff-trace IDs, and compact `ParsedPatch` `intersection_json`.
  - Surfaces DB, git, invalid stored patch, invalid post-commit patch, and insertion failures as command-failing runtime errors with actionable context.
  - Retains best-effort hook trace artifact persistence for post-commit invocation input/outcome.
- `post-rewrite` is a deterministic no-op entrypoint.
- `diff-trace` reads STDIN JSON, validates required non-empty `sessionID`/`diff` plus required `u64` `time` (Unix epoch milliseconds), rejects `time` values that cannot fit the Agent Trace DB signed `time_ms` column, writes one payload artifact per invocation to `context/tmp/<timestamp>-000000-diff-trace.json` with atomic create-new retry semantics, and inserts the same payload into AgentTraceDb via `DiffTraceInsert` + `insert_diff_trace()`.
- `diff-trace` success requires both persistence paths to succeed; artifact write failures and AgentTraceDb open/insert failures are command-failing runtime errors logged through `sce.hooks.diff_trace.error`.

## Explicit non-goals in the current baseline
- No checkpoint handoff file
- No git-notes persistence
- No backfill/import of existing `context/tmp/*-diff-trace.json` artifacts into AgentTraceDb
- No full `AgentTrace` JSON persistence from `post-commit`; only the intermediate `ParsedPatch` intersection JSON is stored
- No retry queue replay
- No rewrite remap ingestion
