# Codex hook runtime (SCE)

Rust-side runtime behind `sce hooks codex`, the single dispatcher subcommand
every registered `.codex/hooks.json` event routes to. Source: `cli/src/services/hooks/codex/`.
See [Codex generated assets](../architecture.md) for the Pkl-authored
`.codex/hooks.json`/hook-script side of this integration and
[agent-trace-hooks-command-routing.md](agent-trace-hooks-command-routing.md)
for how the other three tools intake conversation/diff evidence.

## Generated hook invocation

The generated `.codex/hooks.json` routes all four registrations through the
same command. That command resolves `git rev-parse --show-toplevel` at
invocation time, then invokes the repository-root
`.codex/hooks/run-sce-or-show-install-guidance.sh` helper with quoted
expansions. It therefore works from the repository root, arbitrary nested
Codex working directories, and repository paths containing spaces. Git-root
resolution failures exit successfully without stdout; the helper retains its
existing missing-`sce` stderr guidance and forwards the hook JSON STDIN
unchanged. The exact four-registration and invocation contract is covered by
the generated contract and `codex-hook-command` flake check. See [the ADR](../decisions/2026-08-23-codex-root-aware-hook-invocation.md).

## Non-destructive hook configuration ownership

`.codex/hooks.json` is a user-owned document. `sce setup --codex` and
`--all` merge the generated SCE fragment instead of replacing the whole file.
The shared `cli/src/services/codex_hook_config.rs` service mirrors current
Codex deserialization: top-level `description`/`hooks` only, the eleven
supported event names, defaulted matcher groups, and `command`, `mcp_tool`,
`prompt`, or `agent` handlers with their typed fields. It preserves unrelated
valid Codex fields, event groups, matcher groups, and handlers, and replaces stale or duplicate SCE-owned handlers with
one current handler for each of the four required registrations. Ownership
requires both `.codex/hooks/run-sce-or-show-install-guidance.sh` and the
`sce hooks codex` command contract; a generic `sce` substring is not enough.
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
for all four registrations. A structurally current registration is only
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
  under `PreToolUse` — no such registration exists in `.codex/hooks.json` —
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

`cli/src/services/hooks/codex/apply_patch/` implements the
`PostToolUse(apply_patch)` arm: `parser.rs` parses Codex's own `apply_patch`
text format (`*** Begin Patch` ... `*** End Patch`, with `Add File`/`Delete
File`/`Update File` operations and an optional `Update File` + `Move to`)
into a typed `CodexPatch`; `path.rs` resolves its paths from the event cwd to
safe repository-relative paths; `normalize.rs` normalizes it into SCE
`Index:`-form unified-diff text `crate::services::patch::parse_patch` already
accepts; `mod.rs`'s `handle` wires the stages together and persists the result:

