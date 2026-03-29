# Plan: Move `sce` default persisted files to canonical XDG directories

## Change summary
- Move the default per-user locations for `sce` CLI-created files to canonical XDG directories instead of the current mixed state/data placement.
- Adopt this target mapping for default paths: global config under `$XDG_CONFIG_HOME/sce/config.json`, auth tokens under `$XDG_STATE_HOME/sce/auth/tokens.json`, stateful local DB / history / logs under `$XDG_STATE_HOME/sce/...`, and cache-like artifacts under `$XDG_CACHE_HOME/sce/...` when such artifacts exist.
- Centralize default-location definitions behind one canonical code seam so services do not hardcode or duplicate persisted-file locations.
- Require that reads, writes, validation, and operator-facing reporting all consume that same canonical location-definition seam.
- Remove backward-compatibility fallback behavior for old default locations; the new XDG locations become the only supported defaults.

## Success criteria
- Global config discovery and validation default to `$XDG_CONFIG_HOME/sce/config.json` (or platform-equivalent `dirs::config_dir()` fallback) instead of the current state/data root.
- Auth token storage defaults to `$XDG_STATE_HOME/sce/auth/tokens.json` (or platform-equivalent state root fallback).
- Agent Trace local DB and any other stateful per-user runtime artifacts continue under the state root and remain documented as such.
- Any cache/temp artifact paths owned by `sce` resolve under the cache root when the CLI currently persists such artifacts; if no cache-backed persisted artifact exists yet, no speculative cache feature is introduced.
- Code that writes or reports default persisted-file locations uses one shared location-definition seam instead of duplicating path literals or per-service root assembly.
- Code that reads, writes, validates, or reports default persisted-file locations uses one shared location-definition seam instead of duplicating path literals or per-service root assembly.
- Tests cover XDG-env-present resolution, XDG-env-absent platform fallback resolution, and explicit no-legacy-fallback behavior for each migrated default path.
- Unsupported-root resolution behavior remains explicit and deterministic when platform directory helpers cannot provide a required root.
- Doctor/config/auth/runtime surfaces that report or validate default locations reflect the new XDG mapping with deterministic output and tests.
- No runtime fallback reads or auto-migration logic are added for legacy default paths.
- Context files are updated to describe the new current-state path contract.

## Constraints and non-goals
- No backward compatibility for prior default per-user paths.
- No automatic migration of old files into new XDG locations.
- No changes to repository-local `.sce/config.json`; this plan covers per-user default storage roots only.
- No new persistence feature should be invented solely to “use” `$XDG_CACHE_HOME`; only existing persisted artifacts are remapped.
- The implementation must confirm whether any current persisted cache artifacts actually exist before adding cache-root wiring; if none exist, no speculative cache path code should be introduced.
- Keep command/output/error contracts deterministic outside path-specific changes.

## Task stack
- [ ] T01: `Create a canonical XDG location-definition seam for per-user sce storage` (status:todo)
  - Task ID: T01
  - Goal: Introduce one canonical service/helper contract that owns both per-user `sce` root resolution and the named persisted-file locations consumed by the CLI so path policy is no longer duplicated across config, token, local DB, diagnostics, and related services.
  - Boundaries (in/out of scope): In - shared path-resolution helpers/types, canonical named accessors for persisted-file locations, replacement of duplicated root-resolution or file-location assembly logic across read/write/report/validation callers, focused unit tests for Linux/macOS/Windows/other fallback behavior and deterministic root-resolution failures. Out - changing command-facing storage behavior beyond wiring callers to the new shared contract.
  - Done when: A single canonical path-policy seam exists for per-user `sce` roots and default persisted-file locations, all current callers that read/write/report/validate these locations can consume it without ad hoc path assembly, and tests cover the approved XDG mapping plus fallback/error cases.
  - Verification notes (commands or checks): `nix flake check`.

- [ ] T02: `Move default global config discovery to XDG config root` (status:todo)
  - Task ID: T02
  - Goal: Update config-path discovery, validation, and related diagnostics so the default global config path becomes `$XDG_CONFIG_HOME/sce/config.json` (or `dirs::config_dir()` platform equivalent).
  - Boundaries (in/out of scope): In - `cli/src/services/config.rs`, any command/help/diagnostic text or tests that assert the discovered global config location, doctor checks that reference the global config path. Out - repo-local `.sce/config.json`, config schema changes, migration/fallback support for old global paths.
  - Done when: Global config discovery uses the config root only, output/tests reflect the new location, and no legacy state/data-root fallback remains for the global config default.
  - Verification notes (commands or checks): `nix flake check`.

- [ ] T03: `Move auth and runtime state artifacts to approved XDG roots` (status:todo)
  - Task ID: T03
  - Goal: Apply the approved path mapping to auth tokens and all currently implemented per-user runtime artifacts owned by `sce`, keeping stateful artifacts under the state root and cache artifacts under the cache root where applicable.
  - Boundaries (in/out of scope): In - explicit artifact inventory and migration coverage for auth tokens, Agent Trace local DB, persisted logs if present, and any existing persisted temp/history/cache artifact path resolution, plus targeted tests for exact default file paths. Out - auto-migration, fallback reads from old locations, creation of new cache persistence features when none exist.
  - Done when: The implementation has an explicit inventory of current per-user persisted artifacts; token storage resolves to `$XDG_STATE_HOME/sce/auth/tokens.json`; stateful runtime artifacts resolve through the approved state-root policy; any existing cache-backed artifact uses the cache root if applicable; and tests/assertions cover the final path contract, including env-set/env-unset behavior and no legacy fallback.
  - Verification notes (commands or checks): `nix flake check`.

- [ ] T04: `Update operator-facing diagnostics and path-sensitive command coverage` (status:todo)
  - Task ID: T04
  - Goal: Align doctor/auth/config/runtime diagnostics and their tests with the new XDG path contract so user-facing reporting stays deterministic.
  - Boundaries (in/out of scope): In - doctor problem reporting/path facts, auth/token-related diagnostics, config/help text or JSON fields that expose default path provenance, targeted tests that assert these surfaces, and confirmation that these surfaces resolve locations only through the shared seam. Out - unrelated command-surface redesign, broader UX rewriting, backward-compatibility warnings.
  - Done when: Path-sensitive command outputs and diagnostics consistently reference the new XDG defaults, tests cover the changed reporting, no stale old-path wording remains in runtime-facing messages, and no reporting surface still computes these locations independently.
  - Verification notes (commands or checks): `nix flake check`.

- [ ] T05: `Sync context for the new XDG storage contract` (status:todo)
  - Task ID: T05
  - Goal: Update current-state context files to describe the new per-user storage mapping and remove stale references to the old defaults.
  - Boundaries (in/out of scope): In - focused updates to `context/overview.md`, `context/glossary.md`, and any relevant CLI/SCE path-contract docs. Out - historical change logs, implementation beyond documentation, broad context rewrites unrelated to storage paths.
  - Done when: Durable context files reflect the XDG config/state/cache mapping and no longer describe the previous global-config default path.
  - Verification notes (commands or checks): Manual review against code truth.

- [ ] T06: `Validation and cleanup` (status:todo)
  - Task ID: T06
  - Goal: Run the full required verification pass, confirm no task-scoped scaffolding remains, and leave the plan ready for completion tracking.
  - Boundaries (in/out of scope): In - full repo validation, generated-output parity check if touched, plan status updates. Out - new feature work or post-plan enhancements.
  - Done when: Required validation passes, plan state is current, and any temporary scaffolding introduced during implementation is removed.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated` (if generated outputs change); `nix flake check`.

## Open questions
- None.
