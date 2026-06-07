# Plan: cli-maintenance-hazards

## Change summary

Resolve the architectural/maintenance hazards in the Rust CLI without changing user-facing behavior:

1. Remove the duplicate `OutputFormat` and `LogLevel` type hierarchies split between `cli/src/cli_schema.rs` and service modules.
2. Remove the copy-pasted synchronous database operation methods shared by `TursoDb<M>` and `EncryptedTursoDb<M>` in `cli/src/services/db/mod.rs`, keeping encryption as the constructor-only difference.
3. Split the monolithic `cli/src/services/config/mod.rs` into focused config submodules for types, schema/loading, policy validation, runtime resolution, and rendering while preserving the existing `services::config` public API.

## Success criteria

- `cli/src/cli_schema.rs` no longer defines independent `OutputFormat` and `LogLevel` enums that duplicate service-owned runtime types.
- Adding a new CLI output format or log level requires changing one canonical enum owner plus intentional parser/rendering tests, not multiple parallel enum hierarchies and manual conversion functions.
- `TursoDb<M>` and `EncryptedTursoDb<M>` share one implementation path for `execute`, `query`, `query_map`, and `run_migrations`; their only substantive divergence is encrypted vs unencrypted connection construction.
- Existing database error messages, migration metadata behavior, and local/encrypted DB initialization behavior remain stable unless tests require a deterministic refactor-only adjustment.
- `cli/src/services/config/mod.rs` becomes a small module facade that declares/re-exports focused submodules instead of owning resolution, formatting, policy validation, rendering, and JSON schema concerns inline.
- Existing `sce config show`, `sce config validate`, startup config resolution, doctor config validation, auth config lookup, attribution-hooks config lookup, and observability config behavior remain unchanged.
- Rust formatting, linting, tests, generated-output parity, and repo-level validation pass using the repository-preferred checks.

## Constraints and non-goals

- Pure maintenance/refactor plan: no new CLI commands, flags, output formats, log levels, config keys, database features, migrations, or behavior changes.
- No new third-party dependencies.
- Preserve public/user-facing output contracts unless compile/tests reveal an unavoidable deterministic refactor-safe update.
- Preserve the current `services::config` import surface through facade re-exports so downstream modules do not need broad unrelated edits.
- Keep each executable task as one atomic commit unit; if implementation uncovers an independent behavior change, stop and split before proceeding.
- Prefer repository validation through `nix flake check`; use narrower Nix-wrapped checks only for targeted development feedback.

## Task stack

- [x] T01: `Unify CLI schema format and log-level enums with service-owned types` (status:done)
  - Task ID: T01
  - Goal: Remove duplicate `OutputFormat` and `LogLevel` enums from `cli/src/cli_schema.rs` by making clap parsing use the canonical service-owned enum types.
  - Boundaries (in/out of scope): In - `cli_schema.rs`, service enum derives/visibility needed for clap `ValueEnum`, parse-layer conversion removal, focused tests/fixtures affected by enum ownership. Out - adding variants, changing valid values, changing help text beyond type-path-neutral clap output, changing service rendering behavior.
  - Done when: `cli_schema.rs` imports/reuses canonical `services::output_format::OutputFormat` and `services::config::LogLevel`; manual `convert_output_format` / `convert_log_level` style mappings are removed or reduced to identity; all commands still accept the same `--format <text|json>` and `--log-level <error|warn|info|debug>` values.
  - Verification notes (commands or checks): Prefer `nix flake check`; if narrow feedback is needed, run Nix-wrapped CLI check/test commands for parser/config/version surfaces.
  - Completed: 2026-06-07
  - Files changed: `cli/src/cli_schema.rs`, `cli/src/services/output_format.rs`, `cli/src/services/config/mod.rs`, `cli/src/services/parse/command_runtime.rs`
  - Evidence: `nix flake check` passed. Narrow `cargo fmt --check && cargo check` attempt was blocked by repo policy in favor of flake validation; `nix develop -c sh -c 'cd cli && cargo fmt'` was run for rustfmt.
  - Notes: `cli_schema.rs` now reuses service-owned `OutputFormat` and `LogLevel`; duplicate schema-local enums and parse-layer conversion helpers were removed without adding variants or changing accepted values.

