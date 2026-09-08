# Codex hook runtime (SCE)

Rust-side runtime behind `sce hooks codex`, the dispatcher subcommand the four
conversation/diff `.codex/hooks.json` registrations route to. Source:
`cli/src/services/hooks/codex/`. `.codex/hooks.json` also carries six
mutation-scope registrations routed to the separate
`sce hooks codex-mutation-scope` command (the generic mutation-scope ingress,
[../cli/mutation-scope-hook-ingress.md](../cli/mutation-scope-hook-ingress.md)) —
not part of this dispatcher.
See [Codex generated assets](../architecture.md) for the Pkl-authored
`.codex/hooks.json`/hook-script side of this integration and
[agent-trace-hooks-command-routing.md](agent-trace-hooks-command-routing.md)
for how the other three tools intake conversation/diff evidence.

## Generated hook invocation

The generated `.codex/hooks.json` routes its four conversation/diff
registrations through `sce hooks codex` and its six mutation-scope registrations
through `sce hooks codex-mutation-scope`. Each command resolves
`git rev-parse --show-toplevel` at invocation time, then invokes the
repository-root
`.codex/hooks/run-sce-or-show-install-guidance.sh` helper with quoted
expansions. It therefore works from the repository root, arbitrary nested
Codex working directories, and repository paths containing spaces. Git-root
resolution failures exit successfully without stdout; the helper retains its
existing missing-`sce` stderr guidance and forwards the hook JSON STDIN
unchanged. The exact registration set (four `sce hooks codex` plus six
`sce hooks codex-mutation-scope`) and invocation contract is covered by the
generated contract and `codex-hook-command` flake check. See [the ADR](../decisions/2026-08-23-codex-root-aware-hook-invocation.md).

## Non-destructive hook configuration ownership

`.codex/hooks.json` is a user-owned document. `sce setup --codex` and
`--all` merge the generated SCE fragment instead of replacing the whole file.
The shared `cli/src/services/codex_hook_config.rs` service mirrors current
Codex deserialization: top-level `description`/`hooks` only, the twelve
supported event names, defaulted matcher groups, and `command`, `mcp_tool`,
`prompt`, or `agent` handlers with their typed fields. It preserves unrelated
valid Codex fields, event groups, matcher groups, and handlers. Merge is
command-aware over two contracts: it replaces stale or duplicate handlers with
one current handler per required registration — the four `sce hooks codex`
registrations and six additive `sce hooks codex-mutation-scope` registrations,
each appended in its own unmatched group after the existing groups so an
already-trusted handler keeps its `(event, matcher, group index, handler index)`
identity and computed Codex trust key — touching only the matching command's
handlers. Ownership requires the helper path plus one of the `sce hooks codex` /
`sce hooks codex-mutation-scope` command contracts; a generic `sce` substring is
not enough.
Malformed or structurally invalid existing documents fail before staging, so
the existing file remains untouched. Doctor diagnoses each required
registration structurally (present-and-current, missing, or stale, with a
malformed whole document reported separately), so user-added valid Codex
handlers do not appear as SCE drift and invalid Codex configuration remains
unhealthy; `sce doctor --fix` repairs a structurally unhealthy document
through the same merge service. Codex's own hook-trust state in its durable
`$CODEX_HOME/config.toml` is read-only for doctor, separate from this
structural check; SCE never writes trust or auto-trust state. See [the
ADR](../decisions/2026-08-23-codex-nondestructive-hook-ownership.md) and [the
setup install policy](setup-no-backup-policy-seam.md).

An executable SCE project hook requires a third, independent dimension
beyond structure and trust: Codex's effective hook-discovery *policy*.
Current upstream Codex (`hooks/src/engine/discovery.rs`
`HookDiscoveryPolicy::allows`: `!allow_managed_hooks_only || source.is_managed`)
discards every non-managed hook source — including SCE's project
`.codex/hooks.json` registrations (`HookSource::Project`, non-managed) — when
the effective, admin-controlled `allow_managed_hooks_only` requirement is
`true`. That requirement lives only in `requirements.toml`/managed
configuration layers (never plain `config.toml`) and is composed from
multiple possible sources (system `requirements.toml`, legacy managed
config, MDM managed preferences, backend-delivered enterprise policy), so SCE
cannot safely re-derive it by reading any single file. `cli/src/services/codex_hook_policy.rs`
instead asks the installed `codex` binary for its own composed answer over
`codex app-server --stdio`'s read-only `configRequirements/read` method,
bounded by a strict timeout with the child process always terminated and
reaped. Doctor probes this exactly once per invocation and reuses the result
for every required registration. A structurally current registration is only
`Match`/healthy when policy allows project hooks *and* it is durably trusted;
`allow_managed_hooks_only = true` reports it `PolicyBlocked` (an
Error-severity, manual-only problem) even when fully trusted, and a probe
failure reports `PolicyUnknown` (Warning-severity, manual-only) rather than
ever defaulting to healthy. `sce doctor --fix` cannot change Codex's
managed/enterprise policy and never attempts to.

## Dispatch skeleton

- STDIN carries one raw Codex hook-event JSON payload into a typed
  `CodexHookEvent` (nine documented fields; only `hook_event_name` is
  required).
- `classify_codex_event` matches `(hook_event_name, tool_name)` into one of
  four dispatch arms — `UserPromptSubmit`, `Stop`, `PreToolUse(Bash)`,
  `PostToolUse(apply_patch)` — with every other combination (`apply_patch`
  under `PreToolUse` — no `sce hooks codex` registration matches it —
  unknown tool, `Bash` under `PostToolUse`, unrecognized `hook_event_name`)
  falling through to a deterministic `NoOp` success with empty stdout.
