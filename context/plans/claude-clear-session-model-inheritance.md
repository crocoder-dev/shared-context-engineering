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

### Second phase (2026-09-10): the shipped inheritance never fires

T01–T03 shipped and validated, and production shows the feature is inert. Zero
`claude_model_state` rows have ever been written with `source="bridge_inherited"`
(live repository DB: `picker|3`, `startup|6`, nine rows, two sources). The cause is
not the discovery logic: the deployed Nix binary contains the feature, the hook is
registered for all `SessionStart` sources with no matcher, every real bridge record
sits at transcript line 2–3 well inside `MAX_LEADING_RECORDS = 16`, real siblings
share `bridgeSessionId` correctly, and for each of the three observed `/clear`
sessions a bridge-linked sibling with existing state was available.

**Claude creates a session's `.jsonl` transcript lazily, after the `SessionStart`
hook has already run and exited.** Measured: session `2a56e9bc` wrote its
`claude_model_state` row at 07:56:03Z and its transcript file was not created until
07:56:16Z — 13 seconds later, on the same code path. So `transcript_path` is present
on the payload exactly as T01's evidence recorded, but names a file that does not
exist yet; `File::open` returns `ENOENT`, `extract_claude_bridge_session_id` fails
open to `None`, and `bridge_inheritance_candidate` returns `Ok(None)` before the
sibling scan or any DB read. The unit tests pass because every fixture pre-creates
the transcript, and AC2 explicitly blesses the missing-transcript branch as correct
fail-open behavior — in production that branch is the only branch ever taken.

This phase moves the inheritance to a point where the transcript is guaranteed to
exist: the diff-trace path, which already reads `claude_model_state` and already
holds the event's `transcript_path`. On a state miss for a raw structured Claude
payload it runs bridge discovery, seeds a `bridge_inherited` row for the current
session, and uses it for the trace in hand, so discovery happens at most once per
session. Two further findings shape the selection rule. First, transcript records
past the header carry a snake_case `session_id` alongside camelCase `sessionId`,
pointing at the chain's **origin** session (confirmed across both observed chains
and a fresh session, which points at itself) — but the origin is the wrong member to
inherit from: for `2e1257fa` the origin `2a56e9bc` was sonnet-5 while the model
actually in force was opus-5, set by an intervening `PostModelSwitch`. Second, and
symmetrically, "most recently modified sibling transcript" is only a proxy for
"most recent model observation"; it was correct in all three observed cases by
coincidence of mtime ordering. The correct target is the newest
`claude_model_state` observation across all chain members by `observed_at_ms` — the
filesystem supplies chain membership, the DB decides which member's model wins.
T01–T03 and their evidence stand unchanged; this phase changes the trigger point
and the selection rule.

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

- [x] AC1: A `SessionStart` event with no `model` field, whose `transcript_path`
  file's leading records carry a `bridgeSessionId` that a sibling transcript in
  the same directory also carries, and whose sibling already has a
  `claude_model_state` row, causes the new session to persist a
  `claude_model_state` row with the sibling's model and `source="bridge_inherited"`.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_model_state`.
- [x] AC2: When `transcript_path` is missing/unreadable, the bridge record is
  absent or malformed, no sibling shares the bridge id, or the sibling has no
  recorded state, the handler behaves exactly as today: silent no-op, zero
  stdout, no DB write, and existing state (if any) is never cleared or
  overwritten. Every branch fails open.
  - Validate: focused tests covering each failure branch under the same test command as AC1.
