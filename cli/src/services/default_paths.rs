use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PlatformFamily {
    Linux,
    Macos,
    Windows,
    Other,
}

#[allow(clippy::struct_field_names)]
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct SystemDirectories {
    pub home_dir: Option<PathBuf>,
    pub config_dir: Option<PathBuf>,
    pub state_dir: Option<PathBuf>,
    pub data_dir: Option<PathBuf>,
    pub data_local_dir: Option<PathBuf>,
    pub cache_dir: Option<PathBuf>,
}

impl SystemDirectories {
    fn from_current_system() -> Self {
        Self {
            home_dir: dirs::home_dir(),
            config_dir: dirs::config_dir(),
            state_dir: dirs::state_dir(),
            data_dir: dirs::data_dir(),
            data_local_dir: dirs::data_local_dir(),
            cache_dir: dirs::cache_dir(),
        }
    }
}

#[allow(clippy::struct_field_names)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SceDirectoryRoots {
    config_root: PathBuf,
    state_root: PathBuf,
    cache_root: PathBuf,
}

impl SceDirectoryRoots {
    pub(crate) fn config_root(&self) -> &Path {
        &self.config_root
    }

    pub(crate) fn state_root(&self) -> &Path {
        &self.state_root
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn cache_root(&self) -> &Path {
        &self.cache_root
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SceDefaultLocations {
    roots: SceDirectoryRoots,
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PersistedArtifactRootKind {
    Config,
    State,
    Cache,
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PersistedArtifactId {
    GlobalConfig,
    AuthTokens,
    AgentTraceLocalDb,
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PersistedArtifactLocation {
    pub id: PersistedArtifactId,
    pub root_kind: PersistedArtifactRootKind,
    pub path: PathBuf,
}

impl SceDefaultLocations {
    pub(crate) fn roots(&self) -> &SceDirectoryRoots {
        &self.roots
    }

    pub(crate) fn global_config_file(&self) -> PathBuf {
        self.roots.config_root().join("sce").join("config.json")
    }

    pub(crate) fn auth_tokens_file(&self) -> PathBuf {
        self.roots
            .state_root()
            .join("sce")
            .join("auth")
            .join("tokens.json")
    }

    pub(crate) fn agent_trace_local_db(&self) -> PathBuf {
        self.roots
            .state_root()
            .join("sce")
            .join("agent-trace")
            .join("local.db")
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn persisted_artifact_locations(&self) -> Vec<PersistedArtifactLocation> {
        vec![
            PersistedArtifactLocation {
                id: PersistedArtifactId::GlobalConfig,
                root_kind: PersistedArtifactRootKind::Config,
                path: self.global_config_file(),
            },
            PersistedArtifactLocation {
                id: PersistedArtifactId::AuthTokens,
                root_kind: PersistedArtifactRootKind::State,
                path: self.auth_tokens_file(),
            },
            PersistedArtifactLocation {
                id: PersistedArtifactId::AgentTraceLocalDb,
                root_kind: PersistedArtifactRootKind::State,
                path: self.agent_trace_local_db(),
            },
        ]
    }
}

pub(crate) fn resolve_sce_default_locations() -> Result<SceDefaultLocations> {
    resolve_sce_default_locations_for(
        current_platform_family(),
        &SystemDirectories::from_current_system(),
    )
}

pub(crate) fn resolve_sce_default_locations_for(
    platform: PlatformFamily,
    directories: &SystemDirectories,
) -> Result<SceDefaultLocations> {
    Ok(SceDefaultLocations {
        roots: SceDirectoryRoots {
            config_root: resolve_config_root(platform, directories)?,
            state_root: resolve_state_root(platform, directories)?,
            cache_root: resolve_cache_root(platform, directories)?,
        },
    })
}

fn resolve_config_root(
    platform: PlatformFamily,
    directories: &SystemDirectories,
) -> Result<PathBuf> {
    match platform {
        PlatformFamily::Linux => directories
            .config_dir
            .clone()
            .or_else(|| {
                directories
                    .home_dir
                    .as_ref()
                    .map(|home| home.join(".config"))
            })
            .ok_or_else(|| {
                anyhow!(
                    "Unable to resolve config directory: neither XDG_CONFIG_HOME nor HOME is set"
                )
            }),
        PlatformFamily::Macos => directories
            .config_dir
            .clone()
            .ok_or_else(|| anyhow!("Unable to resolve config directory for macOS")),
        PlatformFamily::Windows => directories
            .config_dir
            .clone()
            .or_else(|| directories.data_dir.clone())
            .ok_or_else(|| anyhow!("Unable to resolve config directory for Windows")),
        PlatformFamily::Other => directories
            .config_dir
            .clone()
            .or_else(|| {
                directories
                    .home_dir
                    .as_ref()
                    .map(|home| home.join(".config"))
            })
            .ok_or_else(|| anyhow!("Unable to resolve config directory")),
    }
}

fn resolve_state_root(
    platform: PlatformFamily,
    directories: &SystemDirectories,
) -> Result<PathBuf> {
    match platform {
        PlatformFamily::Linux => directories
            .state_dir
            .clone()
            .or_else(|| {
                directories
                    .home_dir
                    .as_ref()
                    .map(|home| home.join(".local").join("state"))
            })
            .ok_or_else(|| {
                anyhow!("Unable to resolve state directory: neither XDG_STATE_HOME nor HOME is set")
            }),
        PlatformFamily::Macos => directories
            .data_dir
            .clone()
            .ok_or_else(|| anyhow!("Unable to resolve data directory for macOS")),
        PlatformFamily::Windows => directories
            .data_local_dir
            .clone()
            .or_else(|| directories.data_dir.clone())
            .ok_or_else(|| anyhow!("Unable to resolve local data directory for Windows")),
        PlatformFamily::Other => directories
            .state_dir
            .clone()
            .or_else(|| directories.data_dir.clone())
            .or_else(|| {
                directories
                    .home_dir
                    .as_ref()
                    .map(|home| home.join(".local").join("state"))
            })
            .ok_or_else(|| anyhow!("Unable to resolve state or data directory")),
    }
}

fn resolve_cache_root(
    platform: PlatformFamily,
    directories: &SystemDirectories,
) -> Result<PathBuf> {
    match platform {
        PlatformFamily::Linux => directories
            .cache_dir
            .clone()
            .or_else(|| {
                directories
                    .home_dir
                    .as_ref()
                    .map(|home| home.join(".cache"))
            })
            .ok_or_else(|| {
                anyhow!("Unable to resolve cache directory: neither XDG_CACHE_HOME nor HOME is set")
            }),
        PlatformFamily::Macos => directories
            .cache_dir
            .clone()
            .ok_or_else(|| anyhow!("Unable to resolve cache directory for macOS")),
        PlatformFamily::Windows => directories
            .cache_dir
            .clone()
            .or_else(|| directories.data_local_dir.clone())
            .or_else(|| directories.data_dir.clone())
            .ok_or_else(|| anyhow!("Unable to resolve cache directory for Windows")),
        PlatformFamily::Other => directories
            .cache_dir
            .clone()
            .or_else(|| {
                directories
                    .home_dir
                    .as_ref()
                    .map(|home| home.join(".cache"))
            })
            .ok_or_else(|| anyhow!("Unable to resolve cache directory")),
    }
}

fn current_platform_family() -> PlatformFamily {
    #[cfg(target_os = "linux")]
    {
        PlatformFamily::Linux
    }

    #[cfg(target_os = "macos")]
    {
        PlatformFamily::Macos
    }

    #[cfg(target_os = "windows")]
    {
        PlatformFamily::Windows
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        PlatformFamily::Other
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use anyhow::Result;

    use super::{
        resolve_sce_default_locations_for, PersistedArtifactId, PersistedArtifactRootKind,
        PlatformFamily, SystemDirectories,
    };

    #[test]
    fn linux_prefers_xdg_directories_for_all_roots() -> Result<()> {
        let locations = resolve_sce_default_locations_for(
            PlatformFamily::Linux,
            &SystemDirectories {
                home_dir: Some(PathBuf::from("/home/alice")),
                config_dir: Some(PathBuf::from("/xdg/config")),
                state_dir: Some(PathBuf::from("/xdg/state")),
                cache_dir: Some(PathBuf::from("/xdg/cache")),
                ..SystemDirectories::default()
            },
        )?;

        assert_eq!(
            locations.global_config_file(),
            PathBuf::from("/xdg/config/sce/config.json")
        );
        assert_eq!(
            locations.auth_tokens_file(),
            PathBuf::from("/xdg/state/sce/auth/tokens.json")
        );
        assert_eq!(
            locations.agent_trace_local_db(),
            PathBuf::from("/xdg/state/sce/agent-trace/local.db")
        );
        assert_eq!(locations.roots().cache_root(), Path::new("/xdg/cache"));
        Ok(())
    }

    #[test]
    fn linux_falls_back_to_home_based_xdg_defaults_when_env_roots_are_missing() -> Result<()> {
        let locations = resolve_sce_default_locations_for(
            PlatformFamily::Linux,
            &SystemDirectories {
                home_dir: Some(PathBuf::from("/home/alice")),
                ..SystemDirectories::default()
            },
        )?;

        assert_eq!(
            locations.global_config_file(),
            PathBuf::from("/home/alice/.config/sce/config.json")
        );
        assert_eq!(
            locations.auth_tokens_file(),
            PathBuf::from("/home/alice/.local/state/sce/auth/tokens.json")
        );
        assert_eq!(
            locations.agent_trace_local_db(),
            PathBuf::from("/home/alice/.local/state/sce/agent-trace/local.db")
        );
        assert_eq!(
            locations.roots().cache_root(),
            Path::new("/home/alice/.cache")
        );
        Ok(())
    }

    #[test]
    fn persisted_artifact_inventory_lists_only_current_default_artifacts() -> Result<()> {
        let locations = resolve_sce_default_locations_for(
            PlatformFamily::Linux,
            &SystemDirectories {
                home_dir: Some(PathBuf::from("/home/alice")),
                config_dir: Some(PathBuf::from("/xdg/config")),
                state_dir: Some(PathBuf::from("/xdg/state")),
                cache_dir: Some(PathBuf::from("/xdg/cache")),
                ..SystemDirectories::default()
            },
        )?;

        assert_eq!(
            locations.persisted_artifact_locations(),
            vec![
                super::PersistedArtifactLocation {
                    id: PersistedArtifactId::GlobalConfig,
                    root_kind: PersistedArtifactRootKind::Config,
                    path: PathBuf::from("/xdg/config/sce/config.json"),
                },
                super::PersistedArtifactLocation {
                    id: PersistedArtifactId::AuthTokens,
                    root_kind: PersistedArtifactRootKind::State,
                    path: PathBuf::from("/xdg/state/sce/auth/tokens.json"),
                },
                super::PersistedArtifactLocation {
                    id: PersistedArtifactId::AgentTraceLocalDb,
                    root_kind: PersistedArtifactRootKind::State,
                    path: PathBuf::from("/xdg/state/sce/agent-trace/local.db"),
                },
            ]
        );
        assert!(locations
            .persisted_artifact_locations()
            .iter()
            .all(|artifact| artifact.root_kind != PersistedArtifactRootKind::Cache));
        Ok(())
    }

    #[test]
    fn linux_reports_missing_roots_without_legacy_fallbacks() {
        let error =
            resolve_sce_default_locations_for(PlatformFamily::Linux, &SystemDirectories::default())
                .expect_err("missing Linux roots should fail");

        assert_eq!(
            error.to_string(),
            "Unable to resolve config directory: neither XDG_CONFIG_HOME nor HOME is set"
        );
    }

    #[test]
    fn macos_uses_platform_config_data_and_cache_roots() -> Result<()> {
        let locations = resolve_sce_default_locations_for(
            PlatformFamily::Macos,
            &SystemDirectories {
                config_dir: Some(PathBuf::from("/Users/alice/Library/Application Support")),
                data_dir: Some(PathBuf::from("/Users/alice/Library/Application Support")),
                cache_dir: Some(PathBuf::from("/Users/alice/Library/Caches")),
                ..SystemDirectories::default()
            },
        )?;

        assert_eq!(
            locations.global_config_file(),
            PathBuf::from("/Users/alice/Library/Application Support/sce/config.json")
        );
        assert_eq!(
            locations.auth_tokens_file(),
            PathBuf::from("/Users/alice/Library/Application Support/sce/auth/tokens.json")
        );
        assert_eq!(
            locations.roots().cache_root(),
            Path::new("/Users/alice/Library/Caches")
        );
        Ok(())
    }

    #[test]
    fn windows_uses_local_data_for_stateful_artifacts() -> Result<()> {
        let locations = resolve_sce_default_locations_for(
            PlatformFamily::Windows,
            &SystemDirectories {
                config_dir: Some(PathBuf::from(r"C:\Users\alice\AppData\Roaming")),
                data_dir: Some(PathBuf::from(r"C:\Users\alice\AppData\Roaming")),
                data_local_dir: Some(PathBuf::from(r"C:\Users\alice\AppData\Local")),
                cache_dir: Some(PathBuf::from(r"C:\Users\alice\AppData\Local\cache")),
                ..SystemDirectories::default()
            },
        )?;

        assert_eq!(
            locations.global_config_file(),
            PathBuf::from(r"C:\Users\alice\AppData\Roaming/sce/config.json")
        );
        assert_eq!(
            locations.auth_tokens_file(),
            PathBuf::from(r"C:\Users\alice\AppData\Local/sce/auth/tokens.json")
        );
        assert_eq!(
            locations.agent_trace_local_db(),
            PathBuf::from(r"C:\Users\alice\AppData\Local/sce/agent-trace/local.db")
        );
        Ok(())
    }

    #[test]
    fn other_platform_uses_explicit_state_then_data_fallbacks() -> Result<()> {
        let locations = resolve_sce_default_locations_for(
            PlatformFamily::Other,
            &SystemDirectories {
                config_dir: Some(PathBuf::from("/var/config")),
                state_dir: Some(PathBuf::from("/var/state")),
                cache_dir: Some(PathBuf::from("/var/cache")),
                ..SystemDirectories::default()
            },
        )?;

        assert_eq!(
            locations.global_config_file(),
            PathBuf::from("/var/config/sce/config.json")
        );
        assert_eq!(
            locations.auth_tokens_file(),
            PathBuf::from("/var/state/sce/auth/tokens.json")
        );
        assert_eq!(locations.roots().cache_root(), Path::new("/var/cache"));
        Ok(())
    }
}
