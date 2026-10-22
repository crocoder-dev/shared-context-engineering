# Decision: A repository telemetry.enabled true grants telemetry consent

Date: 2026-10-22
Status: Accepted
Plan: `context/plans/setup-auth-mode-repo-local.md`
Task: `T02`
Supersedes: `context/decisions/2026-10-20-consent-gated-access-policy-precedence.md` (the user-side-only consent rule only)

## Context

The [2026-10-20 decision](2026-10-20-consent-gated-access-policy-precedence.md) let only the user side (the global or explicit config) grant telemetry consent. A repository-layer `telemetry.enabled: true` alone resolved to `Disabled(NoUserConsent)` and produced a contradiction warning. The plan moves setup's access answers into the repository's `.sce/config.json`, so each answer applies only to that repository. A consent saved there has to take effect. Under the old rule it could not.

## Decision

A `telemetry.enabled: true` in any layer, including the repository layer, grants telemetry consent unless some layer opts out. The first matching rule decides the effective state:

1. Any skipped invalid discovered layer → `Disabled(InvalidConfig)`.
2. Effective auth is `disabled` → `Disabled(AuthDisabled)`.
3. User or explicit layer is `false` → `Disabled(UserOptOut)`.
4. Repository layer is `false` → `Disabled(RepositoryOptOut)`.
5. Any layer is `true` → `Enabled`.
6. Otherwise → `Disabled(NotEnabled)`.

The `no_user_consent` reason and its repository-true-without-user-consent warning no longer exist. The reported `telemetry.enabled` value and source come from the layer that decided the outcome. Explicit, user, and repository layers are checked in that order for a `false`. If none is `false`, explicit, repository, and user layers are checked in that order for a `true`. The rest of the 2026-10-20 decision is unchanged: `auth.mode` precedence (most restrictive wins), degraded handling, layer discovery, and explicit-layer semantics.

## Rationale

The user asked for per-repository access answers stored in the repository's own config. A consent that only counted from the global file would have made the per-repository answer pointless. Opt-outs still win over any consent, so a global or explicit `false` stays a standing personal prohibition, and a repository `false` still opts the repository out.

## Alternatives considered

- **Keep user-side-only consent:** rejected. A repository-saved setup answer could never enable telemetry.
- **A gitignored per-user repository file (for example `.sce/config.local.json`):** not selected. It would keep per-repository scope and per-person consent, but the user chose the committed `.sce/config.json`.

## Compatibility and risks

- A committed `.sce/config.json` with `telemetry.enabled: true` opts in everyone who clones or pulls the repository, without asking them. A user can still withdraw with `telemetry.enabled: false` in the global config.
- Repositories that already commit `telemetry.enabled: true` switch from `no_user_consent` to `Enabled` for users without a user-side value.
- Consumers that matched on the `no_user_consent` reason string no longer receive it.

## Guardrails

- Any `false` in any layer still disables telemetry. A `true` never overrides an opt-out.
- `auth.mode: disabled` and a degraded config still disable telemetry before consent is considered.
- There is still no env-var or CLI-flag override for `telemetry.enabled`.

## Consequences

- One repository-local answer controls SCE telemetry for that repository.
- The `sce config show` `not_enabled` note tells the user to set `telemetry.enabled: true` in the repository SCE config.

## Follow-up

- Plan task T07 records the setup write-location change that supersedes [`2026-10-21-setup-writes-access-answers-to-user-layer-only`](2026-10-21-setup-writes-access-answers-to-user-layer-only.md).

## References

- Plan: [`setup-auth-mode-repo-local`](../plans/setup-auth-mode-repo-local.md)
- Task: `T02`
- Current-state context: [`access-policy`](../cli/access-policy.md)
- Evidence: [`resolver.rs`](../../cli/src/services/config/resolver.rs)
- Evidence: [`render.rs`](../../cli/src/services/config/render.rs)
- Related decision: [`Consent-gated precedence for the effective access policy`](2026-10-20-consent-gated-access-policy-precedence.md)