- [x] T02: `Deduplicate shared Turso operation methods` (status:done)
  - Task ID: T02
  - Goal: Factor `execute`, `query`, `query_map`, and `run_migrations` into one shared implementation path used by both `TursoDb<M>` and `EncryptedTursoDb<M>`.
  - Boundaries (in/out of scope): In - internal DB helper/core type or helper functions inside `cli/src/services/db/mod.rs`, method delegation from both public adapter types, preservation of existing public method signatures. Out - changing `DbSpec`, changing concrete DB specs, changing encryption-key behavior, adding sync/cloud behavior, adding migrations.
  - Done when: the duplicated method bodies no longer exist in both adapter impls; encrypted and unencrypted constructors still initialize connections exactly as before; all existing DB consumers compile unchanged.
  - Verification notes (commands or checks): Prefer `nix flake check`; inspect `cli/src/services/db/mod.rs` to confirm the operation/migration logic has one owner.
  - Completed: 2026-06-07
  - Files changed: `cli/src/services/db/mod.rs`
  - Evidence: `nix flake check` passed. Initial narrow `nix develop -c sh -c 'cd cli && cargo fmt --check && cargo check'` attempt was blocked by repo policy in favor of flake validation.
  - Notes: `TursoConnectionCore<M>` now owns the shared synchronous `execute`, `query`, `query_map`, and `run_migrations` implementation; `TursoDb<M>` and `EncryptedTursoDb<M>` keep separate constructors for unencrypted vs encrypted connection initialization and delegate public operation methods to the shared core.

- [x] T03: `Create config module facade and shared type submodule` (status:done)
  - Task ID: T03
  - Goal: Establish the config split by moving stable shared config types/constants into a focused submodule while keeping `services::config` re-exports/source compatibility.
  - Boundaries (in/out of scope): In - create a `types`-style submodule for config request/response primitives, log/config enums, source metadata, constants that are safe to move first, and facade re-exports from `mod.rs`. Out - moving resolution logic, schema validation, policy validation, or renderers.
  - Done when: `cli/src/services/config/mod.rs` starts acting as a module facade for shared config primitives; existing callers still import through `services::config::*` as before; behavior is unchanged.
  - Verification notes (commands or checks): Nix-wrapped compile/check or `nix flake check`; confirm no public API churn outside config-owned imports is needed.
  - Completed: 2026-06-07
  - Files changed: `cli/src/services/config/mod.rs`, `cli/src/services/config/types.rs`
  - Evidence: `nix flake check` passed (cli-tests, cli-clippy, cli-fmt, pkl-parity all green); `nix run .#pkl-check-generated` passed. No public API changes required outside `config/` module; all existing callers continue importing through `services::config::*`.
  - Notes: Created `cli/src/services/config/types.rs` containing `LogLevel`, `LogFormat`, `LogFileMode`, `ReportFormat`, `ConfigSubcommand`, `ConfigRequest`, `ValueSource`, `ConfigPathSource`, `LoadedConfigPath`, `ResolvedValue`, `ResolvedOptionalValue`, `ResolvedAuthRuntimeConfig`, `ResolvedObservabilityRuntimeConfig`, `ResolvedHookRuntimeConfig`, `NAME`, env key constants (`ENV_LOG_LEVEL`, `ENV_LOG_FORMAT`, `ENV_LOG_FILE`, `ENV_LOG_FILE_MODE`, `ENV_ATTRIBUTION_HOOKS_ENABLED`), and `parse_bool_value_from`. `mod.rs` now declares `pub mod types;` and `pub use types::*;` as facade re-exports. Resolution logic, schema validation, policy validation, and renderers remain in `mod.rs`.

