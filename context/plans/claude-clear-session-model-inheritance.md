# Plan: claude-clear-session-model-inheritance

## Change summary

A Claude `/clear` fires a fresh `SessionStart` under a brand-new `session_id`, and
that event never carries a `model` field — confirmed tonight against real
captured hook payloads, not inferred. Today this is a legitimate, documented
silent no-op: `claude_model_state` never gets seeded for that session, and
because event-local transcript attribution loses its async-write race far more
often than the existing design assumed, the session then persists `NULL` model
attribution on every diff trace for its entire life unless an unrelated
`PostModelSwitch` happens to occur later.

Claude Code's transcript file (not the hook payload) carries a `bridgeSessionId`
that stays constant across a `/clear`, letting a cleared session be correlated
with the session it continued. This plan adds that correlation as a new,
local-only discovery source for `claude_model_state`: when a `SessionStart` has
no `model`, read that session's own `transcript_path` (already present on every
`SessionStart` payload, confirmed including the model-less `/clear` shape) for
its `bridgeSessionId`, find the most recently modified sibling transcript in the
same Claude project directory sharing that id, and — if that sibling already has
a `claude_model_state` row — inherit its model into the new session's row with
`source="bridge_inherited"`. This extends how a `claude_model_state` observation
can be seeded; it does not change the table, its schema, its export boundary, or
its exact-scope read/write contract, and it does not touch `PostModelSwitch`
(which never lacks a model). No prior work in the repository has read or
correlated `bridgeSessionId`; this is new discovery logic, not an extension of an
existing helper.

### Evidence gathered this session (2026-09-04, `improve-cli-errors` worktree)

All of the following came from real Claude Code hook traffic and real local
files, not synthetic payloads, captured by temporarily instrumenting
`sce hooks claude-model-state` with forced (`warn`, bypasses `log_level`)
diagnostic log lines and rebuilding/redeploying the local dev binary for this
worktree only (`cli/target/debug/sce`, pointed to by a temporary edit to
`.claude/hooks/run-sce-or-show-install-guidance.sh`):

- Three real `SessionStart` payloads were captured in full. Every one of them —
  including the model-less `/clear` case — carried `transcript_path`:
  - `source=clear`, no `model` key: `{cwd, hook_event_name, scratchpad_dir, session_id, source, transcript_path}`.
  - `source=startup`, with `model`: `{cwd, hook_event_name, model, scratchpad_dir, session_id, source, transcript_path}`.
  - A real `PostModelSwitch` payload: `{cache_ttl, context_tokens, cwd, estimated_cache_write_usd, from_model, hook_event_name, pricing, prompt_cache_warm, prompt_id, requested_model, scratchpad_dir, session_id, source, to_model, transcript_path}`.
  - `bridgeSessionId` was absent from all three — confirmed by a recursive
    key-name scan over the full parsed JSON tree, not just a top-level check.
- Repeated real `/clear` events across multiple sessions tonight
  (`6f9d3d40-...`, `c80bd850-...`, `45f33845-...`, `19721678-...`,
  `3baecb2c-...`, `6c40df5a-...`) all showed the identical pattern: `SessionStart`
  with `source=clear` and no `model` key, landing as a silent no-op — this is not
  a one-off, it is the deterministic behavior of `/clear`.
- Each session's own transcript file's second line
  (`{"type":"bridge-session","sessionId":...,"bridgeSessionId":"cse_...",...}`)
  was checked directly. Three real sibling pairs were confirmed sharing a
  `bridgeSessionId` across a `/clear` boundary, e.g. `c80bd850-...`
  (`source=clear`, no model) and `b850dadf-...` (`source=startup`,
  `model=claude/claude-opus-5`) both carry `bridgeSessionId=cse_019wqdgx5vaHPWJNzrLRKDYp`.
  This is the mechanism this plan builds on, not a hypothesis.
