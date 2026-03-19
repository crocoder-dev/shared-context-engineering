# Plan: sce-local-log-config

## Change summary

Add a repo-local `.sce/config.json` that sends `sce` file logs into `context/tmp/` and captures all log levels by default for this repository.
The initial config should use the default log path `context/tmp/sce.log` and rely on the existing config/observability contract rather than introducing new runtime behavior.

## Success criteria

- A repo-local `.sce/config.json` exists and is valid under the existing generated `sce/config.json` schema.
- The config sets `log_level` to `debug` so all current `sce` log severities are captured.
- The config sets `log_file` to `context/tmp/sce.log` and uses a valid file mode so logs are written under `context/tmp/`.
- The change does not modify CLI/runtime code or generated config/schema sources.
- Validation confirms the config file is discoverable and accepted by the current `sce config validate` flow.

## Constraints and non-goals

- Keep the change limited to repo-local configuration under `.sce/`; do not add global config changes.
- Reuse the existing flat observability keys (`log_level`, `log_file`, `log_file_mode`) instead of inventing a new structure.
- Treat `context/tmp/` as the log destination root and use the default filename `sce.log`.
- Do not broaden scope into OpenTelemetry setup, log rotation, or runtime observability feature changes.
- Do not edit application code, generated schema assets, or unrelated context files unless validation exposes a true drift issue.

## Assumptions

- “All logs” maps to the currently supported highest verbosity level, `log_level: "debug"`.
- `log_file_mode: "append"` is the safest repo-local default so repeated runs accumulate logs in `context/tmp/sce.log` instead of truncating prior diagnostics.
- `context/tmp/` already exists and remains the appropriate ephemeral destination for local operator artifacts.

## Task stack

- [x] T01: Create repo-local logging config in `.sce/config.json` (status:done)
  - Task ID: T01
  - Goal: Add a minimal repo-local config file that enables debug-level file logging to `context/tmp/sce.log` using the existing `sce` config contract.
  - Boundaries (in/out of scope): In - creating `.sce/config.json`, setting `log_level`, `log_file`, and `log_file_mode`, and keeping the file schema-valid. Out - CLI/runtime code changes, new config keys, global config changes, or log-directory policy changes outside this repository.
  - Done when: `.sce/config.json` exists with schema-valid observability keys, `log_level` is `debug`, `log_file` is `context/tmp/sce.log`, and the chosen file mode is explicit.
  - Verification notes (commands or checks): Review the file contents against the current schema contract in `context/glossary.md` and `context/sce/cli-observability-contract.md`; confirm the path stays repo-relative and under `context/tmp/`.
  - Completed: 2026-03-19
  - Files changed: `.sce/config.json`, `context/plans/sce-local-log-config.md`
  - Evidence: `nix develop -c cargo run --manifest-path cli/Cargo.toml -- config validate --format json` returned `valid: true` for the discovered local config; `nix develop -c cargo run --manifest-path cli/Cargo.toml -- config show --format json` resolved `log_level=debug`, `log_file=context/tmp/sce.log`, and `log_file_mode=append`; `context/tmp/sce.log` captured runtime entries during verification.

- [x] T02: Validate config recognition and cleanup (status:done)
  - Task ID: T02
  - Status: done
  - Completed: 2026-03-19
  - Files changed: `context/plans/sce-local-log-config.md`
  - Evidence: `nix develop -c cargo run --manifest-path cli/Cargo.toml -- config validate --format json` returned `valid: true` and discovered `/home/davidabram/repos/shared-context-engineering/master/.sce/config.json` as `default_discovered_local`; `nix develop -c cargo run --manifest-path cli/Cargo.toml -- config show --format json` resolved `log_level=debug`, `log_file=context/tmp/sce.log`, and `log_file_mode=append` from `config_file`
  - Notes: No CLI/runtime or generated-config changes were required; this task only verified current repo-local config recognition and recorded the final plan evidence.
  - Goal: Confirm the new repo-local config is accepted by the current CLI validation flow and that no extra context repair is required.
  - Boundaries (in/out of scope): In - `sce config validate` and/or `sce config show` checks for schema acceptance and resolved observability values, plus plan/status updates. Out - feature expansion, runtime bug fixes, or unrelated documentation edits.
  - Done when: Validation shows `.sce/config.json` is readable and valid, the resolved log configuration points at `context/tmp/sce.log`, and any plan-local evidence is recorded.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo run -- config validate'` or equivalent repo-preferred command path; optionally confirm resolved values with `nix develop -c sh -c 'cd cli && cargo run -- config show'`; finish with repo baseline checks only if the implementation session touches code or generated artifacts.

## Open questions

- None. The log destination default is `context/tmp/sce.log` and “all logs” is interpreted as `log_level: "debug"`.

## Handoff

- plan_name: `sce-local-log-config`
- plan_path: `context/plans/sce-local-log-config.md`
- next command: `Plan complete; if new work is needed, start a new planning or follow-up task session.`

## Validation Report

### Commands run
- `nix develop -c cargo run --manifest-path cli/Cargo.toml -- config validate --format json` -> exit 0 (`valid: true`; discovered `/home/davidabram/repos/shared-context-engineering/master/.sce/config.json` as `default_discovered_local`)
- `nix develop -c cargo run --manifest-path cli/Cargo.toml -- config show --format json` -> exit 0 (resolved `log_level=debug`, `log_file=context/tmp/sce.log`, `log_file_mode=append` from `config_file`)
- `nix run .#pkl-check-generated` -> exit 0 (`Generated outputs are up to date.`)
- `nix flake check` -> exit 0 (`cli-tests`, `cli-clippy`, `cli-fmt`, and `pkl-parity` all passed)

### Temporary scaffolding removed
- None. This task only validated the existing repo-local config and updated plan evidence.

### Failed checks and follow-ups
- None.

### Success-criteria verification
- [x] A repo-local `.sce/config.json` exists and is valid under the existing generated `sce/config.json` schema -> confirmed by `sce config validate --format json` returning `valid: true`
- [x] The config sets `log_level` to `debug` -> confirmed by `sce config show --format json`
- [x] The config sets `log_file` to `context/tmp/sce.log` and uses a valid file mode -> confirmed by `sce config show --format json` resolving `log_file=context/tmp/sce.log` and `log_file_mode=append`
- [x] The change does not modify CLI/runtime code or generated config/schema sources -> confirmed by task scope and resulting file changes (`.sce/config.json` from T01 plus plan evidence updates only)
- [x] Validation confirms the config file is discoverable and accepted by the current `sce config validate` flow -> confirmed by discovery source `default_discovered_local` in `sce config validate --format json`

### Residual risks
- None identified. The repo-local logging config is discoverable, valid, and aligned with the existing observability contract.