- [x] T04: `Extract config schema loading and file parsing concerns` (status:done)
  - Task ID: T04
  - Goal: Move JSON schema embedding/validator setup, top-level allowed-key validation, serde DTO definitions, and config-file load/parse helpers out of `mod.rs` into a focused schema/loading submodule.
  - Boundaries (in/out of scope): In - schema constants, `OnceLock` validator ownership, JSON top-level validation, file parse/deserialization helpers, tests directly tied to schema/file parsing. Out - precedence resolution, rendering, policy-specific semantic validation unless already isolated as DTO parsing.
  - Done when: schema and config-file parsing have one focused owner; explicit vs default-discovered invalid-file behavior remains unchanged; `sce config validate` still reports the same issues/warnings for equivalent inputs.
  - Verification notes (commands or checks): Prefer `nix flake check`; include targeted config validation tests if available/needed.
  - Completed: 2026-06-07
  - Files changed: `cli/src/services/config/mod.rs`, `cli/src/services/config/schema.rs`
  - Evidence: `nix flake check` passed (cli-tests, cli-clippy, cli-fmt, pkl-parity all green); `nix run .#pkl-check-generated` passed. No public API changes required outside `config/` module; `validate_config_file` re-exported through `mod.rs` for `lifecycle.rs` and `doctor` consumers.
  - Notes: Created `cli/src/services/config/schema.rs` containing schema constants (`SCE_CONFIG_SCHEMA_JSON`, `CONFIG_SCHEMA_DECLARATION_KEY`, `TOP_LEVEL_CONFIG_KEYS`, `TOP_LEVEL_CONFIG_KEYS_DESCRIPTION`), `OnceLock` validator (`CONFIG_SCHEMA_VALIDATOR`, `config_schema_validator()`), JSON validation functions (`validate_config_value_against_schema`, `validate_object_keys`), serde DTOs (`ParsedFileConfigDocument`, `ParsedPoliciesConfigDocument`, `ParsedBashPolicyConfigDocument`, `ParsedAttributionHooksConfigDocument`, `ParsedCustomBashPolicyEntryDocument`, `ParsedCustomBashPolicyMatchDocument`), `FileConfigValue<T>`, `FileConfig`, type aliases (`ParsedBashPolicyConfig`, `ParsedFilePolicies`), and file parse/deserialization helpers (`validate_config_file`, `deserialize_typed_config`, `parse_file_config`, `map_policies_config`, `map_attribution_hooks_config`, `map_bash_policy_config`). `mod.rs` now declares `pub mod schema` and re-exports `validate_config_file` as `pub(crate)`. Bash-policy catalog/preset/validation functions (`builtin_bash_policy_catalog`, `parse_bash_policy_presets`, `parse_custom_bash_policies`, etc.) remain in `mod.rs` for T05 extraction. `WORKOS_CLIENT_ID_KEY` and `AuthConfigKeySpec` fields made `pub(crate)` for `schema.rs` access via `super::`.

