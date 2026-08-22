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
  `openai/`, mirroring `normalize_claude_model_id`. It is not yet called from
  any dispatch arm — apply_patch diff-trace persistence (a later
  Codex-integration task) is its first consumer.

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

## Still-stub arms

`PreToolUse(apply_patch)` and `PostToolUse(apply_patch)` currently return a
deterministic stub success string naming the future task that implements
them — a before/after repository snapshot and the persisted observed diff.
This document will grow a slice per arm as each lands.

## Verification

- `nix develop -c sh -c './scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml hooks::codex'`
  (also runnable narrowed per-arm, e.g. `hooks::codex::user_prompt_submit`).
- `nix flake check` runs the same tests plus clippy/fmt/generated-asset checks.

See also: [agent-trace-db.md](agent-trace-db.md),
[agent-trace-hooks-command-routing.md](agent-trace-hooks-command-routing.md),
[pi-extension-runtime.md](pi-extension-runtime.md)