- [x] AC3: Bridge discovery reads only the leading records of each candidate
  transcript (never a full-file scan), performs no network access, and leaves
  `PostModelSwitch` handling and the existing diff-trace precedence
  (`direct > exact transcript > exact state > NULL`) unchanged.
  - Validate: inspect the discovery helper for a bounded read; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_model` and `claude_model_attribution` pass unchanged alongside new coverage.
- [x] AC4: A diff-trace event in a session that inherited its model this way, with
  no direct model and no winning transcript match, resolves `diff_traces.model_id`
  from the inherited state exactly as it would from a normal `SessionStart.model`
  seed.
  - Validate: persisted-row regression under `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_model_attribution`.
- [x] AC5: A decision record documents the production evidence (real captured
  `/clear` `SessionStart` payloads confirmed to omit `model`; confirmed absence of
  `bridgeSessionId` in any captured hook payload shape; confirmed presence of
  `bridgeSessionId` in the transcript's bridge-session record; confirmed
  sibling-transcript pairing across real sessions), the mechanism, its
  best-effort/no-ordering-guarantee caveat, and why it stays within the existing
  Claude-specific/local-only/non-exported/no-generic-abstraction guardrails from
  the `2026-09-01-claude-model-attribution-state` decision.
  - Validate: inspect the decision file for each listed element.
- [ ] AC6: A raw structured Claude `PostToolUse` diff-trace event in a session with
  no `claude_model_state` row of its own, whose transcript is bridge-linked to chain
  members that do have state, persists `diff_traces.model_id` from that chain and
  writes a `claude_model_state` row for the current session with
  `source="bridge_inherited"`.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_model_attribution`.
- [ ] AC7: Chain selection resolves the newest `claude_model_state` observation by
  `observed_at_ms`, not the chain origin and not the newest transcript file: for a
  chain whose root holds sonnet-5, whose mid-chain member holds opus-5 from a later
  switch, and whose cleared member's mtime-newest sibling is the root, the resolved
  model is opus-5.
  - Validate: focused regression under the same test command as AC6.
- [ ] AC8: Every discovery and state branch fails open exactly as today — absent
  `transcript_path`, missing or unreadable transcript, absent or malformed bridge
  record, no other chain member, no member state, and DB read or write failure each
  leave `diff_traces.model_id` as it would have been, write no state row, keep hook
  success, and emit zero stdout.
  - Validate: focused per-branch tests under the same test command as AC6.
- [ ] AC9: `transcript_path` carried for this resolution stays ephemeral: it is never
  written to `diff_traces`, any other column, or any exported payload.
  - Validate: parser regression asserting the field is absent from the stored row, in
    the shape of the existing `claude_diff_trace_parser_keeps_agent_id_ephemeral_and_storage_free` test.
- [ ] AC10: The second and later diff traces of a seeded session resolve from the
  session's own exact-scope state with no repeated bridge discovery and no second
  state write.
  - Validate: focused regression asserting one discovery and one state write across
    two consecutive diff-trace events in one session.
