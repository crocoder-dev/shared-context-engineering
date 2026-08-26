# Decision: Fail Closed at Setup and Agent Trace Storage Boundaries for Invalid Discovered Config

Date: 2026-08-26
Status: Accepted
Plan: `context/plans/setup-invalid-config-and-git-locale.md`
Task: `T01`

## Context

Default-discovered configuration remains intentionally degradable for ordinary
startup so commands can continue with defaults and report validation issues.
That behavior is unsafe at two boundaries: setup can create databases, hooks,
context, and target assets, while Agent Trace storage resolution can select a
repository database from a fallback remote after discarding an invalid config
layer. The completed T01 implementation and focused Rust tests establish that
these boundaries must use the existing config validation seam before proceeding.

## Decision

`sce setup` and Agent Trace storage runtime configuration resolution fail closed
when a discovered config file is invalid, while general startup config
consumers retain their existing degraded-default behavior.

## Rationale

Setup must not perform side effects from an invalid repository configuration,
and storage identity must not silently select a potentially different database.
Keeping the stricter behavior at these two consumers preserves the established
startup compatibility contract without weakening repository safety.

## Alternatives considered

- **Continue with remaining layers and defaults everywhere** — preserves the
  existing resolver behavior but permits setup side effects and wrong storage
  identity selection.
- **Make all default-discovered config consumers fatal** — avoids degradation
  but broadens the user-visible startup contract beyond the required boundary.

## Compatibility and risks

- Existing startup, inspection, and observability consumers continue to skip
  invalid discovered layers and report validation errors; setup and Agent Trace
  storage now return actionable validation failures instead. No schema or
  precedence semantics change.

## Guardrails

- Validate only an existing repo-local config during setup, leaving absent-file
  bootstrap behavior unchanged.
- Reuse the existing generated-schema and typed-config validation seam.
- Keep the strict storage check limited to invalid discovered config layers;
  explicit identity, remote precedence, and repository canonicalization remain
  unchanged.

## Consequences

- Invalid repo-local config cannot trigger setup prompts, context bootstrap,
  lifecycle initialization, hooks, or target asset installation.
- Agent Trace storage resolution no longer returns fallback identity values when
  a discovered config layer failed validation.
- Operators must repair invalid config before rerunning setup or storage-backed
  Agent Trace operations.

## Follow-up

- `T02` continues the same plan's independent Git locale-stability change.

## References

- Plan: [`setup-invalid-config-and-git-locale`](../plans/setup-invalid-config-and-git-locale.md)
- Task: `T01`
- Current-state context: [`CLI config precedence contract`](../cli/config-precedence-contract.md)
- Current-state context: [`SCE setup local bootstrap`](../sce/setup-repo-local-config-bootstrap.md)
- Current-state context: [`Repository-scoped Agent Trace storage resolver`](../cli/agent-trace-storage.md)
- Evidence: [`config resolver`](../../cli/src/services/config/resolver.rs)
- Evidence: [`setup command`](../../cli/src/services/setup/command.rs)
