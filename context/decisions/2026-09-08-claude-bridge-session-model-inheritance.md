# Decision: Inherit Claude model state across bridge-linked sessions

Date: 2026-09-08
Status: Accepted
Plan: `context/plans/claude-clear-session-model-inheritance.md`
Task: T01

## Context

Real Claude Code hook traffic confirmed that `/clear` starts a new session with
a new `session_id`, while its `SessionStart` payload omits `model`. The captured
model-less `/clear` payload still included `transcript_path`. A recursive scan
of the captured hook payloads also confirmed that `bridgeSessionId` is absent
from the hook payload shape; it is available in the transcript instead.

The leading bridge-session record in each inspected transcript contained a
`bridgeSessionId`. Real sibling transcript pairs across `/clear` boundaries
were confirmed to share that identifier, including a model-bearing startup
session and a later model-less clear session. The sibling session's existing
`claude_model_state` therefore provides a local, best-effort source for seeding
the new session when the lifecycle payload has no model.

## Decision

When a model-less `SessionStart` has a readable `transcript_path`, inspect only
the leading transcript records for its `bridgeSessionId`. In the same Claude
project directory, inspect the leading records of sibling `.jsonl` transcripts
and select the most recently modified other transcript sharing that bridge ID.
When that sibling has an existing exact-scope `claude_model_state` row, seed the
current session with the sibling's model using
`source="bridge_inherited"` and the existing `SessionStart` observation kind.

Bridge discovery is local-only, bounded, and fail-open. A missing or unreadable
transcript, absent or malformed bridge record, missing matching sibling, or
missing sibling state leaves the existing silent no-op behavior unchanged.
`bridgeSessionId` is used only for transient discovery and is not persisted.

## Rationale

This addresses the deterministic `/clear` shape that otherwise leaves the new
session without a model-state seed, while preserving the existing state table,
exact-scope lookup, and write path. It uses the transcript signal Claude
actually emits without depending on network access, a full transcript scan, or
a generic cross-editor session cache.

The most recently modified matching sibling is a deterministic local choice,
but it is not proof of Claude's causal session order. A clear followed by a
model switch before the first tool call can consequently inherit the previous
model. This changes some failures from unknown to plausibly attributed and is
accepted as a documented best-effort trade-off.

## Alternatives considered

- **Keep the model-less `SessionStart` as a silent no-op** — rejected because
  real `/clear` sessions then remain unattributed for their entire lifetime
  unless a later `PostModelSwitch` supplies state.
- **Scan the complete transcript or wait for transcript convergence** —
  rejected because it violates the bounded, minimal-work, fail-open hook
  boundary and still cannot establish causal ordering.
- **Persist `bridgeSessionId` or restore a generic session cache** — rejected
  because the correlation is Claude-specific and local, and broadening the
  shared persistence/export model is unnecessary.

## Compatibility and risks

- The fallback applies to model-less `SessionStart` events regardless of their
  source; narrowing it to `source="clear"` would leave other model-less shapes
  uncovered without a stated benefit.
- The sibling session ID is taken from the transcript filename stem, matching
  the existing session/transcript naming convention.
- Filesystem races, malformed records, and database read failures remain
  fail-open and preserve the existing no-op contract.
- The fallback cannot guarantee correctness when a user clears and switches
  models before any tool call; no upstream ordering signal is available.

## Guardrails

- Keep the mechanism Claude-specific and local-only.
- Do not restore `session_models` or introduce a generic cross-editor
  session-level attribution abstraction.
- Do not persist, export, synchronize, or expose `bridgeSessionId` or
  `claude_model_state` through the control plane.
- Preserve exact `(session_id, agent_id)` state scoping and do not alter the
  existing direct > exact transcript > exact state > `NULL` attribution
  precedence.
- Do not change `PostModelSwitch`, which already carries the model needed for
  its own observation.

These guardrails remain consistent with the accepted
`2026-09-01-claude-model-attribution-state` decision.

## Consequences

New Claude sessions created by `/clear` can receive a local model-state seed
before their first tool call, improving diff-trace attribution without a schema
or export change. Some sessions may receive a stale-but-plausible previous
model when a model switch races the first tool call. Existing failure branches
remain silent, zero-stdout, and non-fatal.

## Follow-up

T02 implements bounded bridge-session discovery. T03 wires inheritance into the
model-less `SessionStart` path and adds persisted-row attribution regression
coverage.

## References

- Plan: [`claude-clear-session-model-inheritance`](../plans/claude-clear-session-model-inheritance.md)
- Existing model-state decision: [`Claude latest model state`](2026-09-01-claude-model-attribution-state.md)
