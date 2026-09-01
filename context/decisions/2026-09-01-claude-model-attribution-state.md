# Decision: Add Claude-specific latest model state for diff-trace attribution

Date: 2026-09-01
Status: Accepted
Plan: `context/plans/claude-model-attribution-state.md`
Task: T01
Supersedes: The no-session-level-cache constraint for Claude model attribution in `context/plans/remove-session-models-direct-claude-model-id.md` (only that constraint; the historical plan remains unchanged)

## Context

Production data shows that most Claude `diff_traces` rows have a `NULL`
`model_id`. Claude `PostToolUse` events do not normally expose model identity,
and the existing event-local transcript fallback is often unable to help because
Claude writes the transcript asynchronously and it may not yet contain the
matching assistant record when the hook runs. Direct metadata and exact
transcript lookup therefore remain useful, but they cannot close the attribution
gap at hook time.

Claude now provides two lifecycle signals that can supply local context:
`SessionStart.model` and `PostModelSwitch` with `from_model`, `to_model`, and a
source such as `command`, `picker`, `sdk`, `auto`, or `resume`. The hook contract
does not provide an authoritative upstream sequence number or event timestamp,
and it provides no synchronization barrier between a lifecycle hook and later
`PostToolUse` hooks.

The former generic `session_models` abstraction was deliberately removed. Its
removal simplified the cross-editor data model, but restoring it would recreate
a broad session-level architecture for a problem specific to Claude's lifecycle
signals.

## Decision

Add a small Claude-specific latest-model-state register to each existing
repository-scoped Agent Trace DB. The `claude_model_state` register is keyed by
canonical `(session_id, agent_id)`, with the empty agent ID representing the
main conversation. A model-bearing `SessionStart` records normalized
`claude/<model>` state, and `PostModelSwitch` records normalized `to_model`
state after the local hook write completes. `from_model` is validated and may
inform diagnostics, but it is not a compare-and-swap precondition.

Diff-trace attribution uses this state only as a final local fallback, preserving
this precedence:

1. direct Claude event metadata;
2. an exact `tool_use_id` match in the event's transcript;
3. current Claude state for the exact canonical session and agent scope;
4. `NULL`.

The state is local-only. It is not exported, synchronized, exposed through a
control-plane endpoint, or added to the Agent Trace export streams. The durable
exported attribution remains only `diff_traces.model_id`; ephemeral Claude
`agent_id` context is used for exact local lookup and is not persisted in that
schema or sent to the control plane. There is no historical backfill.

`observed_at_ms` means the local SCE time at which the hook observed the event,
not Claude's authoritative causal event time. The register is therefore a
best-effort latest-locally-observed register, not an event log and not a proof
of switch order. Strictly older local observations cannot overwrite newer ones.
PostModelSwitch wins equal-timestamp conflicts against SessionStart, and equal
timestamps within one observation kind use a stable deterministic tie-breaker
rather than arrival order. Replayed identical observations are idempotent.

## Rationale

A Claude-specific register addresses the observed timing gap without changing
the shared Agent Trace schema or reintroducing a generic cross-editor session
model. Seeding at `SessionStart` covers initial attribution, while
`PostModelSwitch.to_model` updates local state after an in-session switch. Exact
agent scoping prevents a subagent without its own state from inheriting the
parent conversation's model.

The local observation-time guard is the strongest deterministic protection
available from this hook contract. It intentionally does not claim to recover
Claude's causal ordering, because no upstream sequence/timestamp metadata or
synchronization barrier is available to SCE.

## Alternatives considered

- **Restore the generic `session_models` table and command** — rejected: it
  would revive a retired cross-editor abstraction for a Claude-specific
  lifecycle problem and broaden the persistence/export surface unnecessarily.
- **Keep direct and transcript attribution only** — rejected: production `NULL`
  rates and asynchronous transcript visibility show that event-local lookup
  alone is insufficient.
- **Treat `observed_at_ms` as Claude event time or require strict causal ordering**
  — rejected: SCE observes hook delivery locally and Claude exposes neither an
  authoritative sequence/timestamp nor a synchronization barrier through this
  contract.
- **Poll, sleep, or wait for state convergence before persisting a diff trace**
  — rejected: this would make a high-frequency hook depend on another process,
  violate the fail-open/minimal-work boundary, and still would not establish
  causal ordering.

## Compatibility and risks

- Claude invokes `SessionStart` synchronously relative to Claude's execution
  and `PostModelSwitch` asynchronously relative to Claude's execution. In both
  SCE handlers, the local database write is performed directly before the hook
  process exits. A post-switch visibility race remains: Claude may continue
  before the asynchronous PostModelSwitch hook process has completed its local
  write. That trace can observe stale state or `NULL`; later traces can use the
  new state. SCE does not spawn, detach, background, or defer the
  `claude_model_state` write, and does not poll, retry, or delay persistence to
  eliminate this race.
- Rapid consecutive switches can launch overlapping lifecycle hooks. The
  local-observation guard rejects older observations and resolves equal-time
  conflicts deterministically, but cannot prove Claude's causal order. Only an
  upstream sequence number or authoritative event timestamp could provide that
  guarantee.
- The state fallback is best-effort and cannot detect a one-turn fallback-chain
  model substitution when Claude exposes that substitution neither through
  lifecycle state nor through the exact transcript lookup.
- Clients that do not emit `PostModelSwitch` remain seeded by `SessionStart` and
  can become stale after an in-session model switch; compatibility policy is
  handled by the lifecycle-hook installation task.

## Guardrails

- Do not restore `session_models` or add a generic session-level attribution
  abstraction.
- Do not export, sync, or add control-plane schema for `claude_model_state`.
- Do not add `agent_id` to `diff_traces`, Agent Trace payloads, or related
  external interfaces.
- Do not claim that local observation time proves Claude's event order.
- Keep direct and exact-transcript attribution ahead of state, and keep
  subagent lookups isolated to their exact `(session_id, agent_id)` pair.

## Consequences

Claude diff traces gain a local state fallback for model attribution while
unresolved cases remain nullable. Repository setup gains one additive local
migration. Claude invokes SessionStart synchronously relative to Claude's
execution and PostModelSwitch asynchronously relative to Claude's execution. In
both SCE handlers, the local database write is performed directly before the
hook process exits. The existing export boundary, sync streams, Control Plane
protocol, and historical plans remain unchanged.

The design accepts two upstream timing limitations: a post-switch trace can
arrive before the state write is visible, and overlapping switches cannot be
ordered beyond the deterministic local-observation guard. These are explicit
best-effort semantics rather than hidden correctness claims.

## Follow-up

T02–T06 implement and verify the register, silent hook intake, persistence
fallback, lifecycle-hook registration, end-to-end regressions, and current-state
context synchronization. The compatibility smoke in T05 must record whether
unsupported older Claude Code builds require a raised minimum version or a
capability-gated `PostModelSwitch` registration.

## References

- Plan: [`claude-model-attribution-state`](../plans/claude-model-attribution-state.md)
- Historical constraint: [`remove-session-models-direct-claude-model-id`](../plans/remove-session-models-direct-claude-model-id.md)
- Related Claude attribution behavior: [`fix-claude-model-attribution`](../plans/fix-claude-model-attribution.md)
