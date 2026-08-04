# Decision: Route setup integration-asset installation through application ports and outbound adapters

Date: 2026-08-04
Status: Accepted
Plan: `context/plans/install-integration-assets.md`
Task: T01, T02, T03, T04, T05, T06, T07, T08, T09, T10, T11

## Context

The setup command must preserve its existing installed files, destination paths,
message text, error behavior, and target ordering while reducing the
responsibility held by `services::setup`. The embedded integration-asset flow
crosses domain target selection, application orchestration, generated asset
catalog access, and filesystem staging/swap behavior. The repository's internal
hexagonal architecture requires application code to remain independent of
services and filesystem infrastructure, while migration compatibility requires
the existing `install_embedded_setup_assets` entrypoint and `SetupInstallOutcome`
shape to remain stable.

## Decision

The setup embedded integration-asset capability is implemented as an internal
hexagonal vertical slice: application-owned `IntegrationAssetCatalog` and
`IntegrationInstaller` ports are orchestrated by `InstallIntegrationAssets`,
concrete asset selection and filesystem staging/swap are outbound adapters, and
`services::setup::install_embedded_setup_assets` remains a compatibility facade.

## Rationale

This preserves the public setup behavior while establishing the intended
inward dependency direction. Expanding `IntegrationTargetSelection::All` in the
use case prevents meta-target values from crossing the adapter boundary, and a
request-level installer preflight gives the filesystem adapter one place to
check repository writability before any target work begins. Keeping generated
catalog coupling in the outbound adapter avoids forcing the hook and
integration catalogs apart during migration.

## Alternatives considered

- **Keep the complete implementation in `services::setup`** — preserves the
  current location but leaves infrastructure and orchestration coupled and
  does not advance the repository's hexagonal migration.
- **Wire the whole setup command through the composition root immediately** —
  broadens the slice beyond embedded asset installation and risks changing
  unrelated setup, persistence, prompting, and hook behavior.
- **Split generated hook and integration catalogs as part of this slice** —
  adds unrelated generator and packaging scope; the outbound catalog adapter
  can preserve the existing generated catalog boundary during migration.

## Compatibility and risks

- The compatibility facade maps between legacy `SetupTarget`/
  `SetupInstallOutcome` values and the new domain/application types, preserving
  target order, installed counts, paths, messages, and remove-then-rename
  behavior.
- Outbound adapters still transitionally depend on retained `services` helpers
  and the generated catalog. The architecture check prevents that transitional
  dependency from leaking into domain or application modules.
- Filesystem staging failures and rename failures retain cleanup and recovery
  guidance; targeted adapter and facade tests guard the no-backup policy.

## Guardrails

- `IntegrationTargetSelection::All` is expanded before either port is called;
  ports and adapters accept only concrete `IntegrationTarget` values.
- Domain and application modules must not import `crate::services` or
  filesystem APIs.
- The use case owns orchestration only; staging, writes, removal, swapping, and
  cleanup remain in the filesystem outbound adapter.
- `composition::run`, persistence, prompting, repository discovery, and hook
  installation remain outside this slice.

## Consequences

- The CLI now has a second landed internal hexagonal vertical slice and a
  stable application boundary for future setup migrations.
- The setup service retains a compatibility facade until later slices migrate
  additional setup responsibilities.
- New catalogs or installers can provide owned asset bytes through
  `Cow<'static, [u8]>`, while embedded assets remain zero-copy through
  `Cow::Borrowed`.

## Follow-up

- Future setup slices may migrate persistence and composition-root wiring; no
  such migration is part of this decision.

## References

- Plan: [`install-integration-assets`](../plans/install-integration-assets.md)
- Task: T01–T11
- Current-state context: [`CLI internal hexagonal architecture`](../architecture.md)
- Evidence: [`Validation Report`](../plans/install-integration-assets.md)
- Related decision: None.
