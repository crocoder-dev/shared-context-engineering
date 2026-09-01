# Decision: Install Claude PostModelSwitch registration unconditionally

Date: 2026-09-01
Status: Accepted
Plan: `context/plans/claude-model-attribution-state.md`
Task: `T05`

## Context

SCE needs to register Claude's `PostModelSwitch` lifecycle event so the local
Claude model-state register can observe in-session model changes. The
immediately older Claude Code build must not reject the unknown event in a way
that loses existing SCE hooks or the user's settings file.

A compatibility smoke used a temporary settings file containing an existing
`PreToolUse` registration and an unknown `PostModelSwitch` registration. Claude
Code 2.1.251 (the event-supporting build) and 2.1.250 (the immediately older
build) both reached the expected invalid-API-key termination rather than a
settings/configuration failure, and the settings file remained byte-unchanged.

## Decision

Install the `PostModelSwitch` SCE registration unconditionally alongside
`SessionStart`; do not raise SCE's Claude Code minimum version or capability-gate
the registration.

## Rationale

The pre-event-supporting 2.1.250 build safely tolerates the unknown registration
and preserves the settings file and existing hook configuration. Unconditional
installation keeps the generated configuration deterministic while allowing
older clients to operate in a degraded mode without post-switch updates.

## Alternatives considered

- **Raise the minimum Claude Code version to the first `PostModelSwitch` build**
  — not selected because the immediately older supported build passed the
  compatibility smoke without losing configuration.
- **Capability-gate `PostModelSwitch` installation** — not selected because the
  tested older client safely ignores the unknown event and a capability probe
  would add install-time complexity without improving this compatibility path.

## Compatibility and risks

- The tested Claude Code 2.1.250 and 2.1.251 builds retain the existing SCE
  registrations and user settings when this registration is present; clients
  that do not emit the event remain dependent on `SessionStart` and may have
  stale in-session state.
- The smoke used an invalid API key to stop before network work; it proves
  configuration acceptance and preservation, not model-switch execution.

## Guardrails

- The registration remains an SCE-owned command hook routed through the existing
  helper and `sce hooks claude-model-state` command.
- This decision does not change the lifecycle hook's local-only, fail-open,
  zero-stdout behavior or the model-state synchronization boundaries.

## Consequences

The canonical Claude settings renderer always emits both lifecycle registrations
and setup/doctor treat either missing registration as SCE settings drift. Older
clients without `PostModelSwitch` support continue to work but receive no
post-switch state updates.

## Follow-up

T06 covers end-to-end attribution regression and the remaining upgrade note.

## References

- Plan: [`claude-model-attribution-state`](../plans/claude-model-attribution-state.md)
- Task: `T05`
- Current-state context: [`Agent Trace hook routing`](../sce/agent-trace-hooks-command-routing.md)
- Evidence: [`Claude settings renderer`](../../config/pkl/renderers/claude-content.pkl)
- Related decision: [`Claude-specific latest model state`](2026-09-01-claude-model-attribution-state.md)
