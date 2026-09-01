# CLI Config Precedence Contract

## Scope

This contract documents the implemented `sce config` command behavior, runtime resolver, renderer, and canonical Pkl-authored `sce/config.json` schema. The schema is emitted to payload-relative `config/schema/sce-config.schema.json` under Cargo `OUT_DIR` or packaging fallbacks and embedded by `cli/src/services/config/schema.rs` as `SCE_CONFIG_SCHEMA_JSON`; no generated schema is committed.

The current implementation resolves flat logging keys and Agent Trace runtime keys with deterministic precedence and source metadata, exposes resolved-value inspection through `sce config show`, and keeps `sce config validate` focused on validation status plus errors/warnings. File logging is explicitly controlled by the config-file/default `log_to_file` boolean, which defaults to `true`; `log_to_file` and `log_dir` resolve independently, with omitted `log_dir` falling back to the default location. Threshold, format, directory, and `log_file_retention_limit` values are consumed by runtime logging; the concrete logger uses the retention value for primary and v2 creation-triggered cleanup. The default-enabled `agent_trace.auto_sync` value is consumed by the post-commit trigger boundary and by doctor readiness reporting, and can be disabled explicitly.

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
- `agent_trace.auto_sync` — boolean for the post-commit Agent Trace synchronization trigger; config-file only, with no flag or environment layer, and defaults to `true` (set `false` to opt out).

Resolved observability values that currently have no CLI flag layer follow the same lower-precedence chain without a flag step:

1. environment values (`SCE_LOG_FORMAT`, `SCE_LOG_DIR`)
2. config file values (`log_format`, `log_to_file`, `log_dir`)
3. defaults (`log_format=text`, `log_to_file=true`; `log_dir=<state_root>/sce/logs` through `default_paths::observability_log_dir()`, resolving on Linux to `$XDG_STATE_HOME/sce/logs` or `~/.local/state/sce/logs` when `XDG_STATE_HOME` is unset)

`log_to_file` is config-file/default only; unlike `log_dir`, it has no environment variable or CLI flag. Omitting either property does not produce a cross-property validation error: `log_to_file` defaults to `true`, and omitted `log_dir` resolves to the default location. An explicit empty config value for `log_dir` is rejected by the generated schema.

`log_file_retention_limit` intentionally has no environment or CLI-flag layer:

1. config file value (`log_file_retention_limit`)
2. default (`10`)

Supported auth-adjacent runtime keys can participate in one shared key-declared precedence path without defining CLI flags. Each key declares its config-file name, environment variable name, and whether a baked default is allowed. The shared resolver supports keys that allow a baked default and keys that intentionally omit one. Two keys are migrated onto this shared path:

- `workos_client_id`, which resolves as:
  1. environment value (`WORKOS_CLIENT_ID`)
  2. config file value (`workos_client_id`)
  3. baked default (`client_01KZE4DDA8HM1JHZGF2QCF49RP`)
- `control_plane_base_url`, the base URL of the control-plane Agent Trace ingestion API, which resolves as:
  1. environment value (`SCE_CONTROL_PLANE_BASE_URL`)
  2. config file value (`control_plane_base_url`)
  3. baked default (`https://sce.crocoderlab.dev`)

The control-plane default is separate from the config schema declaration (`https://sce.crocoder.dev/config.json`) and from the SCE web URL owner used for Agent Trace links.

When a supported auth-adjacent key omits a baked default, the same resolver still reports `value: null` / `(unset)` with no resolved source when both env and config inputs are absent.

Config file selection follows this deterministic order:

1. `--config <path>`
2. `SCE_CONFIG_FILE`
3. discovered defaults when no explicit path/env override is provided:
   - global: `${config_root}/sce/config.json`, where `config_root` comes from the shared default-path policy seam in `cli/src/services/default_paths.rs` and resolves to `dirs::config_dir()` on supported platforms (Linux fallback: `~/.config` when `XDG_CONFIG_HOME` is unset)
   - local: `.sce/config.json` under current working directory

When both discovered defaults exist, they are merged in memory in deterministic order `global -> local`, and local values override global values per key.

