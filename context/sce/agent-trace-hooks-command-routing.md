# Agent Trace Hooks Command Routing

## Scope

- Current trace-removal baseline for `cli/src/services/hooks/mod.rs`
- Focus: concrete `sce hooks` subcommand routing plus current minimal runtime behavior

## Implemented command surface

- `sce hooks pre-commit`
- `sce hooks commit-msg <message-file>`
- `sce hooks post-commit [--vcs <value>]`
- `sce hooks post-rewrite <amend|rebase|other>`
- `sce hooks diff-trace`

## Parser and dispatch behavior

- `cli/src/app.rs` routes `hooks` through dedicated hook-subcommand parsing.
- `cli/src/services/hooks/mod.rs` owns deterministic runtime dispatch through `HookSubcommand` + `run_hooks_subcommand`.
- `post-commit` now parses optional `--vcs` input tolerantly at the command boundary: recognized values (`git|jj|hg|svn`) map to `Some(AgentTraceVcsType)`, while unknown values map to `None` without parse failure.
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
  - Recent-row patch parsing carries nullable row `model_id` into each produced `PatchHunk`, so combined/intersection patch inputs retain per-hunk model provenance for downstream Agent Trace attribution building.
  - Combines valid recent patches in chronological order via `patch::combine_patches`.
  - Intersects the combined recent patch with the post-commit patch via `patch::intersect_patches`.
  - Persists the serialized intersection result to `post_commit_patch_intersections` table with commit metadata (OID, timestamp), window bounds (cutoff_ms, end_ms), and loaded/skipped counts.
  - Empty recent patch set produces deterministic empty intersection result (no crash).
  - Internal orchestration now returns a typed `PostCommitIntersectionFlowResult` (`combined_recent_patch`, `post_commit_data`, optional `tool_name`, optional `tool_version`) from `run_post_commit_intersection_flow_with()`, where tool metadata comes from the most recent ordered parsed recent-patch row and falls back to `None` when the recent set is empty.
  - `run_post_commit_subcommand(...)` now threads the parsed optional `vcs_type` through `run_post_commit_agent_trace_flow(...)` into `run_post_commit_agent_trace_flow_with(...)`.
- At the current runtime boundary, optional parsed `vcs_type` is forwarded unchanged into `agent_trace::build_agent_trace(...)`; when absent, the built payload omits top-level `vcs`.
  - The run-flow path maps commit-time metadata to RFC3339 and calls `agent_trace::build_agent_trace(...)`.
  - The same run-flow call now also forwards optional `tool_name` / `tool_version` from `PostCommitIntersectionFlowResult` into `AgentTraceMetadataInput`, so built post-commit payloads preserve tool metadata derived from recent parsed diff-trace rows.
  - The built Agent Trace payload includes top-level `metadata.sce.version` from the compiled `sce` CLI package version before conversion to JSON.
  - The built Agent Trace payload is converted to JSON `Value` and validated via `agent_trace::validate_agent_trace_value(...)` before persistence.
  - Validation failures are returned through the same post-commit runtime failure path/class used for Agent Trace DB insertion failures (no silent fallback).
  - When validation passes, the payload is serialized and inserted into Agent Trace DB `agent_traces` using `commit_id` from flow-result commit metadata and `commit_time_ms` from flow-result post-commit timestamp metadata.
  - Post-commit Agent Trace success requires both schema validation and Agent Trace DB `agent_traces` persistence to succeed.
  - Current command-surface success output is: `post-commit hook processed intersection: commit=<oid>, intersection_files=<n>`.
- `post-rewrite` is a deterministic no-op entrypoint.
- `diff-trace` reads STDIN JSON, validates required non-empty `sessionID`/`diff`/`model_id`/`tool_name`, validates required `tool_version` (must be present and either `null` or a non-empty string), validates required `u64` `time` (Unix epoch milliseconds), rejects `time` values that cannot fit the Agent Trace DB signed `time_ms` column, writes one parsed-payload artifact per invocation to `context/tmp/<timestamp>-000000-diff-trace.json` with atomic create-new retry semantics, and inserts the parsed payload fields into AgentTraceDb via `DiffTraceInsert` + `insert_diff_trace()` including `model_id`.
- `diff-trace` success requires both persistence paths to succeed; artifact write failures and AgentTraceDb open/insert failures are command-failing runtime errors logged through `sce.hooks.diff_trace.error`.

## Explicit non-goals in the current baseline

- No checkpoint handoff file
- No git-notes persistence
- No backfill/import of existing `context/tmp/*-diff-trace.json` artifacts into AgentTraceDb
- No retry queue replay
- No rewrite remap ingestion
