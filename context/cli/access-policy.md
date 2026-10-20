# Effective Access Policy

## Scope

This file describes how the CLI turns the per-layer `auth.mode` and `telemetry.enabled` config values into one effective access policy (`ResolvedAccessPolicy`). Key syntax and schema validation are in [config precedence contract](config-precedence-contract.md). The precedence model is recorded in [ADR: consent-gated access policy precedence](../decisions/2026-10-20-consent-gated-access-policy-precedence.md).

Status: the resolver computes and exposes the policy. `sce config show`/`validate` rendering and consumer enforcement (managed telemetry, the headers helper, auth commands, sync, auto-sync, setup) are not wired yet. Until they are, the policy has no runtime effect beyond the warnings it adds to config resolution.

## Terms

- **Auth mode** (`AuthMode`): `disabled | optional | mandatory`. Omitted means `optional`.
- **Policy layer** (`PolicyLayer`): how a config file is classified for access policy.
  - **User layer:** a `DefaultDiscoveredGlobal` file, or the single explicit file (`Flag` / `Env`).
  - **Repository layer:** a `DefaultDiscoveredLocal` file (`<cwd>/.sce/config.json`).
  - Neither key has an environment-variable or CLI-flag layer.
- **Effective access policy** (`ResolvedAccessPolicy`), with these fields:
  - `auth_mode`: the effective mode, with the deciding layer as its source.
  - `telemetry_enabled`: the configured merged value with its provenance.
  - `telemetry`: the effective state, `TelemetryPolicyState` (`Enabled`, or `Disabled(reason)`).
  - `background_cloud_allowed`.
- **Degraded:** at least one default-discovered file was skipped as invalid.

## Resolution

`resolve_runtime_config_with` records each parsed layer's `auth_mode` / `telemetry_enabled` into a user or a repository `AccessPolicyLayer` during the merge loop. It then calls the pure `resolve_access_policy(user, repository, degraded)` (`cli/src/services/config/resolver.rs`).

```mermaid
flowchart TD
  D{degraded?} -- yes --> I[Disabled InvalidConfig]
  D -- no --> A{effective auth disabled?}
  A -- yes --> AD[Disabled AuthDisabled]
  A -- no --> U0{user false?}
  U0 -- yes --> UO[Disabled UserOptOut]
  U0 -- no --> R0{repository false?}
  R0 -- yes --> RO[Disabled RepositoryOptOut]
  R0 -- no --> U1{user true?}
  U1 -- yes --> E[Enabled]
  U1 -- no --> R1{repository true?}
  R1 -- yes --> NC[Disabled NoUserConsent]
  R1 -- no --> NE[Disabled NotEnabled]
```

- **Auth mode:** the most restrictive layer wins (`disabled > mandatory > optional`).
  - The source is the deciding layer. When both layers carry the same mode, the repository layer is the source.
  - When neither layer sets it, the result is `optional` with `source: default`.
- **`telemetry_enabled` value:** the repository value if set, otherwise the user value, otherwise `false` with `source: default`. This is the configured value, not the effective state.
- **`background_cloud_allowed`:** `!auth_disabled && !degraded`.
- **Helpers:**
  - `telemetry_enabled()` is true only for `Enabled`.
  - `auth_allowed()` is true when the mode is not `disabled`.
  - `auto_sync_allowed(configured)` is `configured && background_cloud_allowed`.
- **Explicit config files:** an invalid explicit config file stays fatal, so it never produces a degraded policy.

## Contradiction warnings

The resolver appends these fixed strings to `validation_warnings`, after the bash-policy redundancy warnings, in this order:

1. Repository `telemetry.enabled: true` without a user-layer `true`:
   `telemetry.enabled is true in the repository config but not in the user config; SCE telemetry stays disabled until the user config sets telemetry.enabled to true.`
2. Any layer has `telemetry.enabled: true` while the effective auth is `disabled`:
   `telemetry.enabled is true but auth.mode resolves to disabled; SCE telemetry is disabled.`
3. `agent_trace.auto_sync` resolves `true` while the effective auth is `disabled`:
   `agent_trace.auto_sync is true but auth.mode resolves to disabled; post-commit auto-sync is skipped.`
4. Repository `mandatory` while the user layer is `disabled`:
   `auth.mode is mandatory in the repository config but disabled in the user config; auth.mode resolves to disabled.`
5. Degraded:
   `A discovered config file was skipped as invalid; SCE telemetry and background cloud operations are disabled.`

Each string is a `WARNING_*` constant in `resolver.rs`. The skipped file's own error still appears in `validation_errors`.

## Exposure

`ResolvedAuthRuntimeConfig`, `ResolvedHookRuntimeConfig`, and `ResolvedObservabilityRuntimeConfig` each carry `access_policy`, cloned from the same resolution that produced the rest of the projection. No consumer resolves config a second time to get the policy.
