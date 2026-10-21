//! Runtime config discovery, merge, and precedence resolution.
//!
//! This submodule owns config-file discovery, file-layer merging,
//! env/flag/default precedence, auth-key resolution, observability resolution,
//! and default-discovered invalid-file degradation.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};

use crate::services::default_paths::{
    enclosing_repository_root_with, observability_log_dir, resolve_sce_default_locations, RepoPaths,
};

use crate::services::managed_telemetry::MANAGED_OTLP_DEFAULTS;

use super::policy::{build_validation_warnings, resolve_bash_policy_config, BashPolicyConfig};
use super::schema;
use super::types::{
    parse_bool_value_from, AuthMode, ConfigPathSource, ConfigRequest, DatabaseRetryConfig,
    LoadedConfigPath, LogFormat, LogLevel, PolicyLayer, ReportFormat, ResolvedAccessPolicy,
    ResolvedAgentTraceStorageRuntimeConfig, ResolvedAuthRuntimeConfig, ResolvedHookRuntimeConfig,
    ResolvedObservabilityRuntimeConfig, ResolvedOptionalValue, ResolvedValue,
    TelemetryDisabledReason, TelemetryPolicyState, ValueSource, DEFAULT_LOG_FILE_RETENTION_LIMIT,
    ENV_ATTRIBUTION_HOOKS_DISABLED, ENV_LOG_DIR, ENV_LOG_FORMAT, ENV_LOG_LEVEL,
};

pub(crate) const DEFAULT_AGENT_TRACE_REPOSITORY_REMOTE: &str = "origin";
pub(crate) const PRECEDENCE_DESCRIPTION: &str = "flags > env > config file > defaults";
const WORKOS_CLIENT_ID_ENV: &str = "WORKOS_CLIENT_ID";
const WORKOS_CLIENT_ID_BAKED_DEFAULT: &str = "client_01KZE4DDA8HM1JHZGF2QCF49RP";

pub(crate) const WORKOS_CLIENT_ID_KEY: AuthConfigKeySpec = AuthConfigKeySpec {
    config_key: "workos_client_id",
    env_key: WORKOS_CLIENT_ID_ENV,
    baked_default: Some(WORKOS_CLIENT_ID_BAKED_DEFAULT),
};

const CONTROL_PLANE_BASE_URL_ENV: &str = "SCE_CONTROL_PLANE_BASE_URL";
const CONTROL_PLANE_BASE_URL_BAKED_DEFAULT: &str = "https://sce.crocoderlab.dev";

pub(crate) const WARNING_REPOSITORY_TELEMETRY_WITHOUT_USER_CONSENT: &str = "telemetry.enabled is true in the repository config but not in the user config; SCE telemetry stays disabled until the user config sets telemetry.enabled to true.";
pub(crate) const WARNING_TELEMETRY_ENABLED_WITH_AUTH_DISABLED: &str =
    "telemetry.enabled is true but auth.mode resolves to disabled; SCE telemetry is disabled.";
pub(crate) const WARNING_AUTO_SYNC_WITH_AUTH_DISABLED: &str =
    "agent_trace.auto_sync is true but auth.mode resolves to disabled; post-commit auto-sync is skipped.";
pub(crate) const WARNING_REPOSITORY_MANDATORY_OVERRIDDEN_BY_USER_DISABLED: &str = "auth.mode is mandatory in the repository config but disabled in the user config; auth.mode resolves to disabled.";
pub(crate) const WARNING_EXPLICIT_AUTH_MODE_OVERRIDDEN: &str = "auth.mode in the explicitly selected config is less restrictive than the global or repository config; the most restrictive auth.mode applies.";
pub(crate) const WARNING_EXPLICIT_TELEMETRY_OVERRIDDEN: &str = "telemetry.enabled is true in the explicitly selected config but false in the global or repository config; SCE telemetry is disabled.";
pub(crate) const WARNING_INVALID_LAYER_DISABLES_CLOUD: &str = "A discovered config file was skipped as invalid; SCE telemetry and background cloud operations are disabled.";

