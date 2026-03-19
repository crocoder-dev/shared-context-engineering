# Plan: sce-doctor-database-and-config-coverage

## Change summary

Expand `sce doctor` so it covers the remaining gaps around SCE-owned database visibility and config validation, while moving `sce/config.json` schema ownership out of Rust and into canonical Pkl generation. The work first audited what `sce doctor` still missed, then added deterministic doctor coverage for repo-local databases plus a discoverable inventory of all databases created by SCE, and now needs to migrate the config schema so Pkl generates the canonical artifact into `config/` and the CLI embeds that generated file at compile time.

The requested scope includes three concrete outcomes:

- identify what `sce doctor` does not yet cover and record how that audit updates this plan
- add a doctor-facing way to list databases relevant to the current repo and to list all databases created by SCE
- validate both global and repo-local `sce/config.json` files against a canonical JSON Schema that is generated from Pkl into `config/` and reused by both `sce config validate` and `sce doctor`

## Success criteria

- The plan starts with an explicit audit task that inventories uncovered `sce doctor` gaps and records a deterministic rule for how newly found gaps are added back into this plan before implementation proceeds.
- `sce doctor` reports the current repo's SCE-managed database surfaces, including repo-local MCP cache databases and other repo-scoped SCE databases already created by the CLI.
- `sce doctor` exposes a deterministic way to list all databases created by SCE, separate from the current-repo readiness view.
- A canonical JSON Schema for `sce/config.json` is authored in Pkl, generated into `config/`, and covers both global and repo-local config files.
- The CLI embeds the generated schema artifact at compile time rather than keeping the source-of-truth schema inline in Rust.
- `sce config validate` and `sce doctor` share the generated config-schema contract rather than duplicating validation rules.
- Durable context documents the expanded doctor coverage, database inventory contract, and shared config-schema ownership.

## Constraints and non-goals

- Keep `sce doctor` as the canonical health-check entrypoint; do not introduce a separate top-level diagnostics command for this work unless the audit proves it is required and the plan is explicitly updated.
- Preserve diagnosis-by-default behavior; any new inventory/reporting surface under doctor must remain read-only unless an existing approved repair path applies.
- Treat code as source of truth for existing SCE-created database locations and ownership boundaries.
- Scope config validation to both global and repo-local `sce/config.json` files; out of scope are unrelated JSON files, broader schema systems for non-SCE config, and unrelated generated-config behavior outside the config schema artifact needed for this migration.
- Keep task slicing atomic: audit, runtime behavior changes, schema introduction, context sync, and validation must land as separate coherent commits.
- Do not silently expand implementation to every possible doctor enhancement found during the audit; the audit must classify findings and update this plan with approved follow-on tasks or explicit defer decisions.

## Assumptions

- "Only the repo's databases" means doctor's readiness view should focus on the active repository plus a separate explicit inventory path for all SCE-created databases across the local machine.
- The all-databases inventory should cover databases created by current SCE features, including MCP cache storage and Agent Trace persistence, and should not attempt to enumerate arbitrary non-SCE databases.
- The audit task may revise this plan file before implementation tasks proceed, but it must preserve stable task IDs for any task not materially resliced.
- The interim Rust-owned schema artifact introduced earlier is a stepping stone, not the final ownership model; the final state for this plan is Pkl-authored schema generation into `config/` with compile-time embedding in `cli`.

## Audit findings (T01)

- Current `sce doctor` coverage matches the earlier operator-environment slice: it reports global config locations, validates the global `state_root/sce/config.json` when present, reports the canonical Agent Trace local DB path, and diagnoses repo hook rollout state.
- Current `sce doctor` does not yet validate repo-local `.sce/config.json`; it only reports that path as an expected/present location. Shared global+local schema validation therefore remains a real implementation gap and stays in scope for `T04` + `T05`.
- Current `sce doctor` does not yet report repo-scoped SCE database surfaces beyond the global Agent Trace DB path. In particular, it does not surface the active repo's MCP cache database path or classify repo-scoped database readiness.
- Current command surface has no explicit way to request an all-SCE-databases inventory. `cli/src/cli_schema.rs` only exposes `sce doctor [--fix] [--format <text|json>]`, so the plan must explicitly cover command-surface/output-shape work for the all-databases listing instead of assuming it is only a `doctor.rs` internal change.
- Current code-owned SCE database inventory is narrower than a generic "all sqlite files" scan and should stay ownership-based. The audit confirms two canonical SCE database families today: the global Agent Trace database at `${state_root}/sce/agent-trace/local.db` and repo-scoped MCP cache databases at `${state_root}/sce/cache/repos/<repo-hash>/cache.db`.
- Current MCP cache global config file at `${state_root}/sce/cache/config.json` is related state, but it is not a database and should not be listed in the all-databases inventory except as supporting context if the future contract needs it.
- No additional doctor gaps were approved into this plan beyond database inventory and shared config-schema validation. Other possible doctor enhancements remain out of scope unless a future plan explicitly adds them.

