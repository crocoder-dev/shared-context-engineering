# Decision: Config schema accepts `$schema` declarations by release series

Date: 2026-11-10
Status: Accepted
Plan: `context/plans/config-schema-series-compatibility.md`
Task: `T01`

## Context

The generated SCE config schema used the CLI's full release version, read from `.version`, as an exact `const` on the `$schema` property. Every `nix run .#bump-version` therefore made the next CLI reject every config file written by an earlier release, including earlier releases in the same `major.minor` series. In a default-discovered file, the rejection discarded every setting in that file. In an explicit `--config` / `SCE_CONFIG_FILE` file, the rejection was fatal.

## Decision

The SCE config schema accepts any canonical `$schema` declaration of the form `https://sce.crocoder.dev/v<major>.<minor>.<patch>[-prerelease][+build]/config.json` within the CLI's `major.minor` release series.

- The series is derived solely from `.version`, through `config/pkl/base/sce-config-schema-url.pkl`.
- The `$schema` property is `type: string` with an anchored series `pattern`, replacing the exact-version `const`.
- The schema `$id` stays the CLI's concrete versioned URL.
- Declarations from other series, malformed or foreign URLs, and non-string values remain rejected.
- Every other config field is still validated against the embedded schema.

## Rationale

Patch and prerelease releases within one series do not change the config contract enough to invalidate existing files. Deriving the series from `.version` keeps a single version source, so a release bump moves the accepted series without editing constants. Keeping a concrete `$id` keeps each published snapshot uniquely identified.

## Alternatives considered

- **Keep the exact-version `const`** — Not selected: it breaks every existing config file on every release.
- **Accept any `sce.crocoder.dev` versioned URL** — Not selected: it would silently accept declarations from incompatible series.

## Compatibility and risks

- Files declaring an older or newer series are still rejected. The plan's later tasks add actionable messages and a setup-driven `$schema` upgrade for older-series files.
- Historical `schema/v*/config.json` snapshots keep their exact-version `const`. The next `nix run .#bump-version` publishes the first series-pattern snapshot.

## Guardrails

- The accepted series comes only from `.version`; no other hard-coded series exists outside test fixtures.
- Only the `major.minor` series is relaxed. The URL host, scheme, path shape, and semver form stay strict.

## Consequences

- A same-series release no longer invalidates existing config files.
- A series bump (`major.minor`) still requires users to update their declarations.

## Follow-up

- Remaining tasks T02–T08 of the plan: a typed classifier, actionable `$schema` errors, the setup `$schema` upgrade, end-to-end checks, and documentation.

## References

- Plan: [`config-schema-series-compatibility`](../plans/config-schema-series-compatibility.md)
- Task: `T01`
- Current-state context: [`config-schema-publication.md`](../sce/config-schema-publication.md)
- Current-state context: [`config-precedence-contract.md`](../cli/config-precedence-contract.md)
- Evidence: [`sce-config-schema-url.pkl`](../../config/pkl/base/sce-config-schema-url.pkl)
- Evidence: [`generation-contract-check.pkl`](../../config/pkl/renderers/generation-contract-check.pkl)