- Reads the raw patch text from `tool_input.command` (a working assumption
  mirroring `PreToolUse(Bash)`'s own `tool_input.command` shape); a missing or
  non-string `command` fails open with no evidence.
- Before canonical parsing, outer intake preserves raw patch input and unwraps
  exactly the upstream-compatible `<<EOF`, `<<'EOF'`, and `<<\"EOF\"` forms.
  The wrapper is removed only when it has a complete canonical patch body;
  unsupported shell prefixes or quoting, missing/malformed delimiters, and
  trailing garbage fail open. Boundary-marker whitespace, environment IDs,
  empty/context-only patches, multiple operations/hunks, and end-of-file
  markers remain supported by the canonical parser.
- After parsing, the handler discovers the real Git root rather than assuming
  the process cwd is the repository root. It requires `cwd` to be a valid
  absolute directory inside that root, resolves every source and move
  destination independently from that cwd, and emits only lossless
  repository-relative UTF-8 paths. Valid `..` components and absolute paths
  are accepted when canonical resolution remains inside the worktree. Missing
  targets are checked through their nearest existing prefix, so Add File paths
  can remain absent while existing and missing symlink escapes are rejected.
  Outside-repository cwd, outside paths, malformed/NUL paths, and ambiguous
  mappings are logged as
  `sce.hooks.codex.apply_patch.path_resolution_failed` and fail open before
  normalization or database access. This canonical worktree containment rule
  is an accepted compatibility and security decision; see [the ADR](../decisions/2026-08-23-codex-canonical-worktree-path-resolution.md).
- A parse failure is logged (`sce.hooks.codex.apply_patch.parse_failed`) and
  fails open with no evidence — never a deny response, since `apply_patch`
  tracing is `PostToolUse`-only.
- Normalization keeps only the touched (`+`/`-`) lines of each `Add`/`Update`
  operation under deterministic, event-scoped synthetic line identities. The
  bounded range is derived from the stable `tool_use_id`, and checked local
  offsets are allocated across all emitted operations, hunks, and files so
  separate events do not collide in the existing `combine_patches` identity
  key. These positions are evidence identities, not source line numbers;
  missing/invalid identities or exhausted ranges fail open. After cwd-aware
  path resolution, Codex's unchanged context lines are dropped entirely and
  never persisted or claimed as real filesystem positions. `Delete File`
  operations, and an `Update File` + `Move to` with no changed lines, produce
  no evidence; a wholly-empty normalized result (e.g. delete-only) is a
  successful no-op with no `diff_traces` insert.
- A non-empty result is persisted as exactly one `diff_traces` row via the
  existing `insert_diff_trace` — `session_id = cx_<session_id>` after required
  trimmed non-empty validation, `model_id = normalize_codex_model_id(event.model)`
  when a model is reported, `tool_name = "codex"`, `tool_version = None`,
  `payload_type = "patch"` — no new persistence adapter. The event-scoped
  synthetic identity scheme is an accepted durable decision; see [the ADR](../decisions/2026-08-23-codex-event-scoped-apply-patch-evidence-identities.md).
- The timestamp comes from `current_unix_time_ms()`; a timestamp-acquisition
  failure here skips the insert entirely (fails open) rather than
  substituting a fabricated epoch-zero value, matching `UserPromptSubmit`
  and `Stop`'s own fail-open timestamp behavior above.
- Every path — success, empty-normalize no-op, and every fail-open branch —
  returns exactly empty stdout; Bash denial is the only structured Codex
  response.

Once committed, Codex evidence still attributes correctly through the
existing, unmodified `intersect_patches` historical `kind`+`content` fallback
(`cli/src/services/patch.rs`) even when the real committed lines land at
different real line numbers. Multiple same-content events retain separate
synthetic identities through the existing `combine_patches` behavior and can
match corresponding committed additions. This module does not touch the
fallback or combination semantics, and no `diff_traces`/Agent Trace schema
migration was added to support it.

## Conservative attribution boundary

This pipeline proves supplied touched content, not the physical occurrence of
that content in the repository. Codex provides no true source line ranges, and
SCE intentionally takes no filesystem snapshot or maintains pending tool state.
When repeated identical lines occur, `combine_patches` preserves separate
event-scoped evidence identities, but the existing content-based intersection
can only match available occurrences deterministically; it cannot prove which
identical physical occurrence came from which event. The focused regression test
covers this ambiguity and deliberately does not claim that issue 8 is solved.

The complete supported path is therefore `PostToolUse apply_patch` →
`tool_input.command` outer normalization and parsing → event-cwd/real-Git-root
path resolution → SCE `payload_type = "patch"` `diff_traces` persistence →
existing recent-row parsing, `combine_patches`, and post-commit intersection →
Agent Trace. Delete File, pure rename, and Bash-created filesystem mutations
remain without line-level evidence. There is no snapshot, pending-state,
Codex-specific Agent Trace builder, schema migration, or generic intersection
redesign in this path; malformed or unsafe inputs fail open silently.

## No remaining stub arms

All four registered dispatch arms (`UserPromptSubmit`, `Stop`,
`PreToolUse(Bash)`, `PostToolUse(apply_patch)`) now have real behavior.
`PreToolUse(apply_patch)` is deliberately never registered (see plan
`context/plans/codex-cli-integration.md`'s no-snapshot design) and falls
open as a `NoOp` like any other unsupported combination.

## Verification

- `nix develop -c sh -c './scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml hooks::codex'`
  (also runnable narrowed per-arm, e.g. `hooks::codex::user_prompt_submit`).
  This includes the realistic repository-scoped PostToolUse/post-commit
  regression and the repeated-identical-content ambiguity test.
- `nix run .#pkl-check-generated` verifies the four generated Codex hook
  registrations and root-aware invocation contract.
- `nix flake check` runs the same tests plus clippy/fmt/generated-asset checks.

See also: [agent-trace-db.md](agent-trace-db.md),
[agent-trace-hooks-command-routing.md](agent-trace-hooks-command-routing.md),
[pi-extension-runtime.md](pi-extension-runtime.md)
