# Plan: claude-subagent-attribution

## Change summary

A live run of five concurrent Claude `general-purpose` subagents (parent session
`e51f92c1-4aad-4319-8b98-cae1f5d74c26`, 2026-11-23) showed that SCE captures
subagent file mutations but loses most of the subagent identity on the way to
the control plane:

- All ten subagent mutation scopes stored `mutation_trace_scope_provenance.model_id = NULL`. Main-agent scopes in the same session stored `claude/claude-opus-5-5`.
- `claude_model_state` held only the main-agent row (`agent_id = ""`). Claude sends no `SessionStart` for a subagent, and the accepted decision `2026-09-01-claude-model-attribution-state` forbids inheritance from the main agent. A subagent therefore always resolves to `NULL`.
- Each subagent `Write` produced an assistant `patch` part under the main session. Its patch JSON carried `model_id: null` on every hunk and `session_id: null` on every line, and no agent or tool identity. The ClickHouse warehouse could attribute a patch to a subagent only by timing.

Claude already writes the needed facts locally. Each subagent has its own
transcript at `<project-dir>/<session_id>/subagents/agent-<agent_id>.jsonl`.
Its assistant records carry `message.model`, `agentId`, and `isSidechain: true`,
and each record that issues a tool call names that call's `tool_use_id`. A
sibling `agent-<agent_id>.meta.json` carries `agentType` and `toolUseId`, which
is the `tool_use_id` of the parent `Agent` call that spawned the subagent.

This plan extends existing behavior. It moves the Claude model attribution
chain into one Claude-specific resolver. Structured diff traces, mutation-scope
provenance, and conversation-trace patch parts all call that resolver. For a
subagent event, the resolver uses only exact evidence: the direct event model,
then the exact `tool_use_id` record in the subagent transcript, then `NULL`. It
never uses a transcript record that is not proven to belong to the event, it
never reads the main-agent state row, and it seeds no `claude_model_state` row.
Main-agent resolution keeps its existing precedence unchanged.

Transcript lookup becomes bounded. The reader scans only a fixed tail window of
the transcript, skips oversized records, and returns `NULL` when the exact
record is not inside those bounds. Each hook invocation scans a given
transcript for a given `tool_use_id` at most once.

Mutation-scope provenance gains one narrow write-once transition. When a
Claude scope was admitted with `model_id = NULL`, the `PostToolUse` or
`PostToolUseFailure` completion boundary may set `model_id` once, from exact
evidence only. A non-`NULL` model is never overwritten, and every other
provenance field stays insert-once.

Patch parts fill the existing hunk `model_id` and line `session_id` fields and
add one optional `attribution` object. The plan supersedes one guardrail of
`2026-09-01-claude-model-attribution-state`: Claude `agent_id` may now leave
the machine inside synchronized patch parts. It also replaces the "a stored
`NULL` is never backfilled" rule of `context/cli/mutation-scope-provenance.md`
for Claude scopes with the exact-evidence completion fill.

Missing attribution is preferred to incorrect attribution.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the
check that proves it. `/validate` runs these checks; no task in the stack
performs final validation.

- [x] AC1: An accepted decision record states the subagent precedence (direct event model, exact `tool_use_id` record in the subagent transcript, `NULL`), the preserved no-inheritance rule, the rule that transcript-derived models are never written to `claude_model_state`, the single-resolver rule, the transcript bounds and their rationale, the write-once completion fill of `mutation_trace_scope_provenance.model_id` from exact evidence, and the synchronized patch-part `attribution` fields. It names the superseded `agent_id` export guardrail of `2026-09-01-claude-model-attribution-state` and the replaced no-backfill rule for Claude provenance.
  - Validate: inspect `context/decisions/` for the new record and its `Supersedes` lines; `git diff --stat -- context/decisions/2026-09-01-claude-model-attribution-state.md` is empty.
- [x] AC2: A subagent event (`agent_id` present) resolves its model from the direct event model when present. Otherwise it resolves from the assistant record in the subagent transcript whose content names the event's `tool_use_id`. When no such record exists inside the transcript bounds, the result is `NULL`, even when other assistant records in that transcript name a model.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_subagent_model`.
- [x] AC3: When a subagent switches models during its run, each event resolves to the model on its own tool-call record. With a fixture where tool call A is on model X and a later tool call B is on model Y, A resolves to X and B resolves to Y. A tool call C with no record yet resolves to `NULL`, not Y.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_subagent_model`.
- [x] AC4: A subagent event never resolves through the main-agent `(cc_<session_id>, "")` state row or the bridge-session chain, and subagent resolution writes no `claude_model_state` row. With a fixture that has a main-agent state row and no subagent transcript record, the subagent result is `NULL` and the state table is unchanged.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_subagent_model`.
- [x] AC5: One Claude resolver module owns the attribution chain. Structured diff traces, mutation-scope provenance admission, the mutation-scope completion fill, and conversation-trace patch parts call it. No consumer calls the transcript reader or the model-state lookup directly.
  - Validate: `rg -n "claude_transcript::|claude_model_state_by_session_and_agent|seed_diff_trace_model_from_bridge_chain" cli/src/services --glob '!**/tests/**'` lists call sites only inside the resolver module and the definitions.
