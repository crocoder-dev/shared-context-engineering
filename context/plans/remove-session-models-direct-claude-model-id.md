# Plan: Remove `session_models` and write Claude model IDs directly to `diff_traces`

## Change summary

Remove the Agent Trace `session_models` fallback model-attribution path and make `diff_traces.model_id` the only primary persisted model-attribution source for diff traces. For Claude Code, attempt to extract `model_id` directly from each supported `PostToolUse` diff-trace payload and store it in `diff_traces.model_id` through `DiffTracePayload`; if Claude omits model metadata, the value remains `NULL` and generated Agent Trace JSON omits `contributor.model_id` per the current schema/Serde contract.

This intentionally accepts the known Claude limitation that some events, especially after `/clear`, may not include model metadata. The change favors a simpler data model over session-level fallback enrichment.

## Success criteria

- `session_models` is no longer part of the Agent Trace DB schema for fresh databases.
- Rust Agent Trace DB code no longer exposes `SessionModelUpsert`, `SessionModelAttribution`, session-model SQL constants, upsert helpers, or lookup helpers.
- `sce hooks session-model` is removed from CLI parsing, runtime dispatch, generated Claude settings, and generated OpenCode plugin/runtime integration if present.
- `sce hooks diff-trace` no longer looks up missing attribution from `session_models`.
- Claude structured diff-trace parsing attempts to set `DiffTracePayload.model_id` from the raw `PostToolUse` payload using the existing Claude model extraction/normalization rules.
- `diff_traces.model_id` remains nullable and receives only the direct payload-derived model attribution.
- Agent Trace JSON remains valid when `model_id` is absent because `Contributor.model_id: Option<String>` is omitted and Trace Record Schema only requires contributor `type`.

## Constraints and non-goals

- Do not invent or persist placeholder model IDs such as `unknown`; absent Claude model metadata remains `NULL`/omitted.
- Do not add a replacement session-level attribution table in this plan.
- Do not change the Agent Trace record schema to require `model_id`.
- Do not change contributor classification semantics (`ai`, `mixed`, `unknown`).
- Preserve OpenCode normalized diff-trace behavior except for removing any now-unused session-model producer path.
- Existing databases may already have `session_models`; the implementation task must choose a safe migration strategy, but runtime code should stop depending on that table.

## Task stack

- [x] T01: `Remove session-model command surface and generated producers` (status:done)
  - Task ID: T01
  - Goal: Stop producing and routing `sce hooks session-model` events.
  - Boundaries (in/out of scope):
    - In — Remove `SessionModel` hook subcommand variants/parsing/help references; remove generated Claude `SessionStart -> sce hooks session-model` hook registration; remove OpenCode session-model producer wiring if present; update generated config from canonical Pkl/source as required.
    - Out — Agent Trace DB schema/code removal; diff-trace attribution behavior changes; context documentation updates beyond any inline generated output comments required by code.
  - Done when: `sce hooks session-model` is no longer a supported runtime route; generated Claude/OpenCode assets no longer call it; generated-output parity can be restored after regeneration.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; targeted parser/help tests if existing; generated config diff inspection for removed session-model hook commands.
  - Completion evidence (2026-06-30 rerun): Removed the `session-model` hooks subcommand from clap parsing/runtime conversion and `HookSubcommand` dispatch, deleted the session-model hook runtime/parser from `hooks/mod.rs`, and removed generated Claude `SessionStart -> sce hooks session-model` registration from canonical Pkl plus regenerated `config/.claude/settings.json`. Left Agent Trace DB `session_models` API/schema and diff-trace fallback in place for later tasks; temporarily marked now-unused session-model DB upsert APIs with `#[allow(dead_code)]` to preserve T02 scope. `nix run .#pkl-check-generated` passed (`Generated outputs are up to date.`). `nix flake check` passed (`all checks passed`).