When a default-discovered global or repo-local config file exists but fails JSON parsing, top-level-object validation, or schema validation, runtime resolution now skips that file, collects the failure text in `validation_errors`, and continues with remaining discovered layers plus defaults. Explicit `--config <path>` and `SCE_CONFIG_FILE` selections remain fatal on those errors. This means normal command startup still reaches dispatch for commands such as `sce version`, `sce doctor`, and `sce hooks commit-msg` even when discovered config is invalid. Setup and Agent Trace storage are deliberate stricter consumers: setup validates an existing repo-local file after Git-root resolution and before prompts, context bootstrap, lifecycle work, or asset installation, while storage resolution errors on any invalid discovered layer instead of using fallback identity values. See [the fail-closed boundary decision](../decisions/2026-08-26-setup-storage-fail-closed-on-invalid-config.md).

## Validation contract

- The canonical JSON Schema for both global and repo-local `sce/config.json` files is authored in `config/pkl/base/sce-config-schema.pkl` and generated beneath `OUT_DIR/pkl-generated/config/schema/sce-config.schema.json` for repository builds.
- `cli/src/services/config/schema.rs` embeds that `OUT_DIR` artifact at compile time as `SCE_CONFIG_SCHEMA_JSON`; packaged builds receive the same path from their validated fallback.
- `sce config validate` and `sce doctor` both validate config-file structure against that shared generated schema before applying Rust-owned semantic checks such as duplicate custom `argv_prefix` detection and redundancy warnings.
- Each reported schema-validation error is prefixed with the failing value's JSON-pointer location when it has one (for example `/integrations/optional_workflows/0: "nonesuch" is not one of "brownfield"`), so a rejected value names the key it came from; root-level errors keep their unprefixed text. Errors remain sorted and joined with ` | `.
- After schema validation, `cli/src/services/config/schema.rs` deserializes top-level and nested config structure (`policies`, `policies.bash`, `policies.attribution_hooks`) into typed serde DTOs and applies focused Rust-owned mapping helpers for enum conversion and source attribution; policy-specific semantic checks are owned by `cli/src/services/config/policy.rs`.
- The canonical top-level schema declaration `"$schema": "https://sce.crocoder.dev/v<version>/config.json"` (where `<version>` is the CLI release version) is a supported config key for both explicit and discovered `sce/config.json` files, including command-startup paths like `sce version` and other config-loading commands that parse config before normal command dispatch.
- Startup/runtime config resolution now degrades gracefully only for default-discovered files: invalid discovered files are skipped and reported via collected `validation_errors`, while explicit `--config` / `SCE_CONFIG_FILE` targets still fail immediately on the same parse or validation errors.

- Config file content must be valid JSON with a top-level object.
- Allowed keys: `$schema`, `log_level`, `log_format`, `log_to_file`, `log_dir`, `log_file_retention_limit`, `timeout_ms`, `workos_client_id`, `control_plane_base_url`, `agent_trace`, `policies`, `integrations`.
- Unknown keys fail validation.
- `log_to_file` must be a boolean when present and defaults to `true`; it is independent of `log_dir`.
- `log_level` must be one of `error|warn|info|debug`.
- `log_format` must be `text` or `json` when present.
- `log_dir` must be a non-empty string when present.
- `log_file_retention_limit` must be an integer with minimum `1`; zero, negative, fractional, string, and object values fail schema validation.
- `timeout_ms` must be an unsigned integer.
- `workos_client_id` must be a string when present.
- `control_plane_base_url` must be a non-empty string when present.

- `agent_trace` must be an object when present and currently allows `repository_id`, `repository_remote`, and `auto_sync`.
- `agent_trace.repository_id` must be a non-empty string when present.
- `agent_trace.repository_remote` must be a non-empty string when present; omitted values resolve to `origin`.
- `agent_trace.auto_sync` must be a boolean when present; omitted values resolve to `true`.

- `integrations` must be an object when present and currently allows `target` and `optional_workflows`; either key alone yields a parsed `IntegrationsConfig` with the other defaulting to empty.
- `integrations.target` must be an array of unique canonical target IDs when present.
- Supported target ID values: `opencode`, `claude`, `pi`, `codex`.
- Unknown target IDs fail schema validation.
- `integrations.optional_workflows` must be an array of unique optional-workflow IDs when present; it records which optional workflows a repository has opted into. Its enum is derived in `config/pkl/base/sce-config-schema.pkl` from the workflow catalog's `optional` records rather than hand-listed, so marking a workflow optional in Pkl extends the accepted values with no Rust or schema edit. Currently the only accepted value is `brownfield`.
- Unknown optional-workflow IDs and duplicate entries fail schema validation. Rust-side mapping validates each ID a second time against the embedded optional-workflow catalog (`parse_optional_workflow_id` in `cli/src/services/config/types.rs`), reporting the catalog's available IDs.
- `sce setup` writes this key: it records the selection resolved for the run and reads the stored value back when `--workflow` is absent, which is the only consumer of the key today. See [setup local bootstrap](../sce/setup-repo-local-config-bootstrap.md).

