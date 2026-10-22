# Decision: Consent-gated precedence for the effective access policy

Date: 2026-10-20
Status: Accepted (partially superseded)
Partially superseded by: `context/decisions/2026-10-22-repository-telemetry-enabled-grants-consent.md` (the user-side-only consent rule only)
Plan: `context/plans/auth-telemetry-policy-config.md`
Task: `T02`

> Partially superseded on 2026-10-22 by [A repository telemetry.enabled true grants telemetry consent](2026-10-22-repository-telemetry-enabled-grants-consent.md). A `telemetry.enabled: true` in any layer, including the repository layer, now grants consent unless some layer opts out, and the `no_user_consent` reason and its contradiction warning are removed. Layer discovery, `auth.mode` precedence, degraded handling, and explicit-layer semantics below still apply.

## Context

Before this decision, credential state alone decided whether SCE exported telemetry or used cloud access. Once a user logged in, a telemetry credential was provisioned and Claude hooks exported spans. No setting let a user opt out of telemetry while staying logged in, forbid authentication on a machine, or stop a repository from turning these features on. The new `auth.mode` and `telemetry.enabled` config keys can appear in both the global and the repo-local `sce/config.json`. The ordinary per-key merge (last layer wins) would let a repository enable telemetry without the user's consent, or override a user's prohibition.

## Decision

SCE resolves one effective access policy from up to three config layers, using consent-gated precedence.

**Layers:**
- The user layer is the default-discovered global config.
- The repository layer is the default-discovered `.sce/config.json` at the enclosing Git worktree root: the nearest ancestor of the working directory (inclusive) that contains a `.git` entry. Outside a Git repository, it is `<cwd>/.sce/config.json`.
- The explicit layer is the single file selected with `--config` or `SCE_CONFIG_FILE` (the flag wins when both are given).
- The global and repository files are always discovered, also when an explicit file is selected. The explicit file is added on top of them; it never replaces them.
- Neither key has an environment-variable or flag layer of its own.

**Ordinary keys versus access-policy keys:** When an explicit file is selected, ordinary keys (logging, Agent Trace, `otel`, `policies`, `integrations`, auth-adjacent URLs and IDs) still come only from that file, exactly as before. The discovered files feed only `auth.mode` and `telemetry.enabled`. An explicit file can therefore override ordinary configuration, but it cannot lift a security or consent restriction set in the global or repository config.

**`auth.mode`:** the most restrictive layer wins. The order is `disabled > mandatory > optional`, and the default is `optional`. Layers are compared in the order user, repository, explicit; when layers tie, the later one is recorded as the source.

**`telemetry.enabled`:** the first matching rule applies. "User-side" means the user layer or the explicit layer, because both are the user's own choice.
1. Any skipped invalid discovered layer → `Disabled(InvalidConfig)`.
2. Effective auth is `disabled` → `Disabled(AuthDisabled)`.
3. User layer or explicit layer is `false` → `UserOptOut`.
4. Repository layer is `false` → `RepositoryOptOut`.
5. User layer or explicit layer is `true` → `Enabled`.
6. Repository layer is `true` with no user-side `true` → `NoUserConsent`.
7. Otherwise → `NotEnabled`.

Any opt-out wins over any consent, so an explicit `true` cannot override a global or repository `false`, and an explicit `false` overrides a global `true`.

**Background cloud operations** are allowed only when auth is not `disabled` and no discovered layer was skipped as invalid.

Contradictions between layers produce deterministic `validation_warnings`, never errors. The pure `resolve_access_policy` in `cli/src/services/config/resolver.rs` computes the policy once, inside the existing resolver. `ResolvedAuthRuntimeConfig`, `ResolvedHookRuntimeConfig`, and `ResolvedObservabilityRuntimeConfig` expose it.

## Rationale

Telemetry export is a privacy-sensitive action that belongs to the user, so only the user side (global or explicit file) can grant consent. Every layer can withdraw consent or restrict access. A repository can therefore opt itself out, but it can never opt a user in or override a user's prohibition. If a config file is invalid and skipped, falling back to defaults must not silently re-enable anything that file might have prohibited, so the policy degrades to off. Computing the policy once, in the existing resolver, keeps one authoritative answer for every consumer.

An explicit config file is commonly used for scripted or isolated runs, so it keeps replacing discovered ordinary values. But it is a per-invocation choice that anything able to set an environment variable can make, so it must not silently undo a standing global prohibition or a repository opt-out. Treating it as an additional, restriction-only-against-others layer keeps both properties.

Locating the repository layer at the Git worktree root makes the policy independent of which subdirectory a tool (Claude hooks, the headers helper, a terminal) was started from. The lookup probes only the filesystem for `.git`, so hot hook paths add no Git subprocess.

## Alternatives considered

- **Plain last-layer-wins merge for both keys:** rejected. A repository could enable telemetry or override a user's `disabled` mode.
- **Inferring consent from an existing `telemetry-ready.json` or `auth.db`:** rejected. Hidden state would decide consent.
- **Treating cross-layer contradictions as validation errors:** rejected. Each file is valid on its own, and errors would drop unrelated settings.
- **A separate policy resolver or service:** rejected. It would duplicate config discovery and risk diverging answers.
- **Explicit file replaces discovery entirely (the original rule):** rejected. `--config` or `SCE_CONFIG_FILE` could enable telemetry or authentication the global config forbids, or bypass a repository opt-out.
- **Merging all ordinary keys from discovered files under an explicit file:** rejected. It would change the long-standing meaning of `--config` for non-security settings.
- **`git rev-parse --show-toplevel` for repository discovery:** rejected. It costs a subprocess on every hook invocation; the `.git` ancestor walk gives the same worktree root for ordinary repositories, linked worktrees, submodules, and nested repositories.

## Compatibility and risks

- Telemetry becomes opt-in. Existing logged-in installations without an explicit user-layer `telemetry.enabled: true` lose managed SCE export once consumers enforce the policy.
- Repository discovery is Git-root relative. A process launched from a subdirectory now applies the repository opt-out it previously missed. A `.sce/config.json` in a non-root subdirectory of a repository is no longer discovered from that subdirectory. In a nested repository, the inner repository's root is used. Outside a Git repository, discovery still uses the working directory.
- `GIT_DIR` / `GIT_WORK_TREE` are not consulted; discovery follows the working directory's ancestors.
- With `--config` / `SCE_CONFIG_FILE`, the global and repository files are now read. Their access-policy keys apply, they appear in `sce config show` config paths, and if one is invalid it is reported and disables telemetry and background cloud operations (the explicit file itself stays fatal when invalid). Explicit runs now also need a resolvable global config location, as discovered runs already did.
- Older SCE binaries reject the new keys and skip the whole discovered file as invalid.

## Guardrails

- Consent comes only from the user side (global or explicit file). A repository value can only restrict.
- An explicit file can only add restrictions or consent; it can never lift a global or repository restriction.
- There is no env-var or CLI-flag override for `auth.mode` or `telemetry.enabled` beyond selecting the explicit file.
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