- Malformed/non-JSON STDIN is logged through `sce.hooks.codex.error` and the
  command still returns hook success with empty stdout (fails open), matching
  the other hook intakes' producer-facing failure posture.

## Session and model identity

- `prefixed_session_id`/`prefixed_diff_trace_session_id`/`prefixed_conversation_trace_session_id`
  (`cli/src/services/hooks/mod.rs`) carry a `"codex" -> cx_` arm alongside
  `oc_`/`cc_`/`pi_`, idempotent for an already-prefixed session ID.
- `normalize_codex_model_id` trims a Codex model ID, returns `None` for blank
  values, and otherwise preserves the reported ID unchanged — no inferred or
  fabricated provider prefix, since Codex exposes no separate provider field.
  `PostToolUse(apply_patch)` calls it to derive `diff_traces.model_id` when
  the event reports a model. This provider-preserving rule is an accepted
  durable decision; see [the ADR](../decisions/2026-08-23-codex-truthful-model-provenance.md).

## Implemented slices: `UserPromptSubmit` and `Stop` capture

`cli/src/services/hooks/codex/user_prompt_submit.rs` and
`cli/src/services/hooks/codex/stop.rs` implement the `UserPromptSubmit` and
`Stop` arms — conversation-capture dispatch arms with real behavior (see
"`PreToolUse(Bash)` policy delegation" and "`PostToolUse(apply_patch)` diff
capture" below for the other two). Both follow the same shape:

- `UserPromptSubmit` requires non-empty `session_id`, `turn_id`, and
  `prompt`. `Stop` requires non-empty `session_id`/`turn_id`; a `null`
  `last_assistant_message` (upstream types the field `string | null`) is a
  legitimate no-op — `stop::handle` returns silently before the Agent Trace
  DB opens, writing no message or part. An explicit empty string is a
  present value and still persists (unlike `null`). `session_id`/`turn_id`
  are trimmed before use, and a timestamp-acquisition failure fails open
  with no write for both arms, matching `PostToolUse(apply_patch)` below.
- `session_id` is stored as `cx_<session_id>` (idempotent) for both arms.
  `message_id` is deterministic rather than a generated UUID — `cx:<turn_id>:user`
  for `UserPromptSubmit`, `cx:<turn_id>:assistant` for `Stop`.
- `UserPromptSubmit` persists one `role = "user"` row with a `part_type = "text"`
  part (`text = prompt`); `Stop` persists one `role = "assistant"` row with a
  `part_type = "text"` part (`text = last_assistant_message`). Both call
  `RepositoryAgentTraceDb::insert_conversation_text_event`, which runs the
  existence check plus both inserts inside one `BEGIN IMMEDIATE` transaction
  (`TursoDb::execute_transactional_insert_pair_if_absent` in
  `cli/src/services/db/mod.rs`): a replayed or concurrent duplicate delivery is
  a no-op leaving exactly one message row and one part row, not only the
  parent message row that the plain `messages` table's own `ON CONFLICT
  (session_id, message_id) DO NOTHING` constraint alone would guarantee. This
  is one shared transactional primitive for both arms, not a Codex-specific DB
  adapter; OpenCode/Claude/Pi's conversation-trace writers still use the
  separate `insert_messages`/`insert_parts` calls unchanged.
- The DB is opened per invocation through the same
  `open_agent_trace_db_for_hook_runtime` repository-storage resolution the
  other hook intakes use.
- Both successful conversation-capture arms return empty stdout; their
  diagnostics and persistence failures remain logger-only through the outer
  fail-open dispatcher.

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
  (via `openai/codex` issue #28437) identical in shape to Claude's own deny
  response (`render_claude_hook_result` in `bash_policy.rs`), built directly
  rather than by calling that Claude-specific function.

Neither branch reads or writes `diff_traces`, a snapshot, or any
pending-state file; Bash-triggered filesystem mutations remain untracked for
Codex (see "Explicit non-goals" in
[agent-trace-hooks-command-routing.md](agent-trace-hooks-command-routing.md)).

## `PostToolUse(apply_patch)` diff capture

The existing Codex `PostToolUse(apply_patch)` evidence path remains separate
from mutation-scope attribution. Its parser, cwd-aware path containment,
event-scoped synthetic line identities, and `diff_traces` persistence contract
are documented in
[`codex-apply-patch-diff-runtime.md`](codex-apply-patch-diff-runtime.md).
Malformed or unsafe input remains fail-open and Bash denial remains the only
structured response from this dispatcher.

## No remaining stub arms

All four `sce hooks codex` dispatch arms (`UserPromptSubmit`, `Stop`,
`PreToolUse(Bash)`, `PostToolUse(apply_patch)`) now have real behavior.
`PreToolUse(apply_patch)` is deliberately not a `sce hooks codex` arm (see plan
`context/plans/codex-cli-integration.md`'s no-snapshot design) and falls
open as a `NoOp` like any other unsupported combination; the unmatched
`PreToolUse` group routed to `sce hooks codex-mutation-scope` is a separate
concern handled by that command.

## Verification

- `nix develop -c sh -c './scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml hooks::codex'`
  (also runnable narrowed per-arm, e.g. `hooks::codex::user_prompt_submit`).
  This includes the realistic repository-scoped PostToolUse/post-commit
  regression and the repeated-identical-content ambiguity test.
- `nix run .#pkl-check-generated` verifies the generated Codex hook
  registrations (four `sce hooks codex` plus six `sce hooks codex-mutation-scope`)
  and root-aware invocation contract.
- `nix flake check` runs the same tests plus clippy/fmt/generated-asset checks.

See also: [agent-trace-db.md](agent-trace-db.md),
[agent-trace-hooks-command-routing.md](agent-trace-hooks-command-routing.md),
[pi-extension-runtime.md](pi-extension-runtime.md)
