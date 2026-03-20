# CLI Config Precedence Contract

## Scope

This contract documents the implemented `sce config` command behavior in `cli/src/services/config.rs`, the canonical Pkl-authored `sce/config.json` schema artifact generated to `config/schema/sce-config.schema.json` and embedded there as `SCE_CONFIG_SCHEMA_JSON`, and parser/dispatch wiring in `cli/src/app.rs`.

The current implementation resolves flat logging keys plus nested `otel` keys with deterministic env-over-config precedence and source metadata, uses those resolved values in `cli/src/app.rs` / `cli/src/services/observability.rs` for runtime logging and OTEL bootstrap, and reports the same observability values through operator-facing `sce config show|validate` text and JSON output.

## Command surface

- `sce config show [--config <path>] [--log-level <error|warn|info|debug>] [--timeout-ms <value>] [--format <text|json>]`
- `sce config validate [--config <path>] [--log-level <error|warn|info|debug>] [--timeout-ms <value>] [--format <text|json>]`
- `sce config --help`
- Help text for `sce config`, `sce config show`, and `sce config validate` frames the command family as the operator entrypoint for inspecting and validating resolved runtime config, including config-backed observability values and their provenance.

## Resolution precedence

Resolved runtime values follow this deterministic order:

1. flag values (`--log-level`, `--timeout-ms`)
2. environment values (`SCE_LOG_LEVEL`, `SCE_TIMEOUT_MS`)
3. config file values (`log_level`, `timeout_ms`)
4. defaults (`log_level=error`, `timeout_ms=30000`)

Repo-configured bash-tool policy values are config-file only in this task slice: they load from `policies.bash` in the selected config files, merge `global -> local` alongside the rest of the config object, and currently have no flag or environment override layer.

Resolved observability values that currently have no CLI flag layer follow the same lower-precedence chain without a flag step:

1. environment values (`SCE_LOG_FORMAT`, `SCE_LOG_FILE`, `SCE_LOG_FILE_MODE`, `SCE_OTEL_ENABLED`, `OTEL_EXPORTER_OTLP_ENDPOINT`, `OTEL_EXPORTER_OTLP_PROTOCOL`)
2. config file values (`log_format`, `log_file`, `log_file_mode`, `otel.enabled`, `otel.exporter_otlp_endpoint`, `otel.exporter_otlp_protocol`)
3. defaults where defined (`log_format=text`, `log_file_mode=truncate`, `otel.enabled=false`, `otel.exporter_otlp_endpoint=http://127.0.0.1:4317`, `otel.exporter_otlp_protocol=grpc`); `log_file` remains unset when no env/config value is present

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

- The canonical JSON Schema artifact for both global and repo-local `sce/config.json` files is authored in `config/pkl/base/sce-config-schema.pkl` and generated to `config/schema/sce-config.schema.json`.
- `cli/src/services/config.rs` embeds that generated artifact at compile time as `SCE_CONFIG_SCHEMA_JSON` and uses it for runtime schema validation.
- `sce config validate` and `sce doctor` both validate config-file structure against that shared generated schema before applying Rust-owned semantic checks such as duplicate custom `argv_prefix` detection and redundancy warnings.
- The canonical top-level schema declaration `"$schema": "https://sce.crocoder.dev/config.json"` is a supported config key for both explicit and discovered `sce/config.json` files, including command-startup paths like `sce version` and other config-loading commands that parse config before normal command dispatch.

