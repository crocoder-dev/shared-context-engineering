# Agent Trace Hooks Command Routing

## Scope

- Current trace-removal baseline for `cli/src/services/hooks/mod.rs`
- Focus: concrete `sce hooks` subcommand routing plus current minimal runtime behavior

## Implemented command surface

- `sce hooks pre-commit`
- `sce hooks commit-msg <message-file>`
- `sce hooks post-commit [--vcs <value>] --remote-url <url>`
- `sce hooks post-rewrite <amend|rebase|other>`
- `sce hooks diff-trace`
- `sce hooks conversation-trace`
- `sce hooks session-model` for normalized model attribution intake

## Parser and dispatch behavior

- `cli/src/app.rs` routes `hooks` through dedicated hook-subcommand parsing.
- `cli/src/services/hooks/mod.rs` owns deterministic runtime dispatch through `HookSubcommand` + `run_hooks_subcommand`.
- `post-commit` now enforces required parse-time validation for `--remote-url` in `cli/src/services/parse/command_runtime.rs`.
- `--vcs` remains optional and, when provided, must be one of `git|jj|hg|svn`; unsupported values fail with a validation-classified error.
- Missing or blank `--remote-url` fails with a validation-classified error before runtime dispatch.
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
  - Agent Trace DB access uses `AgentTraceDb::open_for_hooks_without_migrations()` followed by `ensure_schema_ready_for_hooks()` before both recent-patch reads/intersection writes and built Agent Trace persistence.
  - Missing or incomplete Agent Trace DB schema is a command-failing runtime error with `Run 'sce setup'.` guidance; hook runtime does not run migrations as a fallback.
  - Captures the current commit's patch from git using `capture_post_commit_patch_from_git()`.
  - Queries recent `diff_traces` patches from the past 7 days via `AgentTraceDb::recent_diff_trace_patches()`.
  - Recent-row parsing dispatches on `payload_type`: `patch` rows parse through existing `parse_patch`, while `structured` rows parse stored JSON through `structured_patch::derive_claude_structured_patch` at read time to produce `ParsedPatch`.
  - Parsed `PatchHunk` entries carry nullable row `model_id` for both paths, so combined/intersection patch inputs retain per-hunk model provenance for downstream Agent Trace attribution building.
  - Combines valid recent patches in chronological order via `patch::combine_patches`.
  - Intersects the combined recent patch with the post-commit patch via `patch::intersect_patches`.
  - Persists the serialized intersection result to `post_commit_patch_intersections` table with commit metadata (OID, timestamp), window bounds (cutoff_ms, end_ms), and loaded/skipped counts.
  - Empty recent patch set produces deterministic empty intersection result (no crash).
  - Internal orchestration now returns a typed `PostCommitIntersectionFlowResult` (`combined_recent_patch`, `post_commit_data`, optional `tool_name`, optional `tool_version`) from `run_post_commit_intersection_flow_with()`, where tool metadata comes from the most recent ordered parsed recent-patch row and falls back to `None` when the recent set is empty.
- `run_post_commit_subcommand(...)` now threads parsed optional `vcs_type` and validated `remote_url` through post-commit runtime flow into `run_post_commit_agent_trace_flow_with(...)`.
- `run_post_commit_agent_trace_flow_with(...)` prints the received remote URL to stderr as `post-commit remote_url=<url>` before building/validating/persisting the Agent Trace payload.
- At the current runtime boundary, parsed optional `vcs_type` is forwarded into `agent_trace::build_agent_trace(...)`; when absent, top-level `vcs` metadata is omitted.
  - The run-flow path maps commit-time metadata to RFC3339 and calls `agent_trace::build_agent_trace(...)`.
  - The same run-flow call now also forwards optional `tool_name` / `tool_version` from `PostCommitIntersectionFlowResult` into `AgentTraceMetadataInput`, so built post-commit payloads preserve tool metadata derived from recent parsed diff-trace rows.
  - The built Agent Trace payload includes top-level `metadata.sce.version` from the compiled `sce` CLI package version and range-level `content_hash` values computed from touched post-commit hunk content before conversion to JSON.
  - The built Agent Trace payload is converted to JSON `Value` and validated via `agent_trace::validate_agent_trace_value(...)` before persistence.
  - Validation failures are returned through the same post-commit runtime failure path/class used for Agent Trace DB insertion failures (no silent fallback).
  - When validation passes, the payload is serialized and inserted into Agent Trace DB `agent_traces` using `commit_id` from flow-result commit metadata, `commit_time_ms` from flow-result post-commit timestamp metadata, a derived non-null `url` value formatted as `sce.crocoder.dev/trace/<agent_trace.id>`, and the validated runtime `--remote-url` value persisted to nullable `agent_traces.remote_url`.
  - Post-commit Agent Trace success requires both schema validation and Agent Trace DB `agent_traces` persistence to succeed.
  - Current command-surface success output is: `post-commit hook processed intersection: commit=<oid>, intersection_files=<n>`.