## Clarified direction

- The canonical `sce/config.json` JSON Schema source of truth must move into Pkl authoring and generate into `config/`.
- Rust must stop owning the schema definition and instead embed the generated schema artifact in `cli` at compile time.
- Remaining scope covers the full doctor/config coverage stream, but anything outside this database-inventory plus generated-schema migration remains out of scope.

## Audit-driven plan update rule

- Any newly discovered doctor gap found during later tasks must be classified immediately as one of: `in-scope for an existing task`, `requires a new approved follow-on task in this plan`, or `deferred/out-of-scope`.
- A newly discovered gap may be folded into an existing task only when it keeps that task a single coherent atomic commit and does not change the task's core goal.
- If a discovered gap requires new command-surface, runtime, validation, or context work that would broaden an existing task beyond one coherent commit, this plan must be updated before implementation proceeds further.
- Deferred gaps must be recorded in this plan's notes or open-questions area with a short rationale so later sessions do not silently rediscover and re-scope them.

## Task stack

- [x] T01: `Audit uncovered doctor gaps and update this plan` (status:done)
  - Task ID: T01
  - Goal: Inspect current `sce doctor`, related context contracts, and code-owned SCE surfaces to inventory what doctor still does not cover, then update this plan with any newly approved tasks, defer decisions, or tightened acceptance criteria before feature implementation starts.
  - Boundaries (in/out of scope): In - doctor gap audit, database/config coverage inventory, explicit plan-update procedure, and edits to this plan file only. Out - application-code changes, implementation of new checks, or context sync beyond plan artifacts.
  - Done when: This plan file records the audit findings, identifies which new gaps are in scope vs deferred, and captures any needed task-stack adjustments without bundling runtime changes.
  - Verification notes (commands or checks): Review current doctor contract, existing config contract, and SCE database-owning services; confirm the updated plan preserves one-task/one-atomic-commit slicing and states how discovered gaps map to follow-on tasks or deferrals.
  - Completed: 2026-03-18
  - Evidence: Reviewed `context/sce/agent-trace-hook-doctor.md`, `context/cli/config-precedence-contract.md`, `cli/src/services/doctor.rs`, `cli/src/services/local_db.rs`, and `cli/src/services/mcp.rs`; confirmed current gaps are repo-local config validation, repo-scoped MCP/DB inventory, and missing all-databases doctor command surface.
  - Notes: Audit kept the task stack stable, but tightened `T02` and `T03` so the all-databases inventory explicitly includes doctor command-surface and output-shape design/implementation.

- [x] T02: `Define the doctor database inventory contract` (status:done)
  - Task ID: T02
  - Goal: Write the current-state contract for how `sce doctor` reports database coverage, distinguishing current-repo readiness from explicit all-SCE-databases inventory, including MCP databases and the doctor command surface needed to request that inventory.
  - Boundaries (in/out of scope): In - focused `context/sce/` contract updates, output-shape expectations, doctor CLI-surface expectations for repo vs all-database views, ownership rules for SCE-created databases, and doctor/setup alignment for future database-owning surfaces. Out - Rust implementation and non-database doctor changes.
  - Done when: Context clearly defines which databases appear in repo-scoped doctor output, how all-SCE database listing is requested and rendered, and how future SCE-created databases must register doctor coverage.
  - Verification notes (commands or checks): Read-through against existing doctor, MCP, and local DB contracts; ensure repo-scoped vs global inventory behavior is deterministic and ownership-based.
  - Completed: 2026-03-18
  - Evidence: Added `context/sce/doctor-database-inventory-contract.md`; linked `context/sce/agent-trace-hook-doctor.md` to the focused database inventory contract after read-through against `context/sce/mcp-smart-cache-storage-foundation.md` and `context/cli/config-precedence-contract.md`.
  - Notes: The contract keeps root shared files unchanged for now; `T06` remains responsible for root-level discoverability after implementation lands.

