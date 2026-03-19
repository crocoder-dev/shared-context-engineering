# Plan: sce-config-observability-settings

## Change summary

Extend `sce` runtime configuration so persistent logging and OpenTelemetry settings can be declared in `sce/config.json` and validated by the shared generated schema, instead of being env-only.
The config surface should keep existing top-level observability keys flat for logging (`log_level`, `log_format`, `log_file`, `log_file_mode`) while introducing OTEL settings under a nested `otel` object.

## Success criteria

- `sce/config.json` schema accepts and validates persistent logging keys plus nested OTEL settings.
- Runtime precedence remains deterministic and explicit: flags > env > config file > defaults for supported observability values.
- Existing env controls continue to work, with config-file values acting as the lower-precedence fallback.
- `sce config show` and `sce config validate` render the new observability settings and their sources deterministically in text and JSON output.
- Invalid config-file observability values fail schema or semantic validation with actionable guidance.
- CLI observability and config context files describe the new config-backed behavior, including the mixed flat-plus-nested shape.

## Constraints and non-goals

- Keep `log_level` as an existing top-level key; do not redesign the entire config file around a new top-level `observability` object.
- Use top-level snake_case keys for non-OTEL logging settings: `log_format`, `log_file`, `log_file_mode`.
- Use nested OTEL config only under `otel`.
- Preserve current env variable names and env support; this is an additive config-path expansion, not an env migration.
- Keep command flags limited to the current implemented surface unless implementation discovers a strong need for new flags.
- Do not broaden this change into unrelated config schema cleanup or output-format redesign.

## Assumptions

- The nested OTEL object will mirror the current env-controlled values with config keys `otel.enabled`, `otel.exporter_otlp_endpoint`, and `otel.exporter_otlp_protocol`.
- `sce config show|validate` should report config-derived observability values alongside existing resolved config values rather than inventing a separate command.
- Existing observability validation rules remain canonical: log level `error|warn|info|debug`, log format `text|json`, file mode `truncate|append`, OTEL protocol `grpc|http/protobuf`, and OTLP endpoint must be an absolute `http(s)` URL when enabled.

## Task stack

- [x] T01: Extend the canonical `sce/config.json` schema for observability settings (status:done)
  - Task ID: T01
  - Goal: Update the Pkl-authored schema and generated JSON Schema so repo-local and global config files can declare persistent logging keys plus nested OTEL settings.
  - Boundaries (in/out of scope): In - `config/pkl/base/sce-config-schema.pkl`, generated schema output expectations, allowed keys and value enums/types, nested `otel` object shape. Out - Rust runtime parsing or output rendering.
  - Done when: Schema authoring allows `log_format`, `log_file`, `log_file_mode`, and `otel.{enabled,exporter_otlp_endpoint,exporter_otlp_protocol}`; unknown-key behavior remains deterministic; required/conditional constraints are clearly defined for config validation.
  - Verification notes (commands or checks): Review generated schema shape against the planned config examples; verify allowed keys, enums, and nested object constraints are explicit and machine-validatable.
  - Completed: 2026-03-19
  - Files changed: `config/pkl/base/sce-config-schema.pkl`, `config/schema/sce-config.schema.json`
  - Evidence: `nix run .#pkl-check-generated` passed; `nix flake check` failed in existing test `services::hooks::tests::prompt_capture_flow_persists_and_queries_end_to_end` with `git add src/lib.rs failed: fatal: not a git repository (or any of the parent directories): .git`

- [x] T02: Add config-service resolution for logging and OTEL settings (status:done)
  - Task ID: T02
  - Goal: Extend config parsing/resolution so observability settings resolve from flags/env/config/defaults with the documented precedence contract.
  - Boundaries (in/out of scope): In - `cli/src/services/config.rs` resolved-value models, shared resolution helpers, config-file merge behavior, and precedence/source metadata for new keys. In - mapping nested `otel` config fields to the existing runtime concepts. Out - help copy and context/doc updates.
  - Done when: The config service can resolve top-level logging keys and nested OTEL keys from config files, apply env overrides, preserve existing defaults, and expose deterministic source metadata for each supported value.
  - Verification notes (commands or checks): Add targeted config-service tests covering config-only, env-over-config, discovered-global-plus-local override, and unset/default cases for each new observability field.
  - Completed: 2026-03-19
  - Files changed: `cli/src/services/config.rs`, `context/overview.md`, `context/architecture.md`, `context/glossary.md`, `context/cli/config-precedence-contract.md`, `context/sce/cli-observability-contract.md`, `context/context-map.md`
  - Evidence: `nix develop -c sh -c 'cd cli && cargo test config -- --nocapture'` passed (43 tests); `nix develop -c sh -c 'cd cli && cargo fmt --check'` passed; `nix develop -c sh -c 'cd cli && cargo build'` passed

