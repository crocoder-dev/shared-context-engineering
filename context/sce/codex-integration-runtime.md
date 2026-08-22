# Codex hook runtime (SCE)

Rust-side runtime behind `sce hooks codex`, the single dispatcher subcommand
every registered `.codex/hooks.json` event routes to. Source: `cli/src/services/hooks/codex/`.
See [Codex generated assets](../architecture.md) for the Pkl-authored
`.codex/hooks.json`/hook-script side of this integration and
[agent-trace-hooks-command-routing.md](agent-trace-hooks-command-routing.md)
for how the other three tools intake conversation/diff evidence.

## Dispatch skeleton

- STDIN carries one raw Codex hook-event JSON payload, deserialized into a
  typed `CodexHookEvent` (`hook_event_name`, `session_id`, `turn_id`, `cwd`,
  `model`, `tool_name`, `tool_use_id`, `tool_input`, `tool_response`,
  `prompt`, `last_assistant_message`; only `hook_event_name` is required,
  matching the working contract in `context/plans/codex-cli-integration.md`).
- `classify_codex_event` matches `(hook_event_name, tool_name)` into one of
  five dispatch arms — `UserPromptSubmit`, `Stop`, `PreToolUse(Bash)`,
  `PreToolUse(apply_patch)`, `PostToolUse(apply_patch)` — with every other
  combination (unknown tool, `Bash` under `PostToolUse`, unrecognized
  `hook_event_name`) falling through to a deterministic `NoOp` success.
- Malformed/non-JSON STDIN is logged through `sce.hooks.codex.error` and the
  command still returns hook success (fails open), matching the other hook
  intakes' producer-facing failure posture.

## Session and model identity

- `prefixed_session_id`/`prefixed_diff_trace_session_id`/`prefixed_conversation_trace_session_id`
  (`cli/src/services/hooks/mod.rs`) carry a `"codex" -> cx_` arm alongside
  `oc_`/`cc_`/`pi_`, idempotent for an already-prefixed session ID.
- `normalize_codex_model_id` idempotently prefixes a raw Codex model ID with
  `openai/`, mirroring `normalize_claude_model_id`. Its first and only caller
  is `apply_patch/persist.rs`'s diff-trace persistence (see below).

## Implemented slices: `UserPromptSubmit` and `Stop` capture

`cli/src/services/hooks/codex/user_prompt_submit.rs` and
`cli/src/services/hooks/codex/stop.rs` implement the `UserPromptSubmit` and
`Stop` arms — conversation-capture dispatch arms with real behavior (see
"`PreToolUse(Bash)` policy delegation" below for the third). Both follow the
same shape:

- `UserPromptSubmit` requires non-empty `session_id`, `turn_id`, and
  `prompt`; `Stop` requires non-empty `session_id`, `turn_id`, and
  `last_assistant_message`. A missing or blank required field is a
  validation error (logged and failed open by the outer dispatcher).