- [x] AC6: Main-agent events keep the existing precedence: direct event model, exact `tool_use_id` record in the main transcript, exact `(cc_<session_id>, "")` state row, bridge-session chain, then `NULL`. The existing main-agent attribution and model-state tests pass without change.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_model_attribution`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_model_state`.
- [x] AC7: The transcript reader scans at most the last 4 MiB of a transcript, starting at the first complete record inside that window. It skips any record longer than 1 MiB and any record that is not valid UTF-8 JSON, and it continues with the next record. It returns the model of the exact `tool_use_id` record, or no model when that record is outside the window, is oversized, is malformed, or is absent. An open or read failure is reported as a typed reader error, and the resolver maps every reader error to `NULL`.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_transcript_bounds`.
- [x] AC8: A lookup of a missing `tool_use_id` in a synthetic 32 MiB transcript reads no more than 5 MiB (the 4 MiB window plus one 1 MiB record) and returns no model. The test also asserts a loose wall-clock bound of 500 ms; the byte bound is the authoritative check.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_transcript_bounds`.
- [x] AC9: Within one hook invocation, each `(transcript path, tool_use_id)` pair is scanned at most once. The resolver result for an event is computed once and passed to every use in that invocation. No persistent cache, index, table, or cross-process coordination exists.
  - Validate: inspect the diff-trace, conversation-trace, and mutation-scope invocation paths for one resolver call per event; `git diff --stat origin/main -- cli/migrations` is empty.
- [x] AC10: Claude mutation-scope provenance attribution is correct at both boundaries:
  - (a) When the exact record exists at admission, the scope stores that model at admission, and completion leaves it unchanged.
  - (b) When the exact record is missing at admission but present at `PostToolUse` or `PostToolUseFailure`, completion changes `model_id` from `NULL` to that model.
  - (c) When the exact record is missing at both boundaries, `model_id` stays `NULL`.
  - (d) A stored non-`NULL` `model_id` is never overwritten, including when completion resolves a different model.
  - (e) A duplicate completion event leaves the row unchanged and succeeds.
  - The completion fill uses only the direct event model and the exact `tool_use_id` record, never `claude_model_state` or the bridge-session chain. It changes no field other than `model_id`. A fill failure never fails the hook or the close.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_provenance_fill`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_mutation_scope`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml scope_provenance`.
- [x] AC11: A Claude `PostToolUse` `Write` or `Edit` patch part sets every touched line's `session_id` to `cc_<session_id>` and every hunk's `model_id` to the resolver result or `null`. It carries one `attribution` object with `agent_id`, `agent_type`, `tool_use_id`, and `parent_tool_use_id`. A subagent patch has the event's `agent_id`; `agent_type` comes from the event, or from `agentType` in the meta file when the event has none; `parent_tool_use_id` comes from `toolUseId` in the meta file. A main-agent patch has `agent_id`, `agent_type`, and `parent_tool_use_id` set to `null`. No line or hunk carries agent fields.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml conversation_trace`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml patch`.
- [x] AC12: A missing, unreadable, or malformed subagent transcript or meta file yields `NULL` for each affected field. The hook still exits successfully, writes nothing to stdout, persists the patch part or scope, and never denies the tool.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_subagent_model`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml conversation_trace`.
- [x] AC13: Patch JSON serialized before this change, with no `attribution` key, still loads and still feeds the Agent Trace generator. The `parts` sync stream sends the enriched patch JSON unchanged in the existing `text` field, with no new remote stream, cursor, column, migration, or `/state` field.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml patch`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml sync`; `git diff --stat origin/main -- cli/src/services/sync cli/src/services/agent_trace_export cli/migrations` is empty.
- [x] AC14 (waived: not necessary, per user decision on 2026-11-23): A manual rerun of the concurrent-subagent experiment shows correct attribution in the local repository Agent Trace DB. Every non-`NULL` subagent scope `model_id` equals the model on that tool call's record in the subagent transcript. Every subagent patch part has the `agent_id` of the subagent that wrote the file and the `parent_tool_use_id` from that subagent's meta file. A `NULL` model is acceptable only when the exact record was in the transcript at neither boundary.
  - Validate: manual. Rerun five concurrent `general-purpose` subagents that each `Write` one file. Query `mutation_trace_scope_provenance` and the session's `patch` parts with `turso --readonly --experimental-multiprocess-wal <repo agent-trace.db>`, and compare each value with `<session_id>/subagents/agent-<agent_id>.jsonl` and `.meta.json`.

### Full validation

Repository-wide checks `/validate` runs after the last task, regardless of
which criterion they map to.

- `nix flake check`
- `nix run .#pkl-check-generated`