- [x] T03: Wire observability runtime to consume config-backed settings (status:done)
  - Task ID: T03
  - Goal: Make app/runtime observability honor resolved config-file fallback values for persistent logs and OTEL without breaking existing env-driven behavior.
  - Boundaries (in/out of scope): In - `cli/src/services/observability.rs`, app wiring in `cli/src/app.rs` as needed, validation/error mapping, and redaction-safe handling of new config-backed file/endpoint inputs. Out - unrelated tracing/event contract changes.
  - Done when: Runtime observability uses env values when present, otherwise uses config-file values, otherwise defaults; file sink and OTEL startup behave consistently across env and config sources; actionable validation failures remain stable.
  - Verification notes (commands or checks): Add targeted runtime/app tests covering config-backed file logging enablement, `log_file_mode` dependency behavior, OTEL enablement via config, invalid endpoint/protocol handling, and env-over-config precedence.
  - Completed: 2026-03-19
  - Files changed: `cli/src/app.rs`, `cli/src/services/config.rs`, `cli/src/services/observability.rs`
  - Evidence: `nix develop -c sh -c 'cd cli && cargo test observability && cargo test config && cargo fmt --check && cargo build'` passed; `nix run .#pkl-check-generated` passed; `nix flake check` passed

- [x] T04: Expose the new observability settings in `sce config show` and `sce config validate` (status:done)
  - Task ID: T04
  - Goal: Update command output so operators can inspect and validate the newly supported observability settings and their provenance.
  - Boundaries (in/out of scope): In - text and JSON rendering contracts for `show` and `validate`, deterministic source/config_source reporting, and tests that lock the new output shape. Out - adding new top-level commands or changing unrelated output fields.
  - Done when: `sce config show` and `sce config validate` include the new logging and OTEL settings in a deterministic structure/text layout, including nested OTEL reporting that matches the schema surface.
  - Verification notes (commands or checks): Add output tests for text and JSON modes covering config-file, env, and default-sourced observability values.
  - Completed: 2026-03-19
  - Files changed: `cli/src/services/config.rs`, `context/plans/sce-config-observability-settings.md`
  - Evidence: `nix develop -c sh -c 'cd cli && cargo test config -- --nocapture'` passed (49 tests); `nix develop -c sh -c 'cd cli && cargo fmt --check'` passed; `nix develop -c sh -c 'cd cli && cargo build'` passed

- [x] T05: Update CLI help and current-state context for config-backed observability (status:done)
  - Task ID: T05
  - Goal: Sync durable docs and command guidance so future sessions treat config-backed persistent logs and OTEL settings as current behavior.
  - Boundaries (in/out of scope): In - `context/overview.md`, `context/architecture.md`, `context/glossary.md`, `context/cli/config-precedence-contract.md`, `context/sce/cli-observability-contract.md`, `context/context-map.md`, and command/help text if needed to mention the expanded config support. Out - unrelated documentation cleanup.
  - Done when: Current-state docs describe the new config keys, precedence behavior, schema-validation ownership, and the flat logging plus nested OTEL shape without stale env-only wording.
  - Verification notes (commands or checks): Read-through audit for stale observability/config wording; ensure focused context files remain the canonical reference points for future implementation sessions.
  - Completed: 2026-03-19
  - Files changed: `cli/src/cli_schema.rs`, `context/cli/config-precedence-contract.md`
  - Evidence: `nix develop -c sh -c 'cd cli && cargo test cli_schema && cargo fmt --check && cargo build'` passed; `nix run .#pkl-check-generated` passed; `nix flake check` failed in pre-existing `cli/src/services/config.rs` clippy/test derivations (`clippy::too_many_lines`, `clippy::uninlined_format_args`) outside this task's touched files

