# Decision: Consent-gated precedence for the effective access policy

Date: 2026-10-20
Status: Accepted
Plan: `context/plans/auth-telemetry-policy-config.md`
Task: `T02`

## Context

Before this decision, credential state alone decided whether SCE exported telemetry or used cloud access. Once a user logged in, a telemetry credential was provisioned and Claude hooks exported spans. No setting let a user opt out of telemetry while staying logged in, forbid authentication on a machine, or stop a repository from turning these features on. The new `auth.mode` and `telemetry.enabled` config keys can appear in both the global and the repo-local `sce/config.json`. The ordinary per-key merge (last layer wins) would let a repository enable telemetry without the user's consent, or override a user's prohibition.

## Decision

SCE resolves one effective access policy from two config layers, using consent-gated precedence.

**Layers:**
- The user layer is the default-discovered global config, or the single explicit `--config` / `SCE_CONFIG_FILE` file.
- The repository layer is the default-discovered `<cwd>/.sce/config.json`.
- Neither key has an environment-variable or flag layer.

**`auth.mode`:** the most restrictive layer wins. The order is `disabled > mandatory > optional`, and the default is `optional`. When both layers agree, the repository layer is recorded as the source.

**`telemetry.enabled`:** the first matching rule applies:
1. Any skipped invalid discovered layer → `Disabled(InvalidConfig)`.
2. Effective auth is `disabled` → `Disabled(AuthDisabled)`.
3. User layer is `false` → `UserOptOut`.
4. Repository layer is `false` → `RepositoryOptOut`.
5. User layer is `true` → `Enabled`.
6. Repository layer is `true` with no user `true` → `NoUserConsent`.
7. Otherwise → `NotEnabled`.

**Background cloud operations** are allowed only when auth is not `disabled` and no discovered layer was skipped as invalid.

Contradictions between layers produce deterministic `validation_warnings`, never errors. The pure `resolve_access_policy` in `cli/src/services/config/resolver.rs` computes the policy once, inside the existing resolver. `ResolvedAuthRuntimeConfig`, `ResolvedHookRuntimeConfig`, and `ResolvedObservabilityRuntimeConfig` expose it.

## Rationale

Telemetry export is a privacy-sensitive action that belongs to the user, so only the user layer can grant consent. Either layer can withdraw consent or restrict access. A repository can therefore opt itself out, but it can never opt a user in or override a user's prohibition. If a config file is invalid and skipped, falling back to defaults must not silently re-enable anything that file might have prohibited, so the policy degrades to off. Computing the policy once, in the existing resolver, keeps one authoritative answer for every consumer.

## Alternatives considered

- **Plain last-layer-wins merge for both keys:** rejected. A repository could enable telemetry or override a user's `disabled` mode.
- **Inferring consent from an existing `telemetry-ready.json` or `auth.db`:** rejected. Hidden state would decide consent.
- **Treating cross-layer contradictions as validation errors:** rejected. Each file is valid on its own, and errors would drop unrelated settings.
- **A separate policy resolver or service:** rejected. It would duplicate config discovery and risk diverging answers.

## Compatibility and risks

- Telemetry becomes opt-in. Existing logged-in installations without an explicit user-layer `telemetry.enabled: true` lose managed SCE export once consumers enforce the policy.
- Repository discovery uses the cwd. A process launched from a subdirectory sees only the user layer, which fails open toward the user's own consent.
- Older SCE binaries reject the new keys and skip the whole discovered file as invalid.

## Guardrails

- Consent comes only from the user layer. A repository value can only restrict.
- There is no env-var or CLI-flag override for `auth.mode` or `telemetry.enabled`.
- There is no global mutable policy state and no second resolver.
- Warnings are fixed strings, emitted in a fixed order.

## Consequences

- Consumers (rendering, managed telemetry, the headers helper, auth commands, sync, auto-sync, and setup) read one `ResolvedAccessPolicy` instead of deciding from credential state.
- A degraded config disables telemetry and background cloud operations. Explicit commands still use the auth mode resolved from the remaining valid layers.

## Follow-up

- Plan tasks T03–T08 render and enforce the policy at each consumer.

## References

- Plan: [`auth-telemetry-policy-config`](../plans/auth-telemetry-policy-config.md)
- Task: `T02`
- Current-state context: [`config-precedence-contract`](../cli/config-precedence-contract.md)
- Current-state context: [`access-policy`](../cli/access-policy.md)
- Evidence: [`resolver.rs`](../../cli/src/services/config/resolver.rs)
