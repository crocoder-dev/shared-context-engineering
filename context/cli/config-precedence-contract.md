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

Config file selection follows this deterministic order:

1. `--config <path>`
2. `SCE_CONFIG_FILE`
3. discovered default path: `.sce/config.json` under current working directory (only when present)

## Validation contract

- Config file content must be valid JSON with a top-level object.
- Allowed keys: `log_level`, `timeout_ms`.
- Unknown keys fail validation.
- `log_level` must be one of `error|warn|info|debug`.
- `timeout_ms` must be an unsigned integer.

## Output contract

- `show` and `validate` support deterministic `text` and `json` outputs.
- JSON responses include a top-level `status` and nested `result` object.
- Text output includes the canonical precedence string: `flags > env > config file > defaults`.

## Related files

- `cli/src/app.rs`
- `cli/src/command_surface.rs`
- `cli/src/services/config.rs`
