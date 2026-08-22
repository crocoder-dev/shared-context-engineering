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
  four dispatch arms — `UserPromptSubmit`, `Stop`, `PreToolUse(Bash)`,
  `PostToolUse(apply_patch)` — with every other combination (`apply_patch`
  under `PreToolUse` — no such registration exists in `.codex/hooks.json` —
  unknown tool, `Bash` under `PostToolUse`, unrecognized `hook_event_name`)
  falling through to a deterministic `NoOp` success.
- Malformed/non-JSON STDIN is logged through `sce.hooks.codex.error` and the
  command still returns hook success (fails open), matching the other hook
  intakes' producer-facing failure posture.

## Session and model identity

- `prefixed_session_id`/`prefixed_diff_trace_session_id`/`prefixed_conversation_trace_session_id`
  (`cli/src/services/hooks/mod.rs`) carry a `"codex" -> cx_` arm alongside
  `oc_`/`cc_`/`pi_`, idempotent for an already-prefixed session ID.
- `normalize_codex_model_id` idempotently prefixes a raw Codex model ID with
  `openai/`, mirroring `normalize_claude_model_id`. `PostToolUse(apply_patch)`
  calls it to derive a `diff_traces.model_id` value when the event reports a
  model.

## Implemented slices: `UserPromptSubmit` and `Stop` capture

`cli/src/services/hooks/codex/user_prompt_submit.rs` and
`cli/src/services/hooks/codex/stop.rs` implement the `UserPromptSubmit` and
`Stop` arms — conversation-capture dispatch arms with real behavior (see
"`PreToolUse(Bash)` policy delegation" and "`PostToolUse(apply_patch)` diff
capture" below for the other two). Both follow the same shape:

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

## `PostToolUse(apply_patch)` diff capture

`cli/src/services/hooks/codex/apply_patch/` implements the
`PostToolUse(apply_patch)` arm: `parser.rs` parses Codex's own `apply_patch`
text format (`*** Begin Patch` ... `*** End Patch`, with `Add File`/`Delete
File`/`Update File` operations and an optional `Update File` + `Move to`)
into a typed `CodexPatch`; `normalize.rs` normalizes it into SCE `Index:`-form
unified-diff text `crate::services::patch::parse_patch` already accepts;
`mod.rs`'s `handle` wires the two together and persists the result:

- Reads the raw patch text from `tool_input.command` (a working assumption
  mirroring `PreToolUse(Bash)`'s own `tool_input.command` shape); a missing or
  non-string `command` fails open with no evidence.
- A parse failure is logged (`sce.hooks.codex.apply_patch.parse_failed`) and
  fails open with no evidence — never a deny response, since `apply_patch`
  tracing is `PostToolUse`-only.
- Normalization keeps only the touched (`+`/`-`) lines of each `Add`/`Update`
  operation under deterministic, patch-local synthetic line numbers (starting
  at 1 per file) — Codex's own unchanged context lines are dropped entirely,
  never persisted or claimed as real filesystem positions. `Delete File`
  operations, and an `Update File` + `Move to` with no changed lines, produce
  no evidence; a wholly-empty normalized result (e.g. delete-only) is a
  successful no-op with no `diff_traces` insert.
- A non-empty result is persisted as exactly one `diff_traces` row via the
  existing `insert_diff_trace` — `session_id = cx_<session_id>`, `model_id =
  normalize_codex_model_id(event.model)` when a model is reported, `tool_name
  = "codex"`, `tool_version = None`, `payload_type = "patch"` — no new
  persistence adapter.
- The timestamp comes from `current_unix_time_ms()`; unlike every other
  Codex arm (which falls back to epoch zero via `.unwrap_or(0)`), a
  timestamp-acquisition failure here skips the insert entirely (fails open)
  rather than substituting a fabricated epoch-zero value.
- Every path — success, empty-normalize no-op, and every fail-open branch —
  returns empty stdout.

Once committed, a Codex Update's synthetic patch-local line numbers still
attribute correctly through the existing, unmodified `intersect_patches`
historical `kind`+`content` fallback (`cli/src/services/patch.rs`) even when
the real committed lines land at different real line numbers — this module
does not touch that fallback, and no `diff_traces`/Agent Trace schema
migration was added to support it.

## No remaining stub arms

All four registered dispatch arms (`UserPromptSubmit`, `Stop`,
`PreToolUse(Bash)`, `PostToolUse(apply_patch)`) now have real behavior.
`PreToolUse(apply_patch)` is deliberately never registered (see plan
`context/plans/codex-cli-integration.md`'s no-snapshot design) and falls
open as a `NoOp` like any other unsupported combination.

## Verification

- `nix develop -c sh -c './scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml hooks::codex'`
  (also runnable narrowed per-arm, e.g. `hooks::codex::user_prompt_submit`).
- `nix flake check` runs the same tests plus clippy/fmt/generated-asset checks.

See also: [agent-trace-db.md](agent-trace-db.md),
[agent-trace-hooks-command-routing.md](agent-trace-hooks-command-routing.md),
[pi-extension-runtime.md](pi-extension-runtime.md)