- [ ] AC11: Discovery stays bounded and local-only — leading records only, no
  full-transcript scan, no network — and one shared selection rule serves both the
  `SessionStart` and diff-trace call sites, with no second rule left in the code.
  - Validate: inspect the discovery and selection helpers for a bounded read and a
    single selection implementation; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_bridge_session` and `claude_model` pass alongside new coverage.
- [ ] AC12: A decision record documents the amended attribution precedence
  (`direct > exact transcript > exact state > bridge-derived chain state > NULL`),
  that a resolution path now writes state, the measured transcript-creation race that
  makes `SessionStart` unable to read its own transcript, the origin-vs-newest
  selection evidence, and why all of it stays inside the local-only,
  non-exported, Claude-specific guardrails of the `2026-09-01` and `2026-09-08`
  decisions.
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

Second phase:

- `context/sce/agent-trace-hooks-command-routing.md` — correct the current claim that
  model-less `SessionStart` inheritance seeds state (it cannot read its own
  transcript); describe the diff-trace state-miss seeding path, the amended
  `direct > exact transcript > exact state > bridge-derived chain state > NULL`
  precedence, and the newest-chain-observation selection rule.
- `context/glossary.md` — update `bridge session correlation` (selection is the newest
  chain observation, not the most recently modified sibling), `Claude diff-trace
  attribution` (amended precedence and the read-path seeding write), and
  `sce hooks claude-model-state` (its inheritance branch no longer carries the fix on
  its own).
- `context/context-map.md` — refresh the `agent-trace-hooks-command-routing.md` and
  decisions-index annotations for the amended precedence and the new decision record.

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
  tests; one new decision record; the listed context-sync files. Second phase adds
  `cli/src/services/hooks/mod.rs` (the `DiffTracePayload` ephemeral field and the
  diff-trace state-miss seeding path) and one further decision record.
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
- **Second-phase constraints:** the amended precedence appends one step and never
  reorders the existing three; the diff-trace hook keeps its fail-open, zero-stdout,
  no-exit-2 contract on every new branch; discovery stays bounded to leading records
  with no network and no full-transcript scan; seeding writes only the exact
  `(cc_<session_id>, agent_id)` scope through the existing guarded write path, so
  subagents still never inherit main-session state; only raw structured Claude
  payloads qualify, matching the existing exact-state lookup restriction; no schema,
  migration, export, or sync change; no new dependency.
- **Second-phase non-goal:** does not register a new Claude hook event, change
  `.claude/settings.json`, or regenerate Pkl-owned settings — the diff-trace hook
  already runs at the required moment; does not backfill the existing `NULL`
  `diff_traces.model_id` rows; does not make `claude_model_state` exported or
  readable by any other producer.

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
  session ordering. **Superseded by the second phase:** selection becomes the newest
  `claude_model_state` observation across chain members by `observed_at_ms`, on the
  evidence in *Second phase* above.

Second phase:

- Seeding lives in the diff-trace persistence flow beside the existing exact-state
  read rather than inside a pure resolver helper, so the one write on that path stays
  explicit and in a single place.
- `transcript_path` is threaded to persistence as a `#[serde(skip)]` field on
  `DiffTracePayload`, mirroring how ephemeral `agent_id` is already carried and
  populated at Claude structured parse time (`cli/src/services/hooks/mod.rs:995`).
- New coverage follows the existing `claude_bridge_session.rs` module's temp-directory
  fixture precedent rather than `context/patterns.md`'s no-filesystem unit-test rule,
  matching the surrounding module; the drift between that rule and the existing tests
  is pre-existing and out of scope here.
- The `SessionStart` bridge attempt is kept rather than removed: it costs one failed
  `File::open` and begins working unchanged if Claude ever creates transcripts
  eagerly.

## Task stack

- [x] T01: `Record the bridge-session model-inheritance decision` (status:done)
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
  - Completed: 2026-09-08
  - Files changed: `context/decisions/2026-09-08-claude-bridge-session-model-inheritance.md`
  - Result: Added the accepted decision for bounded, local-only inheritance of
    Claude model state across bridge-linked model-less SessionStart events,
    documenting production evidence, best-effort ordering caveats, and the
    existing Claude-specific attribution guardrails.
  - Verify: ADR inspection passed against AC5: the file records the real
    model-less `/clear` payloads, absence of `bridgeSessionId` in hook payloads,
    transcript bridge records and sibling pairing, the discovery mechanism,
    bounded/fail-open semantics, best-effort/no-ordering-guarantee caveat, and
    compliance with the 2026-09-01 Claude model-state decision's local-only,
    non-exported, non-generic guardrails. Baseline-relative comparison found
    only the new decision file changed before this plan record.
  - Done checks: All satisfied — the ADR exists in repository format, contains
    every AC5 element, and no implementation or unrelated context file changed.
  - Context impact: cross-cutting decision — establishes the bounded
    Claude-specific bridge-inheritance exception and its guardrails; context
    synchronization must reconcile the decision and inspect the mandatory root
    context files before another task starts.
  - Context synchronization: synced