- `policies` must be an object when present and currently allows `attribution_hooks`, `database_retry`, and `bash`.
- `policies.attribution_hooks` must be an object when present and currently allows `enabled`; explicit `enabled: false` remains a valid opt-out alongside the runtime `SCE_ATTRIBUTION_HOOKS_DISABLED` environment opt-out.
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
- `show` includes resolved observability values directly in `result.resolved`, preserving flat logging keys (`log_level`, `log_format`, `log_to_file`, `log_dir`, `log_file_retention_limit`) and their source metadata.
- `validate` text output is limited to `SCE config validation`, `Validation issues`, and `Validation warnings` lines.
- `validate` JSON output is limited to `result.command`, `result.valid`, `result.issues`, and `result.warnings`.
- `show` includes resolved Agent Trace configuration under `result.resolved.agent_trace` (JSON: `repository_id` optional-value shape, `repository_remote` and `auto_sync` resolved-value shapes) and as per-key text lines, reporting `(unset)` for a missing `repository_id`, `source: default` for the `origin` remote fallback, and `source: default` for omitted `auto_sync`.
- Doctor consumes the same resolved `agent_trace.auto_sync` value and source metadata; its separate `post_commit_auto_sync` report fact documents hook readiness without launching synchronization.
- `show` includes resolved bash-tool policies under `result.resolved.policies.bash`.
- Bash-policy output includes resolved preset IDs, expanded custom entries (`id`, `match.argv_prefix`, `message`), and config-file source metadata when present.
- `show` text output renders `policies.bash` as a single deterministic line and reports `(unset)` when no policy config resolves.
- `show` text output renders observability values as deterministic per-key lines, reporting the default `log_dir` with `source: default` when no env/config value resolves.
- `show` reports `log_file_retention_limit=10` with `source: default` when omitted; configured values report `source: config_file` and the winning global/local `config_source`.
- `show` and `validate` both include `warnings`; this list is empty for normal valid config and carries deterministic redundancy messaging for valid-but-overlapping preset combinations such as `forbid-git-all` plus `forbid-git-commit`.
- `validate` reports skipped invalid discovered config files through `result.valid = false` plus `result.issues`, using the collected `validation_errors` verbatim in both text and JSON output rather than hard-failing before render.
- `validate` reaches its normal renderer for invalid discovered config; invalid discovered config is reported as a validation result rather than causing a pre-render startup failure.
- `show` continues to report resolved values from the remaining discovered layers plus defaults when discovered config is invalid, and surfaces each skipped discovered-file failure in `warnings` with the prefix `Skipped invalid config: ...`.
- Runtime config resolution also carries `validation_errors` for skipped invalid discovered config files; `show` maps them into user-facing warnings, while `validate` maps them into validation issues.
- Auth-key JSON output in `show` includes `value`, text-oriented `display_value`, `source`, optional `config_source`, and a key-specific `precedence` string describing the allowed resolution chain.
- Auth-key text output in `show` includes `auth_precedence` and abbreviates full values when they look credential-like; fully secret-bearing key classes remain redacted.
- For the migrated keys `workos_client_id` and `control_plane_base_url`, `show` reports the baked default with `source: default` when env/config inputs are absent.

## Auth diagnostics contract

- Auth failure guidance for migrated auth keys no longer assumes env-only configuration.
- Missing-client-id guidance for `workos_client_id` describes the full allowed chain for this key: `WORKOS_CLIENT_ID`, config-file key `workos_client_id`, or fallback to the baked default when no higher-precedence invalid override blocks it.
- Auth login runtime guidance refers to the resolved source chain generically (`WORKOS_CLIENT_ID`, config file, or baked default for `workos_client_id`) instead of env-only wording.
- `control_plane_base_url` resolves through the same shared auth-adjacent key path but has no dedicated auth failure guidance of its own; it is consumed by the Agent Trace control-plane client (`sce sync`).

## Related files

- `config/pkl/base/sce-config-schema.pkl`
- `config/pkl/generate.pkl`
- `cli/src/app.rs`
- `cli/src/command_surface.rs`
- `cli/src/services/config/mod.rs`
- `cli/src/services/config/resolver.rs`
- `cli/src/services/config/render.rs`
- `cli/src/services/config/schema.rs`
- `cli/src/services/config/policy.rs`
- `context/cli/agent-trace-auto-sync.md`