### Context sync

- `context/cli/mutation-scope-provenance.md`: the Claude producer obtains its model from the shared Claude resolver, and Claude scopes have one write-once `NULL`-to-model completion fill from exact evidence. The "no `UPDATE` path" and "a stored `NULL` is never backfilled" statements must name this exception.
- `context/sce/claude-raw-hook-capture.md`: the single Claude resolver, its main-agent and subagent precedence, the transcript bounds and their consequences, and the conversation-trace patch part attribution.
- `context/cli/patch-service.md`: the optional `attribution` object on `ParsedPatch`.
- `context/cli/agent-trace-sync-command.md`: the `parts` stream carries the enriched patch JSON as opaque text, control-plane compatibility was verified, and warehouse joins against Claude OpenTelemetry metrics must strip the `cc_` prefix from SCE session IDs.
- `context/glossary.md`: `subagent transcript` and `parent tool use ID`, if the glossary does not already define them.

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** `cli/src/services/hooks/claude_transcript.rs`, `cli/src/services/hooks/diff_trace.rs`, `cli/src/services/hooks/claude_model_state.rs`, one resolver module under `cli/src/services/hooks/`, `cli/src/services/hooks/claude_mutation_scope/`, `cli/src/services/mutation_trace/store.rs` (one conditional provenance model fill), `cli/src/services/hooks/claude_transforms.rs`, `cli/src/services/hooks/conversation_trace.rs`, `cli/src/services/patch.rs`, their tests, one new decision record, and the context files listed under `Context sync`.
- **Out of scope:** synchronizing `mutation_trace_scopes`, `mutation_trace_scope_provenance`, or `mutation_trace_events` to the control plane; control-plane, warehouse, and ClickHouse changes; per-subagent token or cost metrics, which Claude Code OpenTelemetry owns; changing the canonical `cc_` session prefix; capturing subagent prompts, `Read` calls, or model turns as conversation messages; changing main-agent precedence or the accepted main-agent post-switch race; changes to the generic mutation-scope ingress or to the Codex, OpenCode, and Pi producers; a provenance fill for non-Claude scopes.
- **Constraints:** a subagent must never inherit the main-agent `claude_model_state` row. A model observation that is not proven to belong to the event must not attribute it. Every transcript and meta read must be local, bounded, and fail-open, and must never deny a Claude tool or write to stdout. The resolver must be a concrete Claude-specific module with a concrete evidence struct; no new trait, provider interface, injection seam, or cross-harness abstraction. The existing `ClaudeModelStateResolver` closure seam may change its argument to that evidence struct. The bounded reader is generic over `Read + Seek` for fixture tests, which is static dispatch. New code uses typed errors and does not introduce `anyhow`; the store method keeps the store's existing error type. No background job, polling, retry, or new lifecycle phase. Run Cargo only through `nix develop -c ./scripts/run-cli-cargo.sh`. Automated tests must use fixture transcripts, never real `~/.claude` data.
- **Compatibility (verified):** control-plane commit `d016aeba` was inspected on 2026-11-23. The ingestion API decodes part `text` as `Schema.String` (`apps/api/src/api.ts`). The loader stores it verbatim in `raw.parts.text String` (`apps/agent-trace-dwh-loader/migrations/001_raw_tables.sql`). The event transformer hashes and gzips it as an opaque payload bounded by `MAX_PART_PAYLOAD_BYTES` (64 MiB) (`apps/event-dwh-transformer/src/transform.ts`, `limits.ts`). The API presentation layer and the web app pass `text` through unparsed. No control-plane component decodes the patch JSON shape, so the added `attribution` key and filled `model_id` and `session_id` values are preserved. No control-plane change and no release ordering are required. Querying the new fields in ClickHouse needs JSON extraction from `raw.parts.text`; that is later warehouse work. Mutation-scope provenance stays local, so the completion fill has no control-plane effect.
- **Non-goal:** a generic cross-editor subagent model, a model-state cache for subagents, a generic provenance reconciliation framework, or a transcript index.

## Assumptions