- Config file content must be valid JSON with a top-level object.
- Allowed keys: `$schema`, `log_level`, `log_format`, `log_file`, `log_file_mode`, `timeout_ms`, `workos_client_id`, `otel`, `policies`.
- Unknown keys fail validation.
- `log_level` must be one of `error|warn|info|debug`.
- `log_format` must be `text` or `json` when present.
- `log_file` must be a non-empty string when present.
- `log_file_mode` must be `truncate` or `append` when present.
- `log_file_mode` requires `log_file`.
- `timeout_ms` must be an unsigned integer.
- `workos_client_id` must be a string when present.
- `otel` must be an object when present and currently allows only `enabled`, `exporter_otlp_endpoint`, and `exporter_otlp_protocol`.
- `otel.enabled` must be a boolean when present.
- `otel.exporter_otlp_endpoint` must be an absolute `http(s)` URL when present.
- `otel.exporter_otlp_protocol` must be `grpc` or `http/protobuf` when present.
- `policies` must be an object when present and currently allows only `bash`.
- `policies.bash` must be an object when present and currently allows only `presets` and `custom`.
- `policies.bash.presets` must be an array of unique built-in preset IDs: `forbid-git-all`, `forbid-git-commit`, `use-pnpm-over-npm`, `use-bun-over-npm`, `use-nix-flake-over-cargo`.
- `use-pnpm-over-npm` and `use-bun-over-npm` are mutually exclusive and fail validation when both are present.
- `policies.bash.custom` must be an array of objects containing exactly `id`, `match`, and `message`.
- `match` currently allows only `argv_prefix`, which must be a non-empty array of non-empty strings.
- Custom policy IDs must be unique, must not collide with built-in preset IDs, and exact duplicate custom `argv_prefix` values fail validation.
- `forbid-git-all` plus `forbid-git-commit` remains valid but is reported as a deterministic redundancy warning.

## Output contract

- `show` and `validate` support deterministic `text` and `json` outputs.
- JSON responses include a top-level `status` and nested `result` object.
- Text output includes the canonical precedence string: `flags > env > config file > defaults`.
- Output reports discovered config files as `config_paths` (JSON) / `Config files:` (text).
- Resolved values continue to report `source`; when source is `config_file`, output also reports a deterministic `config_source` value (`flag`, `env`, `default_discovered_global`, `default_discovered_local`).
- `show` includes migrated supported auth keys in `result.resolved`; `validate` includes them in `result.resolved_auth`.
- `show` includes resolved observability values directly in `result.resolved`, preserving flat logging keys (`log_level`, `log_format`, `log_file`, `log_file_mode`) plus nested `otel.{enabled,exporter_otlp_endpoint,exporter_otlp_protocol}`.
- `validate` includes the same observability values under `result.resolved_observability`, preserving the mixed flat-plus-nested shape.
- `show` includes resolved bash-tool policies under `result.resolved.policies.bash`; `validate` includes them under `result.resolved_policies.bash`.
- Bash-policy output includes resolved preset IDs, expanded custom entries (`id`, `match.argv_prefix`, `message`), and config-file source metadata when present.
- Text output renders `policies.bash` as a single deterministic line and reports `(unset)` when no policy config resolves.
- Text output renders observability values as deterministic per-key lines, using `otel.` prefixes for nested OTEL keys and reporting `(unset)` for `log_file` when no value resolves.
- `show` and `validate` both include `warnings`; this list is empty for normal valid config and carries deterministic redundancy messaging for valid-but-overlapping preset combinations such as `forbid-git-all` plus `forbid-git-commit`.
- Auth-key JSON output includes `value`, text-oriented `display_value`, `source`, optional `config_source`, and a key-specific `precedence` string describing the allowed resolution chain.
- Auth-key text output includes `auth_precedence` and abbreviates full values when they look credential-like; fully secret-bearing key classes remain redacted.
- For the currently migrated key `workos_client_id`, `show` reports the baked default with `source: default` when env/config inputs are absent.

## Auth diagnostics contract

- Auth failure guidance for migrated auth keys no longer assumes env-only configuration.
- Missing-client-id guidance for `workos_client_id` describes the full allowed chain for this key: `WORKOS_CLIENT_ID`, config-file key `workos_client_id`, or fallback to the baked default when no higher-precedence invalid override blocks it.
- Auth login runtime guidance refers to the resolved source chain generically (`WORKOS_CLIENT_ID`, config file, or baked default for `workos_client_id`) instead of env-only wording.

## Related files

- `config/pkl/base/sce-config-schema.pkl`
- `config/schema/sce-config.schema.json`
- `cli/src/app.rs`
- `cli/src/command_surface.rs`
- `cli/src/services/config.rs`
