# CLI Config Precedence Contract

## Scope

This contract documents the implemented `sce config` command behavior in `cli/src/services/config/mod.rs`, the runtime resolver in `cli/src/services/config/resolver.rs`, the text/JSON output renderer in `cli/src/services/config/render.rs`, the canonical Pkl-authored `sce/config.json` schema artifact generated to `config/schema/sce-config.schema.json` and embedded by `cli/src/services/config/schema.rs` as `SCE_CONFIG_SCHEMA_JSON`, the typed serde DTO + mapping pipeline used for config-file parsing, and parser/dispatch wiring in `cli/src/app.rs`.

The current implementation resolves flat logging keys with deterministic env-over-config precedence and source metadata, uses those resolved values in `cli/src/app.rs` / `cli/src/services/observability.rs` for runtime logging, exposes resolved-value inspection through `sce config show`, and keeps `sce config validate` focused on validation status plus errors/warnings.

## Command surface

- `sce config show [--config <path>] [--log-level <error|warn|info|debug>] [--timeout-ms <value>] [--format <text|json>]`
- `sce config validate [--config <path>] [--log-level <error|warn|info|debug>] [--timeout-ms <value>] [--format <text|json>]`
- bare `sce config` returns the same help payload as `sce config --help`
- `sce config --help`
- Help text for `sce config`, `sce config show`, and `sce config validate` frames the command family as the operator entrypoint for config inspection and validation; `show` covers resolved runtime values with provenance, `validate` covers pass/fail plus validation issues and warnings, and bare `sce config` is help-first rather than defaulting to `show`.

## Resolution precedence

Resolved runtime values follow this deterministic order:

1. flag values (`--log-level`, `--timeout-ms`)
2. environment values (`SCE_LOG_LEVEL`, `SCE_TIMEOUT_MS`)
3. config file values (`log_level`, `timeout_ms`)
4. defaults (`log_level=error`, `timeout_ms=30000`)

Repo-configured bash-tool policy values are config-file only in this task slice: they load from `policies.bash` in the selected config files, merge `global -> local` alongside the rest of the config object, and currently have no flag or environment override layer.

Agent Trace repository identity keys are also config-file only with per-key `global -> local` merge and no flag or environment layer:

- `agent_trace.repository_id` — optional explicit repository identity; resolves as an optional value with no default.
- `agent_trace.repository_remote` — Git remote name used to derive repository identity; defaults to `origin` (`DEFAULT_AGENT_TRACE_REPOSITORY_REMOTE` in `cli/src/services/config/resolver.rs`) when no config file sets it.

Resolved observability values that currently have no CLI flag layer follow the same lower-precedence chain without a flag step:

1. environment values (`SCE_LOG_FORMAT`, `SCE_LOG_FILE`, `SCE_LOG_FILE_MODE`)
2. config file values (`log_format`, `log_file`, `log_file_mode`)
3. defaults where defined (`log_format=text`, `log_file_mode=truncate`); `log_file` remains unset when no env/config value is present

Supported auth-adjacent runtime keys can participate in one shared key-declared precedence path without defining CLI flags. Each key declares its config-file name, environment variable name, and whether a baked default is allowed. The shared resolver supports keys that allow a baked default and keys that intentionally omit one. The first implemented migrated key is `workos_client_id`, which resolves as:

1. environment value (`WORKOS_CLIENT_ID`)
2. config file value (`workos_client_id`)
3. baked default (`client_sce_default`)

When a supported auth-adjacent key omits a baked default, the same resolver still reports `value: null` / `(unset)` with no resolved source when both env and config inputs are absent.

Config file selection follows this deterministic order:

1. `--config <path>`
2. `SCE_CONFIG_FILE`
3. discovered defaults when no explicit path/env override is provided:
   - global: `${config_root}/sce/config.json`, where `config_root` comes from the shared default-path policy seam in `cli/src/services/default_paths.rs` and resolves to `dirs::config_dir()` on supported platforms (Linux fallback: `~/.config` when `XDG_CONFIG_HOME` is unset)
   - local: `.sce/config.json` under current working directory

When both discovered defaults exist, they are merged in memory in deterministic order `global -> local`, and local values override global values per key.

When a default-discovered global or repo-local config file exists but fails JSON parsing, top-level-object validation, or schema validation, runtime resolution now skips that file, collects the failure text in `validation_errors`, and continues with remaining discovered layers plus defaults. Explicit `--config <path>` and `SCE_CONFIG_FILE` selections remain fatal on those errors. This means normal command startup still reaches dispatch for commands such as `sce version`, `sce doctor`, and `sce hooks commit-msg` even when discovered config is invalid.

## Validation contract