- Separately, the same investigation found and fixed an unrelated cause of
  missing attribution: the shared Turso-backed repository `agent-trace.db`
  intermittently failed to open with `I/O error: short read on WAL frame at
  offset 309032`, observed across several real `SessionStart`/`PostModelSwitch`/
  diff-trace/conversation-trace hook calls over a multi-minute window. This was
  manually repaired (backup taken, stale `.db-tshm`/`.db-wal` removed so Turso
  rebuilt them, repair verified via real write round-trips through the actual
  `sce`/Turso binary) and confirmed via a follow-up batch of real hook calls
  that all persisted cleanly afterward. That bug is fixed and is **not** part of
  this plan — it explains some, but not all, of the missing attribution seen
  during this investigation; the `/clear`-with-no-model gap this plan targets is
  independent and still present after the DB repair.

### What has already been done, and what T01–T03 still need to do

Already done, outside this plan's task stack (local investigation artifacts, not
committed change):

- Temporary diagnostic logging in `claude_model_state.rs` (`diag_invoked`,
  `diag_raw_payload`, `diag_resolved`, `diag_noop`, `diag_persisted`) that proved
  the evidence above. T03 removes this, since it replaces the exact code path
  the diagnostics were added to observe.
- A local dev build (`cli/target/debug/sce`) and a temporary redirect in this
  worktree's `.claude/hooks/run-sce-or-show-install-guidance.sh` so real hook
  traffic in `improve-cli-errors` runs that build instead of the installed Nix
  binary. This redirect is local-environment wiring, not a source change, and is
  out of this plan's scope to revert or keep; whoever implements T03 should
  rebuild the same way to keep testing against real hook traffic (see below).
- The Turso WAL-open-failure repair described above (already fixed, unrelated to
  this plan's task stack).

Still to build: the bridge-session discovery helper (T02) and its wiring into
the `SessionStart` no-op path (T03). Nothing in `claude_bridge_session.rs` or the
inheritance branch exists yet.

### How to retest against real Claude Code hook traffic

Unit tests (`Verify:` lines on T02/T03) prove the logic in isolation. To confirm
it against real Claude Code behavior the way this evidence was gathered:

1. Build the dev binary: `nix develop -c ./scripts/run-cli-cargo.sh build --manifest-path cli/Cargo.toml`.
2. Point this worktree's hooks at it (prepend `cli/target/debug` to `PATH` inside
   `.claude/hooks/run-sce-or-show-install-guidance.sh` before its `exec "$@"`, or
   restore the equivalent temporary redirect described above).
3. Trigger a real `/clear` in a Claude Code session running in this worktree.
4. Check that session's own log file, `context/tmp/sce-<date>-<new-session-id>.log`
   (find it with `ls -t context/tmp/*.log | head`): before T03, it shows
   `diag_noop` for the model-less `SessionStart`; after T03, it should show the
   new observation persisted with `source=bridge_inherited` (or an explicit log
   line naming that path, if T03 adds one) instead.
5. Confirm the inherited row directly:
   `RepositoryAgentTraceDb`'s existing exact-scope read for
   `(cc_<new-session-id>, "")`, e.g. through a focused test harness rather than
   raw `sqlite3` — a stock SQLite client was used earlier in this investigation
   to inspect the live Turso-managed DB and is suspected to have contributed to
   the WAL corruption above; avoid it against this DB while Turso holds it open,
   and prefer the `sce`/Turso binary or the repository's own test helpers for
   any live inspection.
6. Send at least one real tool call (`Write`/`Edit`) in the new session and
   confirm its `diff_traces.model_id` resolves to the inherited model (AC4),
   the same way `claude_model_attribution`'s persisted-row tests check it.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the
check that proves it. `/validate` runs these checks; no task in the stack
performs final validation.

- [ ] AC1: A `SessionStart` event with no `model` field, whose `transcript_path`
  file's leading records carry a `bridgeSessionId` that a sibling transcript in
  the same directory also carries, and whose sibling already has a
  `claude_model_state` row, causes the new session to persist a
  `claude_model_state` row with the sibling's model and `source="bridge_inherited"`.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_model_state`.