- [x] T02: `Remove session_models database API and schema dependency` (status:done)
  - Task ID: T02
  - Goal: Remove Rust DB adapter support for `session_models` and ensure fresh Agent Trace DB schema no longer creates or requires that table.
  - Boundaries (in/out of scope):
    - In — Remove session-model SQL constants, structs, upsert/lookup methods, row mappers, and tests; remove or supersede migration `015_create_session_models.sql` from the fresh schema path using the repository's migration policy; update migration readiness expectations accordingly.
    - Out — Diff-trace parser behavior; generated hook config; unrelated message/part tables.
  - Done when: `agent_trace_db` compiles without any session-model API; fresh Agent Trace DB creation no longer includes `session_models`; migration/readiness tests reflect the new schema contract.
  - Verification notes (commands or checks): `nix flake check` (all checks passed).
  - Completion evidence (2026-06-30):
    - Deleted `cli/migrations/agent-trace/015_create_session_models.sql` from fresh schema path.
    - Kept `diff_traces.payload_type` in the fresh schema path as `cli/migrations/agent-trace/015_add_diff_traces_payload_type.sql`; existing development databases with a previous `016_add_diff_traces_payload_type` metadata ID may be deleted/recreated during this migration renumbering.
    - Added `DbSpec::retired_migration_ids()` to `cli/src/services/db/mod.rs` so existing upgraded DBs with applied `015_create_session_models` are not flagged as having unexpected migrations.
    - Removed `SessionModelUpsert`, `SessionModelAttribution`, `UPSERT_SESSION_MODEL_SQL`, `SELECT_SESSION_MODEL_SQL`, `upsert_session_model`, `session_model_by_tool_and_session`, `upsert_session_model_with`, `session_model_by_tool_and_session_with`, and `session_model_attribution_from_turso` from `cli/src/services/agent_trace_db/mod.rs`.
    - Added `AgentTraceDbSpec::retired_migration_ids()` returning `&["015_create_session_models"]` (also added to both test spec impls).
    - Updated baseline migration test: `assert!(!sqlite_object_exists(&db, "table", "session_models"))` confirms fresh DB no longer creates the table.
    - Removed hook-side session-model fallback attribution from `cli/src/services/hooks/mod.rs`:
      - `ResolvedDiffTraceAttribution` struct removed.
      - `resolve_diff_trace_attribution` function removed.
      - `resolve_attribution` closure in `run_diff_trace_subcommand_from_payload` removed.
      - `run_diff_trace_subcommand_from_payload_with` simplified to non-generic, passing `payload.model_id`/`payload.tool_version` directly to persistence.
      - Removed `session_model_attribution` test helper and four session-model resolution tests.
      - Removed `SessionModelAttribution` import.
    - `SessionModelUpsert`, `SessionModelAttribution`, `ResolvedDiffTraceAttribution`, `resolve_diff_trace_attribution`, `session_model_by_tool_and_session`, `upsert_session_model` no longer exist in the codebase.
    - `nix flake check` passed: all 84 tests pass, clippy clean, fmt clean, pkl-parity up to date.

- [x] T03: `Write direct Claude model_id into DiffTracePayload` (status:done)
  - Task ID: T03
  - Goal: Populate `DiffTracePayload.model_id` directly from supported Claude `PostToolUse` payloads when model metadata is present.
  - Boundaries (in/out of scope):
    - In — Reuse or refactor existing Claude model extraction/normalization logic (`model`, `model_id`, `modelId`, nested model identifiers, `claude/` prefix normalization) for diff-trace parsing; keep `model_id` optional; persist the direct value into `diff_traces.model_id` through the existing `DiffTraceInsert` path.
    - Out — Session-level fallback lookup; placeholder model values; Agent Trace schema changes.
  - Done when: Claude structured `DiffTracePayload` carries `Some(model_id)` when the raw `PostToolUse` payload includes model metadata and `None` when it does not; persisted `diff_traces.model_id` mirrors that direct value; tests cover present and omitted Claude model metadata.
  - Verification notes (commands or checks): targeted hooks tests for Claude diff-trace payload parsing/persistence; inspect `parse_claude_diff_trace_payload` no longer hardcodes `model_id: None` when payload has extractable model info.
  - Completion evidence (2026-07-01): Added direct Claude model extraction in `cli/src/services/hooks/mod.rs` for supported structured `PostToolUse` diff-trace payloads. The parser now reads direct `model`/`model_id`/`modelId` strings or nested `model.id`/`model.model`/`model.name`, trims non-empty values, normalizes them with the existing `claude/` prefix rule, and leaves `model_id=None` when absent. Added hooks tests covering direct model extraction plus DB insert mirroring, nested already-prefixed extraction, and omitted model metadata. Context sync classified this as an important localized Agent Trace runtime-contract change and refreshed current-state hook/DB context files plus root summaries. `cargo test claude_diff_trace_payload -- --exact` was attempted but blocked by the repository bash policy requiring `nix flake check`; `nix flake check` passed (`all checks passed`), and `nix run .#pkl-check-generated` passed (`Generated outputs are up to date.`).