- [x] T05: `Extract config policy semantic validation` (status:done)
  - Task ID: T05
  - Goal: Move bash-policy and attribution-hooks semantic validation/merge helpers into a focused policy submodule consumed by config resolution and rendering.
  - Boundaries (in/out of scope): In - built-in/custom bash-policy validation, duplicate/conflict/redundancy checks, attribution-hooks config parsing helpers, policy resolved-data structs if needed for cohesion. Out - changing policy schema, changing preset catalog generation, changing OpenCode plugin runtime behavior.
  - Done when: policy-specific rules are no longer interleaved with generic config resolution/rendering in `mod.rs`; existing warnings/errors for policy conflicts and redundancy remain stable.
  - Verification notes (commands or checks): Prefer `nix flake check`; run targeted config-policy tests if implementation adds/moves them.
  - Completed: 2026-06-07
  - Files changed: `cli/src/services/config/mod.rs`, `cli/src/services/config/policy.rs`, `cli/src/services/config/schema.rs`
  - Evidence: `nix flake check` passed (cli-tests, cli-clippy, cli-fmt, pkl-parity all green); `nix run .#pkl-check-generated` passed. No public API changes required outside `config/` module; all existing callers continue importing through `services::config` unchanged.
  - Notes: Created `cli/src/services/config/policy.rs` containing `BashPolicyConfig`, `BuiltinBashPolicyCatalog`, `BuiltinBashPolicyPreset`, `BuiltinBashPolicyMatcher`, `BuiltinBashPolicyRedundancyWarning`, `CustomBashPolicyEntry`, `BUILTIN_BASH_POLICY_CATALOG` OnceLock, `BASH_POLICY_PRESET_CATALOG_JSON` include_str, `builtin_bash_policy_catalog()`, `builtin_bash_policy_preset_ids()`, `is_builtin_bash_policy_preset_id()`, `parse_bash_policy_presets()`, `parse_custom_bash_policies()`, `parse_custom_bash_policy_entry()`, `parse_custom_bash_policy_match()`, `parse_custom_bash_policy_argv_prefix()`, `resolve_bash_policy_config()`, `build_validation_warnings()`, `format_bash_policies_text()`, `format_bash_policies_json()`. `mod.rs` now declares `pub mod policy` and imports `BashPolicyConfig`, `build_validation_warnings`, `format_bash_policies_json`, `format_bash_policies_text`, `resolve_bash_policy_config` from `policy`. `schema.rs` now imports `parse_bash_policy_presets`, `parse_custom_bash_policies`, `CustomBashPolicyEntry` from `super::policy` instead of `super`.

- [ ] T06: `Extract runtime config resolution and precedence flow` (status:todo)
  - Task ID: T06
  - Goal: Move config-file discovery, merge order, env/flag/default precedence, auth-key resolution, observability resolution, and invalid-default-discovered fallback flow into a focused resolver submodule.
  - Boundaries (in/out of scope): In - resolution functions consumed by startup, `sce config show/validate`, auth runtime, observability runtime, and attribution-hooks gate; source/provenance preservation. Out - rendering output shape, schema policy changes, adding new keys.
  - Done when: precedence behavior remains `flags > env > config file > defaults` where applicable; default-discovered invalid config still degrades gracefully while explicit config remains fatal; `mod.rs` delegates resolution instead of containing it inline.
  - Verification notes (commands or checks): Prefer `nix flake check`; include focused tests/smoke checks for `sce config show`, `sce config validate`, startup config loading, and attribution-hooks gate if available.

- [ ] T07: `Extract config text and JSON rendering` (status:todo)
  - Task ID: T07
  - Goal: Move `sce config show` and `sce config validate` text/JSON rendering into a focused render submodule without changing output contracts.
  - Boundaries (in/out of scope): In - text rendering, JSON response construction, display-value/redaction helpers that are rendering-specific, render tests/golden assertions if present. Out - changing resolved data semantics, schema validation, policy validation, or command parsing.
  - Done when: config renderers have one focused owner; output for representative `show` and `validate` cases remains stable; `mod.rs` is reduced to facade/orchestration-level exports and `run_config_subcommand` delegation.
  - Verification notes (commands or checks): Prefer `nix flake check`; include targeted config show/validate output tests if available/needed.

- [ ] T08: `Final validation and context sync` (status:todo)
  - Task ID: T08
  - Goal: Run full validation, remove temporary scaffolding, and sync durable context for the resulting CLI architecture and maintenance boundaries.
  - Boundaries (in/out of scope): In - full repo validation, generated-output parity, cleanup of task-owned temporary files, context updates for current-state architecture/glossary/domain docs. Out - new refactors or behavior changes beyond documenting the completed plan outcome.
  - Done when: `nix run .#pkl-check-generated` and `nix flake check` pass; config/DB/CLI enum context reflects the new ownership; this plan records validation evidence and any residual risks.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; `nix flake check`; verify `context/overview.md`, `context/architecture.md`, `context/glossary.md`, `context/cli/config-precedence-contract.md`, and `context/sce/shared-turso-db.md` are current or explicitly verified unchanged.

## Open questions

None. The request is treated as a refactor-only maintenance plan covering all three listed hazards with no user-facing behavior changes.
