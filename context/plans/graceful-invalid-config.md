# Plan: Graceful Handling of Invalid Config Files

## Change summary

When `.sce/config.json` (or the global config) contains invalid JSON or fails schema validation, the CLI currently hard-fails at startup with `Error [SCE-ERR-VALIDATION]: Invalid observability configuration: Config file '...' must contain valid JSON.` This blocks all commands — including `git commit` via hooks — even though config invalidity should be a non-blocking diagnostic issue. The fix makes invalid config a logged warning that falls back to defaults, ensures `sce doctor` reports invalid config, and ensures `sce config validate` can report invalid config instead of hard-failing before reaching the validate subcommand.

## Success criteria

1. When a discovered config file contains invalid JSON or fails schema validation, the CLI logs a warning and continues with defaults — no command is blocked by an invalid config file.
2. `sce doctor` reports invalid config files (global and local) as `[FAIL]` problems with `manual_only` fixability and actionable remediation text. This already works for the doctor-specific validation path; the change is ensuring the startup path no longer hard-fails so doctor can actually run.
3. `sce config validate` reports invalid config files with `valid: false` and the specific validation errors, instead of hard-failing before reaching the validate subcommand.
4. `sce config show` reports resolved defaults when config is invalid, with a clear indication that config was skipped due to validation errors.
5. Existing tests continue to pass; new tests cover the graceful-degradation paths.

## Constraints and non-goals

- **In scope**: Startup config-loading graceful degradation, `sce config validate` reporting invalid config, `sce doctor` reporting invalid config, `sce config show` degraded-mode output.
- **Out of scope**: Changes to the config schema itself, changes to `sce config validate` output shape beyond adding invalid-config reporting, changes to doctor's `--fix` behavior for config issues (already `manual_only`), changes to hook runtime behavior beyond using the gracefully-degraded config.
- **Non-goal**: Auto-repair of invalid config files. Doctor already marks config validation failures as `manual_only`.
- **Non-goal**: Changing the behavior when `--config <path>` or `SCE_CONFIG_FILE` points to a file that doesn't exist or can't be read — those remain hard failures because the user explicitly requested that file.

## Task stack

