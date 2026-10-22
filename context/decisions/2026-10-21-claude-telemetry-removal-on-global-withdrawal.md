# Decision: Remove SCE-owned Claude telemetry settings only on global consent withdrawal

Date: 2026-10-21
Status: Accepted
Plan: `context/plans/claude-telemetry-consent-reconciliation.md`
Task: `T03`

## Context

`sce setup` installs Claude Code's native OpenTelemetry settings into user-level Claude settings when telemetry is enabled. Before this decision, withdrawing consent left those settings in place. Claude kept sending OTLP payloads to the managed collector without an `Authorization` header, so data still left the machine even though the server rejected it. Claude settings are user-owned files outside the repository, they may also hold user or third-party telemetry, and one user-level file serves every repository on the machine.

## Decision

`sce setup` removes SCE-owned entries from user-level Claude settings only when the global user config itself records a withdrawal (`telemetry.enabled: false` or `auth.mode: disabled`, read with `read_global_access_preferences`). Repository-layer and explicit-layer (`--config` / `SCE_CONFIG_FILE`) opt-outs never modify Claude settings.

Ownership is established by exact-value matching. The anchor is `otelHeadersHelper == "sce auth telemetry-headers"`, and every telemetry key present must equal SCE's written value (`claude_telemetry_conflict == None`). Interactive runs reconcile whatever the target, before the setup login step. Non-interactive runs reconcile only with the Claude target selected.

## Rationale

Under the resolver, a global withdrawal disables SCE telemetry in every repository, and no repository or explicit layer can lift it. Removing user-level settings therefore cannot break another repository. Repository and explicit opt-outs are local, so the `sce auth telemetry-headers` helper already enforces them by failing closed. Exact-value matching with an SCE-specific anchor identifies SCE's writes without new durable state.

## Alternatives considered

- **Remove on any effective disable (including repository or explicit opt-out)** — would disable Claude telemetry that other repositories still rely on.
- **Remove every `OTEL_*` / `CLAUDE_CODE_*` key** — would destroy user or third-party telemetry.
- **Marker file, ownership registry, or database** — adds durable state that drifts from user edits and causes multi-machine and dotfile-sync problems, and still needs value checks.
- **Partial removal from modified settings** — removing shared switches would disable the user's own exporter.

## Compatibility and risks

- Removal re-sorts keys and pretty-prints the file, the same effect install already has.
- A key the user set before install with exactly SCE's value cannot be distinguished from SCE's write and is removed.
- In interactive runs, the removal line appears before the `SCE login` section.
- An older SCE binary may reinstall the settings when it runs with consent enabled.

## Guardrails

- Managed (enterprise) Claude settings are never written. A note line reports when they also configure telemetry.
- Modified, partial, malformed, symlinked, or read-only settings are left byte-identical, with a fixed report line and manual remediation.
- Removal never opens `auth.db` or the credential store and never runs repository authorization or login.
- Cleanup failure is non-fatal and never rolls back the saved global consent.
- `sce auth logout` does not touch Claude settings.

## Consequences

- After a global withdrawal, the next `sce setup` run stops Claude-native export to the managed collector.
- Re-enabling global consent reinstalls the settings through the unchanged install path.
- Repository-level opt-outs keep relying on the helper's fail-closed behavior.

## Follow-up

- None.

## References

- Plan: [`claude-telemetry-consent-reconciliation`](../plans/claude-telemetry-consent-reconciliation.md)
- Task: `T03`
- Current-state context: [`telemetry-credential.md`](../sce/telemetry-credential.md)
- Current-state context: [`access-policy.md`](../cli/access-policy.md)
- Current-state context: [`setup-repo-local-config-bootstrap.md`](../sce/setup-repo-local-config-bootstrap.md)
- Evidence: [`command.rs`](../../cli/src/services/setup/command.rs)
- Evidence: [`claude_telemetry.rs`](../../cli/src/services/setup/claude_telemetry.rs)
- Related decision: [Setup writes access answers to user layer only](2026-10-21-setup-writes-access-answers-to-user-layer-only.md)
- Related decision: [Consent-gated access policy precedence](2026-10-20-consent-gated-access-policy-precedence.md)