- The subagent transcript path is `<dirname(transcript_path)>/<session_id>/subagents/agent-<agent_id>.jsonl`, and the meta file has the same stem with `.meta.json`. The layout was observed in Claude Code transcripts on 2026-11-23. When the event `transcript_path` already names `subagents/agent-<agent_id>.jsonl`, the resolver uses it directly. One function owns this derivation.
- The Claude hook contract provides no authoritative event timestamp or sequence number (decision `2026-09-01-claude-model-attribution-state`). No reliable ordering exists to prove that a non-matching transcript record precedes an event, so the "latest preceding observation" step is omitted for subagents.
- `claude_model_state` records only local observation time. A transcript-derived model written there would look like a current observation and could overwrite a newer `PostModelSwitch`. The plan therefore seeds no transcript-derived row.
- Transcript bounds rest on 634 local Claude transcripts measured on 2026-11-23. File size: median 1.1 MB, p99 4.1 MB, maximum 8.9 MB. Record length: median 1.1 KB, p99 47 KB, p99.9 131 KB, maximum 0.99 MB. The record for the event's tool call is appended last or near last before the hook runs, so a 4 MiB tail window covers it with wide margin and covers the whole file for about 99% of transcripts. A 1 MiB record cap covers every measured record. Consequence of exceeding a bound: an assistant record that carries a `Write` input larger than about 1 MiB, or an exact record more than 4 MiB from the end, resolves to `NULL`. For a main-agent event, resolution then continues to the existing state and bridge steps, as it does today when the transcript has no match.
- The existing reader aborts the whole lookup on the first unreadable line (`line.ok()?`). The bounded reader skips a malformed record instead. This can only add exact matches, never change one.
- The completion fill is placed in the Claude adapter, after `handle_close` reports a successful close and before the adapter attempt is removed. It calls one store method that runs `UPDATE mutation_trace_scope_provenance SET model_id = ? WHERE scope_id = ? AND model_id IS NULL` and returns a typed outcome (`Filled`, `AlreadySet`, `NoRow`). This keeps the generic ingress, the protocol state, and the CAS batch unchanged. The trade-off: the fill is a Claude-specific store write outside the generic ingress, and it applies to Claude scopes only. A generic `close` provenance field was rejected because it would change the ingress contract and the conformance suite for every harness.
- The fill runs only when the stored row exists and its `model_id` is `NULL`, so a scope that already has a model costs one indexed read and no transcript scan. The fill applies to main-agent and subagent Claude scopes alike, because both use exact evidence only. Main-agent admission precedence is unchanged.
- Provenance is read at commit time by `mutation_trace/runtime/mutation_attribution.rs`. A fill at `PostToolUse` precedes any commit of that mutation, so the commit-time read observes it.
- The `attribution` object uses `#[serde(default)]` and is serialized only when present, following the existing optional `model_id` and `session_id` fields in `cli/src/services/patch.rs`.
- Patch-part `session_id` uses the canonical `cc_` form, matching `messages.session_id` and `parts.session_id`.

## Task stack

- [x] T01: `Record the Claude subagent attribution decision` (status:done)
  - Task ID: T01
  - Scope: In — one new decision record under `context/decisions/` covering the subagent precedence, the omitted ordering step and why, the no-seeding rule, the preserved no-inheritance rule, the single-resolver rule, the transcript bounds and their consequences, the Claude-only write-once provenance completion fill and its rejected generic alternative, and the synchronized `attribution` fields, with `Supersedes` lines for the `agent_id` export guardrail of `2026-09-01-claude-model-attribution-state` and the Claude no-backfill provenance rule. Out — code changes, and edits to the historical decision file.
  - Dependencies: none
  - Done when: the decision record exists with status Accepted, names this plan, and states each rule that AC2–AC12 rely on.
  - Verify: inspect the new record; `git diff --stat -- context/decisions/2026-09-01-claude-model-attribution-state.md` is empty.
  - Completed: 2026-11-23
  - Files changed: `context/decisions/2026-11-23-claude-subagent-attribution.md`, `context/plans/claude-subagent-attribution.md`
  - Result: Added the Accepted decision `2026-11-23-claude-subagent-attribution` naming this plan. It states the single Claude resolver with full and exact-evidence entry points, unchanged main-agent precedence, subagent precedence (direct model, exact `tool_use_id` record, `NULL`) and the omitted ordering step, preserved no-inheritance and no-seeding rules, subagent path derivation, the 4 MiB window and 1 MiB record transcript bounds with typed fail-open errors, the bounded meta reader, per-invocation single scan, the Claude-only write-once `NULL`-to-model completion fill with its typed outcome and rejected generic `close` alternative, and the patch-part `session_id`, `model_id`, and `attribution` fields. Two `Supersedes` lines name the `agent_id` export guardrail of `2026-09-01-claude-model-attribution-state` and the Claude no-backfill provenance rule.
  - Verify outcomes: inspected the new record against AC2–AC12, and each rule is stated; `git diff --stat -- context/decisions/2026-09-01-claude-model-attribution-state.md` printed nothing.
  - Context impact: localized — one new decision record; current-state context files change in later tasks when behavior lands.
  - Context synchronization: synced

