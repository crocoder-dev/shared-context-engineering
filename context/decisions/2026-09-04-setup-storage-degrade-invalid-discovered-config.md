# Decision: Degrade Invalid Default-Discovered Config at Setup and Storage Boundaries

Date: 2026-09-04
Status: Accepted
Plan: `context/plans/setup-degraded-invalid-config-agent-trace.md`
Task: `T01`
Supersedes: `context/decisions/2026-08-26-setup-storage-fail-closed-on-invalid-config.md`

## Context

Ordinary startup already skips invalid default-discovered global or repo-local
configuration layers, preserves the existing `sce.config.invalid_config`
warning, and continues with remaining layers or defaults. Setup and Agent Trace
storage had retained a stricter boundary: setup rejected an invalid local file
before its normal flow, while storage rejected any invalid discovered layer
before repository identity resolution. This divergence prevented setup and
repository-scoped hook tracing from using the same degraded configuration
behavior. Focused resolver, setup, and hook-runtime tests establish that the
invalid local file can remain untouched while valid remaining configuration or
the default remote continues to provide the required values.

## Decision

` sce setup` and Agent Trace hook storage shall skip invalid default-discovered
configuration layers and continue with the shared resolver's remaining-layer or
default values; explicit `--config` and `SCE_CONFIG_FILE` selections remain
fatal.

## Rationale

Using the shared resolver result keeps startup, setup, and hook-runtime
configuration behavior aligned without weakening explicit operator intent.
Setup can complete its Git/remote preflight, lifecycle, and asset flow without
repairing a user's invalid file, and repository-scoped tracing can continue to
the same identity and database path selected by valid remaining configuration
or the default `origin` remote.

## Alternatives considered

- **Keep setup and storage fail-closed** — preserves the previous safety boundary
  but needlessly blocks normal setup and hook tracing when a lower-priority
  discovered layer is invalid.
- **Make explicit configuration degradable too** — would discard an explicit
  operator selection and weaken the fatal configuration contract.
- **Repair invalid local configuration during setup** — would mutate user-owned
  bytes as a side effect and could destroy information needed for manual repair.

## Compatibility and risks

- Ordinary and setup consumers may proceed using a remaining layer or default
  after a discovered configuration failure; the existing warning and validation
  error reporting remain in place.
- Invalid local configuration is intentionally not rewritten, so target and
  optional-workflow persistence may be omitted for that run.
- Genuine Agent Trace database, identity, Git, and remote failures retain their
  existing diagnostics and fail-open behavior.

## Guardrails

- Only default-discovered global and repo-local layers are degradable.
- Explicit `--config` and `SCE_CONFIG_FILE` parse or validation failures remain
  fatal.
- The generated schema, precedence rules, repository identity canonicalization,
  database schema, and no-migration hook opening contract do not change.
- Setup never repairs, deletes, or rewrites an invalid discovered config file.

## Consequences

- `sce setup` can reach its normal Git/remote preflight, bootstrap, lifecycle,
  and requested asset-install flow despite invalid discovered configuration.
- Agent Trace hook-runtime DB opening uses the same degraded repository identity
  inputs as the shared runtime resolver and can persist representative hook data.
- Operators still receive the established invalid-config warning and must repair
  the file separately when they want its settings or setup persistence restored.

## Follow-up

- `/validate` must verify the plan's setup, resolver, Agent Trace storage, hook,
  and explicit-config acceptance criteria and repository-wide checks.

## References

- Plan: [`setup-degraded-invalid-config-agent-trace`](../plans/setup-degraded-invalid-config-agent-trace.md)
- Task: `T01`
- Current-state context: [`CLI config precedence contract`](../cli/config-precedence-contract.md)
- Current-state context: [`SCE setup local bootstrap`](../sce/setup-repo-local-config-bootstrap.md)
- Current-state context: [`Repository-scoped Agent Trace storage resolver`](../cli/agent-trace-storage.md)
- Current-state context: [`Agent Trace hooks command routing`](../sce/agent-trace-hooks-command-routing.md)
- Evidence: [`config resolver`](../../cli/src/services/config/resolver.rs)
- Evidence: [`setup command`](../../cli/src/services/setup/command.rs)
- Evidence: [`setup service`](../../cli/src/services/setup/mod.rs)
- Evidence: [`hooks service`](../../cli/src/services/hooks/mod.rs)
- Related decision: [`Fail Closed at Setup and Agent Trace Storage Boundaries for Invalid Discovered Config`](2026-08-26-setup-storage-fail-closed-on-invalid-config.md)
