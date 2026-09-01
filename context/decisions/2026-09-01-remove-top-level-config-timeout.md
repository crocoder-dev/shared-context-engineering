# Decision: Remove the unused top-level config timeout surface

Date: 2026-09-01
Status: Accepted
Plan: `context/plans/remove-top-level-config-timeout.md`
Task: `T01`

## Context

The CLI exposed a top-level `timeout_ms` config key, `SCE_TIMEOUT_MS`, and
`--timeout-ms` options for `sce config show` and `sce config validate`, but no
operational runtime path consumed the resolved value. The canonical schema,
Rust config resolver, output contract, and active documentation nevertheless
advertised it. Database retry timeouts and unrelated runtime timeout constants
are independently used and must remain available.

## Decision

Remove the unused top-level timeout configuration surface without introducing a
replacement global timeout setting.

## Rationale

Removing dead configuration prevents users from relying on a setting that has
no operational effect and keeps the schema, CLI, resolver, output, and
documentation contracts aligned. Retaining nested database retry timeout
fields preserves the timeout controls that are actually consumed.

## Alternatives considered

- **Keep the setting for compatibility** — It would continue advertising a
  configuration value with no runtime effect.
- **Replace it with a global operational timeout** — That expands scope into a
  timeout redesign not established by the task.

## Compatibility and risks

- Existing configs using top-level `timeout_ms` and invocations using
  `--timeout-ms` are rejected after this change; the schema and CLI now make
  that removal explicit, while nested retry configuration remains compatible.

## Guardrails

- Do not remove `policies.database_retry.*.timeout_ms` or unrelated auth,
  control-plane, database, and resilience timeout behavior.
- Keep generated schema artifacts ephemeral and document only the active
  configuration contract.

## Consequences

- The supported top-level config key set and `sce config` command surface are
  smaller and no longer imply a configurable global timeout.
- The remaining timeout fields have clearer ownership in retry or operational
  runtime paths.

## Follow-up

- None.

## References

- Plan: [`remove-top-level-config-timeout`](../plans/remove-top-level-config-timeout.md)
- Task: `T01`
- Current-state context: [`CLI Config Precedence Contract`](../cli/config-precedence-contract.md)
- Evidence: [`sce-config-schema.pkl`](../../config/pkl/base/sce-config-schema.pkl)