- [x] T02: `Add the shared Claude model resolver with bounded exact-record transcript lookup` (status:done)
  - Task ID: T02
  - Scope: In — the bounded transcript reader in `claude_transcript.rs` (4 MiB tail window, 1 MiB record cap, skip malformed records, typed reader error, generic over `Read + Seek`); one resolver module with a concrete evidence struct (canonical session, optional `agent_id`, `tool_use_id`, `transcript_path`, direct model) and two entry points: the full chain, and an exact-evidence-only chain (direct model, then exact record) for the completion fill; the main-agent branch moved from `diff_trace.rs` without precedence change; the subagent branch; the subagent path derivation; structured diff traces switched to one resolver call per event; fixture tests named `claude_subagent_model` (AC2, AC3, AC4, transcript cases of AC12) and `claude_transcript_bounds` (AC7, AC8). Out — mutation-scope provenance (T03), patch parts (T04), and any `claude_model_state` write for subagents.
  - Dependencies: T01
  - Done when: AC2, AC3, AC4, AC6, AC7, and AC8 hold, and structured diff traces obtain their model only through the resolver, with one transcript scan per event.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_subagent_model`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_transcript_bounds`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_model_attribution`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_model_state`.
  - Completed: 2026-11-23
  - Files changed: `cli/src/services/hooks/claude_transcript.rs`, `cli/src/services/hooks/claude_model_resolver.rs` (new), `cli/src/services/hooks/diff_trace.rs`, `cli/src/services/hooks/mod.rs`, `context/plans/claude-subagent-attribution.md`
  - Result: Replaced the unbounded transcript reader with `find_claude_transcript_tool_use_model` / `find_tool_use_model_in<R: Read + Seek>`. It scans the last 4 MiB starting at the first complete record, drains records longer than 1 MiB without buffering them, skips non-JSON or invalid-UTF-8 records, and reports open, seek, and read failures as `ClaudeTranscriptReadError`. Added the private `hooks::claude_model_resolver` module with the concrete `ClaudeModelEvidence` struct and two entry points: `resolve_claude_model` (full chain) and `resolve_claude_model_from_exact_evidence` (direct model, then the exact record; no DB access). Main-agent precedence is unchanged: direct, main transcript, `(cc_<session>, "")` row, then the bridge chain, keeping its existing `bridge_inherited` seed. A subagent resolves from direct model, then the exact record in the derived subagent transcript, then `NULL`. It reads no state row, uses no bridge chain, and writes nothing. `claude_subagent_transcript_path` owns the path derivation and rejects `agent_id` or session values that are not `[A-Za-z0-9_-]`. Structured diff traces now keep only the direct model at parse time and resolve once per event at persist time through `resolve_claude_model`. `resolve_claude_model_id`, `resolve_claude_model_id_with`, and `seed_diff_trace_model_from_bridge_chain` were removed from `diff_trace.rs`.
  - Verify outcomes: `claude_subagent_model` 9 passed (resolver AC2–AC4 cases, missing transcript, path derivation, subagent diff-trace persist); `claude_transcript_bounds` 7 passed (window-start boundary on both sides, record cap at 1 MiB and 1 MiB + 1, skip-and-continue past oversized and malformed records, 32 MiB missing-id lookup reads ≤ 5 MiB and < 500 ms, typed open and read errors); `claude_model_attribution` 5 passed (direct > transcript > state row > bridge chain with seed, main-agent diff-trace persist); `claude_model_state` 3 passed. Also: full CLI suite 875 passed; `clippy --all-targets -D warnings` clean; `cargo fmt` applied.
  - Deviation: the `claude_model_attribution` and `claude_model_state` suites named in Verify were deleted in commit `d21869ee`, so there were no existing tests to keep unchanged for AC6. New main-agent precedence tests carry the `claude_model_attribution_` prefix so that filter checks AC6. The `claude_model_state` filter matches two pre-existing setup/parse tests plus one new subagent test.
  - Context impact: localized — Claude model attribution moves into one resolver, and subagent precedence and transcript bounds change. `context/sce/claude-raw-hook-capture.md` should describe the resolver, precedence, and bounds.
  - Context synchronization: synced