- `session_id` is stored as `cx_<session_id>` (idempotent) for both arms.
  `message_id` is deterministic rather than a generated UUID — `cx:<turn_id>:user`
  for `UserPromptSubmit`, `cx:<turn_id>:assistant` for `Stop` — so that
  reprocessing the same turn's event is a no-op for the parent message row
  via the existing `messages` table's `ON CONFLICT (session_id, message_id)
  DO NOTHING` semantics.
- `UserPromptSubmit` persists one `role = "user"` row with a `part_type = "text"`
  part (`text = prompt`); `Stop` persists one `role = "assistant"` row with a
  `part_type = "text"` part (`text = last_assistant_message`). Both go through
  `RepositoryAgentTraceDb::insert_messages`/`insert_parts` — the same insert
  helpers and `messages`/`parts` tables `conversation-trace` already writes;
  there is no Codex-specific DB adapter.
- The `parts` table has no uniqueness constraint (append-only, like every
  other producer's part rows), so only the parent message row's
  non-duplication is guaranteed on reprocess, not the part row's.
- The DB is opened per invocation through the same
  `open_agent_trace_db_for_hook_runtime` repository-storage resolution the
  other hook intakes use.

## `PreToolUse(Bash)` policy delegation

`cli/src/services/hooks/codex/bash_policy.rs` implements the
`PreToolUse(Bash)` arm. It reads the shell command from
`tool_input.command` (a working assumption mirroring Claude's own `Bash`
`tool_input` shape, since no authoritative Codex-specific field-name source
was found; adjustable later without an architecture change) and calls
`evaluate_bash_command_policy` (`cli/src/services/bash_policy.rs`) directly
— the same matching engine `sce policy bash` uses for OpenCode/Claude, with
no reimplemented matching and no Codex-specific DB adapter:

- Allowed: returns an empty string (silent hook success, no model-visible
  output).
- Blocked: returns Codex's own native `PreToolUse` deny response —
  `{"hookSpecificOutput": {"hookEventName": "PreToolUse", "permissionDecision":
  "deny", "permissionDecisionReason": "<policy id + message>"}}` — confirmed
  against Codex's real hook contract (`openai/codex` issue #28437) to be
  identical in shape to Claude's own deny response
  (`render_claude_hook_result` in `bash_policy.rs`), though built directly
  rather than by calling that Claude-specific function.

Neither branch reads or writes `diff_traces`, a snapshot, or any
pending-state file; Bash-triggered filesystem mutations remain untracked for
Codex (see "Explicit non-goals" in
[agent-trace-hooks-command-routing.md](agent-trace-hooks-command-routing.md)).

## `PreToolUse(apply_patch)` before-state snapshot

`cli/src/services/hooks/codex/apply_patch/pre.rs` implements the
`PreToolUse(apply_patch)` arm: it captures the worktree state just before a
Codex `apply_patch` tool call runs, so the `PostToolUse(apply_patch)` finalize
step below can diff against it.

- Requires non-empty `session_id`, `turn_id`, and `tool_use_id`; a missing or
  blank field is a validation error (logged and failed open by the outer
  dispatcher, matching every other arm's posture).
- Derives a deterministic, filesystem-safe **event key** by SHA-256-hashing
  `session_id`, `turn_id`, and `tool_use_id` as separately-delimited byte
  segments (not string-concatenated, so field-boundary ambiguity cannot
  collide two distinct triples) and hex-encoding the digest.
- Snapshots the current worktree — tracked changes plus non-ignored untracked
  files, not just `HEAD` — into a `before_tree_oid` via `git read-tree HEAD`
  + `git add -A` + `git write-tree` run against a **temporary, scratch
  `GIT_INDEX_FILE`**, so the repository's real Git index is never read or
  written. The scratch index file is removed after the snapshot regardless
  of outcome.
- Writes `{before_tree_oid, created_at_unix_ms}` to a pending-state file
  named `<event_key>.json`, atomically (temp file in the same directory,
  then `fs::rename`), under a new per-repository pending-state directory:
  `<state_root>/sce/repos/<repository_id>/hooks/codex/pending/`
  (`codex_apply_patch_pending_dir_for_repository` in
  `cli/src/services/default_paths.rs`, mirroring the repository-scoped
  Agent Trace DB path). `repository_id` is resolved the same way
  `open_agent_trace_db_for_hook_runtime` resolves it — this arm reads no
  Agent Trace DB, and writes none.
- Writes no `diff_traces`, `messages`, or `parts` row. Attribution evidence
  is produced only once `PostToolUse(apply_patch)` finalizes a non-empty
  diff (a later task).

## `PostToolUse(apply_patch)` finalize: after-state, observed diff, cleanup

`cli/src/services/hooks/codex/apply_patch/post.rs` implements the
`PostToolUse(apply_patch)` arm: it finalizes the before/after attribution
`PreToolUse(apply_patch)` set up, by reading back that pending-state file and
computing the observed diff.

- Re-derives the same event key from `session_id`, `turn_id`, `tool_use_id`
  and looks up `<event_key>.json` in the pending-state directory.
- No pending file, or a pending file whose contents fail to parse: fails open
  through the outer dispatcher's fail-open path (same posture as every other
  arm's field-validation errors) and persists no diff evidence. A malformed
  pending file is also removed so a corrupt file does not linger; a missing
  file has nothing to remove.
- A found, valid pending file: takes a second worktree snapshot the same way
  `PreToolUse` did (`read-tree HEAD` + `add -A` + `write-tree` against a
  distinct scratch `GIT_INDEX_FILE`) for `after_tree_oid`, then runs a plain
  (index-less) `git diff --binary --find-renames <before_tree_oid>
  <after_tree_oid>` — a tree-to-tree diff needs no index at all.
- An empty diff is a successful no-op. A non-empty diff is the observed
  patch. Either way, the pending-state file is removed once the diff has been
  computed, so a duplicate `PostToolUse` call for the same event key
  thereafter hits the "no pending file" fail-open branch rather than
  recomputing or erroring differently.
- Because the before-state snapshot already captured any pre-existing dirty
  worktree change, the observed diff naturally excludes it: only the
  incremental delta between the two snapshots appears, regardless of what was
  already dirty when `PreToolUse` fired.
- A non-empty diff is handed to `apply_patch/persist.rs` (below) for
  persistence; a `NoOp` outcome is never persisted.

## Persisting a non-empty diff: `diff_traces` and patch conversation

`cli/src/services/hooks/codex/apply_patch/persist.rs` implements the
persistence step `PostToolUse(apply_patch)` calls for a non-empty observed
diff — the last previously-stub Codex dispatch arm behavior; every Codex
dispatch arm now has real behavior.

- Requires the same non-empty `session_id`, `turn_id`, `tool_use_id` as
  `pre`/`post` (reuses `pre::required_field`).
- Persists one `DiffTraceInsert` row via the existing `insert_diff_trace()`:
  `session_id = cx_<session_id>` (via `prefixed_diff_trace_session_id`),
  `patch` = the observed unified diff, `model_id` = `event.model` passed
  through T01's `normalize_codex_model_id` (`openai/`-prefixed, `None` when
  `event.model` is absent), `tool_name = "codex"`, `tool_version = None`,
  `payload_type = PAYLOAD_TYPE_PATCH`.
- Persists one assistant patch conversation row via the existing
  `insert_messages`/`insert_parts`: `session_id = cx_<session_id>` (via
  `prefixed_conversation_trace_session_id`), `message_id =
  cx:<turn_id>:<tool_use_id>:patch`, `role = Assistant`, one `part_type =
  Patch` part whose `text` is the raw observed diff (stored as-is, not
  JSON-serialized — the DB layer imposes no format constraint on
  `parts.text`).
- No Codex-specific DB adapter: both inserts reuse the exact helpers
  `UserPromptSubmit`/`Stop` and the other three tools' diff-trace/
  conversation-trace intakes already use.
- A persistence failure propagates through `?` to the outer dispatcher's
  existing fail-open path (`run_codex_subcommand` → `log_codex_fail_open`),
  the same posture as every other Codex arm's validation/parse errors.
- The persisted `diff_traces` row is read back and attributed by the
  existing, unmodified post-commit intersection pipeline
  (`recent_diff_trace_patches` → `combine_patches` → `intersect_patches` →
  `build_agent_trace`) exactly like an OpenCode/Claude/Pi row — no
  post-commit code required a change for Codex.

## Verification

- `nix develop -c sh -c './scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml hooks::codex'`
  (also runnable narrowed per-arm, e.g. `hooks::codex::user_prompt_submit`,
  `hooks::codex::apply_patch::persist`).
- The post-commit-pipeline-reuse proof
  (`codex_diff_trace_is_attributed_through_the_post_commit_pipeline`) lives in
  `cli/src/services/hooks/mod.rs`'s own test module, not under `hooks::codex`,
  since it exercises `run_post_commit_intersection_flow_with`/
  `run_post_commit_agent_trace_flow_with`/`capture_post_commit_patch_from_git`
  — private to that module and unreachable from a descendant `codex::` test
  module.
- `nix flake check` runs the same tests plus clippy/fmt/generated-asset checks.

See also: [agent-trace-db.md](agent-trace-db.md),
[agent-trace-hooks-command-routing.md](agent-trace-hooks-command-routing.md),
[pi-extension-runtime.md](pi-extension-runtime.md)