- [ ] AC2: When `transcript_path` is missing/unreadable, the bridge record is
  absent or malformed, no sibling shares the bridge id, or the sibling has no
  recorded state, the handler behaves exactly as today: silent no-op, zero
  stdout, no DB write, and existing state (if any) is never cleared or
  overwritten. Every branch fails open.
  - Validate: focused tests covering each failure branch under the same test command as AC1.
- [ ] AC3: Bridge discovery reads only the leading records of each candidate
  transcript (never a full-file scan), performs no network access, and leaves
  `PostModelSwitch` handling and the existing diff-trace precedence
  (`direct > exact transcript > exact state > NULL`) unchanged.
  - Validate: inspect the discovery helper for a bounded read; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_model` and `claude_model_attribution` pass unchanged alongside new coverage.
- [ ] AC4: A diff-trace event in a session that inherited its model this way, with
  no direct model and no winning transcript match, resolves `diff_traces.model_id`
  from the inherited state exactly as it would from a normal `SessionStart.model`
  seed.
  - Validate: persisted-row regression under `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_model_attribution`.
- [ ] AC5: A decision record documents the production evidence (real captured
  `/clear` `SessionStart` payloads confirmed to omit `model`; confirmed absence of
  `bridgeSessionId` in any captured hook payload shape; confirmed presence of
  `bridgeSessionId` in the transcript's bridge-session record; confirmed
  sibling-transcript pairing across real sessions), the mechanism, its
  best-effort/no-ordering-guarantee caveat, and why it stays within the existing
  Claude-specific/local-only/non-exported/no-generic-abstraction guardrails from
  the `2026-09-01-claude-model-attribution-state` decision.
  - Validate: inspect the decision file for each listed element.

### Full validation

Repository-wide checks `/validate` runs after the last task, regardless of
which criterion they map to.

- `nix run .#pkl-check-generated`
- `nix flake check`

### Context sync

- `context/sce/agent-trace-hooks-command-routing.md` — describe the bridge-inheritance
  fallback on the `SessionStart` no-op path and the `source="bridge_inherited"` value.
- `context/glossary.md` — add a `bridge session correlation` (or equivalent) term.
- `context/context-map.md` — update the `agent-trace-hooks-command-routing.md` annotation
  if its summary would otherwise describe `SessionStart` as unconditionally a no-op
  without a model.

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** `cli/src/services/hooks/claude_model_state.rs`; a new
  `cli/src/services/hooks/claude_bridge_session.rs` discovery module; focused Rust
  tests; one new decision record; the listed context-sync files.
- **Out of scope:** Agent Trace DB schema/migration changes, `PostModelSwitch`
  behavior, export/sync/control-plane changes, OpenCode/Pi/Codex attribution
  behavior, historical backfill of already-`NULL` rows, and the unrelated Turso
  WAL-open-failure issue diagnosed and manually repaired earlier this session
  (that was a database-availability bug, not a missing-signal gap, and is not
  part of this plan).
- **Constraints:** no schema/migration; local filesystem only, no network access;
  bounded/fail-open reads (leading records only, never a full transcript scan);
  `sce hooks claude-model-state` keeps its zero-stdout, fail-open, no-exit-2
  contract on every branch, including every new bridge-discovery branch; the
  inherited write remains exact-scope `(cc_<session_id>, agent_id)` and does not
  change subagent isolation; no new dependency.
- **Non-goal:** does not restore `session_models` or a generic cross-editor
  session cache; does not persist `bridgeSessionId` durably anywhere; does not
  attempt bridge correlation for `PostModelSwitch` (which always carries
  `to_model`); does not guarantee correctness when a user clears and switches
  models before any tool call — this is best-effort inheritance, not a proof.

## Assumptions

- Bridge correlation applies to any model-less `SessionStart` regardless of
  `source` (not only `source="clear"`): nothing in the captured data or existing
  code restricts the gap to that one source value, and narrowing to it would
  leave other model-less `SessionStart` shapes uncovered for no stated reason.