- The canonical JSON Schema artifact for both global and repo-local `sce/config.json` files is authored in `config/pkl/base/sce-config-schema.pkl` and generated to `config/schema/sce-config.schema.json`.
- `cli/src/services/config/schema.rs` embeds that generated artifact at compile time as `SCE_CONFIG_SCHEMA_JSON` and uses it for runtime schema validation before mapping parsed files into typed serde DTOs.
- `sce config validate` and `sce doctor` both validate config-file structure against that shared generated schema before applying Rust-owned semantic checks such as duplicate custom `argv_prefix` detection and redundancy warnings.
- After schema validation, `cli/src/services/config/schema.rs` deserializes top-level and nested config structure (`policies`, `policies.bash`, `policies.attribution_hooks`) into typed serde DTOs and applies focused Rust-owned mapping helpers for enum conversion and source attribution; policy-specific semantic checks are owned by `cli/src/services/config/policy.rs`.
- The canonical top-level schema declaration `"$schema": "https://sce.crocoder.dev/config.json"` is a supported config key for both explicit and discovered `sce/config.json` files, including command-startup paths like `sce version` and other config-loading commands that parse config before normal command dispatch.
- Startup/runtime config resolution now degrades gracefully only for default-discovered files: invalid discovered files are skipped and reported via collected `validation_errors`, while explicit `--config` / `SCE_CONFIG_FILE` targets still fail immediately on the same parse or validation errors.

- Config file content must be valid JSON with a top-level object.
- Allowed keys: `$schema`, `log_level`, `log_format`, `log_file`, `log_file_mode`, `timeout_ms`, `workos_client_id`, `agent_trace`, `policies`, `integrations`.
- Unknown keys fail validation.
- `log_level` must be one of `error|warn|info|debug`.
- `log_format` must be `text` or `json` when present.
- `log_file` must be a non-empty string when present.
- `log_file_mode` must be `truncate` or `append` when present.
- `log_file_mode` requires `log_file`.
- `timeout_ms` must be an unsigned integer.
- `workos_client_id` must be a string when present.

- `agent_trace` must be an object when present and currently allows only `repository_id` and `repository_remote`.
- `agent_trace.repository_id` must be a non-empty string when present.
- `agent_trace.repository_remote` must be a non-empty string when present; the generated schema documents default `origin`.

- `integrations` must be an object when present and currently allows only `target`.
- `integrations.target` must be an array of unique canonical target IDs when present.
- Supported target ID values: `opencode`, `claude`, `pi`.
- Unknown target IDs fail schema validation.

- `policies` must be an object when present and currently allows `attribution_hooks`, `database_retry`, and `bash`.
- `policies.attribution_hooks` must be an object when present and currently allows `enabled`; the generated schema documents default `true`, and explicit `enabled: false` remains a valid opt-out alongside the runtime `SCE_ATTRIBUTION_HOOKS_DISABLED` environment opt-out.
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
- `show` text output includes the canonical precedence string: `flags > env > config file > defaults`.
- `show` reports discovered config files as `config_paths` (JSON) / `Config files:` (text).
- Resolved values in `show` continue to report `source`; when source is `config_file`, output also reports a deterministic `config_source` value (`flag`, `env`, `default_discovered_global`, `default_discovered_local`).
- `show` includes migrated supported auth keys in `result.resolved`.
- `show` includes resolved observability values directly in `result.resolved`, preserving flat logging keys (`log_level`, `log_format`, `log_file`, `log_file_mode`).
- `validate` text output is limited to `SCE config validation`, `Validation issues`, and `Validation warnings` lines.
- `validate` JSON output is limited to `result.command`, `result.valid`, `result.issues`, and `result.warnings`.
- `show` includes resolved Agent Trace repository identity under `result.resolved.agent_trace` (JSON: `repository_id` optional-value shape, `repository_remote` resolved-value shape) and as `agent_trace.repository_id` / `agent_trace.repository_remote` per-key text lines, reporting `(unset)` for a missing `repository_id` and `source: default` for the `origin` remote fallback.
- `show` includes resolved bash-tool policies under `result.resolved.policies.bash`.
- Bash-policy output includes resolved preset IDs, expanded custom entries (`id`, `match.argv_prefix`, `message`), and config-file source metadata when present.
- `show` text output renders `policies.bash` as a single deterministic line and reports `(unset)` when no policy config resolves.
- `show` text output renders observability values as deterministic per-key lines, reporting `(unset)` for `log_file` when no value resolves.
- `show` and `validate` both include `warnings`; this list is empty for normal valid config and carries deterministic redundancy messaging for valid-but-overlapping preset combinations such as `forbid-git-all` plus `forbid-git-commit`.
- `validate` reports skipped invalid discovered config files through `result.valid = false` plus `result.issues`, using the collected `validation_errors` verbatim in both text and JSON output rather than hard-failing before render.
- `validate` reaches its normal renderer for invalid discovered config; invalid discovered config is reported as a validation result rather than causing a pre-render startup failure.
- `show` continues to report resolved values from the remaining discovered layers plus defaults when discovered config is invalid, and surfaces each skipped discovered-file failure in `warnings` with the prefix `Skipped invalid config: ...`.
- Runtime config resolution also carries `validation_errors` for skipped invalid discovered config files; `show` maps them into user-facing warnings, while `validate` maps them into validation issues.
- Auth-key JSON output in `show` includes `value`, text-oriented `display_value`, `source`, optional `config_source`, and a key-specific `precedence` string describing the allowed resolution chain.
- Auth-key text output in `show` includes `auth_precedence` and abbreviates full values when they look credential-like; fully secret-bearing key classes remain redacted.
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
- `cli/src/services/config/mod.rs`
- `cli/src/services/config/resolver.rs`
- `cli/src/services/config/render.rs`
- `cli/src/services/config/schema.rs`
- `cli/src/services/config/policy.rs`