- [x] T06: Validation and cleanup (status:done)
  - Task ID: T06
  - Goal: Verify the implementation end to end, confirm generated-schema parity, and ensure context stays aligned with code truth.
  - Boundaries (in/out of scope): In - targeted CLI/config/observability tests, generated-schema parity validation, repo-level checks required by the repo baseline, and final context-sync verification. Out - new feature work beyond fixing discovered regressions.
  - Done when: Relevant tests pass, generated config/schema artifacts are in sync, observability config behavior is covered by current-state docs, and no stale env-only assumptions remain.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; `nix flake check`; targeted CLI tests for config and observability behavior if narrower checks are useful during implementation.
  - Completed: 2026-03-19
  - Files changed: `cli/src/services/config.rs`, `context/plans/sce-config-observability-settings.md`
  - Evidence: `nix develop -c sh -c 'cd cli && cargo test config'` passed (51 tests); `nix develop -c sh -c 'cd cli && cargo test observability'` passed (24 tests); `nix develop -c sh -c 'cd cli && cargo fmt --check'` passed; `nix develop -c sh -c 'cd cli && cargo build'` passed; `nix run .#pkl-check-generated` passed; `nix flake check` passed

## Open questions

- None. Scope is clarified: logging settings remain flat top-level config keys, and OTEL settings move under nested `otel` config with schema-backed validation.

## Handoff

- plan_name: `sce-config-observability-settings`
- plan_path: `context/plans/sce-config-observability-settings.md`
- next command: `/next-task sce-config-observability-settings T06`

## Validation Report

### Commands run

- `nix develop -c sh -c 'cd cli && cargo test config'` -> exit 0 (51 passed, 0 failed)
- `nix develop -c sh -c 'cd cli && cargo test observability'` -> exit 0 (24 passed, 0 failed)
- `nix develop -c sh -c 'cd cli && cargo fmt --check'` -> exit 0
- `nix develop -c sh -c 'cd cli && cargo build'` -> exit 0
- `nix run .#pkl-check-generated` -> exit 0 (`Generated outputs are up to date.`)
- `nix flake check` -> exit 0

### Failed checks and follow-ups

- Initial `nix flake check` failed on `clippy::too_many_lines` and `clippy::uninlined_format_args` in `cli/src/services/config.rs`; fixed by extracting observability-rendering helpers and inlining the remaining format argument, then reran targeted checks plus `nix flake check` successfully.
- No remaining failed checks.

### Success-criteria verification

- [x] `sce/config.json` schema accepts and validates persistent logging keys plus nested OTEL settings -> confirmed by existing schema/runtime coverage in `cli/src/services/config.rs` tests and final passing `nix flake check`.
- [x] Runtime precedence remains deterministic and explicit: flags > env > config file > defaults for supported observability values -> confirmed by passing config/observability tests and the current-state contracts in `context/overview.md`, `context/architecture.md`, and `context/glossary.md`.
- [x] Existing env controls continue to work, with config-file values acting as lower-precedence fallback -> confirmed by `cargo test config` and `cargo test observability` passing, including env-over-config coverage.
- [x] `sce config show` and `sce config validate` render new observability settings and their sources deterministically in text and JSON output -> confirmed by passing output tests in `cli/src/services/config.rs` and the lint-clean helper refactor in `cli/src/services/config.rs`.
- [x] Invalid config-file observability values fail schema or semantic validation with actionable guidance -> confirmed by passing validation tests in `cli/src/services/config.rs` and final `nix flake check`.
- [x] CLI observability and config context files describe the config-backed mixed flat-plus-nested behavior -> confirmed by verify-only context-sync pass over `context/overview.md`, `context/architecture.md`, `context/glossary.md`, `context/patterns.md`, `context/context-map.md`, `context/cli/config-precedence-contract.md`, and `context/sce/cli-observability-contract.md`.

### Residual risks

- None identified for this plan slice.