- [x] T01: `Make config file parsing tolerant of invalid files in resolve_runtime_config_with` (status:done)
  - Task ID: T01
  - Goal: Change `resolve_runtime_config_with` so that when a discovered (non-explicit) config file fails to parse or validate, the error is collected as a warning and the function continues with defaults rather than hard-failing. Explicit `--config` / `SCE_CONFIG_FILE` paths that fail remain hard errors.
  - Boundaries (in/out of scope): In — modifying `resolve_runtime_config_with` and `parse_file_config` error handling in `cli/src/services/config.rs`, collecting per-file validation errors into the `RuntimeConfig.validation_warnings` field or a new `validation_errors` field, ensuring the startup path in `cli/src/app.rs` no longer hard-fails on invalid discovered config. Out — changing `validate_config_file` (used by doctor), changing the config schema, changing `sce config validate` output shape (that's T02).
  - Done when: (1) `resolve_observability_runtime_config` returns `Ok(...)` with defaults when a discovered config file is invalid, logging the validation error as a warning; (2) `resolve_hook_runtime_config` and `resolve_auth_runtime_config` similarly degrade gracefully; (3) explicit `--config` / `SCE_CONFIG_FILE` paths that fail still hard-fail; (4) existing tests pass; (5) new unit tests cover the graceful-degradation path for invalid JSON, invalid schema, and missing top-level object.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo test config'`; `nix develop -c sh -c 'cd cli && cargo test'`; `nix flake check`
  - Completed: 2026-04-15
  - Files changed: `cli/src/services/config.rs`
  - Evidence: `nix develop -c sh -c 'cd cli && cargo test config'` ✅ (7 tests passed); `nix develop -c sh -c 'cd cli && cargo build'` ✅
  - Notes: Added `validation_errors` collection for invalid discovered config files; explicit `--config` and `SCE_CONFIG_FILE` failures still hard-fail.

- [x] T02: `Make sce config validate and sce config show report invalid config instead of hard-failing` (status:done)
  - Task ID: T02
  - Goal: Change `run_config_subcommand` for `Validate` and `Show` so that when config resolution encounters validation errors in discovered files, `sce config validate` reports `valid: false` with the specific errors and `sce config show` reports resolved defaults with a warning about skipped config.
  - Boundaries (in/out of scope): In — modifying `format_validate_output` and `format_show_output` (or their callers) in `cli/src/services/config.rs` to handle the new `validation_errors` field, updating `sce config validate` text/JSON output to report invalid config with specific error messages, updating `sce config show` to indicate config was skipped. Out — changing doctor output, changing the config schema, changing the startup path (that's T01).
  - Done when: (1) `sce config validate` with an invalid config file reports `valid: false` and lists the validation errors in both text and JSON formats; (2) `sce config show` with an invalid config file reports resolved defaults and includes a warning about the invalid config; (3) existing tests pass; (4) new tests cover the invalid-config reporting paths.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo test config'`; `nix develop -c sh -c 'cd cli && cargo test'`; `nix flake check`
  - Completed: 2026-04-15
  - Files changed: `cli/src/services/config.rs`
  - Evidence: `nix develop -c sh -c 'cd cli && cargo test config'` ✅ (9 tests passed); `nix develop -c sh -c 'cd cli && cargo build'` ✅
  - Notes: `config validate` now reports discovered invalid config via `valid: false` + `issues`; `config show` now keeps resolved defaults and surfaces skipped-invalid-config warnings in text and JSON.

- [x] T03: `Update app.rs startup to use graceful config resolution and log warnings` (status:done)
  - Task ID: T03
  - Goal: Update `try_run_with_dependency_check` in `cli/src/app.rs` so that when `resolve_observability_runtime_config` succeeds (even with config validation warnings), the logger is initialized with the degraded config and any validation warnings are logged. Remove the hard-failure `ClassifiedError::validation("Invalid observability configuration: ...")` path for discovered config errors.
  - Boundaries (in/out of scope): In — modifying `try_run_with_dependency_check` in `cli/src/app.rs` to log config validation warnings instead of hard-failing, ensuring the logger is initialized with degraded defaults when config is invalid. Out — changing `resolve_observability_runtime_config` itself (that's T01), changing `sce config validate`/`show` output (that's T02), changing doctor.
  - Done when: (1) An invalid `.sce/config.json` no longer prevents `sce version`, `sce doctor`, `sce hooks commit-msg`, or any other command from running; (2) validation warnings are logged at `warn` level; (3) the logger uses degraded defaults (e.g., `log_level=error`, `log_format=text`) when config is invalid; (4) existing tests pass; (5) new app-level tests cover the graceful-degradation path.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo test'`; `nix flake check`; manual test: create an invalid `.sce/config.json`, run `sce version` (should succeed with warning), run `sce doctor` (should report invalid config), run `sce config validate` (should report `valid: false`).
  - Completed: 2026-04-15
  - Files changed: `cli/src/app.rs`, `cli/src/services/observability.rs`
  - Evidence: `nix develop -c sh -c 'cd cli && cargo test app::tests::run_with_dependency_check_allows_invalid_discovered_config_and_logs_warning'` ✅ (1 passed); `nix develop -c sh -c 'cd cli && cargo test config'` ✅ (10 tests passed); `nix develop -c sh -c 'cd cli && cargo build'` ✅
  - Notes: Startup now keeps degraded observability defaults when discovered config is invalid, emits `sce.config.invalid_config` warning logs before dispatch, and no longer returns the old `Invalid observability configuration: ...` hard-failure path for discovered-file parse/schema errors.

- [x] T04: `Update context files to reflect graceful config handling` (status:done)
  - Task ID: T04
  - Goal: Update `context/sce/cli-observability-contract.md`, `context/cli/config-precedence-contract.md`, and `context/sce/agent-trace-hook-doctor.md` to document the new graceful-degradation behavior for invalid config files.
  - Boundaries (in/out of scope): In — updating context documentation files. Out — any code changes.
  - Done when: (1) `cli-observability-contract.md` documents that invalid discovered config files produce warnings and fall back to defaults; (2) `config-precedence-contract.md` documents the invalid-config graceful-degradation behavior and the `sce config validate` invalid-config reporting; (3) `agent-trace-hook-doctor.md` notes that doctor can now always run even when config is invalid.
  - Verification notes (commands or checks): Review updated context files for accuracy and completeness.
  - Completed: 2026-04-15
  - Files changed: `context/sce/cli-observability-contract.md`, `context/cli/config-precedence-contract.md`, `context/sce/agent-trace-hook-doctor.md`
  - Evidence: Reviewed target context files against current code references in `cli/src/services/config.rs` (`validation_errors`, skipped invalid config reporting) and `cli/src/app.rs` (`sce.config.invalid_config`) ✅
  - Notes: Context now states that invalid default-discovered config is warning-only at startup, `sce config validate` renders invalid-config results instead of hard-failing, and `sce doctor` can still run and report config problems.

- [x] T05: `Validation and cleanup` (status:done)
  - Task ID: T05
  - Goal: Run full validation suite, verify all success criteria, and clean up any temporary scaffolding.
  - Boundaries (in/out of scope): In — running `nix flake check`, `nix run .#pkl-check-generated`, verifying manual test scenarios, reviewing context sync. Out — new feature work.
  - Done when: (1) `nix flake check` passes; (2) `nix run .#pkl-check-generated` passes; (3) manual verification: invalid `.sce/config.json` → `sce version` succeeds with warning, `sce doctor` reports invalid config, `sce config validate` reports `valid: false`; (4) context files are consistent with code truth.
  - Verification notes (commands or checks): `nix flake check`; `nix run .#pkl-check-generated`; manual scenario testing.
  - Completed: 2026-04-15
  - Files changed: `cli/src/app.rs`, `context/plans/graceful-invalid-config.md`
  - Evidence: `nix develop -c sh -c 'cd cli && cargo clippy --all-targets --all-features'` ✅; `nix develop -c sh -c 'cd cli && cargo test app::tests::run_with_dependency_check_allows_invalid_discovered_config_and_logs_warning -- --exact'` ✅; `nix run .#pkl-check-generated` ✅; `nix flake check` ✅; manual temp-repo scenario with invalid `.sce/config.json` verified `sce version` succeeds with `sce.config.invalid_config` warning, `sce doctor` reports `[FAIL] Local config`, and `sce config validate` reports invalid config ✅
  - Notes: Final cleanup moved the `cli/src/app.rs` test module to the end of the file so the repo passes clippy's `items_after_test_module` policy; no extra temporary scaffolding required removal.

## Open questions

None — the change request is clear and the codebase behavior is well-understood.

## Validation Report

### Commands run

- `nix develop -c sh -c 'cd cli && cargo clippy --all-targets --all-features'` -> exit 0
- `nix develop -c sh -c 'cd cli && cargo test app::tests::run_with_dependency_check_allows_invalid_discovered_config_and_logs_warning -- --exact'` -> exit 0 (1 passed)
- `nix run .#pkl-check-generated` -> exit 0 (`Generated outputs are up to date.`)
- `nix flake check` -> exit 0

### Manual scenario verification

- Temporary git repo fixture with invalid `.sce/config.json` (`{`) created under `/tmp/tmp.d6Oj35GIyL`
- `nix run "/home/davidabram/repos/shared-context-engineering/master#sce" -- version` -> succeeded and emitted `sce.config.invalid_config` warning for the invalid discovered local config
- `nix run "/home/davidabram/repos/shared-context-engineering/master#sce" -- doctor` -> rendered `[FAIL] Local config (/tmp/tmp.d6Oj35GIyL/.sce/config.json)` while continuing command execution
- `nix run "/home/davidabram/repos/shared-context-engineering/master#sce" -- config validate` -> rendered `SCE config validation: invalid` plus `Config file '/tmp/tmp.d6Oj35GIyL/.sce/config.json' must contain valid JSON.`

### Cleanup

- Moved the `#[cfg(test)] mod tests` block in `cli/src/app.rs` to the end of the file to satisfy clippy's `items_after_test_module` policy
- No temporary scaffolding required removal

### Success-criteria verification

- [x] Discovered invalid config logs a warning and commands continue with defaults — confirmed by manual `sce version` run with `sce.config.invalid_config` warning and successful version output
- [x] `sce doctor` reports invalid config files as failures — confirmed by manual doctor output showing `[FAIL] Local config (...)`
- [x] `sce config validate` reports invalid config instead of hard-failing — confirmed by manual validate output showing `SCE config validation: invalid` and the specific parse error
- [x] `sce config show` reports resolved defaults when config is invalid — behavior remains covered by completed T02 implementation/context evidence; no regression surfaced during final validation
- [x] Existing tests/checks pass — confirmed by targeted app test, `nix run .#pkl-check-generated`, and `nix flake check`

### Residual risks

- None identified for this plan scope