pub(crate) const CONTROL_PLANE_BASE_URL_KEY: AuthConfigKeySpec = AuthConfigKeySpec {
    config_key: "control_plane_base_url",
    env_key: CONTROL_PLANE_BASE_URL_ENV,
    baked_default: Some(CONTROL_PLANE_BASE_URL_BAKED_DEFAULT),
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AuthConfigKeySpec {
    pub(crate) config_key: &'static str,
    pub(crate) env_key: &'static str,
    pub(crate) baked_default: Option<&'static str>,
}

impl AuthConfigKeySpec {
    pub(crate) fn precedence_description(self) -> String {
        let mut layers = vec![
            format!("env ({})", self.env_key),
            format!("config file ({})", self.config_key),
        ];

        if let Some(default) = self.baked_default {
            layers.push(format!("baked default ({default})"));
        }

        layers.join(" > ")
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ResolvedOtelConfig {
    pub(super) endpoint: ResolvedValue<String>,
    pub(super) protocol: ResolvedValue<String>,
    pub(super) traces_endpoint: ResolvedValue<String>,
    pub(super) metrics_endpoint: ResolvedValue<String>,
    pub(super) logs_endpoint: ResolvedValue<String>,
}

fn resolve_otel_value(
    file_value: Option<schema::FileConfigValue<String>>,
    code_default: &str,
) -> ResolvedValue<String> {
    match file_value {
        Some(value) => ResolvedValue {
            value: value.value,
            source: ValueSource::ConfigFile(value.source),
        },
        None => ResolvedValue {
            value: code_default.to_string(),
            source: ValueSource::Default,
        },
    }
}

fn resolve_otel_config(file_config: schema::OtelFileConfig) -> ResolvedOtelConfig {
    let defaults = MANAGED_OTLP_DEFAULTS;
    ResolvedOtelConfig {
        endpoint: resolve_otel_value(file_config.endpoint, defaults.endpoint),
        protocol: resolve_otel_value(file_config.protocol, defaults.protocol),
        traces_endpoint: resolve_otel_value(file_config.traces_endpoint, defaults.traces_endpoint),
        metrics_endpoint: resolve_otel_value(
            file_config.metrics_endpoint,
            defaults.metrics_endpoint,
        ),
        logs_endpoint: resolve_otel_value(file_config.logs_endpoint, defaults.logs_endpoint),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct RuntimeConfig {
    pub(super) loaded_config_paths: Vec<LoadedConfigPath>,
    pub(super) log_level: ResolvedValue<LogLevel>,
    pub(super) log_format: ResolvedValue<LogFormat>,
    pub(super) log_to_file: ResolvedValue<bool>,
    pub(super) log_dir: ResolvedOptionalValue<String>,
    pub(super) log_file_retention_limit: ResolvedValue<usize>,
    pub(super) attribution_hooks_enabled: ResolvedValue<bool>,
    pub(super) workos_client_id: ResolvedOptionalValue<String>,
    pub(super) control_plane_base_url: ResolvedOptionalValue<String>,
    pub(super) agent_trace_repository_id: ResolvedOptionalValue<String>,
    pub(super) agent_trace_repository_remote: ResolvedValue<String>,
    pub(super) agent_trace_auto_sync: ResolvedValue<bool>,
    pub(super) otel: ResolvedOtelConfig,
    pub(super) bash_policies: ResolvedOptionalValue<BashPolicyConfig>,
    pub(super) database_retry: ResolvedOptionalValue<DatabaseRetryConfig>,
    pub(super) access_policy: ResolvedAccessPolicy,
    pub(super) validation_errors: Vec<String>,
    pub(super) validation_warnings: Vec<String>,
}

pub(crate) fn resolve_auth_runtime_config(cwd: &Path) -> Result<ResolvedAuthRuntimeConfig> {
    resolve_auth_runtime_config_with(
        cwd,
        |key| std::env::var(key).ok(),
        |path| {
            std::fs::read_to_string(path)
                .with_context(|| format!("Failed to read config file '{}'.", path.display()))
        },
        Path::exists,
        resolve_default_global_config_path,
    )
}

pub(crate) fn resolve_observability_runtime_config(
    cwd: &Path,
) -> Result<ResolvedObservabilityRuntimeConfig> {
    resolve_observability_runtime_config_with(
        cwd,
        |key| std::env::var(key).ok(),
        |path| {
            std::fs::read_to_string(path)
                .with_context(|| format!("Failed to read config file '{}'.", path.display()))
        },
        Path::exists,
        resolve_default_global_config_path,
    )
}

pub(crate) fn resolve_hook_runtime_config(cwd: &Path) -> Result<ResolvedHookRuntimeConfig> {
    resolve_hook_runtime_config_with(
        cwd,
        |key| std::env::var(key).ok(),
        |path| {
            std::fs::read_to_string(path)
                .with_context(|| format!("Failed to read config file '{}'.", path.display()))
        },
        Path::exists,
        resolve_default_global_config_path,
    )
}

pub(crate) fn resolve_agent_trace_auto_sync_runtime_config(
    cwd: &Path,
) -> Result<(ResolvedValue<bool>, ResolvedAccessPolicy)> {
    let runtime = resolve_runtime_config_with(
        &ConfigRequest {
            report_format: ReportFormat::Text,
            config_path: None,
            log_level: None,
        },
        cwd,
        |key| std::env::var(key).ok(),
        |path| {
            std::fs::read_to_string(path)
                .with_context(|| format!("Failed to read config file '{}'.", path.display()))
        },
        Path::exists,
        resolve_default_global_config_path,
    )?;

    Ok((runtime.agent_trace_auto_sync, runtime.access_policy))
}

pub(crate) fn resolve_agent_trace_storage_runtime_config(
    cwd: &Path,
) -> Result<ResolvedAgentTraceStorageRuntimeConfig> {
    resolve_agent_trace_storage_runtime_config_with(
        cwd,
        |key| std::env::var(key).ok(),
        |path| {
            std::fs::read_to_string(path)
                .with_context(|| format!("Failed to read config file '{}'.", path.display()))
        },
        Path::exists,
        resolve_default_global_config_path,
    )
}

fn resolve_agent_trace_storage_runtime_config_with<FEnv, FRead, FGlobalPath>(
    cwd: &Path,
    env_lookup: FEnv,
    read_file: FRead,
    path_exists: fn(&Path) -> bool,
    resolve_global_config_path: FGlobalPath,
) -> Result<ResolvedAgentTraceStorageRuntimeConfig>
where
    FEnv: Fn(&str) -> Option<String>,
    FRead: Fn(&Path) -> Result<String>,
    FGlobalPath: Fn() -> Result<PathBuf>,
{
    let runtime = resolve_runtime_config_with(
        &ConfigRequest {
            report_format: ReportFormat::Text,
            config_path: None,
            log_level: None,
        },
        cwd,
        env_lookup,
        read_file,
        path_exists,
        resolve_global_config_path,
    )?;

    Ok(ResolvedAgentTraceStorageRuntimeConfig {
        repository_id: runtime.agent_trace_repository_id.value,
        repository_remote: runtime.agent_trace_repository_remote.value,
    })
}

pub(crate) fn resolve_bash_policy_runtime_config(cwd: &Path) -> Result<Option<BashPolicyConfig>> {
    let runtime = resolve_runtime_config_with(
        &ConfigRequest {
            report_format: ReportFormat::Text,
            config_path: None,
            log_level: None,
        },
        cwd,
        |key| std::env::var(key).ok(),
        |path| {
            std::fs::read_to_string(path)
                .with_context(|| format!("Failed to read config file '{}'.", path.display()))
        },
        Path::exists,
        resolve_default_global_config_path,
    )?;

    Ok(runtime.bash_policies.value)
}

pub(crate) fn resolve_auth_runtime_config_with<FEnv, FRead, FGlobalPath>(
    cwd: &Path,
    env_lookup: FEnv,
    read_file: FRead,
    path_exists: fn(&Path) -> bool,
    resolve_global_config_path: FGlobalPath,
) -> Result<ResolvedAuthRuntimeConfig>
where
    FEnv: Fn(&str) -> Option<String>,
    FRead: Fn(&Path) -> Result<String>,
    FGlobalPath: Fn() -> Result<PathBuf>,
{
    let runtime = resolve_runtime_config_with(
        &ConfigRequest {
            report_format: ReportFormat::Text,
            config_path: None,
            log_level: None,
        },
        cwd,
        env_lookup,
        read_file,
        path_exists,
        resolve_global_config_path,
    )?;

    Ok(ResolvedAuthRuntimeConfig {
        workos_client_id: runtime.workos_client_id,
        control_plane_base_url: runtime.control_plane_base_url,
        access_policy: runtime.access_policy,
    })
}

pub(crate) fn resolve_observability_runtime_config_with<FEnv, FRead, FGlobalPath>(
    cwd: &Path,
    env_lookup: FEnv,
    read_file: FRead,
    path_exists: fn(&Path) -> bool,
    resolve_global_config_path: FGlobalPath,
) -> Result<ResolvedObservabilityRuntimeConfig>
where
    FEnv: Fn(&str) -> Option<String>,
    FRead: Fn(&Path) -> Result<String>,
    FGlobalPath: Fn() -> Result<PathBuf>,
{
    let runtime = resolve_runtime_config_with(
        &ConfigRequest {
            report_format: ReportFormat::Text,
            config_path: None,
            log_level: None,
        },
        cwd,
        env_lookup,
        read_file,
        path_exists,
        resolve_global_config_path,
    )?;

    Ok(ResolvedObservabilityRuntimeConfig {
        log_level: runtime.log_level.value,
        log_format: runtime.log_format.value,
        log_to_file: runtime.log_to_file.value,
        log_dir: runtime.log_dir.value,
        log_file_retention_limit: runtime.log_file_retention_limit.value,
        loaded_config_paths: runtime.loaded_config_paths,
        validation_errors: runtime.validation_errors,
        access_policy: runtime.access_policy,
    })
}

pub(crate) fn resolve_hook_runtime_config_with<FEnv, FRead, FGlobalPath>(
    cwd: &Path,
    env_lookup: FEnv,
    read_file: FRead,
    path_exists: fn(&Path) -> bool,
    resolve_global_config_path: FGlobalPath,
) -> Result<ResolvedHookRuntimeConfig>
where
    FEnv: Fn(&str) -> Option<String>,
    FRead: Fn(&Path) -> Result<String>,
    FGlobalPath: Fn() -> Result<PathBuf>,
{
    let runtime = resolve_runtime_config_with(
        &ConfigRequest {
            report_format: ReportFormat::Text,
            config_path: None,
            log_level: None,
        },
        cwd,
        env_lookup,
        read_file,
        path_exists,
        resolve_global_config_path,
    )?;

    Ok(ResolvedHookRuntimeConfig {
        attribution_hooks_enabled: runtime.attribution_hooks_enabled.value,
        agent_trace_auto_sync: runtime.agent_trace_auto_sync.value,
        access_policy: runtime.access_policy,
    })
}

pub(super) fn resolve_runtime_config(request: &ConfigRequest, cwd: &Path) -> Result<RuntimeConfig> {
    resolve_runtime_config_with(
        request,
        cwd,
        |key| std::env::var(key).ok(),
        |path| {
            std::fs::read_to_string(path)
                .with_context(|| format!("Failed to read config file '{}'.", path.display()))
        },
        Path::exists,
        resolve_default_global_config_path,
    )
}

#[allow(clippy::too_many_lines)]
pub(super) fn resolve_runtime_config_with<FEnv, FRead, FGlobalPath>(
    request: &ConfigRequest,
    cwd: &Path,
    env_lookup: FEnv,
    read_file: FRead,
    path_exists: fn(&Path) -> bool,
    resolve_global_config_path: FGlobalPath,
) -> Result<RuntimeConfig>
where
    FEnv: Fn(&str) -> Option<String>,
    FRead: Fn(&Path) -> Result<String>,
    FGlobalPath: Fn() -> Result<PathBuf>,
{
    let loaded_config_paths = resolve_config_paths(
        request,
        cwd,
        &env_lookup,
        path_exists,
        resolve_global_config_path,
    )?;

    let mut file_config = schema::FileConfig {
        log_level: None,
        log_format: None,
        log_to_file: None,
        log_dir: None,
        log_file_retention_limit: None,
        attribution_hooks_enabled: None,
        workos_client_id: None,
        control_plane_base_url: None,
        agent_trace_repository_id: None,
        agent_trace_repository_remote: None,
        agent_trace_auto_sync: None,
        auth_mode: None,
        telemetry_enabled: None,
        otel: schema::OtelFileConfig::default(),
        bash_policy_presets: None,
        bash_policy_custom: None,
        database_retry: None,
        integrations: None,
    };
    let mut validation_errors = Vec::new();
    let mut user_policy_layer = AccessPolicyLayer::default();
    let mut repository_policy_layer = AccessPolicyLayer::default();
    let mut explicit_policy_layer = AccessPolicyLayer::default();
    let explicit_config_selected = loaded_config_paths
        .iter()
        .any(|loaded_path| !loaded_path.source.is_default_discovered());
    for loaded_path in &loaded_config_paths {
        let raw = read_file(&loaded_path.path)?;
        let layer = match schema::parse_file_config(&raw, &loaded_path.path, loaded_path.source) {
            Ok(layer) => layer,
            Err(error) if loaded_path.source.is_default_discovered() => {
                validation_errors.push(error.to_string());
                continue;
            }
            Err(error) => return Err(error),
        };
        let policy_layer = match PolicyLayer::from_config_path_source(loaded_path.source) {
            PolicyLayer::User => &mut user_policy_layer,
            PolicyLayer::Repository => &mut repository_policy_layer,
            PolicyLayer::Explicit => &mut explicit_policy_layer,
        };
        policy_layer.auth_mode = layer.auth_mode;
        policy_layer.telemetry_enabled = layer.telemetry_enabled;
        if explicit_config_selected && loaded_path.source.is_default_discovered() {
            continue;
        }
        if let Some(log_level) = layer.log_level {
            file_config.log_level = Some(log_level);
        }
        if let Some(log_format) = layer.log_format {
            file_config.log_format = Some(log_format);
        }
        if let Some(log_to_file) = layer.log_to_file {
            file_config.log_to_file = Some(log_to_file);
        }
        if let Some(log_dir) = layer.log_dir {
            file_config.log_dir = Some(log_dir);
        }
        if let Some(log_file_retention_limit) = layer.log_file_retention_limit {
            file_config.log_file_retention_limit = Some(log_file_retention_limit);
        }
        if let Some(attribution_hooks_enabled) = layer.attribution_hooks_enabled {
            file_config.attribution_hooks_enabled = Some(attribution_hooks_enabled);
        }
        if let Some(workos_client_id) = layer.workos_client_id {
            file_config.workos_client_id = Some(workos_client_id);
        }
        if let Some(control_plane_base_url) = layer.control_plane_base_url {
            file_config.control_plane_base_url = Some(control_plane_base_url);
        }
        if let Some(agent_trace_repository_id) = layer.agent_trace_repository_id {
            file_config.agent_trace_repository_id = Some(agent_trace_repository_id);
        }
        if let Some(agent_trace_repository_remote) = layer.agent_trace_repository_remote {
            file_config.agent_trace_repository_remote = Some(agent_trace_repository_remote);
        }
        if let Some(agent_trace_auto_sync) = layer.agent_trace_auto_sync {
            file_config.agent_trace_auto_sync = Some(agent_trace_auto_sync);
        }
        merge_otel_layer(&mut file_config.otel, layer.otel);
        if let Some(bash_policy_presets) = layer.bash_policy_presets {
            file_config.bash_policy_presets = Some(bash_policy_presets);
        }
        if let Some(bash_policy_custom) = layer.bash_policy_custom {
            file_config.bash_policy_custom = Some(bash_policy_custom);
        }
        if let Some(database_retry) = layer.database_retry {
            file_config.database_retry = Some(database_retry);
        }
        if let Some(integrations) = layer.integrations {
            file_config.integrations = Some(integrations);
        }
    }

    let mut resolved_log_level = ResolvedValue {
        value: LogLevel::Error,
        source: ValueSource::Default,
    };
    if let Some(value) = file_config.log_level {
        resolved_log_level = ResolvedValue {
            value: value.value,
            source: ValueSource::ConfigFile(value.source),
        };
    }
    if let Some(raw) = env_lookup(ENV_LOG_LEVEL) {
        resolved_log_level = ResolvedValue {
            value: LogLevel::parse(&raw, ENV_LOG_LEVEL)?,
            source: ValueSource::Env,
        };
    }
    if let Some(value) = request.log_level {
        resolved_log_level = ResolvedValue {
            value,
            source: ValueSource::Flag,
        };
    }

    let mut resolved_log_format = ResolvedValue {
        value: LogFormat::Text,
        source: ValueSource::Default,
    };
    if let Some(value) = file_config.log_format {
        resolved_log_format = ResolvedValue {
            value: value.value,
            source: ValueSource::ConfigFile(value.source),
        };
    }
    if let Some(raw) = env_lookup(ENV_LOG_FORMAT) {
        resolved_log_format = ResolvedValue {
            value: LogFormat::parse(&raw, ENV_LOG_FORMAT)?,
            source: ValueSource::Env,
        };
    }

    let resolved_log_to_file = match file_config.log_to_file {
        Some(value) => ResolvedValue {
            value: value.value,
            source: ValueSource::ConfigFile(value.source),
        },
        None => ResolvedValue {
            value: true,
            source: ValueSource::Default,
        },
    };

    let resolved_log_dir = if let Some(raw) = env_lookup(ENV_LOG_DIR) {
        ResolvedOptionalValue {
            value: Some(raw),
            source: Some(ValueSource::Env),
        }
    } else if let Some(value) = file_config.log_dir.as_ref() {
        ResolvedOptionalValue {
            value: Some(value.value.clone()),
            source: Some(ValueSource::ConfigFile(value.source)),
        }
    } else {
        default_observability_log_dir()?
    };

    let resolved_log_file_retention_limit = match file_config.log_file_retention_limit {
        Some(value) => ResolvedValue {
            value: value.value,
            source: ValueSource::ConfigFile(value.source),
        },
        None => ResolvedValue {
            value: DEFAULT_LOG_FILE_RETENTION_LIMIT,
            source: ValueSource::Default,
        },
    };

    let mut resolved_attribution_hooks_enabled = ResolvedValue {
        value: true,
        source: ValueSource::Default,
    };
    if let Some(value) = file_config.attribution_hooks_enabled {
        resolved_attribution_hooks_enabled = ResolvedValue {
            value: value.value,
            source: ValueSource::ConfigFile(value.source),
        };
    }
    if let Some(raw) = env_lookup(ENV_ATTRIBUTION_HOOKS_DISABLED) {
        resolved_attribution_hooks_enabled = ResolvedValue {
            value: !parse_bool_value_from(
                ENV_ATTRIBUTION_HOOKS_DISABLED,
                &raw,
                ENV_ATTRIBUTION_HOOKS_DISABLED,
            )?,
            source: ValueSource::Env,
        };
    }
    let resolved_workos_client_id = resolve_optional_auth_config_value(
        WORKOS_CLIENT_ID_KEY,
        file_config.workos_client_id,
        &env_lookup,
    );
    let resolved_control_plane_base_url = resolve_optional_auth_config_value(
        CONTROL_PLANE_BASE_URL_KEY,
        file_config.control_plane_base_url,
        &env_lookup,
    );

    let resolved_agent_trace_repository_id = ResolvedOptionalValue {
        value: file_config
            .agent_trace_repository_id
            .as_ref()
            .map(|value| value.value.clone()),
        source: file_config
            .agent_trace_repository_id
            .as_ref()
            .map(|value| ValueSource::ConfigFile(value.source)),
    };

    let mut resolved_agent_trace_repository_remote = ResolvedValue {
        value: DEFAULT_AGENT_TRACE_REPOSITORY_REMOTE.to_string(),
        source: ValueSource::Default,
    };
    if let Some(value) = file_config.agent_trace_repository_remote {
        resolved_agent_trace_repository_remote = ResolvedValue {
            value: value.value,
            source: ValueSource::ConfigFile(value.source),
        };
    }

    let resolved_agent_trace_auto_sync = match file_config.agent_trace_auto_sync {
        Some(value) => ResolvedValue {
            value: value.value,
            source: ValueSource::ConfigFile(value.source),
        },
        None => ResolvedValue {
            value: false,
            source: ValueSource::Default,
        },
    };

    let resolved_bash_policies = resolve_bash_policy_config(
        file_config.bash_policy_presets.as_ref(),
        file_config.bash_policy_custom.as_ref(),
    );
    let mut validation_warnings = build_validation_warnings(&resolved_bash_policies);

    let degraded = !validation_errors.is_empty();
    let access_policy = resolve_access_policy(
        &user_policy_layer,
        &repository_policy_layer,
        &explicit_policy_layer,
        degraded,
    );
    validation_warnings.extend(build_access_policy_warnings(
        &user_policy_layer,
        &repository_policy_layer,
        &explicit_policy_layer,
        &access_policy,
        resolved_agent_trace_auto_sync.value,
        degraded,
    ));

    let resolved_database_retry =
        resolve_database_retry_config(file_config.database_retry.as_ref());

    Ok(RuntimeConfig {
        loaded_config_paths,
        log_level: resolved_log_level,
        log_format: resolved_log_format,
        log_to_file: resolved_log_to_file,
        log_dir: resolved_log_dir,
        log_file_retention_limit: resolved_log_file_retention_limit,
        attribution_hooks_enabled: resolved_attribution_hooks_enabled,
        workos_client_id: resolved_workos_client_id,
        control_plane_base_url: resolved_control_plane_base_url,
        agent_trace_repository_id: resolved_agent_trace_repository_id,
        agent_trace_repository_remote: resolved_agent_trace_repository_remote,
        agent_trace_auto_sync: resolved_agent_trace_auto_sync,
        otel: resolve_otel_config(file_config.otel),
        bash_policies: resolved_bash_policies,
        database_retry: resolved_database_retry,
        access_policy,
        validation_errors,
        validation_warnings,
    })
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct AccessPolicyLayer {
    pub(super) auth_mode: Option<schema::FileConfigValue<AuthMode>>,
    pub(super) telemetry_enabled: Option<schema::FileConfigValue<bool>>,
}

impl AccessPolicyLayer {
    fn telemetry(&self) -> Option<bool> {
        self.telemetry_enabled.as_ref().map(|value| value.value)
    }

    fn auth_mode(&self) -> Option<AuthMode> {
        self.auth_mode.as_ref().map(|value| value.value)
    }
}

const fn auth_mode_restriction_rank(mode: AuthMode) -> u8 {
    match mode {
        AuthMode::Optional => 0,
        AuthMode::Mandatory => 1,
        AuthMode::Disabled => 2,
    }
}

fn resolve_effective_auth_mode(
    user: &AccessPolicyLayer,
    repository: &AccessPolicyLayer,
    explicit: &AccessPolicyLayer,
) -> ResolvedValue<AuthMode> {
    let mut resolved = ResolvedValue {
        value: AuthMode::Optional,
        source: ValueSource::Default,
    };
    let mut deciding_rank = None;
    for value in [
        user.auth_mode.as_ref(),
        repository.auth_mode.as_ref(),
        explicit.auth_mode.as_ref(),
    ]
    .into_iter()
    .flatten()
    {
        let rank = auth_mode_restriction_rank(value.value);
        if deciding_rank.is_none_or(|deciding| rank >= deciding) {
            deciding_rank = Some(rank);
            resolved = ResolvedValue {
                value: value.value,
                source: ValueSource::ConfigFile(value.source),
            };
        }
    }
    resolved
}

pub(super) fn resolve_access_policy(
    user: &AccessPolicyLayer,
    repository: &AccessPolicyLayer,
    explicit: &AccessPolicyLayer,
    degraded: bool,
) -> ResolvedAccessPolicy {
    let auth_mode = resolve_effective_auth_mode(user, repository, explicit);
    let auth_disabled = auth_mode.value == AuthMode::Disabled;

    let telemetry_enabled = match explicit
        .telemetry_enabled
        .as_ref()
        .or(repository.telemetry_enabled.as_ref())
        .or(user.telemetry_enabled.as_ref())
    {
        Some(value) => ResolvedValue {
            value: value.value,
            source: ValueSource::ConfigFile(value.source),
        },
        None => ResolvedValue {
            value: false,
            source: ValueSource::Default,
        },
    };

    let telemetry = if degraded {
        TelemetryPolicyState::Disabled(TelemetryDisabledReason::InvalidConfig)
    } else if auth_disabled {
        TelemetryPolicyState::Disabled(TelemetryDisabledReason::AuthDisabled)
    } else if user_telemetry_opt_out(user, explicit) {
        TelemetryPolicyState::Disabled(TelemetryDisabledReason::UserOptOut)
    } else if repository.telemetry() == Some(false) {
        TelemetryPolicyState::Disabled(TelemetryDisabledReason::RepositoryOptOut)
    } else if user_telemetry_consent(user, explicit) {
        TelemetryPolicyState::Enabled
    } else if repository.telemetry() == Some(true) {
        TelemetryPolicyState::Disabled(TelemetryDisabledReason::NoUserConsent)
    } else {
        TelemetryPolicyState::Disabled(TelemetryDisabledReason::NotEnabled)
    };

    ResolvedAccessPolicy {
        auth_mode,
        telemetry_enabled,
        telemetry,
        background_cloud_allowed: !auth_disabled && !degraded,
    }
}

fn user_telemetry_opt_out(user: &AccessPolicyLayer, explicit: &AccessPolicyLayer) -> bool {
    user.telemetry() == Some(false) || explicit.telemetry() == Some(false)
}

fn user_telemetry_consent(user: &AccessPolicyLayer, explicit: &AccessPolicyLayer) -> bool {
    user.telemetry() == Some(true) || explicit.telemetry() == Some(true)
}

fn build_access_policy_warnings(
    user: &AccessPolicyLayer,
    repository: &AccessPolicyLayer,
    explicit: &AccessPolicyLayer,
    policy: &ResolvedAccessPolicy,
    agent_trace_auto_sync: bool,
    degraded: bool,
) -> Vec<String> {
    let auth_disabled = policy.auth_mode.value == AuthMode::Disabled;
    let layers = [user, repository, explicit];
    let mut warnings = Vec::new();
    if repository.telemetry() == Some(true) && !user_telemetry_consent(user, explicit) {
        warnings.push(WARNING_REPOSITORY_TELEMETRY_WITHOUT_USER_CONSENT.to_string());
    }
    if auth_disabled && layers.iter().any(|layer| layer.telemetry() == Some(true)) {
        warnings.push(WARNING_TELEMETRY_ENABLED_WITH_AUTH_DISABLED.to_string());
    }
    if auth_disabled && agent_trace_auto_sync {
        warnings.push(WARNING_AUTO_SYNC_WITH_AUTH_DISABLED.to_string());
    }
    if repository.auth_mode() == Some(AuthMode::Mandatory)
        && (user.auth_mode() == Some(AuthMode::Disabled)
            || explicit.auth_mode() == Some(AuthMode::Disabled))
    {
        warnings.push(WARNING_REPOSITORY_MANDATORY_OVERRIDDEN_BY_USER_DISABLED.to_string());
    }
    if explicit.auth_mode().is_some_and(|mode| {
        auth_mode_restriction_rank(mode) < auth_mode_restriction_rank(policy.auth_mode.value)
    }) {
        warnings.push(WARNING_EXPLICIT_AUTH_MODE_OVERRIDDEN.to_string());
    }
    if explicit.telemetry() == Some(true)
        && (user.telemetry() == Some(false) || repository.telemetry() == Some(false))
    {
        warnings.push(WARNING_EXPLICIT_TELEMETRY_OVERRIDDEN.to_string());
    }
    if degraded {
        warnings.push(WARNING_INVALID_LAYER_DISABLES_CLOUD.to_string());
    }
    warnings
}

fn merge_otel_layer(target: &mut schema::OtelFileConfig, layer: schema::OtelFileConfig) {
    if layer.endpoint.is_some() {
        target.endpoint = layer.endpoint;
    }
    if layer.protocol.is_some() {
        target.protocol = layer.protocol;
    }
    if layer.traces_endpoint.is_some() {
        target.traces_endpoint = layer.traces_endpoint;
    }
    if layer.metrics_endpoint.is_some() {
        target.metrics_endpoint = layer.metrics_endpoint;
    }
    if layer.logs_endpoint.is_some() {
        target.logs_endpoint = layer.logs_endpoint;
    }
}

fn resolve_optional_auth_config_value<FEnv>(
    key: AuthConfigKeySpec,
    file_value: Option<schema::FileConfigValue<String>>,
    env_lookup: &FEnv,
) -> ResolvedOptionalValue<String>
where
    FEnv: Fn(&str) -> Option<String>,
{
    if let Some(raw) = env_lookup(key.env_key) {
        return ResolvedOptionalValue {
            value: Some(raw),
            source: Some(ValueSource::Env),
        };
    }

    if let Some(value) = file_value {
        return ResolvedOptionalValue {
            value: Some(value.value),
            source: Some(ValueSource::ConfigFile(value.source)),
        };
    }

    if let Some(value) = key.baked_default {
        return ResolvedOptionalValue {
            value: Some(value.to_string()),
            source: Some(ValueSource::Default),
        };
    }

    ResolvedOptionalValue {
        value: None,
        source: None,
    }
}

fn default_observability_log_dir() -> Result<ResolvedOptionalValue<String>> {
    let path = observability_log_dir().with_context(|| {
        format!(
            "Failed to resolve default observability log directory for {ENV_LOG_DIR}; set {ENV_LOG_DIR} or config log_dir explicitly."
        )
    })?;

    Ok(ResolvedOptionalValue {
        value: Some(path.to_string_lossy().into_owned()),
        source: Some(ValueSource::Default),
    })
}

fn resolve_config_paths<FEnv, FGlobalPath>(
    request: &ConfigRequest,
    cwd: &Path,
    env_lookup: &FEnv,
    path_exists: fn(&Path) -> bool,
    resolve_global_config_path: FGlobalPath,
) -> Result<Vec<LoadedConfigPath>>
where
    FEnv: Fn(&str) -> Option<String>,
    FGlobalPath: Fn() -> Result<PathBuf>,
{
    let explicit_path = resolve_explicit_config_path(request, env_lookup, path_exists)?;

    let mut loaded_paths = Vec::new();

    let global_path = resolve_global_config_path()?;
    if path_exists(&global_path) {
        loaded_paths.push(LoadedConfigPath {
            path: global_path,
            source: ConfigPathSource::DefaultDiscoveredGlobal,
        });
    }

    let repository_root = enclosing_repository_root_with(cwd, path_exists).unwrap_or(cwd);
    let local_path = RepoPaths::new(repository_root).sce_config_file();
    if path_exists(&local_path) {
        loaded_paths.push(LoadedConfigPath {
            path: local_path,
            source: ConfigPathSource::DefaultDiscoveredLocal,
        });
    }

    loaded_paths.extend(explicit_path);
    Ok(loaded_paths)
}

fn resolve_explicit_config_path<FEnv>(
    request: &ConfigRequest,
    env_lookup: &FEnv,
    path_exists: fn(&Path) -> bool,
) -> Result<Option<LoadedConfigPath>>
where
    FEnv: Fn(&str) -> Option<String>,
{
    if let Some(path) = request.config_path.as_ref() {
        if !path_exists(path) {
            bail!(
                "Config file '{}' was provided via --config but does not exist.",
                path.display()
            );
        }
        return Ok(Some(LoadedConfigPath {
            path: path.clone(),
            source: ConfigPathSource::Flag,
        }));
    }

    if let Some(raw) = env_lookup("SCE_CONFIG_FILE") {
        let path = PathBuf::from(raw);
        if !path_exists(&path) {
            bail!(
                "Config file '{}' was provided via SCE_CONFIG_FILE but does not exist.",
                path.display()
            );
        }
        return Ok(Some(LoadedConfigPath {
            path,
            source: ConfigPathSource::Env,
        }));
    }

    Ok(None)
}

fn resolve_default_global_config_path() -> Result<PathBuf> {
    Ok(resolve_sce_default_locations()?.global_config_file())
}

fn resolve_database_retry_config(
    file_config: Option<&schema::FileConfigValue<DatabaseRetryConfig>>,
) -> ResolvedOptionalValue<DatabaseRetryConfig> {
    match file_config {
        Some(value) => ResolvedOptionalValue {
            value: Some(value.value.clone()),
            source: Some(ValueSource::ConfigFile(value.source)),
        },
        None => ResolvedOptionalValue {
            value: None,
            source: None,
        },
    }
}

use std::sync::OnceLock;

static DATABASE_RETRY_CONFIG: OnceLock<DatabaseRetryConfig> = OnceLock::new();

pub(crate) fn init_database_retry_config(config: DatabaseRetryConfig) -> Result<()> {
    DATABASE_RETRY_CONFIG
        .set(config)
        .map_err(|_| anyhow!("Database retry config has already been initialized."))
}

pub(crate) fn get_database_retry_config() -> Option<&'static DatabaseRetryConfig> {
    DATABASE_RETRY_CONFIG.get()
}

/// Resolve the full runtime config from the environment and initialize the
/// database retry `OnceLock`. Silently ignores errors — if the config cannot
/// be resolved, DB adapters fall back to hardcoded defaults.
pub(crate) fn init_database_retry_config_from_environment(cwd: &Path) {
    if let Ok(runtime) = resolve_runtime_config(
        &ConfigRequest {
            report_format: ReportFormat::Text,
            config_path: None,
            log_level: None,
        },
        cwd,
    ) {
        if let Some(config) = runtime.database_retry.value {
            let _ = init_database_retry_config(config);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;

    fn path_exists(path: &Path) -> bool {
        path == Path::new("/tmp/sce-config.json")
    }

    fn missing_path(_: &Path) -> bool {
        false
    }

    fn empty_request() -> ConfigRequest {
        ConfigRequest {
            report_format: ReportFormat::Text,
            config_path: None,
            log_level: None,
        }
    }

    fn explicit_config_request() -> ConfigRequest {
        ConfigRequest {
            config_path: Some(PathBuf::from("/tmp/sce-config.json")),
            ..empty_request()
        }
    }

    fn resolve_hooks_with_env_and_config(
        env: Option<(&'static str, &'static str)>,
        config: Option<&'static str>,
    ) -> Result<ResolvedHookRuntimeConfig> {
        let request = if config.is_some() {
            explicit_config_request()
        } else {
            empty_request()
        };
        let path_exists_fn = if config.is_some() {
            path_exists
        } else {
            missing_path
        };

        let runtime = resolve_runtime_config_with(
            &request,
            Path::new("/tmp/repo"),
            |key| env.and_then(|(env_key, value)| (key == env_key).then_some(value.to_string())),
            |_| Ok(config.unwrap_or("{}").to_string()),
            path_exists_fn,
            || Ok(PathBuf::from("/tmp/missing-global-sce-config.json")),
        )?;

        Ok(ResolvedHookRuntimeConfig {
            attribution_hooks_enabled: runtime.attribution_hooks_enabled.value,
            agent_trace_auto_sync: runtime.agent_trace_auto_sync.value,
            access_policy: runtime.access_policy,
        })
    }

    fn resolve_runtime_with_config(config: Option<&'static str>) -> Result<RuntimeConfig> {
        resolve_runtime_with_env_and_config(None, config)
    }

    fn resolve_runtime_with_env_and_config(
        env: Option<(&'static str, &'static str)>,
        config: Option<&'static str>,
    ) -> Result<RuntimeConfig> {
        let request = if config.is_some() {
            explicit_config_request()
        } else {
            empty_request()
        };
        let path_exists_fn = if config.is_some() {
            path_exists
        } else {
            missing_path
        };

        resolve_runtime_config_with(
            &request,
            Path::new("/tmp/repo"),
            |key| env.and_then(|(env_key, value)| (key == env_key).then_some(value.to_string())),
            |_| Ok(config.unwrap_or("{}").to_string()),
            path_exists_fn,
            || Ok(PathBuf::from("/tmp/missing-global-sce-config.json")),
        )
    }

    #[test]
    fn agent_trace_repository_id_is_unset_by_default() {
        let runtime = resolve_runtime_with_config(None).unwrap();

        assert_eq!(runtime.agent_trace_repository_id.value, None);
        assert_eq!(runtime.agent_trace_repository_id.source, None);
    }

    #[test]
    fn log_to_file_defaults_to_true() {
        let runtime = resolve_runtime_with_config(None).unwrap();

        assert!(runtime.log_to_file.value);
        assert_eq!(runtime.log_to_file.source, ValueSource::Default);
    }

    #[test]
    fn log_to_file_and_log_dir_resolve_independently() {
        let enabled_without_log_dir =
            resolve_runtime_with_config(Some(r#"{"log_to_file":true}"#)).unwrap();
        let log_dir_without_log_to_file =
            resolve_runtime_with_config(Some(r#"{"log_dir":"/tmp/sce-logs"}"#)).unwrap();
        let disabled_without_log_dir =
            resolve_runtime_with_config(Some(r#"{"log_to_file":false}"#)).unwrap();

        assert!(enabled_without_log_dir.log_to_file.value);
        assert!(enabled_without_log_dir.log_dir.value.is_some());
        assert!(enabled_without_log_dir.validation_errors.is_empty());
        assert_eq!(
            log_dir_without_log_to_file.log_dir.value.as_deref(),
            Some("/tmp/sce-logs")
        );
        assert!(log_dir_without_log_to_file.log_to_file.value);
        assert!(disabled_without_log_dir.log_dir.value.is_some());
        assert!(disabled_without_log_dir.validation_errors.is_empty());
        assert_eq!(
            enabled_without_log_dir.log_to_file.source,
            ValueSource::ConfigFile(ConfigPathSource::Flag)
        );
        assert_eq!(
            disabled_without_log_dir.log_to_file.source,
            ValueSource::ConfigFile(ConfigPathSource::Flag)
        );
    }

    #[test]
    fn log_to_file_resolves_both_explicit_boolean_values() {
        let enabled = resolve_runtime_with_config(Some(r#"{"log_to_file":true}"#)).unwrap();
        let disabled = resolve_runtime_with_config(Some(r#"{"log_to_file":false}"#)).unwrap();

        assert!(enabled.log_to_file.value);
        assert!(!disabled.log_to_file.value);
        assert_eq!(
            enabled.log_to_file.source,
            ValueSource::ConfigFile(ConfigPathSource::Flag)
        );
        assert_eq!(
            disabled.log_to_file.source,
            ValueSource::ConfigFile(ConfigPathSource::Flag)
        );
    }

    #[test]
    fn otel_resolves_to_code_defaults_when_block_absent() {
        let runtime = resolve_runtime_with_config(None).unwrap();

        assert_eq!(runtime.otel.endpoint.value, "https://otel.crocoderlab.dev");
        assert_eq!(runtime.otel.protocol.value, "http/protobuf");
        assert_eq!(
            runtime.otel.traces_endpoint.value,
            "https://otel.crocoderlab.dev/v1/traces"
        );
        assert_eq!(
            runtime.otel.metrics_endpoint.value,
            "https://otel.crocoderlab.dev/v1/metrics"
        );
        assert_eq!(
            runtime.otel.logs_endpoint.value,
            "https://otel.crocoderlab.dev/v1/logs"
        );
        assert_eq!(runtime.otel.endpoint.source, ValueSource::Default);
        assert_eq!(runtime.otel.logs_endpoint.source, ValueSource::Default);
    }

    #[test]
    fn otel_partial_block_keeps_code_defaults_for_omitted_fields() {
        let runtime = resolve_runtime_with_config(Some(
            r#"{"otel":{"logs_endpoint":"https://otel.crocoderlab.dev/v1/logs"}}"#,
        ))
        .unwrap();

        assert_eq!(
            runtime.otel.logs_endpoint.source,
            ValueSource::ConfigFile(ConfigPathSource::Flag)
        );
        assert_eq!(runtime.otel.traces_endpoint.source, ValueSource::Default);
        assert_eq!(
            runtime.otel.traces_endpoint.value,
            "https://otel.crocoderlab.dev/v1/traces"
        );
    }

    #[test]
    fn agent_trace_auto_sync_defaults_to_false_when_missing() {
        let runtime = resolve_runtime_with_config(None).unwrap();

        assert!(!runtime.agent_trace_auto_sync.value);
        assert_eq!(runtime.agent_trace_auto_sync.source, ValueSource::Default);
    }

    #[test]
    fn agent_trace_auto_sync_resolves_from_config_file() {
        let runtime =
            resolve_runtime_with_config(Some(r#"{"agent_trace":{"auto_sync":true}}"#)).unwrap();

        assert!(runtime.agent_trace_auto_sync.value);
        assert_eq!(
            runtime.agent_trace_auto_sync.source,
            ValueSource::ConfigFile(ConfigPathSource::Flag)
        );
    }

    #[test]
    fn agent_trace_auto_sync_uses_local_config_over_global_config() {
        let runtime = resolve_runtime_config_with(
            &empty_request(),
            Path::new("/tmp/repo"),
            |_| None,
            |path| {
                if path == Path::new("/tmp/global-sce-config.json") {
                    Ok(r#"{"agent_trace":{"auto_sync":false}}"#.to_string())
                } else {
                    Ok(r#"{"agent_trace":{"auto_sync":true}}"#.to_string())
                }
            },
            |path| {
                path == Path::new("/tmp/global-sce-config.json")
                    || path == Path::new("/tmp/repo/.sce/config.json")
            },
            || Ok(PathBuf::from("/tmp/global-sce-config.json")),
        )
        .unwrap();

        assert!(runtime.agent_trace_auto_sync.value);
        assert_eq!(
            runtime.agent_trace_auto_sync.source,
            ValueSource::ConfigFile(ConfigPathSource::DefaultDiscoveredLocal)
        );
    }

    #[test]
    fn agent_trace_repository_remote_defaults_to_origin() {
        let runtime = resolve_runtime_with_config(None).unwrap();

        assert_eq!(
            runtime.agent_trace_repository_remote.value,
            DEFAULT_AGENT_TRACE_REPOSITORY_REMOTE
        );
        assert_eq!(
            runtime.agent_trace_repository_remote.source,
            ValueSource::Default
        );
    }

    #[test]
    fn agent_trace_explicit_repository_id_resolves_from_config_file() {
        let runtime = resolve_runtime_with_config(Some(
            r#"{"agent_trace":{"repository_id":"team-monorepo"}}"#,
        ))
        .unwrap();

        assert_eq!(
            runtime.agent_trace_repository_id.value.as_deref(),
            Some("team-monorepo")
        );
        assert_eq!(
            runtime.agent_trace_repository_id.source,
            Some(ValueSource::ConfigFile(ConfigPathSource::Flag))
        );
        assert_eq!(
            runtime.agent_trace_repository_remote.value,
            DEFAULT_AGENT_TRACE_REPOSITORY_REMOTE
        );
    }

    #[test]
    fn agent_trace_repository_remote_override_resolves_from_config_file() {
        let runtime = resolve_runtime_with_config(Some(
            r#"{"agent_trace":{"repository_remote":"upstream"}}"#,
        ))
        .unwrap();

        assert_eq!(runtime.agent_trace_repository_remote.value, "upstream");
        assert_eq!(
            runtime.agent_trace_repository_remote.source,
            ValueSource::ConfigFile(ConfigPathSource::Flag)
        );
        assert_eq!(runtime.agent_trace_repository_id.value, None);
    }

    #[test]
    fn attribution_hooks_are_enabled_by_default() {
        let resolved = resolve_hooks_with_env_and_config(None, None).unwrap();

        assert!(resolved.attribution_hooks_enabled);
    }

    #[test]
    fn attribution_hooks_disabled_env_truthy_opts_out() {
        let resolved =
            resolve_hooks_with_env_and_config(Some((ENV_ATTRIBUTION_HOOKS_DISABLED, "1")), None)
                .unwrap();

        assert!(!resolved.attribution_hooks_enabled);
    }

    #[test]
    fn explicit_config_false_opts_out() {
        let resolved = resolve_hooks_with_env_and_config(
            None,
            Some(r#"{"policies":{"attribution_hooks":{"enabled":false}}}"#),
        )
        .unwrap();

        assert!(!resolved.attribution_hooks_enabled);
    }

    #[test]
    fn disabled_env_false_overrides_config_false() {
        let resolved = resolve_hooks_with_env_and_config(
            Some((ENV_ATTRIBUTION_HOOKS_DISABLED, "0")),
            Some(r#"{"policies":{"attribution_hooks":{"enabled":false}}}"#),
        )
        .unwrap();

        assert!(resolved.attribution_hooks_enabled);
    }

    #[test]
    fn explicit_config_false_preserves_legacy_default_off_opt_out() {
        let resolved = resolve_hooks_with_env_and_config(
            None,
            Some(r#"{"policies":{"attribution_hooks":{"enabled":false}}}"#),
        )
        .unwrap();

        assert!(!resolved.attribution_hooks_enabled);
    }

    #[test]
    fn control_plane_base_url_resolves_to_baked_default() {
        let runtime = resolve_runtime_with_config(None).unwrap();

        assert_eq!(
            runtime.control_plane_base_url.value.as_deref(),
            Some(CONTROL_PLANE_BASE_URL_BAKED_DEFAULT)
        );
        assert_eq!(
            runtime.control_plane_base_url.source,
            Some(ValueSource::Default)
        );
    }

    #[test]
    fn control_plane_base_url_resolves_from_config_file_over_default() {
        let runtime = resolve_runtime_with_config(Some(
            r#"{"control_plane_base_url":"https://control-plane.example.test"}"#,
        ))
        .unwrap();

        assert_eq!(
            runtime.control_plane_base_url.value.as_deref(),
            Some("https://control-plane.example.test")
        );
        assert_eq!(
            runtime.control_plane_base_url.source,
            Some(ValueSource::ConfigFile(ConfigPathSource::Flag))
        );
    }

    #[test]
    fn control_plane_base_url_env_overrides_config_file_and_default() {
        let runtime = resolve_runtime_with_env_and_config(
            Some((CONTROL_PLANE_BASE_URL_ENV, "https://control-plane.env.test")),
            Some(r#"{"control_plane_base_url":"https://control-plane.example.test"}"#),
        )
        .unwrap();

        assert_eq!(
            runtime.control_plane_base_url.value.as_deref(),
            Some("https://control-plane.env.test")
        );
        assert_eq!(
            runtime.control_plane_base_url.source,
            Some(ValueSource::Env)
        );
    }
}

#[cfg(test)]
mod access_policy_tests {
    use std::path::{Path, PathBuf};

    use super::*;

    const GLOBAL_PATH: &str = "/tmp/policy-global/sce/config.json";
    const LOCAL_PATH: &str = "/tmp/policy-repo/.sce/config.json";
    const EXPLICIT_PATH: &str = "/tmp/policy-explicit.json";

    #[derive(Clone, Copy, Debug)]
    enum Decider {
        Default,
        User,
        Repository,
    }

    fn layer(
        source: ConfigPathSource,
        auth_mode: Option<AuthMode>,
        telemetry: Option<bool>,
    ) -> AccessPolicyLayer {
        AccessPolicyLayer {
            auth_mode: auth_mode.map(|value| schema::FileConfigValue { value, source }),
            telemetry_enabled: telemetry.map(|value| schema::FileConfigValue { value, source }),
        }
    }

    fn user(auth_mode: Option<AuthMode>, telemetry: Option<bool>) -> AccessPolicyLayer {
        layer(
            ConfigPathSource::DefaultDiscoveredGlobal,
            auth_mode,
            telemetry,
        )
    }

    fn repository(auth_mode: Option<AuthMode>, telemetry: Option<bool>) -> AccessPolicyLayer {
        layer(
            ConfigPathSource::DefaultDiscoveredLocal,
            auth_mode,
            telemetry,
        )
    }

    fn decider_source(decider: Decider) -> ValueSource {
        match decider {
            Decider::Default => ValueSource::Default,
            Decider::User => ValueSource::ConfigFile(ConfigPathSource::DefaultDiscoveredGlobal),
            Decider::Repository => {
                ValueSource::ConfigFile(ConfigPathSource::DefaultDiscoveredLocal)
            }
        }
    }

    #[test]
    fn access_policy_auth_mode_most_restrictive_layer_wins() {
        use AuthMode::{Disabled, Mandatory, Optional};
        use Decider::{Default, Repository, User};

        let cases = [
            (None, None, Optional, Default),
            (None, Some(Disabled), Disabled, Repository),
            (None, Some(Optional), Optional, Repository),
            (None, Some(Mandatory), Mandatory, Repository),
            (Some(Disabled), None, Disabled, User),
            (Some(Disabled), Some(Disabled), Disabled, Repository),
            (Some(Disabled), Some(Optional), Disabled, User),
            (Some(Disabled), Some(Mandatory), Disabled, User),
            (Some(Optional), None, Optional, User),
            (Some(Optional), Some(Disabled), Disabled, Repository),
            (Some(Optional), Some(Optional), Optional, Repository),
            (Some(Optional), Some(Mandatory), Mandatory, Repository),
            (Some(Mandatory), None, Mandatory, User),
            (Some(Mandatory), Some(Disabled), Disabled, Repository),
            (Some(Mandatory), Some(Optional), Mandatory, User),
            (Some(Mandatory), Some(Mandatory), Mandatory, Repository),
        ];

        for (user_mode, repository_mode, expected_mode, decider) in cases {
            for degraded in [false, true] {
                let policy = resolve_access_policy(
                    &user(user_mode, None),
                    &repository(repository_mode, None),
                    &AccessPolicyLayer::default(),
                    degraded,
                );
                let case = format!(
                    "user={user_mode:?} repository={repository_mode:?} degraded={degraded}"
                );

                assert_eq!(policy.auth_mode.value, expected_mode, "{case}");
                assert_eq!(policy.auth_mode.source, decider_source(decider), "{case}");
                assert_eq!(policy.auth_allowed(), expected_mode != Disabled, "{case}");
                assert_eq!(
                    policy.background_cloud_allowed,
                    expected_mode != Disabled && !degraded,
                    "{case}"
                );
                assert_eq!(
                    policy.auto_sync_allowed(true),
                    policy.background_cloud_allowed,
                    "{case}"
                );
                assert!(!policy.auto_sync_allowed(false), "{case}");
            }
        }
    }

    #[test]
    fn access_policy_telemetry_requires_user_consent_and_honors_opt_outs() {
        use TelemetryDisabledReason::{
            AuthDisabled, InvalidConfig, NoUserConsent, NotEnabled, RepositoryOptOut, UserOptOut,
        };
        use TelemetryPolicyState::{Disabled, Enabled};

        let base_cases = [
            (None, None, Disabled(NotEnabled), false, Decider::Default),
            (
                None,
                Some(true),
                Disabled(NoUserConsent),
                true,
                Decider::Repository,
            ),
            (
                None,
                Some(false),
                Disabled(RepositoryOptOut),
                false,
                Decider::Repository,
            ),
            (Some(true), None, Enabled, true, Decider::User),
            (Some(true), Some(true), Enabled, true, Decider::Repository),
            (
                Some(true),
                Some(false),
                Disabled(RepositoryOptOut),
                false,
                Decider::Repository,
            ),
            (
                Some(false),
                None,
                Disabled(UserOptOut),
                false,
                Decider::User,
            ),
            (
                Some(false),
                Some(true),
                Disabled(UserOptOut),
                true,
                Decider::Repository,
            ),
            (
                Some(false),
                Some(false),
                Disabled(UserOptOut),
                false,
                Decider::Repository,
            ),
        ];

        for (user_telemetry, repository_telemetry, base_state, configured, decider) in base_cases {
            for degraded in [false, true] {
                for auth_disabled in [false, true] {
                    let user_mode = auth_disabled.then_some(AuthMode::Disabled);
                    let policy = resolve_access_policy(
                        &user(user_mode, user_telemetry),
                        &repository(None, repository_telemetry),
                        &AccessPolicyLayer::default(),
                        degraded,
                    );
                    let expected = if degraded {
                        Disabled(InvalidConfig)
                    } else if auth_disabled {
                        Disabled(AuthDisabled)
                    } else {
                        base_state
                    };
                    let case = format!(
                        "user={user_telemetry:?} repository={repository_telemetry:?} degraded={degraded} auth_disabled={auth_disabled}"
                    );

                    assert_eq!(policy.telemetry, expected, "{case}");
                    assert_eq!(policy.telemetry_enabled(), expected == Enabled, "{case}");
                    assert_eq!(policy.telemetry_enabled.value, configured, "{case}");
                    assert_eq!(
                        policy.telemetry_enabled.source,
                        decider_source(decider),
                        "{case}"
                    );
                }
            }
        }
    }

    #[test]
    fn access_policy_repository_auth_disabled_forces_telemetry_off() {
        let policy = resolve_access_policy(
            &user(None, Some(true)),
            &repository(Some(AuthMode::Disabled), None),
            &AccessPolicyLayer::default(),
            false,
        );

        assert_eq!(
            policy.telemetry,
            TelemetryPolicyState::Disabled(TelemetryDisabledReason::AuthDisabled)
        );
        assert!(!policy.background_cloud_allowed);
    }

    #[test]
    fn access_policy_warnings_are_deterministic_and_ordered() {
        let user_layer = user(Some(AuthMode::Disabled), None);
        let repository_layer = repository(Some(AuthMode::Mandatory), Some(true));
        let explicit_layer = AccessPolicyLayer::default();
        let policy = resolve_access_policy(&user_layer, &repository_layer, &explicit_layer, true);

        assert_eq!(
            build_access_policy_warnings(
                &user_layer,
                &repository_layer,
                &explicit_layer,
                &policy,
                true,
                true
            ),
            vec![
                WARNING_REPOSITORY_TELEMETRY_WITHOUT_USER_CONSENT.to_string(),
                WARNING_TELEMETRY_ENABLED_WITH_AUTH_DISABLED.to_string(),
                WARNING_AUTO_SYNC_WITH_AUTH_DISABLED.to_string(),
                WARNING_REPOSITORY_MANDATORY_OVERRIDDEN_BY_USER_DISABLED.to_string(),
                WARNING_INVALID_LAYER_DISABLES_CLOUD.to_string(),
            ]
        );

        let consenting_user = user(Some(AuthMode::Optional), Some(true));
        let consenting_policy =
            resolve_access_policy(&consenting_user, &repository_layer, &explicit_layer, false);
        assert!(build_access_policy_warnings(
            &consenting_user,
            &repository_layer,
            &explicit_layer,
            &consenting_policy,
            true,
            false,
        )
        .is_empty());

        let restricting_user = user(Some(AuthMode::Mandatory), Some(false));
        let silent_repository = repository(None, None);
        let permissive_explicit =
            layer(ConfigPathSource::Flag, Some(AuthMode::Optional), Some(true));
        let overridden_policy = resolve_access_policy(
            &restricting_user,
            &silent_repository,
            &permissive_explicit,
            false,
        );
        assert_eq!(
            build_access_policy_warnings(
                &restricting_user,
                &silent_repository,
                &permissive_explicit,
                &overridden_policy,
                false,
                false,
            ),
            vec![
                WARNING_EXPLICIT_AUTH_MODE_OVERRIDDEN.to_string(),
                WARNING_EXPLICIT_TELEMETRY_OVERRIDDEN.to_string(),
            ]
        );
    }

    fn discovered_paths_exist(path: &Path) -> bool {
        path == Path::new(GLOBAL_PATH) || path == Path::new(LOCAL_PATH)
    }

    fn resolve_discovered(global: &'static str, local: &'static str) -> RuntimeConfig {
        resolve_runtime_config_with(
            &ConfigRequest {
                report_format: ReportFormat::Text,
                config_path: None,
                log_level: None,
            },
            Path::new("/tmp/policy-repo"),
            |_| None,
            move |path| {
                if path == Path::new(GLOBAL_PATH) {
                    Ok(global.to_string())
                } else {
                    Ok(local.to_string())
                }
            },
            discovered_paths_exist,
            || Ok(PathBuf::from(GLOBAL_PATH)),
        )
        .unwrap()
    }

    fn all_policy_paths_exist(path: &Path) -> bool {
        [GLOBAL_PATH, LOCAL_PATH, EXPLICIT_PATH]
            .iter()
            .any(|candidate| path == Path::new(candidate))
    }

    fn resolve_with_explicit(
        source: ConfigPathSource,
        global: &'static str,
        repository: &'static str,
        explicit: &'static str,
    ) -> RuntimeConfig {
        let request = ConfigRequest {
            config_path: (source == ConfigPathSource::Flag).then(|| PathBuf::from(EXPLICIT_PATH)),
            report_format: ReportFormat::Text,
            log_level: None,
        };
        resolve_runtime_config_with(
            &request,
            Path::new("/tmp/policy-repo"),
            move |key| {
                (source == ConfigPathSource::Env && key == "SCE_CONFIG_FILE")
                    .then(|| EXPLICIT_PATH.to_string())
            },
            move |path| {
                Ok(if path == Path::new(GLOBAL_PATH) {
                    global
                } else if path == Path::new(LOCAL_PATH) {
                    repository
                } else {
                    explicit
                }
                .to_string())
            },
            all_policy_paths_exist,
            || Ok(PathBuf::from(GLOBAL_PATH)),
        )
        .unwrap()
    }

    #[test]
    fn explicit_config_cannot_bypass_global_or_repository_restrictions() {
        use TelemetryDisabledReason::{AuthDisabled, InvalidConfig, RepositoryOptOut, UserOptOut};
        use TelemetryPolicyState::{Disabled, Enabled};

        #[derive(Clone, Copy, Debug)]
        enum Deciding {
            Default,
            Global,
            Repository,
            Explicit,
        }

        const TELEMETRY_ON: &str = r#"{"telemetry":{"enabled":true}}"#;
        const TELEMETRY_OFF: &str = r#"{"telemetry":{"enabled":false}}"#;
        let cases = [
            (
                TELEMETRY_OFF,
                "{}",
                TELEMETRY_ON,
                AuthMode::Optional,
                Deciding::Default,
                Disabled(UserOptOut),
            ),
            (
                r#"{"auth":{"mode":"disabled"}}"#,
                "{}",
                r#"{"auth":{"mode":"optional"},"telemetry":{"enabled":true}}"#,
                AuthMode::Disabled,
                Deciding::Global,
                Disabled(AuthDisabled),
            ),
            (
                TELEMETRY_ON,
                TELEMETRY_OFF,
                TELEMETRY_ON,
                AuthMode::Optional,
                Deciding::Default,
                Disabled(RepositoryOptOut),
            ),
            (
                r#"{"auth":{"mode":"optional"}}"#,
                r#"{"auth":{"mode":"disabled"}}"#,
                r#"{"auth":{"mode":"mandatory"},"telemetry":{"enabled":true}}"#,
                AuthMode::Disabled,
                Deciding::Repository,
                Disabled(AuthDisabled),
            ),
            (
                TELEMETRY_OFF,
                TELEMETRY_ON,
                TELEMETRY_ON,
                AuthMode::Optional,
                Deciding::Default,
                Disabled(UserOptOut),
            ),
            (
                "{}",
                "{}",
                r#"{"auth":{"mode":"mandatory"},"telemetry":{"enabled":true}}"#,
                AuthMode::Mandatory,
                Deciding::Explicit,
                Enabled,
            ),
            (
                r#"{"telemetry":{"enabled":"no"}}"#,
                "{}",
                TELEMETRY_ON,
                AuthMode::Optional,
                Deciding::Default,
                Disabled(InvalidConfig),
            ),
        ];

        for (global, repository, explicit, auth_mode, deciding, telemetry) in cases {
            for source in [ConfigPathSource::Flag, ConfigPathSource::Env] {
                let runtime = resolve_with_explicit(source, global, repository, explicit);
                let policy = &runtime.access_policy;
                let case = format!(
                    "global={global} repository={repository} explicit={explicit} source={source:?}"
                );
                let expected_auth_source = match deciding {
                    Deciding::Default => ValueSource::Default,
                    Deciding::Global => {
                        ValueSource::ConfigFile(ConfigPathSource::DefaultDiscoveredGlobal)
                    }
                    Deciding::Repository => {
                        ValueSource::ConfigFile(ConfigPathSource::DefaultDiscoveredLocal)
                    }
                    Deciding::Explicit => ValueSource::ConfigFile(source),
                };
                let cloud_allowed =
                    auth_mode != AuthMode::Disabled && telemetry != Disabled(InvalidConfig);

                assert_eq!(policy.auth_mode.value, auth_mode, "{case}");
                assert_eq!(policy.auth_mode.source, expected_auth_source, "{case}");
                assert_eq!(policy.telemetry, telemetry, "{case}");
                assert_eq!(policy.background_cloud_allowed, cloud_allowed, "{case}");
            }
        }
    }

    #[test]
    fn explicit_config_keeps_replacing_discovered_ordinary_values() {
        for source in [ConfigPathSource::Flag, ConfigPathSource::Env] {
            let runtime = resolve_with_explicit(
                source,
                r#"{"log_level":"warn","agent_trace":{"auto_sync":true}}"#,
                r#"{"agent_trace":{"repository_remote":"upstream"}}"#,
                r#"{"log_level":"debug"}"#,
            );

            assert_eq!(runtime.log_level.value, LogLevel::Debug);
            assert_eq!(runtime.log_level.source, ValueSource::ConfigFile(source));
            assert_eq!(runtime.agent_trace_auto_sync.source, ValueSource::Default);
            assert_eq!(
                runtime.agent_trace_repository_remote.source,
                ValueSource::Default
            );
            assert_eq!(
                runtime
                    .loaded_config_paths
                    .iter()
                    .map(|loaded| loaded.source)
                    .collect::<Vec<_>>(),
                vec![
                    ConfigPathSource::DefaultDiscoveredGlobal,
                    ConfigPathSource::DefaultDiscoveredLocal,
                    source,
                ]
            );
        }
    }

    fn git(dir: &Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args([
                "-c",
                "user.name=SCE Test",
                "-c",
                "user.email=sce@example.invalid",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?} in {}", dir.display());
    }

    fn write_repository_config(dir: &Path, contents: &str) -> PathBuf {
        let path = RepoPaths::new(dir).sce_config_file();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn repository_policy_outside_git_is_discovered_from_the_working_directory_only() {
        let at_directory = resolve_discovered(
            r#"{"telemetry":{"enabled":true}}"#,
            r#"{"telemetry":{"enabled":false}}"#,
        );
        assert_eq!(
            at_directory.access_policy.telemetry,
            TelemetryPolicyState::Disabled(TelemetryDisabledReason::RepositoryOptOut)
        );

        let below_directory = resolve_runtime_config_with(
            &ConfigRequest {
                report_format: ReportFormat::Text,
                config_path: None,
                log_level: None,
            },
            Path::new("/tmp/policy-repo/src"),
            |_| None,
            |_| Ok(r#"{"telemetry":{"enabled":true}}"#.to_string()),
            discovered_paths_exist,
            || Ok(PathBuf::from(GLOBAL_PATH)),
        )
        .unwrap();
        assert_eq!(below_directory.loaded_config_paths.len(), 1);
        assert_eq!(
            below_directory.access_policy.telemetry,
            TelemetryPolicyState::Enabled
        );
    }

    #[test]
    fn repository_policy_is_discovered_from_the_enclosing_git_root() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let global = base.join("global-config.json");
        std::fs::write(
            &global,
            r#"{"auth":{"mode":"optional"},"telemetry":{"enabled":true}}"#,
        )
        .unwrap();

        let repo = base.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "--quiet"]);
        let repo_config = write_repository_config(&repo, r#"{"telemetry":{"enabled":false}}"#);
        git(&repo, &["add", ".sce/config.json"]);
        git(&repo, &["commit", "--quiet", "-m", "init"]);
        let worktree = base.join("worktree");
        git(
            &repo,
            &["worktree", "add", "--quiet", worktree.to_str().unwrap()],
        );
        let worktree_config = RepoPaths::new(&worktree).sce_config_file();

        let inner = repo.join("vendor").join("inner");
        std::fs::create_dir_all(&inner).unwrap();
        git(&inner, &["init", "--quiet"]);
        let inner_config = write_repository_config(&inner, r#"{"auth":{"mode":"disabled"}}"#);

        for nested in [&repo, &worktree, &inner] {
            std::fs::create_dir_all(nested.join("src").join("module")).unwrap();
        }

        let resolve = |cwd: &Path| {
            resolve_runtime_config_with(
                &ConfigRequest {
                    report_format: ReportFormat::Text,
                    config_path: None,
                    log_level: None,
                },
                cwd,
                |_| None,
                |path| Ok(std::fs::read_to_string(path)?),
                Path::exists,
                || Ok(global.clone()),
            )
            .unwrap()
        };
        let repository_opt_out =
            TelemetryPolicyState::Disabled(TelemetryDisabledReason::RepositoryOptOut);
        let auth_disabled = TelemetryPolicyState::Disabled(TelemetryDisabledReason::AuthDisabled);
        let cases = [
            (
                &repo,
                Some(&repo_config),
                AuthMode::Optional,
                repository_opt_out,
            ),
            (
                &worktree,
                Some(&worktree_config),
                AuthMode::Optional,
                repository_opt_out,
            ),
            (
                &inner,
                Some(&inner_config),
                AuthMode::Disabled,
                auth_disabled,
            ),
        ];

        for (root, config, auth_mode, telemetry) in cases {
            for cwd in [root.clone(), root.join("src").join("module")] {
                let runtime = resolve(&cwd);
                let case = cwd.display().to_string();
                assert_eq!(
                    runtime
                        .loaded_config_paths
                        .last()
                        .map(|loaded| &loaded.path),
                    config,
                    "{case}"
                );
                assert_eq!(runtime.access_policy.auth_mode.value, auth_mode, "{case}");
                assert_eq!(runtime.access_policy.telemetry, telemetry, "{case}");
            }
        }
    }

    #[test]
    fn access_policy_invalid_global_layer_is_skipped_and_degrades_policy() {
        let runtime = resolve_discovered(
            r#"{"auth":{"mode":"sometimes"}}"#,
            r#"{"telemetry":{"enabled":true}}"#,
        );

        assert_eq!(runtime.validation_errors.len(), 1);
        assert_eq!(
            runtime.access_policy.telemetry,
            TelemetryPolicyState::Disabled(TelemetryDisabledReason::InvalidConfig)
        );
        assert_eq!(runtime.access_policy.auth_mode.value, AuthMode::Optional);
        assert!(!runtime.access_policy.background_cloud_allowed);
        assert_eq!(
            runtime.validation_warnings,
            vec![
                WARNING_REPOSITORY_TELEMETRY_WITHOUT_USER_CONSENT.to_string(),
                WARNING_INVALID_LAYER_DISABLES_CLOUD.to_string(),
            ]
        );
    }

    #[test]
    fn access_policy_invalid_local_layer_is_skipped_and_degrades_policy() {
        let runtime = resolve_discovered(
            r#"{"auth":{"mode":"mandatory"},"telemetry":{"enabled":true}}"#,
            r#"{"telemetry":{"enabled":"yes"}}"#,
        );

        assert_eq!(runtime.validation_errors.len(), 1);
        assert_eq!(
            runtime.access_policy.telemetry,
            TelemetryPolicyState::Disabled(TelemetryDisabledReason::InvalidConfig)
        );
        assert_eq!(runtime.access_policy.auth_mode.value, AuthMode::Mandatory);
        assert_eq!(
            runtime.access_policy.auth_mode.source,
            ValueSource::ConfigFile(ConfigPathSource::DefaultDiscoveredGlobal)
        );
        assert!(!runtime.access_policy.background_cloud_allowed);
        assert_eq!(
            runtime.validation_warnings,
            vec![WARNING_INVALID_LAYER_DISABLES_CLOUD.to_string()]
        );
    }

    #[test]
    fn access_policy_merge_loop_routes_layers_and_appends_ordered_warnings() {
        let runtime = resolve_discovered(
            r#"{"auth":{"mode":"disabled"}}"#,
            r#"{"auth":{"mode":"mandatory"},"telemetry":{"enabled":true},"agent_trace":{"auto_sync":true}}"#,
        );

        assert!(runtime.validation_errors.is_empty());
        assert_eq!(runtime.access_policy.auth_mode.value, AuthMode::Disabled);
        assert_eq!(
            runtime.access_policy.auth_mode.source,
            ValueSource::ConfigFile(ConfigPathSource::DefaultDiscoveredGlobal)
        );
        assert_eq!(
            runtime.access_policy.telemetry,
            TelemetryPolicyState::Disabled(TelemetryDisabledReason::AuthDisabled)
        );
        assert_eq!(
            runtime.validation_warnings,
            vec![
                WARNING_REPOSITORY_TELEMETRY_WITHOUT_USER_CONSENT.to_string(),
                WARNING_TELEMETRY_ENABLED_WITH_AUTH_DISABLED.to_string(),
                WARNING_AUTO_SYNC_WITH_AUTH_DISABLED.to_string(),
                WARNING_REPOSITORY_MANDATORY_OVERRIDDEN_BY_USER_DISABLED.to_string(),
            ]
        );
    }

    #[test]
    fn access_policy_defaults_when_no_config_files_exist() {
        let runtime = resolve_runtime_config_with(
            &ConfigRequest {
                report_format: ReportFormat::Text,
                config_path: None,
                log_level: None,
            },
            Path::new("/tmp/policy-repo"),
            |_| None,
            |_| Ok("{}".to_string()),
            |_| false,
            || Ok(PathBuf::from(GLOBAL_PATH)),
        )
        .unwrap();

        assert_eq!(
            runtime.access_policy,
            ResolvedAccessPolicy {
                auth_mode: ResolvedValue {
                    value: AuthMode::Optional,
                    source: ValueSource::Default,
                },
                telemetry_enabled: ResolvedValue {
                    value: false,
                    source: ValueSource::Default,
                },
                telemetry: TelemetryPolicyState::Disabled(TelemetryDisabledReason::NotEnabled),
                background_cloud_allowed: true,
            }
        );
        assert!(runtime.validation_warnings.is_empty());
    }

    #[test]
    fn access_policy_is_exposed_on_projected_runtime_configs() {
        let expected = resolve_discovered(
            r#"{"telemetry":{"enabled":true}}"#,
            r#"{"auth":{"mode":"mandatory"}}"#,
        )
        .access_policy;
        let read = |path: &Path| {
            if path == Path::new(GLOBAL_PATH) {
                Ok(r#"{"telemetry":{"enabled":true}}"#.to_string())
            } else {
                Ok(r#"{"auth":{"mode":"mandatory"}}"#.to_string())
            }
        };
        let cwd = Path::new("/tmp/policy-repo");
        let global = || Ok(PathBuf::from(GLOBAL_PATH));

        let auth =
            resolve_auth_runtime_config_with(cwd, |_| None, read, discovered_paths_exist, global)
                .unwrap();
        let hook =
            resolve_hook_runtime_config_with(cwd, |_| None, read, discovered_paths_exist, global)
                .unwrap();
        let observability = resolve_observability_runtime_config_with(
            cwd,
            |_| None,
            read,
            discovered_paths_exist,
            global,
        )
        .unwrap();

        assert_eq!(expected.telemetry, TelemetryPolicyState::Enabled);
        assert_eq!(auth.access_policy, expected);
        assert_eq!(hook.access_policy, expected);
        assert_eq!(observability.access_policy, expected);
    }
}
