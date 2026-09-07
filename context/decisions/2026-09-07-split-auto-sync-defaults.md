# Decision: Split setup and runtime auto-sync defaults

Date: 2026-09-07
Status: Accepted
Plan: `context/plans/update-auto-sync-default-behavior.md`
Task: `T01`

## Context

The repo-local config created by `sce setup` and the runtime resolver previously
shared an implicit `agent_trace.auto_sync` default. The desired rollout needs
newly bootstrapped repositories to opt into post-commit synchronization while
repositories and config layers that omit the key remain conservative. The
existing boolean schema, explicit values, and global-before-local merge are
already established and must remain compatible.

## Decision

`sce setup` writes an explicit `agent_trace.auto_sync: true` in a newly created
repo-local `.sce/config.json`, while the runtime resolver resolves an omitted
`agent_trace.auto_sync` value to `false` with default provenance.

## Rationale

The generated setup file provides an intentional, visible opt-in for new
repositories without changing the behavior of existing repositories that have
no such setting. Explicit configuration remains the sole higher-precedence
input, so global/local precedence and opt-out behavior remain stable.

## Alternatives considered

- **Keep one implicit `true` default everywhere** — Existing repositories would
  continue opting into automatic synchronization without an explicit config
  declaration.
- **Use `false` for setup bootstrap and runtime fallback** — New repositories
  would not receive the requested setup opt-in.

## Compatibility and risks

- Existing config files are left untouched and explicit `true`/`false` values
  retain their current meaning; only omitted runtime values change to `false`.
- The setup payload now contains an additional schema-supported field, and its
  explicit value is covered by setup bootstrap tests.

## Guardrails

- Do not change the schema shape, config precedence, post-commit launcher, or
  synchronization protocol.
- Only a newly created repo-local setup config receives the explicit `true`;
  existing files are never rewritten by bootstrap.

## Consequences

- New repositories created through setup are explicitly opted into automatic
  post-commit synchronization.
- Omitted values in global/local config layers resolve to a disabled runtime
  gate, making the setup-generated declaration the visible opt-in boundary.

## Follow-up

- Update current durable setup and Agent Trace configuration context to state
  the split defaults.

## References

- Plan: [`update-auto-sync-default-behavior`](../plans/update-auto-sync-default-behavior.md)
- Task: `T01`
- Current-state context: [`CLI config precedence contract`](../cli/config-precedence-contract.md)
- Current-state context: [`Automatic Agent Trace synchronization`](../cli/agent-trace-auto-sync.md)
- Evidence: [`setup bootstrap implementation`](../../cli/src/services/setup/mod.rs)
- Evidence: [`runtime resolver implementation`](../../cli/src/services/config/resolver.rs)
