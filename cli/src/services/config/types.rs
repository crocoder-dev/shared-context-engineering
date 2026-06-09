//! Shared config types, enums, constants, and source metadata.
//!
//! This submodule owns the stable config primitives that are consumed across
//! multiple services. The parent `mod.rs` re-exports everything here through
//! `pub use types::*` so that existing `services::config::TypeName` imports
//! continue to work unchanged.

use std::path::PathBuf;

use clap::ValueEnum;

use crate::services::output_format::OutputFormat;

pub const NAME: &str = "config";

pub(crate) const ENV_LOG_LEVEL: &str = "SCE_LOG_LEVEL";
pub(crate) const ENV_LOG_FORMAT: &str = "SCE_LOG_FORMAT";
pub(crate) const ENV_LOG_FILE: &str = "SCE_LOG_FILE";
pub(crate) const ENV_LOG_FILE_MODE: &str = "SCE_LOG_FILE_MODE";
pub(crate) const ENV_ATTRIBUTION_HOOKS_ENABLED: &str = "SCE_ATTRIBUTION_HOOKS_ENABLED";

pub type ReportFormat = OutputFormat;

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
}

impl LogLevel {
    pub(crate) fn parse(raw: &str, source: &str) -> anyhow::Result<Self> {
        match raw {
            "error" => Ok(Self::Error),
            "warn" => Ok(Self::Warn),
            "info" => Ok(Self::Info),
            "debug" => Ok(Self::Debug),
            _ => anyhow::bail!(
                "Invalid log level '{raw}' from {source}. Valid values: error, warn, info, debug."
            ),
        }
    }

    pub(crate) fn parse_env(raw: &str, key: &str) -> anyhow::Result<Self> {
        match raw {
            "error" => Ok(Self::Error),
            "warn" => Ok(Self::Warn),
            "info" => Ok(Self::Info),
            "debug" => Ok(Self::Debug),
            _ => anyhow::bail!("Invalid {key} '{raw}'. Valid values: error, warn, info, debug."),
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warn => "warn",
            Self::Info => "info",
            Self::Debug => "debug",
        }
    }

    pub(crate) fn severity(self) -> u8 {
        match self {
            Self::Error => 1,
            Self::Warn => 2,
            Self::Info => 3,
            Self::Debug => 4,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LogFormat {
    Text,
    Json,
}

impl LogFormat {
    pub(crate) fn parse(raw: &str, source: &str) -> anyhow::Result<Self> {
        match raw {
            "text" => Ok(Self::Text),
            "json" => Ok(Self::Json),
            _ => {
                anyhow::bail!("Invalid log format '{raw}' from {source}. Valid values: text, json.")
            }
        }
    }

    pub(crate) fn parse_env(raw: &str, key: &str) -> anyhow::Result<Self> {
        match raw {
            "text" => Ok(Self::Text),
            "json" => Ok(Self::Json),
            _ => anyhow::bail!("Invalid {key} '{raw}'. Valid values: text, json."),
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Json => "json",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LogFileMode {
    Truncate,
    Append,
}

impl LogFileMode {
    pub(crate) fn parse(raw: &str, source: &str) -> anyhow::Result<Self> {
        match raw {
            "truncate" => Ok(Self::Truncate),
            "append" => Ok(Self::Append),
            _ => anyhow::bail!(
                "Invalid log file mode '{raw}' from {source}. Valid values: truncate, append."
            ),
        }
    }

    pub(crate) fn parse_env(raw: &str, key: &str) -> anyhow::Result<Self> {
        match raw {
            "truncate" => Ok(Self::Truncate),
            "append" => Ok(Self::Append),
            _ => anyhow::bail!("Invalid {key} '{raw}'. Valid values: truncate, append."),
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Truncate => "truncate",
            Self::Append => "append",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ValueSource {
    Flag,
    Env,
    ConfigFile(ConfigPathSource),
    Default,
}

impl ValueSource {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Flag => "flag",
            Self::Env => "env",
            Self::ConfigFile(_) => "config_file",
            Self::Default => "default",
        }
    }

    pub(crate) fn config_source(self) -> Option<ConfigPathSource> {
        match self {
            Self::ConfigFile(source) => Some(source),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ConfigPathSource {
    Flag,
    Env,
    DefaultDiscoveredGlobal,
    DefaultDiscoveredLocal,
}

impl ConfigPathSource {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Flag => "flag",
            Self::Env => "env",
            Self::DefaultDiscoveredGlobal => "default_discovered_global",
            Self::DefaultDiscoveredLocal => "default_discovered_local",
        }
    }

    pub(crate) const fn is_default_discovered(self) -> bool {
        matches!(
            self,
            Self::DefaultDiscoveredGlobal | Self::DefaultDiscoveredLocal
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConfigSubcommand {
    Show(ConfigRequest),
    Validate(ConfigRequest),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigRequest {
    pub report_format: ReportFormat,
    pub config_path: Option<PathBuf>,
    pub log_level: Option<LogLevel>,
    pub timeout_ms: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ResolvedValue<T> {
    pub(crate) value: T,
    pub(crate) source: ValueSource,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ResolvedOptionalValue<T> {
    pub(crate) value: Option<T>,
    pub(crate) source: Option<ValueSource>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LoadedConfigPath {
    pub(crate) path: PathBuf,
    pub(crate) source: ConfigPathSource,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ResolvedAuthRuntimeConfig {
    pub(crate) workos_client_id: ResolvedOptionalValue<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ResolvedObservabilityRuntimeConfig {
    pub(crate) log_level: LogLevel,
    pub(crate) log_format: LogFormat,
    pub(crate) log_file: Option<String>,
    pub(crate) log_file_mode: LogFileMode,
    pub(crate) loaded_config_paths: Vec<LoadedConfigPath>,
    pub(crate) validation_errors: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ResolvedHookRuntimeConfig {
    pub(crate) attribution_hooks_enabled: bool,
}

pub(crate) fn parse_bool_value_from(key: &str, raw: &str, source: &str) -> anyhow::Result<bool> {
    match raw {
        "1" | "true" => Ok(true),
        "0" | "false" => Ok(false),
        _ => anyhow::bail!("Invalid {key} '{raw}' from {source}. Valid values: true, false, 1, 0."),
    }
}
