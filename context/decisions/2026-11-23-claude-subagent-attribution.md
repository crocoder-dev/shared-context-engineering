# Decision: Attribute Claude subagent events from exact local evidence only

Date: 2026-11-23
Status: Accepted
Plan: `context/plans/claude-subagent-attribution.md`
Task: T01
Supersedes: The guardrail "Do not add `agent_id` to `diff_traces`, Agent Trace payloads, or related external interfaces" in `2026-09-01-claude-model-attribution-state`, only for the conversation-trace patch-part `attribution` object defined below (the historical decision file remains unchanged)
Supersedes: The rule in `context/cli/mutation-scope-provenance.md` that a stored provenance `model_id` of `NULL` is never backfilled and that no `UPDATE` path exists, only for Claude scopes and only for the write-once completion fill defined below

## Context

A live run of five concurrent Claude `general-purpose` subagents (parent
session `e51f92c1-4aad-4319-8b98-cae1f5d74c26`, 2026-11-23) showed that SCE
captures subagent file mutations but loses the subagent identity:

- Every subagent mutation scope stored `mutation_trace_scope_provenance.model_id = NULL`.
  Main-agent scopes in the same session stored a model.
- `claude_model_state` held only the main-agent row (`agent_id = ""`). Claude
  sends no `SessionStart` for a subagent, and
  `2026-09-01-claude-model-attribution-state` forbids inheritance from the main
  agent. A subagent therefore always resolved to `NULL`.
- Each subagent `Write` produced a `patch` part under the main session whose
  hunks carried `model_id: null`, whose lines carried `session_id: null`, and
  which named no agent or tool. The warehouse could attribute it only by timing.

Claude already writes the needed facts locally. Each subagent has its own
transcript at `<project-dir>/<session_id>/subagents/agent-<agent_id>.jsonl`.
Its assistant records carry `message.model`, `agentId`, and
`isSidechain: true`, and each record that issues a tool call names that call's
`tool_use_id`. A sibling `agent-<agent_id>.meta.json` carries `agentType` and
`toolUseId`, which is the `tool_use_id` of the parent `Agent` call that spawned
the subagent.

The Claude hook contract provides no authoritative event timestamp or sequence
number. SCE cannot prove that a transcript record that does not name the
event's `tool_use_id` precedes that event.

Attribution logic is currently duplicated across structured diff traces,
mutation-scope provenance admission, and conversation-trace patch parts.

## Decision

### One Claude resolver

One concrete Claude-specific resolver module under `cli/src/services/hooks/`
owns the Claude model attribution chain. It accepts one concrete evidence
struct: canonical session ID, optional `agent_id`, `tool_use_id`,
`transcript_path`, and direct event model. It exposes two entry points:

- the **full chain**, used by structured diff traces, mutation-scope
  provenance admission, and conversation-trace patch parts;
- the **exact-evidence chain** (direct event model, then exact `tool_use_id`
  record), used by the mutation-scope completion fill.

No consumer calls the transcript reader, the `claude_model_state` lookup, or the
bridge-session chain directly. The resolver adds no trait, provider interface,
injection seam, or cross-harness abstraction. The existing
`ClaudeModelStateResolver` closure seam may take the evidence struct as its
argument.

Within one hook invocation, the resolver result for an event is computed once
and passed to every use, so each `(transcript path, tool_use_id)` pair is
scanned at most once. No persistent cache, index, table, or cross-process
coordination is added.

### Main-agent precedence (unchanged)

An event without `agent_id` resolves in this order:

1. direct Claude event model;
2. the exact `tool_use_id` record in the main transcript;
3. the exact `(cc_<session_id>, "")` `claude_model_state` row;
4. the bridge-session chain;
5. `NULL`.

### Subagent precedence

An event with `agent_id` resolves in this order:

1. direct Claude event model;
2. the assistant record in the subagent transcript whose content names the
   event's `tool_use_id`;
3. `NULL`.

A subagent event never resolves through the main-agent state row or the
bridge-session chain. It never uses a transcript record that does not name the
event's `tool_use_id`, even when other records in that transcript name a model.
When a subagent switches models, each event resolves to the model on its own
tool-call record, and an event whose record is not yet written resolves to
`NULL`, not to the latest observed model.

The "latest preceding observation" step is omitted for subagents. Without an
authoritative event order, a non-matching record cannot be proven to precede
the event, so using it could attribute the event to the wrong model.

### No inheritance and no seeding

The no-inheritance rule of `2026-09-01-claude-model-attribution-state` is
preserved: a subagent never inherits the main-agent `claude_model_state` row.
Subagent resolution writes no `claude_model_state` row, and no
transcript-derived model is written to `claude_model_state` for any agent.
`claude_model_state` records only local observation time, so a
transcript-derived row would look like a current observation and could
overwrite a newer `PostModelSwitch`.