- [x] T04: `Remove diff-trace session fallback and repair tests` (status:done)
  - Task ID: T04
  - Goal: Make `diff_traces` the sole primary model attribution source by removing fallback resolution from `session_models`.
  - Boundaries (in/out of scope):
    - In — Delete `ResolvedDiffTraceAttribution`, `resolve_diff_trace_attribution`, and DB lookup closure code if no longer needed; pass `payload.model_id`/`payload.tool_version` directly to DB insert; update tests that previously expected session fallback.
    - Out — Changing artifact persistence to `context/tmp`; changing OpenCode required/optional model validation except where compile cleanup requires it.
  - Done when: `sce hooks diff-trace` never queries `session_models`; direct payload model/tool metadata is persisted as-is; missing model metadata remains nullable and non-failing.
  - Verification notes (commands or checks): targeted hooks tests for direct model persistence and missing-model persistence; grep for `session_model`/`session_models` in Rust hook and DB code should only find historical context/docs before context sync.
  - Completion evidence (2026-07-01): Verified the active diff-trace runtime no longer contains `ResolvedDiffTraceAttribution`, `resolve_diff_trace_attribution`, or session-model DB lookup logic. `run_diff_trace_subcommand_from_payload_with` now passes `payload.model_id.as_deref()` and `payload.tool_version.as_deref()` directly to Agent Trace DB persistence, and the DB insert helper writes those values as-is. Repaired hook tests by renaming the direct-payload persistence test and adding missing-model coverage that asserts `model_id=None` and `tool_version=None` remain nullable/non-failing. Focused `cargo test diff_trace_db_persistence -- --exact` was attempted through `nix develop` but blocked by repository bash policy requiring `nix flake check`; `nix flake check` passed (`all checks passed`). Rust grep for `session_model|session_models|SessionModel|ResolvedDiffTraceAttribution|resolve_diff_trace_attribution` now finds only the retired migration ID and fresh-schema absence assertion in `agent_trace_db` code.

- [ ] T05: `Sync Agent Trace context and docs` (status:todo)
  - Task ID: T05
  - Goal: Update durable context/docs to describe the simplified direct `diff_traces.model_id` attribution model.
  - Boundaries (in/out of scope):
    - In — Update `context/sce/agent-trace-db.md`, `context/sce/agent-trace-hooks-command-routing.md`, `context/context-map.md`, and any focused context files that currently document `session_models` or session-model fallback; update generated command/help docs if owned by code generation; remove stale references from root overview/glossary only if they describe current runtime behavior.
    - Out — Adding historical narrative; editing unrelated Agent Trace design artifacts retained as historical references unless they claim current behavior.
  - Done when: durable current-state context no longer says active runtime uses `session_models`; docs state Claude model IDs are direct best-effort diff-trace metadata and may be nullable/omitted.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; focused grep for `session_models`/`session-model` in current-state docs to ensure remaining references are intentional historical references or removed.

- [ ] T06: `Validation and cleanup` (status:todo)
  - Task ID: T06
  - Goal: Run full repository validation and clean up stale artifacts after the session-model removal.
  - Boundaries (in/out of scope):
    - In — Full checks, generated-output parity, formatting/lint/test validation, removal of dead code/imports/tests, and final plan status/evidence updates.
    - Out — New product behavior beyond the planned session-model removal and direct Claude diff-trace model capture.
  - Done when: `nix run .#pkl-check-generated` and `nix flake check` pass; no dead `session_models` runtime references remain; plan execution evidence is recorded.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; `nix flake check`; optional targeted grep for `session_models`, `SessionModel`, and `session-model` to verify only intentional historical/docs references remain.

## Open questions

- Should existing user databases with an already-created `session_models` table actively drop it via a new migration, or is it acceptable to leave the unused table in upgraded databases while removing it from the active runtime/fresh schema? This should be decided during T02 before changing migrations.
