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
- **`post-commit` is an active intersection entrypoint** (see [agent-trace-db.md](agent-trace-db.md)):
  - Captures the current commit's patch from git using `capture_post_commit_patch_from_git()`.
  - Queries recent `diff_traces` patches from the past 7 days via `AgentTraceDb::recent_diff_trace_patches()`.
  - Combines valid recent patches in chronological order via `patch::combine_patches`.
  - Intersects the combined recent patch with the post-commit patch via `patch::intersect_patches`.
  - Persists the serialized intersection result to `post_commit_patch_intersections` table with commit metadata (OID, timestamp), window bounds (cutoff_ms, end_ms), and loaded/skipped counts.
  - Empty recent patch set produces deterministic empty intersection result (no crash).
  - Returns structured success output: `post-commit hook processed intersection: commit=<oid>, loaded=<n>, skipped=<n>, intersection_files=<n>`.
- `post-rewrite` is a deterministic no-op entrypoint.
- `diff-trace` reads STDIN JSON, validates required non-empty `sessionID`/`diff`, required `u64` `time` (Unix epoch milliseconds), and optional non-empty string `model_id` when present; empty or non-string `model_id` values fail validation.
- `diff-trace` writes one payload artifact per invocation to `context/tmp/<timestamp>-000000-diff-trace.json` with atomic create-new retry semantics. Legacy payloads omit `model_id` from artifacts; accepted `model_id` values are serialized into the artifact.
- `diff-trace` attempts to insert the same payload into AgentTraceDb via `DiffTraceInsert` + `insert_diff_trace()`, mapping absent `model_id` to `NULL` and accepted values to `diff_traces.model_id`; `time` values that cannot fit the Agent Trace DB signed `time_ms` column are logged and prevent DB insertion while leaving artifact persistence intact.
- `diff-trace` success requires artifact persistence. AgentTraceDb open/insert failures are logged through `sce.hooks.diff_trace.agent_trace_db_write_failed` and return the alternate success text `diff-trace hook intake persisted payload to context/tmp; AgentTraceDb persistence failed.`

## Explicit non-goals in the current baseline
- No checkpoint handoff file
- No git-notes persistence
- No backfill/import of existing `context/tmp/*-diff-trace.json` artifacts into AgentTraceDb
- No retry queue replay
- No rewrite remap ingestion