- [x] T03: `Implement doctor inventory for repo-scoped and all-SCE databases` (status:done)
  - Task ID: T03
  - Goal: Extend doctor runtime/output so it can surface the active repo's SCE-managed databases and provide a deterministic listing of all databases created by SCE, including MCP cache databases.
  - Boundaries (in/out of scope): In - `cli/src/cli_schema.rs`, `cli/src/app.rs`, `cli/src/command_surface.rs`, `cli/src/services/doctor.rs`, database path-discovery helpers, JSON/text output fields, and targeted tests for repo-local vs all-SCE inventory cases. Out - config-schema work and unrelated remediation behavior.
  - Done when: Doctor reports current-repo database readiness and can list all SCE-created databases through a stable explicit doctor surface with deterministic text/JSON output, covering MCP cache databases and existing Agent Trace persistence surfaces where applicable.
  - Verification notes (commands or checks): Targeted CLI/service tests for doctor parser/help coverage, repo-scoped database reporting, cross-repo/all-database inventory rendering, and missing/unexpected-path handling.
  - Completed: 2026-03-18
  - Evidence: Added `--all-databases` doctor surface plus repo/all inventory JSON+text records in `cli/src/services/doctor.rs`, reused Smart Cache ownership metadata through new `cli/src/services/mcp.rs` inventory helpers, and added parser/help/runtime coverage in `cli/src/cli_schema.rs`, `cli/src/app.rs`, and doctor tests including all-database JSON rendering.
  - Notes: Repo-scoped inventory now reports the active repository's Smart Cache database readiness without treating a missing cache DB as a hook/global-state failure; all-databases mode lists canonical Agent Trace state plus registered/discovered Smart Cache databases in deterministic order.

- [x] T04: `Author a canonical JSON Schema for sce config` (status:done)
  - Task ID: T04
  - Goal: Introduce one canonical JSON Schema that defines valid structure and fields for both global and repo-local `sce/config.json` files.
  - Boundaries (in/out of scope): In - schema source file, schema ownership location, field coverage for current config keys, and tests or fixtures proving the schema matches the existing config contract. Out - doctor runtime changes and unrelated config-surface expansion.
  - Done when: A single schema artifact exists for `sce/config.json`, covers the currently supported keys and constraints, and is documented as the canonical validation source.
  - Verification notes (commands or checks): Schema-level tests/fixtures for valid and invalid config examples; read-through against `context/cli/config-precedence-contract.md` to confirm parity.
  - Completed: 2026-03-18
  - Evidence: Added the canonical schema artifact as `SCE_CONFIG_SCHEMA_JSON` in `cli/src/services/config.rs`; added schema-focused tests for valid shape, unknown keys, invalid enums, conflicting presets, built-in ID collisions, empty argv tokens, and preset-catalog parity.
  - Notes: This is an important change for context sync because the config contract now has a code-owned canonical schema artifact that later runtime validation tasks will reuse.

- [x] T05: `Move sce config schema ownership into Pkl and reuse it from cli` (status:done)
  - Task ID: T05
  - Goal: Replace the interim Rust-owned `sce/config.json` schema source with a canonical Pkl-authored schema artifact generated into `config/`, then wire the CLI to embed and consume that generated artifact for both `sce config validate` and doctor.
  - Boundaries (in/out of scope): In - canonical Pkl schema source, generated schema artifact path under `config/`, compile-time embedding changes in `cli`, shared runtime validation for `sce config validate` and doctor, deterministic diagnostics, and targeted tests proving generated-schema behavior. Out - new config keys, non-JSON config formats, unrelated generated target config changes, and unrelated doctor audits.
  - Done when: One generated schema artifact under `config/` is documented as the canonical source, the CLI embeds that generated file at compile time, and both command surfaces validate the same config inputs through it with stable actionable diagnostics.
  - Verification notes (commands or checks): Targeted tests for global-only, local-only, merged-config, invalid-schema, and unknown-key scenarios through both `sce config validate` and `sce doctor`; generated-artifact parity check for the new schema output.
  - Completed: 2026-03-18
  - Evidence: Added canonical Pkl schema source at `config/pkl/base/sce-config-schema.pkl`, generated `config/schema/sce-config.schema.json` from `config/pkl/generate.pkl`, switched `cli/src/services/config.rs` to compile-time `include_str!` embedding plus runtime schema validation via the generated artifact, extended doctor to validate repo-local `.sce/config.json`, and added focused config/doctor tests covering generated-schema failures and local doctor validation.
  - Notes: Structural config validation now flows through the generated schema artifact while Rust keeps semantic checks for catalog-backed policy conflicts, duplicate custom prefixes, and output/reporting behavior.

- [x] T06: `Sync doctor/database/config context` (status:done)
  - Task ID: T06
  - Goal: Update durable context so future sessions can discover the new doctor database inventory behavior and the Pkl-generated `sce/config.json` schema ownership + compile-time embedding contract.
  - Boundaries (in/out of scope): In - `context/overview.md`, `context/context-map.md`, `context/glossary.md`, and focused `context/sce/` or `context/cli/` docs required by the final current-state behavior. Out - implementation beyond documentation/context artifacts.
  - Done when: Current-state context reflects the expanded doctor surface, database inventory behavior, and Pkl-generated schema ownership without stale Rust-owned-schema wording.
  - Verification notes (commands or checks): Read-through audit for stale duplicated validation-rule prose, stale Rust-owned-schema references, and missing references to the database inventory contract plus generated schema location.
  - Completed: 2026-03-18
  - Evidence: Verified root context references in `context/overview.md`, `context/context-map.md`, and `context/glossary.md`; refreshed focused wording in `context/cli/placeholder-foundation.md` and `context/sce/agent-trace-hook-doctor.md` so doctor now explicitly documents repo-local config validation alongside repo/all database inventory and the generated schema contract.
  - Notes: Classified as an important change because the task updates repository-wide doctor/config terminology and durable current-state discoverability.