### Subagent file paths

The subagent transcript path is
`<dirname(transcript_path)>/<session_id>/subagents/agent-<agent_id>.jsonl`,
and the meta file has the same stem with `.meta.json`. When the event
`transcript_path` already names a file under `<session_id>/subagents/`, the
resolver uses the sibling `agent-<agent_id>.jsonl` in that directory, which is
the event path itself for the matching subagent. A `subagents/` path under a
different session directory yields no path. One function owns this derivation.

### Transcript bounds

The transcript reader:

- scans at most the last 4 MiB of a transcript, starting at the first complete
  record inside that window;
- skips any record longer than 1 MiB and any record that is not valid UTF-8
  JSON, and continues with the next record;
- returns the model of the exact `tool_use_id` record, or no model when that
  record is outside the window, oversized, malformed, or absent;
- reports an open or read failure as a typed reader error, which the resolver
  maps to `NULL`;
- is generic over `Read + Seek` so fixture tests use static dispatch.

A lookup of a missing `tool_use_id` reads no more than the window plus one
capped record (5 MiB).

The meta-file reader is bounded to 1 MiB, uses a typed error, and fails open:
a missing, unreadable, or malformed meta file yields `NULL` for each field it
would supply.

### Write-once completion fill for Claude provenance

When a Claude mutation scope was admitted with
`mutation_trace_scope_provenance.model_id = NULL`, the Claude adapter may set
`model_id` once at the `PostToolUse` or `PostToolUseFailure` completion
boundary:

- The fill runs after `handle_close` reports a successful close and before the
  adapter attempt is removed.
- It uses only the exact-evidence chain, never `claude_model_state` or the
  bridge-session chain.
- It calls one store method that runs
  `UPDATE mutation_trace_scope_provenance SET model_id = ? WHERE scope_id = ? AND model_id IS NULL`
  and returns a typed outcome (`Filled`, `AlreadySet`, `NoRow`).
- A non-`NULL` `model_id` is never overwritten, including when completion
  resolves a different model. A duplicate completion leaves the row unchanged
  and succeeds.
- It changes no field other than `model_id`. Every other provenance field stays
  insert-once.
- A resolver or fill failure is logged and never fails the hook or the close.
- When the stored row already has a model, the fill costs one indexed read and
  no transcript scan.

The fill applies to main-agent and subagent Claude scopes alike, because both
use exact evidence only. Main-agent admission precedence is unchanged.
The fill does not apply to non-Claude scopes.

The fill is not ordered against commits. Commit-time attribution reads the
provenance stored at that moment. A commit can run before the completion fill,
for example when it is made concurrently or outside Claude's tool lifecycle.
That commit observes `model_id = NULL`. The fill never rewrites an Agent Trace
that was already generated. It stays best-effort, atomic, and write-once, and no
synchronization or retry is added to order it before commits.

### Patch-part attribution

A Claude `PostToolUse` `Write` or `Edit` patch part:

- sets every touched line's `session_id` to `cc_<session_id>`;
- sets every hunk's `model_id` to the full-chain resolver result, or `null`;
- carries one optional `attribution` object with `agent_id`, `agent_type`,
  `tool_use_id`, and `parent_tool_use_id`.

For a subagent patch, `agent_id` is the event's `agent_id`; `agent_type` comes
from the event, or from `agentType` in the meta file when the event has none;
and `parent_tool_use_id` comes from `toolUseId` in the meta file. For a
main-agent patch, `agent_id`, `agent_type`, and `parent_tool_use_id` are
`null`. No line or hunk carries agent fields.

The `attribution` object uses `#[serde(default)]` and is serialized only when
present. Patch JSON without it still loads. The `parts` sync stream sends the
enriched patch JSON unchanged in its existing `text` field. Claude `agent_id`
may therefore leave the machine inside synchronized patch parts. No new remote
stream, cursor, column, migration, or `/state` field is added.

## Rationale

Missing attribution is preferred to incorrect attribution. An exact
`tool_use_id` match proves that a transcript record belongs to the event. Any
weaker signal, such as the latest record, the main-agent state, or a seeded
state row, can name the wrong model when the subagent switches models or when
the main agent and subagent run different models.

One resolver removes duplicated precedence logic and makes AC-level guarantees
checkable at one call site per consumer.