- `post-rewrite` is a deterministic no-op entrypoint.
- `diff-trace` reads STDIN JSON and classifies the payload:
  - **Claude structured payloads** (detected by presence of top-level `hook_event_name`): the STDIN JSON is validated through `derive_claude_structured_patch`. Supported `PostToolUse` `Write` create and `Edit` structured-patch events produce a `DiffTracePayload` with `payload_type="structured"` and the raw event JSON stored as the `diff` column without conversion to unified-diff text. Unsupported Claude events (non-`PostToolUse`, unsupported tools, invalid payloads) produce a deterministic `NoOp` success result.
  - **OpenCode normalized payloads** (no `hook_event_name`): existing flat `{ sessionID, diff, time, model_id?, tool_name, tool_version }` validation applies unchanged, with `payload_type="patch"`.
  - The `DiffTracePayload` struct carries a `payload_type: String` field consumed by `persist_diff_trace_payload_to_agent_trace_db_with` to pass the correct discriminator to `DiffTraceInsert`.
  - When `model_id` is absent from the payload, Rust resolves it from the AgentTraceDb `session_models` table by `(tool_name, session_id)`. If a matching session model row is found, the payload is enriched with the resolved `model_id` before persistence. If no matching row is found, the hook returns success/no-op without writing artifact or DB rows (graceful skip).
  - When `model_id` is present in the payload, it is used directly without DB resolution.
  - Persistence: writes one parsed-payload artifact per invocation to `context/tmp/<timestamp>-000000-diff-trace.json` with atomic create-new retry semantics (only when model_id is resolved or present), and inserts the parsed payload fields into AgentTraceDb via `DiffTraceInsert` + `insert_diff_trace()` including nullable `model_id`.
  - Current TypeScript producers are the OpenCode agent-trace plugin and the generated Claude `sce hooks` command hooks (no TypeScript intermediary).
  - OpenCode forwards user-message `message.updated` diffs with `tool_name="opencode"`, always including `model_id`, and nullable OpenCode client-version metadata.
  - Claude forwards supported `PostToolUse` `Write` create and `Edit` structured-patch diffs with `tool_name="claude"`, omitting `model_id` from the payload and relying on Rust DB resolution.
  - Neither TypeScript runtime writes `context/tmp/*-diff-trace.json` artifacts or AgentTraceDb rows directly.
- `diff-trace` command success requires artifact persistence to succeed. AgentTraceDb open/insert failures are logged through `sce.hooks.diff_trace.agent_trace_db_write_failed` and reflected in the success text as failed DB persistence, while the parsed-payload artifact remains the durable fallback.
- `conversation-trace` is a recognized hook subcommand routed through `HookSubcommand::ConversationTrace`. Rust intake accepts only typed batch STDIN JSON with a top-level `type` discriminator and `payloads` array; the previous single-event envelope is no longer accepted.
  - `type: "message.updated"` parses each item in `payloads` into `InsertMessageInsert` with required non-empty `session_id`, `message_id`, valid `role` (`user|assistant`), and non-negative signed-64-bit `generated_at_unix_ms`; message-level `text`, `agent`, and `summary_diffs` are not required or mapped because body text belongs to `message.part.updated` / `parts.text`.
  - `type: "message.part.updated"` parses each item in `payloads` into `InsertPartInsert` with required non-empty `session_id`, `message_id`, valid `part_type` (`text|reasoning|patch`), string `text`, and non-negative signed-64-bit `generated_at_unix_ms`.
  - Item objects must not declare their own `type`; homogeneous batch type is owned by the top-level discriminator.
  - Per-item validation failures are recorded as skipped-item diagnostics (`index`, `reason`) while valid sibling items remain eligible for persistence; skipped validation items are logged through `sce.hooks.conversation_trace.payload_skipped`. Top-level JSON/type/`payloads` shape failures fail deterministically with `Invalid conversation-trace payload from STDIN: ...` diagnostics.
  - Current persistence opens one no-migration `AgentTraceDb` per hook invocation, checks schema readiness, then inserts each valid `message.updated` batch through one multi-row `AgentTraceDb::insert_messages(...)` call or each valid `message.part.updated` batch through one multi-row `AgentTraceDb::insert_parts(...)` call.
  - DB open or schema-readiness failures are command-failing runtime errors logged through `sce.hooks.conversation_trace.error`; valid-item multi-row insert failures are logged once through `sce.hooks.conversation_trace.agent_trace_db_batch_failed`, count the whole valid-item batch as skipped, and do not fail the command. The hook does not fall back to row-by-row insertion after a multi-row insert failure.
  - Current success output reports deterministic batch accounting: `conversation-trace hook persisted <event-type> payload batch to AgentTraceDb: attempted=<n>, persisted=<n>, skipped=<n>.` The hook does not persist `context/tmp` artifacts.
  - OpenCode's generated agent-trace plugin calls this hook with one-element typed batch envelopes for every captured `message.updated` event before its existing diff-trace flow and for every captured `message.part.updated` event without invoking diff-trace.
- `session-model` reads STDIN JSON and classifies the payload:
  - **Claude `SessionStart` payloads** (detected by presence of top-level `hook_event_name`): extracts `session_id` from `session_id`/`sessionID`, `model_id` from `model`/`model_id` (including nested `model.id`/`model.model`/`model.name` with `claude/` prefix normalization), `time` from `time`/`timestamp` (falls back to current system time), `tool_name="claude"`, and `tool_version` from `tool_version`/`claude_version`/`version`.
  - **OpenCode normalized payloads** (no `hook_event_name`): existing `{ sessionID, time, model_id, tool_name, tool_version }` validation applies unchanged.
  - Valid payloads are upserted into AgentTraceDb `session_models` via `SessionModelUpsert` using `(tool_name, session_id)` as the unique key. No raw hook artifacts are written. DB open/insert failures are logged through `sce.hooks.session_model.agent_trace_db_write_failed` and reported in the success text as failed persistence.

## Explicit non-goals in the current baseline

- No checkpoint handoff file
- No git-notes persistence
- No backfill/import of existing `context/tmp/*-diff-trace.json` artifacts into AgentTraceDb
- No retry queue replay
- No rewrite remap ingestion
- No `conversation-trace` retry/backfill path or `context/tmp` artifact persistence
- No runtime Claude diff-trace persistence or AgentTraceDb writes from the capture route itself, and no direct artifact/DB writes from the Claude or OpenCode TypeScript runtimes
