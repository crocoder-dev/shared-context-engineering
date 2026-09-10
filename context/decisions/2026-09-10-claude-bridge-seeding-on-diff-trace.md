# Decision: Seed Claude model state from the bridge chain on the diff-trace read path

Date: 2026-09-10
Status: Accepted
Plan: `context/plans/claude-clear-session-model-inheritance.md`
Task: T04

## Context

The `2026-09-08-claude-bridge-session-model-inheritance` decision placed
bridge-linked model inheritance on the model-less `SessionStart` path. That
mechanism shipped, was validated by unit and persisted-row regressions, and is
inert in production: no `claude_model_state` row has ever been written with
`source="bridge_inherited"` (live repository database: `picker|3`, `startup|6`,
nine rows across two sources).

The cause is not the discovery logic. The deployed binary contains the feature,
the hook is registered for all `SessionStart` sources with no matcher, every real
bridge record sits at transcript line 2–3 well inside `MAX_LEADING_RECORDS = 16`,
real siblings share `bridgeSessionId` correctly, and each observed `/clear`
session had a bridge-linked sibling holding state.

**Claude creates a session's `.jsonl` transcript lazily, after the `SessionStart`
hook has already run and exited.** Measured on one real session: `2a56e9bc` wrote
its `claude_model_state` row at 07:56:03Z, and its transcript file did not exist
until 07:56:16Z — 13 seconds later, on the same code path. `transcript_path` is
present on the payload exactly as the prior decision recorded, but it names a file
that does not exist yet. `File::open` returns `ENOENT`, bridge-session extraction
fails open to `None`, and the inheritance candidate resolves to `Ok(None)` before
any sibling scan or database read. Unit tests pass because every fixture
pre-creates the transcript, and the prior acceptance criteria explicitly bless the
missing-transcript branch as correct fail-open behavior — in production it is the
only branch ever taken. `SessionStart` is therefore structurally unable to read its
own transcript, and no amount of discovery hardening changes that.

Two further findings from the same investigation shape the selection rule:

- Transcript records past the header carry a snake_case `session_id` alongside the
  camelCase `sessionId`, pointing at the chain's **origin** session. This was
  confirmed across both observed chains and a fresh session, which points at
  itself. The origin is nevertheless the wrong member to inherit from: for chain
  member `2e1257fa` the origin `2a56e9bc` held sonnet-5, while the model actually
  in force was opus-5, set by an intervening `PostModelSwitch`.
- Symmetrically, "most recently modified sibling transcript" — the prior
  decision's rule — is only a proxy for "most recent model observation". It
  happened to be correct in all three observed cases by coincidence of mtime
  ordering, and nothing guarantees that ordering.

The filesystem is the right authority for chain *membership*; the database is the
right authority for which member's model *wins*.

## Decision

Move bridge-derived model-state seeding from `SessionStart` to the diff-trace
persistence path, which already reads `claude_model_state` and already holds the
event's `transcript_path` at a point where the transcript is guaranteed to exist.

When the existing exact-scope `claude_model_state` read misses for a raw
structured Claude diff-trace payload that carries a `transcript_path`:

1. Resolve every bridge-linked chain member from the transcript's
   `bridgeSessionId` using bounded leading-record reads of sibling `.jsonl` files
   in the same Claude project directory, excluding self.
2. Perform one exact-scope `claude_model_state` read per chain member and select
   the **newest observation by `observed_at_ms`**, with a deterministic tie-break.
   Neither the chain origin nor the newest transcript file decides the winner.
3. Persist a `claude_model_state` row for the current session with
   `source="bridge_inherited"` through the existing guarded write path, and use
   that model for the trace in hand.

Because the seed is written on the first miss, discovery runs at most once per
session; the second and later diff traces of that session resolve from the
session's own exact-scope state with no repeated discovery and no second write.

This amends the Claude diff-trace attribution precedence to:

`direct > exact transcript > exact state > bridge-derived chain state > NULL`

A resolution path consequently **writes** state. That is a deliberate departure
from the previous read-only character of diff-trace attribution resolution: the
write is what bounds the cost of the new precedence tier to one discovery per
session, and it targets the same exact-scope `(cc_<session_id>, agent_id)` row the
`SessionStart` path would have written.

`SessionStart`'s bridge attempt is kept rather than removed. It costs one failed
`File::open` and begins working unchanged if Claude ever creates transcripts
eagerly. Both call sites use one shared selection rule; the superseded
mtime-newest single-sibling picker is removed so no second rule remains in the
code.

Every step fails open. An absent `transcript_path`, a missing or unreadable
transcript, an absent or malformed bridge record, no other chain member, no member
state, and any database read or write failure each leave `diff_traces.model_id` as
it would have been, write no state row, keep the hook successful, and emit zero
stdout. `transcript_path` is carried to this resolution as an ephemeral,
non-serialized field: it is never written to `diff_traces`, any other column, or
any exported payload, and `bridgeSessionId` remains transient discovery input that
is never persisted.