The transcript bounds rest on 634 local Claude transcripts measured on
2026-11-23: file size median 1.1 MB, p99 4.1 MB, maximum 8.9 MB; record length
median 1.1 KB, p99 47 KB, p99.9 131 KB, maximum 0.99 MB. The event's tool-call
record is appended last or near last before the hook runs, so a 4 MiB tail
window covers it with wide margin and covers the whole file for about 99% of
transcripts. A 1 MiB record cap covers every measured record. Skipping a
malformed record, instead of aborting the lookup as the previous reader did,
can only add exact matches, never change one.

The completion fill exists because Claude writes the transcript asynchronously:
the exact record is often missing at admission but present by `PostToolUse`.
Restricting the fill to `NULL`-to-model and exact evidence keeps provenance
monotonic and free of guessed values.

## Alternatives considered

- **Seed a subagent `claude_model_state` row from the transcript** — rejected:
  the row would carry local observation time and could overwrite a newer
  `PostModelSwitch`, and it would turn an unproven observation into durable
  state.
- **Use the latest model in the subagent transcript** — rejected: no
  authoritative event order proves that record precedes the event, so a model
  switch would misattribute events.
- **Inherit the main-agent model** — rejected: subagents may run a different
  model, and the existing decision forbids it.
- **A generic `close` provenance field in the mutation-scope ingress** —
  rejected: it would change the ingress contract and the conformance suite for
  every harness to solve a Claude-specific timing gap. The chosen fill is a
  Claude-specific store write outside the generic ingress.
- **Unbounded transcript scan** — rejected: transcripts reach several MB, and a
  high-frequency hook must stay bounded.
- **A transcript index or persistent lookup cache** — rejected: it adds state,
  invalidation, and coordination for a per-invocation lookup.
- **Per-line or per-hunk agent fields** — rejected: agent identity is constant
  per patch, so one object avoids repetition.

## Compatibility and risks

- An exact record more than 4 MiB from the end of the transcript, or longer
  than 1 MiB (for example a `Write` input larger than about 1 MiB), resolves to
  `NULL`. For a main-agent event, resolution then continues to the state and
  bridge steps.
- A subagent tool call whose record is missing at both admission and completion
  keeps `model_id = NULL`.
- The transcript and meta-file layout was observed in Claude Code on
  2026-11-23 and is not a documented contract. A layout change yields `NULL`,
  never a wrong attribution.
- Control-plane commit `d016aeba` was inspected on 2026-11-23. No component
  decodes patch JSON; part `text` is stored and forwarded as an opaque string.
  The added `attribution` key and the filled `model_id` and `session_id` values
  are preserved, and no control-plane change or release ordering is required.
  Querying the new fields in the warehouse needs JSON extraction from
  `raw.parts.text`.
- Warehouse joins against Claude OpenTelemetry metrics must strip the `cc_`
  prefix from SCE session IDs.

## Guardrails

- Do not let a subagent inherit the main-agent `claude_model_state` row or use
  the bridge-session chain.
- Do not write any transcript-derived model to `claude_model_state`.
- Do not attribute an event from a transcript record that does not name its
  `tool_use_id`.
- Do not call the transcript reader, the model-state lookup, or the
  bridge-session chain outside the resolver module.
- Do not read transcripts or meta files without the stated bounds, and never
  let a read failure deny a Claude tool, write to stdout, or fail the hook.
- Do not overwrite a non-`NULL` provenance `model_id`, and do not change any
  other provenance field after insert.
- Do not add a provenance fill for non-Claude scopes.
- Do not export `claude_model_state`, mutation scopes, or provenance; `agent_id`
  leaves the machine only inside the patch-part `attribution` object.
- Do not add a persistent transcript cache, index, background job, polling, or
  retry.

## Consequences

Claude subagent events gain exact model attribution in structured diff traces,
mutation-scope provenance, and patch parts, while unresolved cases remain
`NULL`. Synchronized patch parts identify the writing agent and its parent
`Agent` call. Main-agent attribution is unchanged. Claude provenance gains one
narrow `NULL`-to-model transition; all other provenance stays insert-once. No
migration, sync stream, or control-plane schema changes.

## Follow-up

T02 implements the resolver and bounded reader, T03 the provenance admission and
completion fill, and T04 the patch-part attribution. Whether mutation scopes and
provenance should sync to the control plane remains an open question in the
plan.

## References

- Plan: [`claude-subagent-attribution`](../plans/claude-subagent-attribution.md)
- Partly superseded guardrail: [`2026-09-01-claude-model-attribution-state`](2026-09-01-claude-model-attribution-state.md)
- Bridge-session chain: [`2026-09-08-claude-bridge-session-model-inheritance`](2026-09-08-claude-bridge-session-model-inheritance.md)
- Provenance rules: [`mutation-scope-provenance`](../cli/mutation-scope-provenance.md)
