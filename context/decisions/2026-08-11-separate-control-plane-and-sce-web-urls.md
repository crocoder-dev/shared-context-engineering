# Decision: Separate control-plane sync and SCE web/schema URLs

Date: 2026-08-11
Status: Accepted
Plan: `context/plans/separate-config-schema-and-control-plane-urls.md`
Task: T01

## Context

The CLI has two durable URL responsibilities: `sce trace sync` needs a control-plane ingestion host, while Agent Trace links and the config schema declaration belong to the SCE web application. The existing `control_plane_base_url` default conflated those responsibilities by using the web host. The resolver already provides the compatibility seam for environment and config-file overrides, and the sync client already composes the stable ingestion routes from that resolved base.

## Decision

Use `https://sce.crocoderlab.dev` as the baked default for `control_plane_base_url` and `sce trace sync`; keep `https://sce.crocoder.dev` as the owner of SCE web URLs and the config schema declaration.

## Rationale

This gives control-plane ingestion an explicit endpoint owner without changing the existing configuration seam, request paths, authentication behavior, Agent Trace web links, or schema publication contract.

## Alternatives considered

- **Keep the web host as the control-plane default** — preserves the conflated responsibility and does not route default sync traffic through the dedicated control-plane host.
- **Introduce a second runtime configuration key or URL registry** — broadens the configuration surface beyond the established `control_plane_base_url` seam without being required by the change.

## Compatibility and risks

- Existing `SCE_CONTROL_PLANE_BASE_URL` and `control_plane_base_url` overrides continue to take precedence over the new baked default, so deployments with an intentional endpoint remain compatible.
- The default sync destination changes for installations without an override; the dedicated host must serve the existing `/agent-trace/ingestion/state` and `/agent-trace/ingestion/batch` contract.

## Guardrails

- Do not change ingestion route paths, authentication, WorkOS endpoints, or the control-plane client protocol.
- Do not change `SCE_WEB_BASE_URL`, Agent Trace conversation/session/trace URL construction, or the `https://sce.crocoder.dev/config.json` schema declaration.
- Do not add another runtime configuration key or a general URL registry.

## Consequences

- Default `sce trace sync` requests target the dedicated control-plane host, while schema and web-link consumers remain on the SCE web host.
- Documentation and tests must identify the two URL owners separately, and operators can still override the control-plane host through the existing precedence chain.

## Follow-up

None.

## References

- Plan: [`separate-config-schema-and-control-plane-urls`](../plans/separate-config-schema-and-control-plane-urls.md)
- Task: `T01`
- Current-state context: [`CLI config precedence contract`](../cli/config-precedence-contract.md), [`sce trace command`](../cli/trace-command.md), [`Agent Trace sync architecture`](../cli/agent-trace-sync-command.md), [`Architecture`](../architecture.md), [`Glossary`](../glossary.md)
- Evidence: [`config resolver`](../../cli/src/services/config/resolver.rs), [`trace sync orchestration`](../../cli/src/services/trace/sync.rs), [`generated schema source`](../../config/pkl/base/sce-config-schema.pkl)