- [x] T07: `Validation and cleanup` (status:done)
  - Task ID: T07
  - Goal: Run final verification, confirm context sync, and ensure the audit findings, doctor behavior, and shared schema contract all align.
  - Boundaries (in/out of scope): In - repo validation, targeted doctor/config tests, and final plan/context verification. Out - new feature work beyond fixes required by failed checks.
  - Done when: Verification passes, this plan reflects any audit-driven reslicing, doctor/database/config docs match code truth, generated schema artifacts are in sync, and no temporary scaffolding remains.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; `nix flake check`; targeted CLI/service tests for doctor database inventory and generated shared config-schema validation.
  - Completed: 2026-03-18
  - Evidence: Verified generated parity with `nix run .#pkl-check-generated`; ran `nix flake check`; ran focused CLI/service tests `parse_doctor_all_databases`, `parser_routes_doctor_all_databases_mode`, `render_all_database_inventory_json_includes_global_and_repo_records`, `doctor_reports_local_config_validation_failures`, and `validate_config_file_reports_generated_schema_path_for_invalid_shape`.
  - Notes: No additional in-scope fixes were required during validation; durable context already matched the implemented doctor database inventory and generated config-schema contract before final sync/validation.

## Open questions

- None. Scope is clarified: repo-scoped doctor coverage plus an explicit all-SCE-databases inventory, and generated-schema validation for both global and repo-local `sce/config.json` with Pkl as the canonical source and compile-time embedding in `cli`.

## Validation report

### Commands run

- `nix run .#pkl-check-generated` -> exit 0 (`Generated outputs are up to date.`)
- `nix flake check` -> exit 0 (all configured flake checks evaluated and ran successfully)
- `nix develop -c sh -c 'cd cli && cargo test parse_doctor_all_databases'` -> exit 0 (1 passed)
- `nix develop -c sh -c 'cd cli && cargo test parser_routes_doctor_all_databases_mode'` -> exit 0 (1 passed)
- `nix develop -c sh -c 'cd cli && cargo test render_all_database_inventory_json_includes_global_and_repo_records'` -> exit 0 (1 passed)
- `nix develop -c sh -c 'cd cli && cargo test doctor_reports_local_config_validation_failures'` -> exit 0 (1 passed)
- `nix develop -c sh -c 'cd cli && cargo test validate_config_file_reports_generated_schema_path_for_invalid_shape'` -> exit 0 (1 passed)

### Context sync verification

- Classification: verify-only for `T07`; final validation/cleanup changed execution evidence, not the implemented doctor/config/database contract.
- Verified shared root context remains aligned with code truth in `context/overview.md`, `context/architecture.md`, `context/glossary.md`, `context/patterns.md`, and `context/context-map.md`.
- Verified feature-level documentation remains present and discoverable in `context/sce/doctor-database-inventory-contract.md`, `context/sce/agent-trace-hook-doctor.md`, and `context/cli/config-precedence-contract.md`.

### Success-criteria verification

- [x] The plan starts with an explicit audit task and deterministic update rule -> confirmed in `T01` audit findings and the `Audit-driven plan update rule` section.
- [x] `sce doctor` reports current-repo SCE-managed databases -> confirmed by `render_all_database_inventory_json_includes_global_and_repo_records` and current-state docs in `context/sce/doctor-database-inventory-contract.md`.
- [x] `sce doctor` exposes a deterministic all-SCE database inventory -> confirmed by parser/routing tests for `--all-databases`, the doctor inventory test, and current-state docs in `context/sce/doctor-database-inventory-contract.md`.
- [x] Canonical `sce/config.json` JSON Schema is authored in Pkl and generated into `config/` -> confirmed by `config/pkl/base/sce-config-schema.pkl` and `config/schema/sce-config.schema.json` remaining in sync via `nix run .#pkl-check-generated`.
- [x] CLI embeds the generated schema artifact at compile time -> confirmed by current code truth in `cli/src/services/config.rs` and retained passing validation coverage.
- [x] `sce config validate` and `sce doctor` share the generated schema contract -> confirmed by `doctor_reports_local_config_validation_failures`, `validate_config_file_reports_generated_schema_path_for_invalid_shape`, and current-state docs in `context/cli/config-precedence-contract.md`.
- [x] Durable context documents doctor coverage, database inventory, and schema ownership -> confirmed by the verified root/domain context set above.

### Failed checks and follow-ups

- None.

### Residual risks

- None identified from task-scope validation.