- [x] T03: `Resolve and fill Claude mutation-scope provenance models through the shared resolver` (status:done)
  - Task ID: T03
  - Scope: In — the production `ClaudeModelStateResolver` in `claude_mutation_scope/lifecycle.rs` builds the evidence struct from the admitted event and calls the T02 full chain; the closure argument changes to that struct; the `PostToolUse` and `PostToolUseFailure` identity carries `transcript_path` when present; one `MutationTraceStore` method for the conditional `NULL`-to-model fill with a typed outcome; the fill call in `handle_close` after a successful close, using the T02 exact-evidence chain, logged and fail-open; tests named `claude_provenance_fill` for AC10 (a)–(e), and a store test under `scope_provenance` for the conditional update. Out — provenance insert-once rules for `session_id`, admission-bounded creation, the generic ingress, protocol state, the CAS batch, non-Claude producers, and new seams.
  - Dependencies: T02
  - Done when: AC10 holds, the lifecycle module no longer calls the model-state lookup directly, and a resolver or fill failure still admits and closes the scope.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_provenance_fill`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_mutation_scope`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml scope_provenance`.
  - Completed: 2026-11-23
  - Files changed: `cli/src/services/hooks/claude_mutation_scope/events.rs`, `cli/src/services/hooks/claude_mutation_scope/lifecycle.rs`, `cli/src/services/hooks/claude_mutation_scope/mod.rs`, `cli/src/services/hooks/claude_mutation_scope/provenance_fill_tests.rs` (new), `cli/src/services/mutation_trace/store.rs`, `context/plans/claude-subagent-attribution.md`
  - Result: `ClaudeToolIdentity` now carries an optional `transcript_path` and direct model, both parsed leniently so malformed values never reject the event, and builds `ClaudeModelEvidence` through `model_evidence()`. The admission resolver closure takes `(&Path, &ClaudeModelEvidence)`, and the production closure calls `resolve_claude_model`. The lifecycle module no longer calls `claude_model_state_by_session_and_agent`. Added `MutationTraceStore::fill_null_scope_provenance_model` (`UPDATE … SET model_id = ? WHERE scope_id = ? AND model_id IS NULL`) with the typed outcome `ScopeProvenanceModelFill { Filled, AlreadySet, NoRow }`. `handle_close` now takes the tool identity. After the seam close succeeds and before the adapter attempt is removed, it calls `fill_provenance_model_after_close`. That function opens the hook-runtime DB and calls `fill_claude_scope_provenance_model`, which reads the stored row first: a missing row or a non-`NULL` model costs no transcript scan. Otherwise it resolves through `resolve_claude_model_from_exact_evidence` once. Any failure is logged as `sce.hooks.claude_mutation_scope.provenance_model_fill_failed` and never fails the close.
  - Verify outcomes: `claude_provenance_fill` 6 passed (AC10 a: admitted exact-record model kept when completion sees a different model; b: `NULL` at admission despite a main-agent state row, filled at completion, no state row written; c: `NULL` at both boundaries; d: stored model never overwritten; ignores the state row and bridge chain for a main-agent fill; e and fail-open: `handle_close` with an unopenable DB succeeds, removes the attempt, logs the fill failure, and a duplicate close succeeds with no further seam call); `claude_mutation_scope` 10 passed; `scope_provenance` 2 passed (store fill: `NULL` → `Filled`, refill → `AlreadySet`, pre-set model kept → `AlreadySet`, missing → `NoRow`, `session_id` unchanged). Also: full CLI suite 882 passed; `clippy --all-targets -D warnings` clean; `cargo fmt` applied.
  - Deviation: the production `fill_provenance_model_after_close` wrapper is exercised only on its failure path, because opening the real hook-runtime DB in tests would write to user state. The fill core is tested with a temporary DB.
  - Context impact: localized — Claude mutation-scope provenance now resolves through the shared resolver and has a write-once `NULL`-to-model completion fill. `context/cli/mutation-scope-provenance.md` must name this exception to its "no `UPDATE` path" and "a stored `NULL` is never backfilled" statements.
  - Context synchronization: synced

- [x] T04: `Fill session, model, and subagent attribution on Claude patch parts` (status:done)
  - Task ID: T04
  - Scope: In — an optional `attribution` object (`agent_id`, `agent_type`, `tool_use_id`, `parent_tool_use_id`) on `ParsedPatch`; the Claude `PostToolUse` patch builder taking the resolved model and attribution as inputs while staying free of storage access; the conversation-trace persist path calling the T02 resolver once per event and a bounded meta-file reader (1 MiB cap, typed error, fail-open); tests for main-agent, subagent, missing or malformed meta, and historical-JSON cases. Out — new `parts` columns, per-line or per-hunk agent fields, sync wire changes, and non-Claude producers.
  - Dependencies: T02
  - Done when: AC5, AC9, AC11, AC12, and AC13 hold, and the Agent Trace generator tests that consume patch parts pass.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml conversation_trace`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml patch`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml sync`.
  - Completed: 2026-11-23
  - Files changed: `cli/src/services/patch.rs`, `cli/src/services/patch/tests.rs`, `cli/src/services/structured_patch.rs`, `cli/src/services/hooks/claude_transcript.rs`, `cli/src/services/hooks/claude_model_resolver.rs`, `cli/src/services/hooks/claude_transforms.rs`, `cli/src/services/hooks/conversation_trace.rs`, `cli/src/services/mutation_trace/attribution.rs`, `cli/src/services/mutation_trace/runtime/mutation_attribution.rs`, `cli/src/services/mutation_trace/runtime/mutation_attribution/tests.rs`, `cli/src/services/hooks/commit_hooks.rs`, `cli/src/services/hooks/commit_hooks/attribution_tests.rs`, `cli/src/services/hooks/commit_hooks/path_identity_tests.rs`, `cli/src/services/agent_trace/tests.rs`, `cli/src/services/agent_trace/committed_content/tests.rs`, `context/plans/claude-subagent-attribution.md`
  - Result: `ParsedPatch` gained `attribution: Option<Box<PatchAttribution>>` (`agent_id`, `agent_type`, `tool_use_id`, `parent_tool_use_id`), `#[serde(default)]` and omitted when absent; `ParsedPatch` now derives `Default`, and empty-patch literals use `ParsedPatch::default()`. Added the bounded meta reader `read_claude_subagent_meta` / `read_subagent_meta_from<R: Read>` in `claude_transcript.rs` (1 MiB cap, typed `ClaudeSubagentMetaReadError { Open, Read, Oversized, Malformed }`). The resolver module gained `claude_subagent_meta_path` (derived from `claude_subagent_transcript_path`) and `resolve_claude_patch_attribution`, which maps every meta error to `None`. A main-agent patch gets only `tool_use_id`; a subagent patch gets `agent_id`, `agent_type` from the event or else the meta `agentType`, and `parent_tool_use_id` from the meta `toolUseId`. `transform_claude_post_tool_use` takes a `ClaudePatchEnrichment { model_id, attribution }` and does no storage access: it stamps `cc_<session_id>` on every line, the model or `null` on every hunk, and attaches `attribution`. The conversation-trace entry now parses the request, opens the hook-runtime DB once, and for a Claude `PostToolUse` `Write`/`Edit` builds `ClaudePatchEvidence` and calls `resolve_claude_model` once per event. A resolver error is logged as `sce.hooks.conversation_trace.claude_model_resolution_failed` and yields `null`. The `parse_conversation_trace_payload` and `persist_conversation_trace_payload_to_agent_trace_db` functions were replaced by `parse_conversation_trace_request`, `conversation_trace_payload_from_request`, and `conversation_trace_payload_with_db`.
  - Verify outcomes: `conversation_trace` 5 passed (main agent: line `session_id`, hunk model from the exact transcript record over a main-agent state row, null-agent `attribution`; subagent: model from the subagent record (not the main transcript or state row), `agent_type` and `parent_tool_use_id` from meta; event `agent_type` wins over meta; missing transcript and meta yield `null` model and fields with a main-agent state row present, and the part still persists; malformed transcript and meta yield `null`s, and the part persists; no agent fields on lines or hunks); `patch` 42 passed (including historical patch JSON without `attribution` loading and reserializing byte-identical, and an `attribution` round trip with explicit nulls); `sync` 36 passed. Also: `claude_transcript` 11 passed (4 new meta-reader tests: fields, missing/non-string fields, cap at 1 MiB and 1 MiB + 1, malformed and missing-file typed errors); `agent_trace` 143 passed; full CLI suite 893 passed; `clippy --all-targets -D warnings` clean; `cargo fmt` applied. AC5: `rg` lists transcript-reader, meta-reader, and state-row call sites only in `claude_model_resolver.rs`, the existing bridge-chain helper in `claude_model_state.rs`, and the definitions. AC13: `git diff --stat origin/main -- cli/src/services/sync cli/src/services/agent_trace_export cli/migrations` is empty.
  - Deviation: `attribution` is boxed (`Option<Box<PatchAttribution>>`) so that `ParsedPatch` stays small; clippy `large_enum_variant` rejected the unboxed field in `ClaudeStructuredPatchDerivationResult`. The JSON shape is unchanged. The hook-runtime DB open moved from the persist step to before the transform, so the resolver and the inserts share one handle; a DB open failure still fails open with the same log event.
  - Context impact: localized — Claude patch parts now carry `session_id`, `model_id`, and an optional `attribution` object, and the conversation-trace path resolves through the shared resolver. `context/cli/patch-service.md`, `context/sce/claude-raw-hook-capture.md`, and `context/cli/agent-trace-sync-command.md` should describe this.
  - Context synchronization: synced

