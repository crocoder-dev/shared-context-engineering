//! Runtime config discovery, merge, and precedence resolution.
//!
//! This submodule owns config-file discovery, file-layer merging,
//! env/flag/default precedence, auth-key resolution, observability resolution,
//! and default-discovered invalid-file degradation.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};

use crate::services::default_paths::{
    observability_log_dir, resolve_sce_default_locations, RepoPaths,
};

use super::policy::{build_validation_warnings, resolve_bash_policy_config, BashPolicyConfig};
use super::schema;
use super::types::{
    parse_bool_value_from, ConfigPathSource, ConfigRequest, DatabaseRetryConfig, LoadedConfigPath,
    LogFormat, LogLevel, ReportFormat, ResolvedAgentTraceStorageRuntimeConfig,
    ResolvedAuthRuntimeConfig, ResolvedHookRuntimeConfig, ResolvedObservabilityRuntimeConfig,
    ResolvedOptionalValue, ResolvedValue, ValueSource, DEFAULT_LOG_FILE_RETENTION_LIMIT,
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
    pub(super) bash_policies: ResolvedOptionalValue<BashPolicyConfig>,
    pub(super) database_retry: ResolvedOptionalValue<DatabaseRetryConfig>,
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
) -> Result<ResolvedValue<bool>> {
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

    Ok(runtime.agent_trace_auto_sync)
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

    if !runtime.validation_errors.is_empty() {
        bail!(
            "Agent Trace storage config resolution failed because a discovered config file is invalid: {}",
            runtime.validation_errors.join(" | ")
        );
    }

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
fn resolve_runtime_config_with<FEnv, FRead, FGlobalPath>(
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
        bash_policy_presets: None,
        bash_policy_custom: None,
        database_retry: None,
        integrations: None,
    };
    let mut validation_errors = Vec::new();
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
            value: true,
            source: ValueSource::Default,
        },
    };

    let resolved_bash_policies = resolve_bash_policy_config(
        file_config.bash_policy_presets.as_ref(),
        file_config.bash_policy_custom.as_ref(),
    );
    let validation_warnings = build_validation_warnings(&resolved_bash_policies);

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
        bash_policies: resolved_bash_policies,
        database_retry: resolved_database_retry,
        validation_errors,
        validation_warnings,
    })
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
    if let Some(path) = request.config_path.as_ref() {
        if !path_exists(path) {
            bail!(
                "Config file '{}' was provided via --config but does not exist.",
                path.display()
            );
        }
        return Ok(vec![LoadedConfigPath {
            path: path.clone(),
            source: ConfigPathSource::Flag,
        }]);
    }

    if let Some(raw) = env_lookup("SCE_CONFIG_FILE") {
        let path = PathBuf::from(raw);
        if !path_exists(&path) {
            bail!(
                "Config file '{}' was provided via SCE_CONFIG_FILE but does not exist.",
                path.display()
            );
        }
        return Ok(vec![LoadedConfigPath {
            path,
            source: ConfigPathSource::Env,
        }]);
    }

    let mut discovered_paths = Vec::new();

    let global_path = resolve_global_config_path()?;
    if path_exists(&global_path) {
        discovered_paths.push(LoadedConfigPath {
            path: global_path,
            source: ConfigPathSource::DefaultDiscoveredGlobal,
        });
    }

    let local_path = RepoPaths::new(cwd).sce_config_file();
    if path_exists(&local_path) {
        discovered_paths.push(LoadedConfigPath {
            path: local_path,
            source: ConfigPathSource::DefaultDiscoveredLocal,
        });
    }

    Ok(discovered_paths)
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
    fn agent_trace_auto_sync_defaults_to_true() {
        let runtime = resolve_runtime_with_config(None).unwrap();

        assert!(runtime.agent_trace_auto_sync.value);
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
