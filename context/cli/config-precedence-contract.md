# CLI Config Precedence Contract

## Scope

This contract documents the implemented `sce config` command behavior in `cli/src/services/config.rs` and parser/dispatch wiring in `cli/src/app.rs`.

## Command surface

- `sce config show [--config <path>] [--log-level <error|warn|info|debug>] [--timeout-ms <value>] [--format <text|json>]`
- `sce config validate [--config <path>] [--log-level <error|warn|info|debug>] [--timeout-ms <value>] [--format <text|json>]`
- `sce config --help`

## Resolution precedence

Resolved runtime values follow this deterministic order:

1. flag values (`--log-level`, `--timeout-ms`)
2. environment values (`SCE_LOG_LEVEL`, `SCE_TIMEOUT_MS`)
3. config file values (`log_level`, `timeout_ms`)
4. defaults (`log_level=error`, `timeout_ms=30000`)

Supported auth-adjacent runtime keys can participate in one shared key-declared precedence path without defining CLI flags. Each key declares its config-file name, environment variable name, and whether a baked default is allowed. The shared resolver supports keys that allow a baked default and keys that intentionally omit one. The first implemented migrated key is `workos_client_id`, which resolves as:

1. environment value (`WORKOS_CLIENT_ID`)
2. config file value (`workos_client_id`)
3. baked default (`client_sce_default`)

When a supported auth-adjacent key omits a baked default, the same resolver still reports `value: null` / `(unset)` with no resolved source when both env and config inputs are absent.

Config file selection follows this deterministic order:

1. `--config <path>`
2. `SCE_CONFIG_FILE`
3. discovered defaults when no explicit path/env override is provided:
   - global: `${global_config_root}/sce/config.json`, where `global_config_root` uses `dirs::state_dir()` on Linux (with `~/.local/state` fallback when needed), `dirs::data_dir()` on macOS/Windows, and `state_dir` then `data_dir` fallback on other targets
   - local: `.sce/config.json` under current working directory

When both discovered defaults exist, they are merged in memory in deterministic order `global -> local`, and local values override global values per key.

## Validation contract

- Config file content must be valid JSON with a top-level object.
- Allowed keys: `log_level`, `timeout_ms`, `workos_client_id`.
- Unknown keys fail validation.
- `log_level` must be one of `error|warn|info|debug`.
- `timeout_ms` must be an unsigned integer.
- `workos_client_id` must be a string when present.

## Output contract

- `show` and `validate` support deterministic `text` and `json` outputs.
- JSON responses include a top-level `status` and nested `result` object.
- Text output includes the canonical precedence string: `flags > env > config file > defaults`.
- Output reports discovered config files as `config_paths` (JSON) / `Config files:` (text).
- Resolved values continue to report `source`; when source is `config_file`, output also reports a deterministic `config_source` value (`flag`, `env`, `default_discovered_global`, `default_discovered_local`).
- `show` includes migrated supported auth keys in `result.resolved`; `validate` includes them in `result.resolved_auth`.
- Auth-key JSON output includes `value`, text-oriented `display_value`, `source`, optional `config_source`, and a key-specific `precedence` string describing the allowed resolution chain.
- Auth-key text output includes `auth_precedence` and abbreviates full values when they look credential-like; fully secret-bearing key classes remain redacted.
- For the currently migrated key `workos_client_id`, `show` reports the baked default with `source: default` when env/config inputs are absent.

## Auth diagnostics contract

- Auth failure guidance for migrated auth keys no longer assumes env-only configuration.
- Missing-client-id guidance for `workos_client_id` describes the full allowed chain for this key: `WORKOS_CLIENT_ID`, config-file key `workos_client_id`, or fallback to the baked default when no higher-precedence invalid override blocks it.
- Auth login runtime guidance refers to the resolved source chain generically (`WORKOS_CLIENT_ID`, config file, or baked default for `workos_client_id`) instead of env-only wording.

## Related files

- `cli/src/app.rs`
- `cli/src/command_surface.rs`
- `cli/src/services/config.rs`