- [x] T02: `Add bounded bridge-session discovery helper` (status:done)
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
  - Completed: 2026-09-08
  - Files changed: `cli/src/services/hooks/claude_bridge_session.rs`,
    `cli/src/services/hooks/mod.rs`
  - Result: Added bounded, fail-open bridge-session extraction and sibling
    discovery for Claude JSONL transcripts, selecting the most recently modified
    matching sibling and returning its filename-derived session ID without DB or
    network access.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_bridge_session` passed: 5 tests passed, 0 failed.
  - Done checks: All satisfied — real-shaped leading records resolve the bridge
    ID and newest matching sibling; missing, unreadable, malformed, empty, and
    unmatched cases fail open; and a regression proves records beyond the bounded
    leading-record limit are not scanned.
  - Context impact: cross-cutting implementation boundary — adds the
    Claude-specific bounded bridge discovery module that T03 will call from the
    model-less `SessionStart` path; context synchronization must reconcile the
    new helper and inspect the mandatory root context files before T03 starts.
  - Context synchronization: synced

- [x] T03: `Wire bridge inheritance into SessionStart and prove end-to-end attribution` (status:done)
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
   - Completed: 2026-09-08
   - Files changed: `cli/src/services/hooks/claude_model_state.rs`,
     `cli/src/services/hooks/mod.rs`
   - Result: Wired model-less Claude `SessionStart` events through bounded
     bridge-session discovery, exact main-session state lookup, and the existing
     guarded persistence path with `source="bridge_inherited"`; removed no
     remaining diagnostic breadcrumbs and added persisted-row diff-trace
     attribution coverage.
   - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_model_state` passed: 16 tests passed, 0 failed; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_model_attribution` passed: 3 tests passed, 0 failed; `nix flake check` passed: all checks passed. The additional `claude_model` filter passed: 22 tests passed, 0 failed.
   - Done checks: All satisfied — AC1 is proven by bridge-linked sibling state
     inheritance with the expected source and observation kind; AC2 remains
     fail-open for missing discovery/state and preserves empty stdout; AC3 is
     covered by T02's bounded helper and unchanged model/precedence suites; and
     AC4 is proven by the persisted inherited-state diff-trace regression.
   - Context impact: cross-cutting implementation boundary — changes Claude
     model-state lifecycle behavior and its attribution handoff; context
     synchronization must reconcile the fallback and inspect the mandatory root
     context files before another task or final validation.
   - Context synchronization: synced

- [x] T04: `Record the read-path bridge-seeding decision` (status:done)
  - Task ID: T04
  - Scope: In — write `context/decisions/{date}-claude-bridge-seeding-on-diff-trace.md`
    covering every element AC12 names: the measured transcript-creation race that makes
    `SessionStart` structurally unable to read its own transcript, the amended
    `direct > exact transcript > exact state > bridge-derived chain state > NULL`
    precedence, the fact that a resolution path now writes state, the
    origin-vs-mtime-vs-newest-observation selection evidence, and guardrail compliance
    against the `2026-09-01` and `2026-09-08` decisions. Out — any code change, any
    edit to the two existing decision records, any context-sync file edit.
  - Dependencies: T03
  - Done when: the decision file exists in repository ADR format and contains every
    element AC12 names; no other file changes.
  - Verify: inspect the file against AC12.
  - Completed: 2026-09-10
  - Files changed: `context/decisions/2026-09-10-claude-bridge-seeding-on-diff-trace.md`
  - Result: Added the accepted decision moving bridge-derived Claude model-state
    seeding from `SessionStart` to the diff-trace state-miss path, recording the
    measured transcript-creation race, the amended attribution precedence, the
    read-path state write, the newest-chain-observation selection rule with its
    origin and mtime counter-evidence, and guardrail compliance with the
    `2026-09-01` and `2026-09-08` decisions.
  - Verify: ADR inspection passed against AC12: the file records the measured
    13-second transcript-creation lag that makes `SessionStart` structurally
    unable to read its own transcript, the amended
    `direct > exact transcript > exact state > bridge-derived chain state > NULL`
    precedence, the explicit statement that a resolution path now writes state,
    the origin-vs-mtime-vs-newest-observation selection evidence (chain origin
    holding sonnet-5 against the in-force opus-5; mtime correct only by
    coincidence), and compliance with the local-only, non-exported,
    Claude-specific guardrails of the `2026-09-01` and `2026-09-08` decisions.
    Baseline-relative comparison found only the new decision file changed before
    this plan record.
  - Done checks: All satisfied — the ADR exists in repository ADR format with the
    established section structure, contains every AC12 element, and no
    implementation or other context file changed.
  - Context impact: cross-cutting decision — amends the Claude diff-trace
    attribution precedence and accepts a state write on a resolution path;
    context synchronization must reconcile the decision and inspect the mandatory
    root context files before another task starts.
  - Context synchronization: synced

- [x] T05: `Carry ephemeral transcript_path through DiffTracePayload` (status:done)
  - Task ID: T05
  - Scope: In — add a `#[serde(skip)]` `transcript_path: Option<String>` field to
    `DiffTracePayload` in `cli/src/services/hooks/mod.rs`, populate it at Claude
    structured parse time from the raw event's `transcript_path` exactly as
    `agent_id` is populated, leave it `None` for every non-Claude producer, and add
    the ephemerality regression. Out — any use of the field in resolution or
    persistence, any selection or discovery change.
  - Dependencies: T04
  - Done when: the field is carried to the persistence boundary, absent from
    `diff_traces` and every serialized payload, `None` for OpenCode/Pi/Codex inputs,
    and AC9's regression passes with no behavior change to attribution.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_diff_trace`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_model_attribution`.
  - Completed: 2026-09-10
  - Files changed: `cli/src/services/hooks/mod.rs`
  - Result: Added a `#[serde(skip)] transcript_path: Option<String>` field to
    `DiffTracePayload`, populated from the raw event's `transcript_path` at Claude
    structured parse time (via `non_empty_string`, mirroring `agent_id`), left
    `None` in the OpenCode/Pi normalized branch and all test constructors. No
    resolution or persistence code reads the field yet. Added three regressions:
    the field is carried on a Claude structured parse, absent from the serialized
    payload, `None` when the raw field is missing, and `None` for a normalized
    OpenCode payload.
  - Verify: `claude_diff_trace` -> exit 0 (6 passed, 0 failed, incl. the 3 new
    regressions); `claude_model_attribution` -> exit 0 (3 passed, 0 failed,
    unchanged).
  - Done checks: All satisfied — the field reaches the persistence boundary as an
    in-memory-only value, `#[serde(skip)]` keeps it out of every serialized
    payload and `diff_traces`, non-Claude producers get `None`, and the
    attribution suite is unchanged.
  - Context impact: local — adds an unused ephemeral carrier field consumed by a
    later task; no user-visible behavior, interface, schema, or terminology
    change. Root-context pass still required before the next task.
  - Context synchronization: synced

