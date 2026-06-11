# Consolidate sce_web.rs into agent_trace.rs

## Change summary

Delete `cli/src/services/sce_web.rs` and relocate its single constant (`SCE_WEB_BASE_URL`) and URL-builder helpers into `cli/src/services/agent_trace.rs`. While touching those files, fix two existing duplications:

1. `SESSION_RELATED_URL_PREFIX` in `agent_trace.rs` hardcodes `"https://sce.crocoder.dev/sessions/"` — replace with a new `agent_trace_session_url()` helper powered by the relocated base URL.
2. `SCE_METADATA_VERSION` in `agent_trace.rs` duplicates `PACKAGE_VERSION` in `version/mod.rs` (both compile `CARGO_PKG_VERSION`) — make `PACKAGE_VERSION` `pub(crate)` and consume it from `agent_trace.rs`.

Inline the one non-agent-trace consumer (`config_schema_url` used by `setup/mod.rs` for a single bootstrap payload) directly into setup.

Net result: -1 file, zero hardcoded URL/version duplication, all SCE-owned web URL construction lives in its primary consumer.

## Success criteria

- `cli/src/services/sce_web.rs` is deleted and `pub mod sce_web;` is removed from `services/mod.rs`.
- `SCE_WEB_BASE_URL` has one canonical definition in `agent_trace.rs` (`pub(crate)`).
- `agent_trace_conversation_url`, `agent_trace_persisted_url`, and `agent_trace_session_url` live in `agent_trace.rs` and are the only URL builders.
- `SESSION_RELATED_URL_PREFIX` is removed; the single call site uses `agent_trace_session_url()`.
- `SCE_METADATA_VERSION` is removed from `agent_trace.rs`; `version::PACKAGE_VERSION` (made `pub(crate)`) is used instead.
- `config_schema_url` is inlined in `setup/mod.rs` as a literal `format!` call; setup no longer imports from `sce_web`.
- All consumers (`agent_trace_db/mod.rs`, `hooks/mod.rs`, `agent_trace/tests.rs`) import from `services::agent_trace` instead of `services::sce_web`.
- `nix flake check` passes.
- Context docs under `context/` no longer reference `sce_web.rs` as a current-state file.

## Constraints and non-goals

- Do not create a new file (no `constants.rs`).
- Do not change any runtime behavior, serialized output, or schema validation.
- Do not add new URL schemes or endpoints beyond the existing three helpers.
- Do not touch DB migrations, hook command routing, or OpenCode plugin code.
- Do not broaden the scope of constants that live in `agent_trace.rs` beyond the relocated SCE web URL helpers.
- Keep the hash-related and schema-related private constants in `agent_trace.rs` unchanged.

## Task stack

- [x] T01: `Relocate SCE_WEB_BASE_URL and URL builders from sce_web.rs to agent_trace.rs` (status:done)
  - Task ID: T01
  - Goal: Move `SCE_WEB_BASE_URL`, `agent_trace_conversation_url`, and `agent_trace_persisted_url` into `agent_trace.rs`. Add `agent_trace_session_url()` replacing the `SESSION_RELATED_URL_PREFIX` constant. Update all consumers (agent_trace_db, hooks, agent_trace tests) to import from `agent_trace` instead of `sce_web`. Inline `config_schema_url` in `setup/mod.rs`. Delete `sce_web.rs` and remove `pub mod sce_web` from `services/mod.rs`.
  - Boundaries (in/out of scope): In — `agent_trace.rs`, `agent_trace_db/mod.rs`, `hooks/mod.rs`, `agent_trace/tests.rs`, `setup/mod.rs`, `sce_web.rs` (delete), `services/mod.rs`. Out — `SCE_METADATA_VERSION` / `PACKAGE_VERSION` duplication (T02), context docs (T03), `agent_trace.rs` hash/schema constants.
  - Done when: `sce_web.rs` is gone; `SCE_WEB_BASE_URL` and three URL builders live in `agent_trace.rs`; all consumers use `services::agent_trace` imports; `SESSION_RELATED_URL_PREFIX` is removed; `config_schema_url` is inlined in `setup/mod.rs`; compilation succeeds; `nix flake check` passes.
  - Verification notes (commands or checks): `nix flake check` from repo root.
  - Context-sync classification: structural module reorganization; context docs referencing `sce_web.rs` need updating (T03).

- [x] T02: `Deduplicate package version constant` (status:done)
  - Task ID: T02
  - Completed: 2026-06-11
  - Files changed: `cli/src/services/version/mod.rs` (pub(crate) on PACKAGE_VERSION), `cli/src/services/agent_trace.rs` (removed SCE_METADATA_VERSION, imported PACKAGE_VERSION from version)
  - Evidence: `nix flake check` passed (cli-tests, cli-clippy, cli-fmt, pkl-parity all green)
  - Goal: Make `PACKAGE_VERSION` in `version/mod.rs` `pub(crate)`, then replace the standalone `SCE_METADATA_VERSION` in `agent_trace.rs` with that shared constant. Remove `SCE_METADATA_VERSION`.
  - Boundaries (in/out of scope): In — `version/mod.rs`, `agent_trace.rs`. Out — all other constants, context docs (T03), URL consolidation (T01).
  - Done when: `PACKAGE_VERSION` is `pub(crate)` in `version/mod.rs`; `agent_trace.rs` uses `services::version::PACKAGE_VERSION` (or equivalent import) instead of its own `SCE_METADATA_VERSION`; `SCE_METADATA_VERSION` is removed; compilation succeeds; tests referencing `SCE_METADATA_VERSION` pass.
  - Verification notes (commands or checks): `nix flake check` from repo root.
  - Context-sync classification: localized constant dedup; context docs referencing `SCE_METADATA_VERSION` need updating (T03).