## Open questions

- Should mutation scopes and provenance sync to the control plane? Only that data can represent contended concurrent writes, such as the `ai_contended` song-3 and song-5 changes in the experiment. Patch-part attribution identifies the writer of each patch, but not contention. This plan leaves it out because it adds a fourth remote stream and a control-plane schema. This question does not block any task.

## Validation Report

**Status:** validated  
**Date:** 2026-11-23

### Commands run

- `nix flake check` -> exit 0 (all 8 checks passed, including CLI tests, clippy, and fmt)
- `nix run .#pkl-check-generated` -> exit 0 (Ephemeral Pkl generation passed: 157 files)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_subagent_model` -> exit 0 (6 passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_transcript_bounds` -> exit 0 (7 passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_model_attribution` -> exit 0 (5 passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_model_state` -> exit 0 (3 passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_provenance_fill` -> exit 0 (6 passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_mutation_scope` -> exit 0 (10 passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml scope_provenance` -> exit 0 (2 passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml conversation_trace` -> exit 0 (4 passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml patch` -> exit 0 (41 passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml sync` -> exit 0 (36 passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_transcript` -> exit 0 (11 passed; supporting evidence for AC12 meta and malformed-record handling)
- `nix run nixpkgs#ripgrep -- -n "claude_transcript::|claude_model_state_by_session_and_agent|seed_diff_trace_model_from_bridge_chain" cli/src/services --glob '!**/tests/**'` -> exit 0 (call sites only in `claude_model_resolver.rs`, the existing bridge-chain helper in `claude_model_state.rs`, and the definitions)
- `git diff --stat origin/main -- cli/migrations` -> exit 0 (empty)
- `git diff --stat origin/main -- cli/src/services/sync cli/src/services/agent_trace_export cli/migrations` -> exit 0 (empty)
- `git diff --stat -- context/decisions/2026-09-01-claude-model-attribution-state.md` -> exit 0 (empty; also empty against the index and `origin/main`)

### Success-criteria verification

- [x] AC1: decision record -> `context/decisions/2026-11-23-claude-subagent-attribution.md` is `Status: Accepted`, names this plan, and has two `Supersedes` lines (the `agent_id` export guardrail and the Claude no-backfill provenance rule); the historical decision file is unchanged.
- [x] AC2: subagent direct model, then exact record, then `NULL` -> `claude_subagent_model` 6 passed (`prefers_direct_event_model`; `follows_each_tool_call_across_a_model_switch` resolves `toolu_c` to `NULL` while other records name models).
- [x] AC3: per-tool-call model across a switch -> `claude_subagent_model_follows_each_tool_call_across_a_model_switch` asserts A→sonnet, B→opus, C→`NULL`.
- [x] AC4: no main-agent row, no bridge chain, no state writes -> `claude_subagent_model_never_reads_or_writes_claude_model_state` and `..._never_uses_bridge_session_chain` passed.
- [x] AC5: single resolver -> the `rg` check lists call sites only in the resolver, the existing bridge-chain helper, and the definitions; consumers call `resolve_claude_model` / `resolve_claude_model_from_exact_evidence` / `resolve_claude_patch_attribution` (`diff_trace.rs:533`, `conversation_trace.rs:162,179`, `claude_mutation_scope/lifecycle.rs:79,433`).
- [x] AC6: main-agent precedence unchanged -> `claude_model_attribution` 5 passed, `claude_model_state` 3 passed (see the T02 deviation on the earlier deleted suites).
- [x] AC7: transcript bounds -> `claude_transcript_bounds` 7 passed (window boundary on both sides, 1 MiB cap at and over, skip-and-continue, typed open and read errors).
- [x] AC8: 32 MiB missing-id lookup reads ≤ 5 MiB -> `claude_transcript_bounds_missing_id_in_32_mib_transcript_reads_at_most_5_mib` passed.
- [x] AC9: one resolver call per event, no persistent cache -> inspected the call sites above: one `resolve_claude_model` per event in the diff-trace persist, conversation-trace patch, and mutation-scope admission paths, and one exact-evidence call in the completion fill; `git diff --stat origin/main -- cli/migrations` is empty.
- [x] AC10: provenance at both boundaries -> `claude_provenance_fill` 6, `claude_mutation_scope` 10, `scope_provenance` 2 passed.
- [x] AC11: patch-part session, model, and attribution -> `conversation_trace` 4 and `patch` 41 passed (main-agent null-agent attribution, subagent record and meta, event `agent_type` wins, no agent fields on lines or hunks).
- [x] AC12: missing or malformed files yield `NULL`s and fail open -> `conversation_trace_claude_subagent_missing_files_yield_nulls_and_still_persist` passed; malformed transcript records and malformed meta files are covered at reader level by `claude_transcript` (11 passed), and `resolve_claude_patch_attribution` maps every meta error to `None` (`claude_model_resolver.rs:118`).
- [x] AC13: historical patch JSON and opaque sync -> `patch_json_without_attribution_still_loads_and_reserializes_unchanged` passed; `sync` 36 passed; the sync/export/migrations diff against `origin/main` is empty.
- [x] AC14: manual concurrent-subagent rerun -> waived as not necessary by user decision on 2026-11-23. The manual experiment was not performed: the installed `sce` (`/nix/store/dnqpr34cj413g980z1y0wkjyxhdmkdfi-sce-0.4.0-pre-alpha-v14/bin/sce`) does not contain this branch's attribution code.

### Failed checks and follow-ups

- None.

### Residual risks

- No live end-to-end run has confirmed subagent attribution, because AC14 was waived; the evidence is fixture-based tests only.
- Test coverage drift: the staged diff removes three subagent resolver tests that T02 recorded (`claude_subagent_model_uses_exact_record_not_latest_record`, `..._is_null_when_exact_record_is_absent`, `..._missing_transcript_is_null`). The malformed-transcript/meta `conversation_trace` test and the `attribution` round-trip `patch` test that T04 recorded are also absent (test counts 6/4/41 against the recorded 9/5/42).
- AC12's malformed-file fail-open path is proven only at reader level plus code inspection, with no end-to-end conversation-trace hook test.
- The `fill_provenance_model_after_close` production wrapper is exercised only on its failure path (T03 deviation).