- The sibling's session id is read from its transcript filename stem, consistent
  with how `session_id` is already read from the hook payload elsewhere in this
  file and how `transcript_path` is already keyed to a session in
  `claude_transcript.rs`.
- "Most recently modified other transcript sharing the bridge id, excluding
  self" is an adequate deterministic tie-break for choosing the sibling to
  inherit from. This is the same best-effort/local-observation framing the
  `2026-09-01-claude-model-attribution-state` decision already accepted for
  `claude_model_state` generally; it does not claim to prove Claude's causal
  session ordering.

## Task stack

- [ ] T01: `Record the bridge-session model-inheritance decision` (status:todo)
  - Task ID: T01
  - Scope: In — write `context/decisions/{date}-claude-bridge-session-model-inheritance.md`
    covering the production evidence, mechanism, best-effort caveat, and guardrail
    compliance listed in AC5. Out — any code change, any edit to another context
    or plan file, any edit to the `2026-09-01-claude-model-attribution-state`
    decision.
  - Dependencies: none
  - Done when: the decision file exists in ADR format and contains every element
    AC5 names; no other file changes.
  - Verify: inspect the file against AC5.
  - Context synchronization: pending

- [ ] T02: `Add bounded bridge-session discovery helper` (status:todo)
  - Task ID: T02
  - Scope: In — new `cli/src/services/hooks/claude_bridge_session.rs` with two
    fail-open functions: (a) extract `bridgeSessionId` from a transcript path's
    leading records; (b) given a transcript path and a bridge id, scan sibling
    `.jsonl` files in the same directory for the most recently modified other
    file whose own leading records share that bridge id, and return its session
    id. No DB access, no network, bounded reads only. Out — wiring into
    `claude_model_state.rs`, any DB read/write.
  - Dependencies: T01
  - Done when: against real-shaped fixture transcripts (matching the payload
    shapes captured this session), the helper resolves the correct sibling
    session id; returns `None` for a missing file, an unreadable file, a
    missing/malformed bridge record, and no matching sibling; and its reads are
    bounded, not full-file scans.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_bridge_session`.
  - Context synchronization: pending

- [ ] T03: `Wire bridge inheritance into SessionStart and prove end-to-end attribution` (status:todo)
  - Task ID: T03
  - Scope: In — in `claude_model_state.rs`, when parsing yields no observation for
    a model-less `SessionStart`, invoke T02's helper against the event's own
    `transcript_path`; on a resolved sibling id, perform one exact-scope
    `claude_model_state` read for `(cc_<sibling_session_id>, "")`, and when found,
    persist a new observation for the *current* session with
    `observation_kind=SessionStart`, `source="bridge_inherited"`, and the
    sibling's model, through the same guarded local-observation-time write path
    used by any other observation; any failure at any step falls through
    unchanged to today's silent no-op. Remove the temporary `diag_*` diagnostic
    breadcrumbs added during this session's investigation, since this task
    replaces the exact no-op branch they were instrumenting. Add a persisted-row
    regression proving a diff-trace event in the newly-seeded session resolves
    `model_id` from the inherited state. Out — schema/migration changes,
    `PostModelSwitch` changes, export/sync changes.
  - Dependencies: T02
  - Done when: AC1, AC2, AC3, and AC4 all hold, and the existing
    `claude_model_state`, `claude_model`, and `claude_model_attribution` suites
    pass unchanged alongside the new coverage.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_model_state`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_model_attribution`; `nix flake check`.
  - Context synchronization: pending

## Open questions

Bridge inheritance is a probabilistic guess, not a guarantee: a session that
clears and switches models before its first tool call inherits the *previous*
model and gets attributed to it instead of correctly staying `NULL`. Today's
baseline is 100% of `/clear` sessions unattributed, so trading silence for
"usually correct, occasionally wrong" is very likely still a net improvement —
but it changes the failure mode from "we don't know" to "we have a plausible but
sometimes-wrong answer," which is a different kind of wrong worth deciding on
deliberately rather than assuming away.
