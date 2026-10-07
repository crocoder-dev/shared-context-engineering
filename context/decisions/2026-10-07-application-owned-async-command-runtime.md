# Decision: Application-owned async command runtime

Date: 2026-10-07
Status: Accepted
Plan: `context/plans/cli-single-tokio-runtime-pr1.md`
Task: `T02`

## Context

Auth and sync previously drove async HTTP through their own current-thread
Tokio runtimes. Activating an application runtime while retaining those bridges
would nest runtimes. The synchronous Turso adapter separately owns runtimes;
its isolated `block_on` bridge protects reentry but runtime destruction still
requires a blocking region.

## Decision

Drive commands through directly awaited static dispatch under one
application-level multi-thread Tokio runtime, retaining synchronous Turso
adapter ownership until the separate PR2 persistence migration.

## Rationale

The connected app/auth/sync migration establishes one async execution owner
without erasing concrete capabilities or requiring borrowed command futures
to be `Send` or `'static`. Caller-side lifetime scopes accommodate existing
persistence without changing its APIs or policies.

## Alternatives considered

- **Activate the entrypoint before migrating commands** — nests auth/sync runtimes.
- **Convert persistence in the same change** — exceeds the PR1 migration boundary.
- **Box or spawn command futures** — obscures static dispatch and constrains
  borrowed writers/context and non-`Send` sync progress unnecessarily.
- **Wrap every synchronous command in blocking scopes** — hides the specific
  DB lifetimes requiring compatibility handling.

## Compatibility and risks

- CLI output, error classification, refresh, sync and foreground-hook contracts
  remain unchanged.
- DB-capable Setup, Doctor and Hooks construct and destroy owning values inside
  `block_in_place`; context-only Setup and DB-free hooks execute directly.
- Sync constructs storage in a blocking scope and owns it in a cleanup guard
  whose blocking Drop runs on normal completion, error, cancellation or unwind.
- Credential operations cross awaited `spawn_blocking` boundaries.
- Generic async telemetry deliberately loses object safety; concrete associated
  capability dispatch remains supported. Future telemetry must attach subscriber
  scope to polling rather than hold a thread-local guard across awaits.

## Guardrails

- Keep synchronous parsing, rendering, lifecycle providers and ordinary command
  implementations; introduce no blanket blocking dispatch or detached command.
- Leave DB runtime construction, isolated bridging and persistence policies intact.
- Retain borrowed dependencies until directly awaited command completion.

## Consequences

- Auth and sync own no production command runtime or `block_on` bridge.
- One application-level runtime does not mean one runtime object process-wide.
- Multi-thread Tokio is required for the temporary DB lifetime scopes.

## Follow-up

- T03/T04 establish focused auth, telemetry, output and cleanup regressions;
  T05 reviews staged architecture documentation.
- PR2 converts persistence and removes DB-owned runtimes, bridging and DB-related
  blocking scopes together; it is not implemented by PR1.

## References

- Plan: [CLI single Tokio runtime — PR1](../plans/cli-single-tokio-runtime-pr1.md)
- Task: `T02`
- Current-state context: [Architecture](../architecture.md),
  [Capability traits](../cli/capability-traits.md),
  [Observability](../sce/cli-observability-contract.md),
  [Shared Turso adapter](../sce/shared-turso-db.md)
- Evidence: [Entrypoint](../../cli/src/main.rs),
  [Static dispatch](../../cli/src/services/command_registry.rs),
  [Sync orchestration](../../cli/src/services/sync/sync.rs)
- Related decision: [Historical CLI refactor draft](cli-refactor-decisions.md)
