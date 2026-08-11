# Plan: separate-config-schema-and-control-plane-urls

## Change summary

Separate the two SCE URL responsibilities that are currently conflated in the CLI's control-plane default. `sce trace sync` will use `https://sce.crocoderlab.dev` as its baked control-plane base URL, producing requests to `/agent-trace/ingestion/state` and `/agent-trace/ingestion/batch`. The existing `control_plane_base_url` configuration and `SCE_CONTROL_PLANE_BASE_URL` override remain supported.

The config schema declaration remains `https://sce.crocoder.dev/config.json`; it is a schema URL for editor and validation tooling, not the sync service endpoint. Existing SCE web URLs for Agent Trace conversations, sessions, and traces also remain on `https://sce.crocoder.dev`.

## Acceptance criteria

- [x] AC1: With no `control_plane_base_url` override, `sce trace sync` resolves `https://sce.crocoderlab.dev` as its control-plane base and targets `POST /agent-trace/ingestion/state` and `POST /agent-trace/ingestion/batch` without changing request paths or authentication behavior.
  - Validate: `nix flake check` passes the control-plane client and config resolver tests, including assertions for the baked default and composed ingestion paths.
- [x] AC2: `SCE_CONTROL_PLANE_BASE_URL` and the `control_plane_base_url` config key still override the new baked default according to the existing precedence contract.
  - Validate: `nix flake check` passes the existing env/config precedence tests with the updated default expectation.
- [x] AC3: The config schema declaration remains `https://sce.crocoder.dev/config.json`, and Agent Trace conversation/session/trace URL construction remains unchanged.
  - Validate: `nix run .#pkl-check-generated`; inspect the generated schema from `nix run .#pkl-generate -- "$(mktemp -d)"` and confirm its `$id`/`$schema` declaration remains the SCE web URL; `nix flake check` passes the existing Agent Trace URL coverage.
- [x] AC4: Durable CLI documentation distinguishes the control-plane base URL from the config schema and SCE web URL owners.
  - Validate: inspect the updated config-precedence, trace-sync, and root architecture/context records for the two-URL contract.

### Full validation

- `nix flake check`
- `nix run .#pkl-check-generated`

### Context sync

- `context/cli/config-precedence-contract.md` — update the baked `control_plane_base_url` default while preserving the separate `$schema` URL.
- `context/cli/trace-command.md` — document the control-plane host used by `sce trace sync`.
- `context/cli/agent-trace-sync-command.md` — record the two-host boundary in the sync data flow.
- `context/overview.md` — distinguish the control-plane endpoint from the SCE web/schema URL.
- `context/architecture.md` — preserve `SCE_WEB_BASE_URL` ownership for web URLs while documenting the config-resolved control-plane owner.
- `context/glossary.md` — clarify the SCE web URL owner is not the sync control-plane default.

## Constraints and non-goals

- **In scope:** the baked `control_plane_base_url` default and its focused resolver/control-plane tests; documentation and durable context describing the split URL contract.
- **Out of scope:** changing the JSON schema URL, moving or republishing `https://sce.crocoder.dev/config.json`, changing Agent Trace conversation/session/trace URLs, changing WorkOS endpoints, changing ingestion route paths, or changing the control-plane server.
- **Constraints:** preserve `env > config file > baked default` precedence; keep the existing configurable base URL override; use the repository's Nix validation entrypoints; do not add a dependency.
- **Non-goal:** introducing a second runtime configuration key or a general URL registry for unrelated SCE web surfaces.

## Assumptions

- “Change to 2 URLs” means the production baked default should use `https://sce.crocoderlab.dev` for control-plane sync while the config schema and SCE web links remain on `https://sce.crocoder.dev`, as established in the preceding discussion.
- The existing `control_plane_base_url` setting is the intended configuration seam; only its default and contract documentation need to change.

## Task stack

- [x] T01: `Route Agent Trace sync through the dedicated control-plane host` (status:done)
  - Task ID: T01
  - Goal: Change the baked control-plane default to `https://sce.crocoderlab.dev`, preserve all existing overrides and web/schema URL ownership, update focused tests and durable documentation, and leave ingestion paths/authentication unchanged.
  - Boundaries (in/out of scope): In — `cli/src/services/config/resolver.rs`, related config/control-plane tests, and the context files listed under Context sync. Out — schema publication, SCE web URL construction, WorkOS endpoint changes, ingestion API changes, and generated target trees.
  - Dependencies: none
  - Done when: the default sync client resolves the dedicated control-plane host; env/config overrides continue to win; the schema remains identified by `https://sce.crocoder.dev/config.json`; existing Agent Trace web URLs are unchanged; focused tests and documentation reflect the split.
  - Verification notes (commands or checks): `nix flake check`; `nix run .#pkl-check-generated`; inspect a fresh generated schema and the relevant URL-owner/context records.
  - Completed: 2026-08-11
  - Files changed: `cli/src/services/config/resolver.rs`
  - Evidence: Updated `CONTROL_PLANE_BASE_URL_BAKED_DEFAULT` to `https://sce.crocoderlab.dev`; existing resolver tests continue to cover baked default, config-file override, and environment-over-config precedence. The sync client continues to compose the unchanged `/agent-trace/ingestion/state` and `/agent-trace/ingestion/batch` routes from the resolved base URL, while `SCE_WEB_BASE_URL` and the generated schema remain unchanged.
  - Verification: `nix flake check` passed; `nix run .#pkl-check-generated` passed with 101 files; fresh `nix run .#pkl-generate -- <temp-dir>` output retained `$id` and `$schema` declaration `https://sce.crocoder.dev/config.json`.

## Open questions

None. The requested two-host split and preservation of the existing schema URL are explicit.

## Validation Report

**Status:** validated
**Date:** 2026-08-11

### Commands run

- `nix flake check` -> exit 0 (all flake checks passed, including CLI tests, Clippy, formatting, and generated-output checks)
- `nix run .#pkl-check-generated` -> exit 0 (ephemeral Pkl generation passed with 101 files)
- `nix run .#pkl-generate -- "$(mktemp -d)"` plus broad all-file schema-URL scan -> exit 1 (exploratory scan included non-schema generated files; no product failure)
- `nix run .#pkl-generate -- "$(mktemp -d)"` plus schema-only `$id`/`$schema` assertions -> exit 0 (both declarations remain `https://sce.crocoder.dev/config.json`)

### Scaffolding removed

- None.

### Success-criteria verification

- [x] AC1: With no `control_plane_base_url` override, `sce trace sync` resolves `https://sce.crocoderlab.dev` as its control-plane base and targets `POST /agent-trace/ingestion/state` and `POST /agent-trace/ingestion/batch` without changing request paths or authentication behavior -> `nix flake check` passed control-plane client/config resolver coverage; source inspection confirmed the unchanged routes and auth flow.
- [x] AC2: `SCE_CONTROL_PLANE_BASE_URL` and the `control_plane_base_url` config key still override the new baked default according to the existing precedence contract -> `nix flake check` passed the resolver precedence tests.
- [x] AC3: The config schema declaration remains `https://sce.crocoder.dev/config.json`, and Agent Trace conversation/session/trace URL construction remains unchanged -> generated schema `$id`/`$schema` assertions passed; `nix run .#pkl-check-generated` and `nix flake check` passed.
- [x] AC4: Durable CLI documentation distinguishes the control-plane base URL from the config schema and SCE web URL owners -> inspected the updated config-precedence, trace-sync, trace-command, overview, architecture, glossary, and context-map records.

### Failed checks and follow-ups

- None.

### Residual risks

- None identified.