- [x] T03: `Sync context docs for sce_web.rs removal and constant consolidation` (status:done)
  - Task ID: T03
  - Completed: 2026-06-11
  - Files changed: `context/overview.md`, `context/architecture.md`, `context/glossary.md`, `context/patterns.md`, `context/context-map.md`, `context/cli/cli-command-surface.md`
  - Evidence: `nix flake check` passed (all 13 checks green); grep confirms zero `sce_web.rs` / `services::sce_web` / `SCE_METADATA_VERSION` references in current-state context files (only plan files remain)
  - Goal: Update current-state context files to reflect the removal of `sce_web.rs`, the relocation of SCE web URL ownership to `agent_trace.rs`, and the version constant deduplication.
  - Boundaries (in/out of scope): In — `context/overview.md`, `context/architecture.md`, `context/glossary.md`, `context/patterns.md`, `context/context-map.md`, `context/cli/cli-command-surface.md`, and any other context files that reference `sce_web.rs` or `SCE_METADATA_VERSION`. Out — historical/reference docs, unrelated context churn.
  - Done when: Current-state context files no longer reference `sce_web.rs` as a live module; `SCE_WEB_BASE_URL` ownership is documented as residing in `agent_trace.rs`; version constant dedup is reflected; `context/context-map.md` entries are accurate.
  - Verification notes (commands or checks): Grep context directory for `sce_web` references and verify only historical docs remain; grep for `SCE_METADATA_VERSION` and verify it is documented as removed/consolidated.
  - Context-sync classification: localized doc updates following module reorganization.

- [x] T04: `Validate and clean up` (status:done)
  - Task ID: T04
  - Completed: 2026-06-11
  - Files changed: `context/cli/cli-command-surface.md` (removed stale `sce_web` from service domains list — drift found and corrected)
  - Evidence: `nix flake check` passed (all 13 checks green); `nix run .#pkl-check-generated` passed ("Generated outputs are up to date"); grep for `sce_web::`, `SCE_METADATA_VERSION`, `SESSION_RELATED_URL_PREFIX` returned zero results in application code; `sce_web` removed from remaining current-state context file; all four tasks complete, plan ready for closure.
  - Goal: Run final validation, confirm no stale references remain, and record evidence.
  - Boundaries (in/out of scope): In — full `nix flake check`, `nix run .#pkl-check-generated`, codebase grep for stale `sce_web` imports/SESSION_RELATED_URL_PREFIX/SCE_METADATA_VERSION references, final plan status update. Out — unrelated refactors, additional cleanup.
  - Done when: All validation commands pass; no stale references to deleted constants/files remain in code or context; plan marked complete.
  - Verification notes (commands or checks): `nix flake check`; `nix run .#pkl-check-generated`; grep for `sce_web::`, `SCE_METADATA_VERSION`, `SESSION_RELATED_URL_PREFIX` expecting zero results in application code.
  - Context-sync classification: validation-only; no new context edits unless drift is found.

## Validation Report

### Commands run
- `nix flake check` → exit 0 (all 13 checks passed: cli-tests, cli-clippy, cli-fmt, integrations-install-tests, integrations-install-clippy, integrations-install-fmt, pkl-parity, npm-bun-tests, npm-biome-check, npm-biome-format, config-lib-bun-tests, config-lib-biome-check, config-lib-biome-format)
- `nix run .#pkl-check-generated` → exit 0 ("Generated outputs are up to date")
- `grep -rn 'sce_web' context/ --include='*.md' | grep -v 'context/plans/' | grep -v 'context/decisions/'` → 0 matches (no current-state context references)
- `fff_multi_grep` for `sce_web::`, `SCE_METADATA_VERSION`, `SESSION_RELATED_URL_PREFIX` → 0 matches in application code (only plan file references remain)

### Success-criteria verification
- [x] `cli/src/services/sce_web.rs` is deleted and `pub mod sce_web;` is removed from `services/mod.rs` — confirmed via grep
- [x] `SCE_WEB_BASE_URL` has one canonical definition in `agent_trace.rs` (`pub(crate)`) — confirmed at line 32
- [x] `agent_trace_conversation_url`, `agent_trace_persisted_url`, and `agent_trace_session_url` live in `agent_trace.rs` — confirmed
- [x] `SESSION_RELATED_URL_PREFIX` is removed — zero grep matches in code
- [x] `SCE_METADATA_VERSION` is removed from `agent_trace.rs`; `version::PACKAGE_VERSION` (made `pub(crate)`) is used instead — confirmed
- [x] `config_schema_url` is inlined in `setup/mod.rs` — confirmed
- [x] All consumers import from `services::agent_trace` instead of `services::sce_web` — confirmed via grep
- [x] `nix flake check` passes — confirmed (all 13 checks green)
- [x] Context docs no longer reference `sce_web.rs` as a current-state file — confirmed (one drift in `context/cli/cli-command-surface.md` found and corrected)

### Drift found and corrected
- `context/cli/cli-command-surface.md` line 14 still listed `sce_web` in the service domains enum. Removed `sce_web` and added `agent_trace` to reflect current module structure.

### Residual risks
- None identified. All four tasks complete, all checks green, zero stale references remain.

## Open questions

- None.
