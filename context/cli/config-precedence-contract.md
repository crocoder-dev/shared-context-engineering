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
4. defaults (`log_level=info`, `timeout_ms=30000`)

Auth-adjacent config keys can also participate in the same shared env-over-config path without defining CLI flags. The first implemented key is `workos_client_id`, which resolves as:

1. environment value (`WORKOS_CLIENT_ID`)
2. config file value (`workos_client_id`)
3. unset when neither layer provides a value

Config file selection follows this deterministic order:

1. `--config <path>`
2. `SCE_CONFIG_FILE`
3. discovered defaults when no explicit path/env override is provided:
   - global: `${state_root}/sce/config.json` where `state_root` follows the same platform policy as Agent Trace local DB path derivation
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
- `show` includes `workos_client_id` in resolved output; when unset, JSON reports `value: null` and `source: null`, and text reports `(unset) (source: none)`.

## Related files

- `cli/src/app.rs`
- `cli/src/command_surface.rs`
- `cli/src/services/config.rs`