- [ ] T06: `Return all bridge-linked chain members from discovery` (status:todo)
  - Task ID: T06
  - Scope: In — in `cli/src/services/hooks/claude_bridge_session.rs`, add a fail-open
    function returning every sibling `.jsonl` session ID sharing the transcript's
    `bridgeSessionId` (bounded leading-record reads, self excluded, deterministic
    order), with tests for the multi-member, single-member, and no-member cases plus
    the existing failure branches. Out — any selection-by-state logic, any DB access,
    any call-site rewiring.
  - Dependencies: T05
  - Done when: chain members are returned for real-shaped multi-member fixtures,
    failure branches still return an empty result fail-open, and reads remain bounded
    with a regression proving records past the leading-record limit are not scanned.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_bridge_session`.
  - Context synchronization: pending

- [ ] T07: `Seed and resolve Claude model state on the diff-trace state miss` (status:todo)
  - Task ID: T07
  - Scope: In — add a shared newest-chain-observation resolver (chain members from
    T06, one exact-scope state read per member, winner by greatest `observed_at_ms`
    with a deterministic tie-break) and wire it into the diff-trace persistence flow
    in `cli/src/services/hooks/mod.rs`: when the existing exact-scope state read
    misses for a raw structured Claude payload that carries T05's `transcript_path`,
    resolve the chain, persist a `claude_model_state` row for the current session with
    `source="bridge_inherited"` through the existing guarded write path, and use that
    model for the trace in hand. Every step falls through to today's `NULL` on
    failure. Out — changing the `SessionStart` call site, removing the superseded
    single-sibling picker, any schema or export change.
  - Dependencies: T06
  - Done when: AC6, AC7, AC8, and AC10 all hold, and the existing
    `claude_model_state`, `claude_model`, `claude_bridge_session`, and
    `claude_model_attribution` suites pass unchanged alongside the new coverage.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_model_attribution`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_model`; `nix flake check`.
  - Context synchronization: pending

- [ ] T08: `Point SessionStart at the shared selection and drop the superseded picker` (status:todo)
  - Task ID: T08
  - Scope: In — switch the model-less `SessionStart` bridge path in
    `cli/src/services/hooks/claude_model_state.rs` to T07's shared
    newest-chain-observation resolver, remove the now-unused mtime-newest
    `find_claude_bridge_sibling_session_id` picker and its tests, and keep the
    `SessionStart` attempt itself in place. Out — any behavior change to the
    diff-trace path, any change to `PostModelSwitch`.
  - Dependencies: T07
  - Done when: exactly one selection rule exists in the code, both call sites use it,
    AC11 holds, and the `claude_model_state` suite passes with its bridge coverage
    updated to the shared rule.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_model_state`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_bridge_session`; `nix flake check`.
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

Second phase:

- The realized cost of this bug today is zero. All three observed `/clear` sessions
  wrote no diff traces at all (single-turn `hello` tests with no file edits), and the
  638 `NULL` Claude `diff_traces.model_id` rows are overwhelmingly July/early-August
  history predating `claude_model_state` entirely. So T04–T08 finish a shipped-inert
  feature rather than stop active data loss. That is still worth doing — the next
  cleared session that edits a file loses its attribution silently — but if something
  else is competing for the same time, this is a defensible thing to defer.
- The selection change (T06–T08) fixes a failure that has never actually occurred:
  the mtime-newest rule was correct in all three real cases. It is planned here
  because T07 is already reading state per candidate, making the marginal cost one
  read per chain member on a path that runs once per session. If that reasoning does
  not convince, the smaller version is T04, T05, and a T07 that keeps the existing
  single-sibling pick — the timing fix alone, which is what makes inheritance fire at
  all.
- `fix-direction.md` at the repository root is an uncommitted working note holding
  this phase's full investigation, and T04's decision record will own the durable
  version of it. Whether it should be deleted, or moved under `context/`, is
  unresolved and does not block implementation.

## Validation Report

**Status:** validated
**Date:** 2026-09-08

### Commands run

- `nix run .#pkl-check-generated` -> exit 0 (ephemeral Pkl generation passed: 141 files)
- `nix flake check` -> exit 0 (all checks passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_bridge_session` -> exit 0 (5 passed, 0 failed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_model_state` -> exit 0 (16 passed, 0 failed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_model` -> exit 0 (22 passed, 0 failed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_model_attribution` -> exit 0 (3 passed, 0 failed)

### Success-criteria verification

- [x] AC1: Model-less `SessionStart` inherits the sibling model and persists `source="bridge_inherited"` -> persisted-row regression passed in `claude_model_attribution_bridge_inheritance_seeds_state_and_diff_trace`.
- [x] AC2: Discovery and state-missing/error branches fail open without output or destructive state changes -> focused model-state and bridge-session failure-path tests passed; implementation inspection confirmed missing/unreadable/malformed/unmatched inputs and missing sibling state return without writes.
- [x] AC3: Discovery is bounded/local-only and attribution precedence plus existing model suites remain unchanged -> bounded-reader regression passed, helper uses `take(MAX_LEADING_RECORDS)`, and `claude_bridge_session`, `claude_model`, and `claude_model_attribution` suites passed.
- [x] AC4: Inherited state supplies diff-trace model attribution -> persisted-row regression passed with `diff_traces.model_id=claude/inherited-model`.
- [x] AC5: Required production evidence, mechanism, caveat, and guardrails are documented -> inspected `context/decisions/2026-09-08-claude-bridge-session-model-inheritance.md`.

### Failed checks and follow-ups

- None.

### Residual risks

- Bridge inheritance remains best-effort and may inherit a stale model if a model switch races the first tool call.
