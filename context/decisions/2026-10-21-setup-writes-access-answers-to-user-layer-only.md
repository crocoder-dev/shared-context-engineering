# Decision: Setup writes access-policy answers to the user layer only

Date: 2026-10-21
Status: Superseded
Superseded by: `context/decisions/2026-10-22-setup-writes-access-answers-to-repository-config.md`
Plan: `context/plans/setup-auth-telemetry-consent.md`
Task: `T01, T05`

> Superseded on 2026-10-22 by [Setup writes access-policy answers to the repository config](2026-10-22-setup-writes-access-answers-to-repository-config.md). Interactive `sce setup` now saves `auth.mode` and `telemetry.enabled` to the repository `.sce/config.json`, and only global or explicit values lock the setup prompts. This record is kept as history.

## Context

Interactive `sce setup` asks for the SCE auth mode and for telemetry consent. Those answers must be saved somewhere the resolver reads as a policy layer. SCE credentials (`auth.db` and its OS credential-store key) are per-user and per-machine. The repository `.sce/config.json` is committed and shared with a team. Older SCE binaries skip a whole config file that holds keys they reject. Under the [consent-gated precedence](2026-10-20-consent-gated-access-policy-precedence.md), only the user side can grant telemetry consent, and a repository `true` grants nothing.

## Decision

Interactive `sce setup` writes `auth.mode` and `telemetry.enabled` only to the global user config (`${config_root}/sce/config.json`, the user policy layer). It never writes them to the repository `.sce/config.json` or to an explicit `--config` / `SCE_CONFIG_FILE` file.

How the write behaves:

- It records explicit answers only. A key the user was not asked about stays absent.
- It is atomic: a sibling temp file, then a rename. The existing file mode is kept, and a new file gets `0o600`.
- It uses best-effort optimistic concurrency: the file's bytes are re-read before the rename, and the write is refused if they changed.
- It never overwrites a global file that is invalid.
- It runs before any repository write. If it fails, setup aborts with no repository files changed.

Resolver precedence (`resolve_access_policy`) is unchanged by this decision.

## Rationale

- Consent and the choice to use credentials belong to the person on the machine, which matches where credentials live.
- Writing a restriction into the shared repository file would amount to team-policy authoring through a personal setup run. It could also make older binaries ignore that repository's whole config.
- A user-layer-only write keeps the existing precedence intact. Repository and explicit restrictions (`auth.mode: disabled`, `telemetry.enabled: false`) still win, and setup cannot grant consent through the repository.
- Writing globally before any repository write means a failed consent write leaves nothing half-done.

## Alternatives considered

- **Write the answers to the repository `.sce/config.json`.** Not selected. The file is shared and committed, a repository `true` cannot grant consent, and older binaries reject the keys.
- **Offer a fourth, repository-scoped restriction choice in setup.** Not selected. It would change shared files and break older binaries. The plan keeps this as an open question rather than shipping it.

## Compatibility and risks

- A global `auth.mode: disabled` chosen during setup in one repository applies to every repository on the machine. The setup report states this scope ("applies to all repositories on this machine").
- The concurrency check is optimistic, not a lock. Two writers that race between the re-read and the rename can still collide.
- The first rewrite does not preserve key order in the global file.

## Guardrails

- Setup never writes `auth` or `telemetry` keys into the repository config.
- Setup never lifts a global prohibition silently: a persisted global value suppresses its prompt on reruns.
- Setup never overwrites an invalid global config. When the global config is invalid at read time, the access prompts are skipped.
- The resolver's precedence is not recomputed or duplicated in setup.

## Consequences

- The effective policy that setup reports comes from re-resolution, so a repository opt-out can still disable telemetry after the user consents.
- Unattended consent requires deliberately editing the global config. No non-interactive consent flags exist.

## Follow-up

- None.

## References

- Plan: [`setup-auth-telemetry-consent`](../plans/setup-auth-telemetry-consent.md)
- Task: `T01`, `T05`
- Current-state context: [`access-policy.md`](../cli/access-policy.md)
- Current-state context: [`setup-repo-local-config-bootstrap.md`](../sce/setup-repo-local-config-bootstrap.md)
- Current-state context: [`config-precedence-contract.md`](../cli/config-precedence-contract.md)
- Evidence: [`access_preferences.rs`](../../cli/src/services/setup/access_preferences.rs)
- Evidence: [`command.rs`](../../cli/src/services/setup/command.rs)
- Related decision: [`Consent-gated precedence for the effective access policy`](2026-10-20-consent-gated-access-policy-precedence.md)