This supersedes the trigger point and the selection rule of the
`2026-09-08-claude-bridge-session-model-inheritance` decision. That decision's
evidence, its bounded and fail-open discovery contract, and its guardrails stand
unchanged and are not edited.

## Rationale

Attribution can only be inherited at a moment when the signal it depends on
exists. The measured 13-second transcript-creation lag makes `SessionStart` that
wrong moment by construction, while the diff-trace path is both guaranteed to have
the transcript and the exact point where a missing model actually costs something.

Selecting the newest observation across chain members replaces two proxies —
chain origin, and sibling file mtime — with the quantity the attribution actually
wants: the most recent model Claude was observed using anywhere in this chain. The
origin proxy is demonstrably wrong (sonnet-5 vs. the in-force opus-5); the mtime
proxy is unfalsified but unprincipled. The marginal cost of the correct rule is one
exact-scope read per chain member on a path that runs once per session.

## Alternatives considered

- **Harden `SessionStart` discovery (retry, wait for the transcript, poll)** —
  rejected. It cannot fix a lag measured in seconds without blocking the hook,
  which violates the bounded, minimal-work hook boundary.
- **Keep seeding on `SessionStart` and accept the inert feature** — rejected. It
  leaves every cleared session that edits a file silently unattributed, which is
  the whole gap the plan targets.
- **Resolve without writing (pure read-path resolution per trace)** — rejected.
  It repeats chain discovery and per-member state reads on every diff trace of the
  session instead of once, for no correctness gain.
- **Inherit from the chain origin named by the transcript's snake_case
  `session_id`** — rejected on direct evidence: the origin held sonnet-5 while the
  model in force was opus-5.
- **Keep the mtime-newest sibling rule** — rejected as a proxy that is correct
  only by coincidence, in favor of the observation the database already records.
- **Persist `bridgeSessionId`, export chain state, or restore a generic
  cross-editor session cache** — rejected, unchanged from the `2026-09-01` and
  `2026-09-08` decisions.

## Compatibility and risks

- Inheritance remains a probabilistic, best-effort guess with no upstream causal
  ordering guarantee. A session that clears and switches models before its first
  tool call inherits the previous model and is attributed to it instead of staying
  `NULL`. The newest-observation rule narrows this window relative to the mtime
  rule but does not close it.
- Attribution resolution now performs a write on one branch. It is exact-scope,
  guarded by the existing write path, and bounded to one occurrence per session.
- Chain discovery cost scales with the number of bridge-linked members, one
  bounded leading-record read and one exact-scope state read each, incurred once
  per session on a state miss.
- No schema, migration, export, sync, or control-plane change. `PostModelSwitch`
  is untouched; it always carries its own model.
- Non-Claude producers (OpenCode, Pi, Codex) are unaffected: the ephemeral
  `transcript_path` field stays `None` and no new branch is reachable for them.

## Guardrails

- Keep the mechanism Claude-specific and local-only: bounded leading-record reads,
  no full-transcript scan, no network access.
- Do not restore `session_models` or introduce a generic cross-editor
  session-level attribution abstraction.
- Do not persist, export, synchronize, or expose `bridgeSessionId`,
  `transcript_path`, or `claude_model_state` through the control plane.
- Preserve exact `(session_id, agent_id)` state scoping; subagent isolation is
  unchanged.
- Keep every branch fail-open with zero stdout, hook success, and no exit 2.
- Maintain exactly one chain-selection rule shared by both call sites.
- Do not change `PostModelSwitch`.

These remain consistent with the `2026-09-01-claude-model-attribution-state` and
`2026-09-08-claude-bridge-session-model-inheritance` decisions; this record amends
only the attribution precedence, the trigger point, and the selection rule those
decisions established, and adds the read-path write as an explicitly accepted
exception to their otherwise read-only resolution contract.

## Consequences

Cleared Claude sessions receive model attribution at the moment they first produce
a diff trace, which is the first moment attribution matters and the first moment
the transcript reliably exists. Attribution precedence gains a documented
bridge-derived tier below exact state, and diff-trace resolution gains a single,
bounded, exact-scope state write. Some sessions may still be attributed a
stale-but-plausible previous model. All existing failure branches remain silent,
zero-stdout, and non-fatal.

## Follow-up

T05 carries `transcript_path` through `DiffTracePayload` as an ephemeral field.
T06 returns all bridge-linked chain members from discovery. T07 adds the shared
newest-chain-observation resolver and wires seeding into the diff-trace state
miss. T08 points `SessionStart` at the shared selection and removes the superseded
mtime-newest picker.

## References

- Plan: [`claude-clear-session-model-inheritance`](../plans/claude-clear-session-model-inheritance.md)
- Prior bridge-inheritance decision: [`Inherit Claude model state across bridge-linked sessions`](2026-09-08-claude-bridge-session-model-inheritance.md)
- Existing model-state decision: [`Claude latest model state`](2026-09-01-claude-model-attribution-state.md)
