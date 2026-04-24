use std::path::PathBuf;

pub(crate) use roots::{resolve_sce_default_locations, resolve_state_data_root};

mod roots {
    use std::path::{Path, PathBuf};

    use anyhow::{anyhow, Result};

    #[allow(dead_code)]
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

        #[allow(dead_code)]
        pub(crate) fn cache_root(&self) -> &Path {
            &self.cache_root
        }
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub(crate) struct SceDefaultLocations {
        roots: SceDirectoryRoots,
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

        #[allow(dead_code)]
        pub(crate) fn persisted_artifact_locations(&self) -> Vec<super::PersistedArtifactLocation> {
            vec![
                super::PersistedArtifactLocation {
                    id: super::PersistedArtifactId::GlobalConfig,
                    root_kind: super::PersistedArtifactRootKind::Config,
                    path: self.global_config_file(),
                },
                super::PersistedArtifactLocation {
                    id: super::PersistedArtifactId::AuthTokens,
                    root_kind: super::PersistedArtifactRootKind::State,
                    path: self.auth_tokens_file(),
                },
            ]
        }
    }

    pub(crate) fn resolve_state_data_root() -> Result<PathBuf> {
        Ok(resolve_sce_default_locations()?
            .roots()
            .state_root()
            .to_path_buf())
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
                .or_else(|| directories.home_dir.as_ref().map(|home| home.join(".config")))
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
                .or_else(|| directories.home_dir.as_ref().map(|home| home.join(".config")))
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
                    anyhow!(
                        "Unable to resolve state directory: neither XDG_STATE_HOME nor HOME is set"
                    )
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
                    anyhow!(
                        "Unable to resolve cache directory: neither XDG_CACHE_HOME nor HOME is set"
                    )
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
}

/// Returns the canonical path to the local Turso database file.
///
/// The path is `<state_root>/sce/local.db`, where `state_root` comes
/// from the shared default-path catalog (`XDG_STATE_HOME` or platform
/// equivalent).
pub fn local_db_path() -> anyhow::Result<PathBuf> {
    Ok(resolve_sce_default_locations()?
        .roots()
        .state_root()
        .join("sce")
        .join("local.db"))
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PersistedArtifactRootKind {
    Config,
    State,
    Cache,
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PersistedArtifactId {
    GlobalConfig,
    AuthTokens,
}

#[allow(dead_code)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PersistedArtifactLocation {
    pub id: PersistedArtifactId,
    pub root_kind: PersistedArtifactRootKind,
    pub path: PathBuf,
}

#[allow(dead_code)]
pub(crate) mod repo_dir {
    pub const SCE: &str = ".sce";
    pub const OPENCODE: &str = ".opencode";
    pub const CLAUDE: &str = ".claude";
    pub const GIT: &str = ".git";
}

#[allow(dead_code)]
pub(crate) mod repo_file {
    pub const SCE_CONFIG: &str = "config.json";
    pub const SCE_LOG: &str = "sce.log";
    pub const OPENCODE_MANIFEST: &str = "opencode.json";
    pub const GIT_COMMIT_EDITMSG: &str = "COMMIT_EDITMSG";
}

#[allow(dead_code)]
pub(crate) mod hook_dir {
    pub const HOOKS: &str = "hooks";
    pub const PRE_COMMIT: &str = "pre-commit";
    pub const COMMIT_MSG: &str = "commit-msg";
    pub const POST_COMMIT: &str = "post-commit";
}

#[allow(dead_code)]
pub(crate) mod embedded_asset_root {
    pub const GENERATED_CONFIG: &str = "assets/generated/config";
    pub const HOOKS: &str = "assets/hooks";
}

#[allow(dead_code)]
pub(crate) mod opencode_asset {
    pub const OPENCODE_DIR: &str = "opencode";
    pub const PLUGINS_DIR: &str = "plugins";
    pub const PLUGIN_FILE: &str = "sce-bash-policy.ts";
    pub const PLUGIN_MANIFEST_ENTRY: &str = "./plugins/sce-bash-policy.ts";
    pub const RUNTIME_DIR: &str = "plugins/bash-policy";
    pub const RUNTIME_FILE: &str = "runtime.ts";
    pub const LIB_DIR: &str = "lib";
    pub const PRESET_CATALOG: &str = "bash-policy-presets.json";
    pub const OPENCODE_AGENT_DIR: &str = "agent";
    pub const OPENCODE_COMMAND_DIR: &str = "command";
    pub const SKILLS_DIR: &str = "skills";
    pub const AGENTS_DIR: &str = "agents";
}

#[allow(dead_code)]
pub(crate) mod claude_asset {
    pub const CLAUDE_DIR: &str = "claude";
    pub const SKILLS_DIR: &str = "skills";
    pub const AGENTS_DIR: &str = "agents";
}

#[allow(dead_code)]
pub(crate) mod context_dir {
    pub const CONTEXT_ROOT: &str = "context";
    pub const PLANS: &str = "plans";
    pub const DECISIONS: &str = "decisions";
    pub const HANDOVERS: &str = "handovers";
    pub const TMP: &str = "tmp";
}

#[allow(dead_code)]
pub(crate) mod context_file {
    pub const OVERVIEW: &str = "overview.md";
    pub const ARCHITECTURE: &str = "architecture.md";
    pub const GLOSSARY: &str = "glossary.md";
    pub const PATTERNS: &str = "patterns.md";
    pub const CONTEXT_MAP: &str = "context-map.md";
    pub const SKILL_DEFINITION: &str = "SKILL.md";
}

#[allow(dead_code)]
pub(crate) mod schema {
    pub const SCHEMA_DIR: &str = "config/schema";
    pub const SCE_CONFIG_SCHEMA: &str = "sce-config.schema.json";
}

#[allow(dead_code)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RepoPaths {
    root: PathBuf,
}

#[allow(dead_code)]
impl RepoPaths {
    pub(crate) fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub(crate) fn sce_dir(&self) -> PathBuf {
        self.root.join(repo_dir::SCE)
    }

    pub(crate) fn sce_config_file(&self) -> PathBuf {
        self.sce_dir().join(repo_file::SCE_CONFIG)
    }

    pub(crate) fn sce_log_file(&self) -> PathBuf {
        self.sce_dir().join(repo_file::SCE_LOG)
    }

    pub(crate) fn opencode_dir(&self) -> PathBuf {
        self.root.join(repo_dir::OPENCODE)
    }

    pub(crate) fn opencode_manifest_file(&self) -> PathBuf {
        self.opencode_dir().join(repo_file::OPENCODE_MANIFEST)
    }

    pub(crate) fn claude_dir(&self) -> PathBuf {
        self.root.join(repo_dir::CLAUDE)
    }

    pub(crate) fn git_dir(&self) -> PathBuf {
        self.root.join(repo_dir::GIT)
    }

    pub(crate) fn git_hooks_dir(&self) -> PathBuf {
        self.git_dir().join(hook_dir::HOOKS)
    }

    pub(crate) fn git_hook_file(&self, hook_name: &str) -> PathBuf {
        self.git_hooks_dir().join(hook_name)
    }

    pub(crate) fn git_commit_editmsg(&self) -> PathBuf {
        self.git_dir().join(repo_file::GIT_COMMIT_EDITMSG)
    }

    pub(crate) fn context_dir(&self) -> PathBuf {
        self.root.join(context_dir::CONTEXT_ROOT)
    }

    pub(crate) fn context_plans_dir(&self) -> PathBuf {
        self.context_dir().join(context_dir::PLANS)
    }

    pub(crate) fn context_decisions_dir(&self) -> PathBuf {
        self.context_dir().join(context_dir::DECISIONS)
    }

    pub(crate) fn context_handovers_dir(&self) -> PathBuf {
        self.context_dir().join(context_dir::HANDOVERS)
    }

    pub(crate) fn context_tmp_dir(&self) -> PathBuf {
        self.context_dir().join(context_dir::TMP)
    }

    pub(crate) fn context_overview_file(&self) -> PathBuf {
        self.context_dir().join(context_file::OVERVIEW)
    }

    pub(crate) fn context_architecture_file(&self) -> PathBuf {
        self.context_dir().join(context_file::ARCHITECTURE)
    }

    pub(crate) fn context_glossary_file(&self) -> PathBuf {
        self.context_dir().join(context_file::GLOSSARY)
    }

    pub(crate) fn context_patterns_file(&self) -> PathBuf {
        self.context_dir().join(context_file::PATTERNS)
    }

    pub(crate) fn context_map_file(&self) -> PathBuf {
        self.context_dir().join(context_file::CONTEXT_MAP)
    }
}

#[allow(dead_code)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EmbeddedAssetPaths {
    cli_root: PathBuf,
}

#[allow(dead_code)]
impl EmbeddedAssetPaths {
    pub(crate) fn new(cli_root: impl Into<PathBuf>) -> Self {
        Self {
            cli_root: cli_root.into(),
        }
    }

    pub(crate) fn generated_config_root(&self) -> PathBuf {
        self.cli_root.join(embedded_asset_root::GENERATED_CONFIG)
    }

    pub(crate) fn hooks_root(&self) -> PathBuf {
        self.cli_root.join(embedded_asset_root::HOOKS)
    }

    pub(crate) fn opencode_assets_dir(&self) -> PathBuf {
        self.generated_config_root()
            .join(opencode_asset::OPENCODE_DIR)
    }

    pub(crate) fn opencode_plugins_dir(&self) -> PathBuf {
        self.opencode_assets_dir().join(opencode_asset::PLUGINS_DIR)
    }

    pub(crate) fn opencode_plugin_file(&self) -> PathBuf {
        self.opencode_plugins_dir()
            .join(opencode_asset::PLUGIN_FILE)
    }

    pub(crate) fn opencode_runtime_dir(&self) -> PathBuf {
        self.opencode_assets_dir().join(opencode_asset::RUNTIME_DIR)
    }

    pub(crate) fn opencode_runtime_file(&self) -> PathBuf {
        self.opencode_runtime_dir()
            .join(opencode_asset::RUNTIME_FILE)
    }

    pub(crate) fn opencode_lib_dir(&self) -> PathBuf {
        self.opencode_assets_dir().join(opencode_asset::LIB_DIR)
    }

    pub(crate) fn opencode_preset_catalog(&self) -> PathBuf {
        self.opencode_lib_dir().join(opencode_asset::PRESET_CATALOG)
    }

    pub(crate) fn opencode_skills_dir(&self) -> PathBuf {
        self.opencode_assets_dir().join(opencode_asset::SKILLS_DIR)
    }

    pub(crate) fn opencode_agents_dir(&self) -> PathBuf {
        self.opencode_assets_dir().join(opencode_asset::AGENTS_DIR)
    }

    pub(crate) fn claude_assets_dir(&self) -> PathBuf {
        self.generated_config_root().join(claude_asset::CLAUDE_DIR)
    }

    pub(crate) fn claude_skills_dir(&self) -> PathBuf {
        self.claude_assets_dir().join(claude_asset::SKILLS_DIR)
    }

    pub(crate) fn claude_agents_dir(&self) -> PathBuf {
        self.claude_assets_dir().join(claude_asset::AGENTS_DIR)
    }

    pub(crate) fn config_schema_dir(&self) -> PathBuf {
        self.cli_root.join(schema::SCHEMA_DIR)
    }

    pub(crate) fn sce_config_schema_file(&self) -> PathBuf {
        self.config_schema_dir().join(schema::SCE_CONFIG_SCHEMA)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct InstallTargetPaths {
    repo_root: PathBuf,
}

#[allow(dead_code)]
impl InstallTargetPaths {
    pub(crate) fn new(repo_root: impl Into<PathBuf>) -> Self {
        Self {
            repo_root: repo_root.into(),
        }
    }

    pub(crate) fn opencode_target_dir(&self) -> PathBuf {
        self.repo_root.join(repo_dir::OPENCODE)
    }

    pub(crate) fn claude_target_dir(&self) -> PathBuf {
        self.repo_root.join(repo_dir::CLAUDE)
    }

    pub(crate) fn opencode_plugin_target(&self) -> PathBuf {
        self.opencode_target_dir()
            .join(opencode_asset::PLUGINS_DIR)
            .join(opencode_asset::PLUGIN_FILE)
    }

    pub(crate) fn opencode_runtime_target(&self) -> PathBuf {
        self.opencode_target_dir()
            .join(opencode_asset::RUNTIME_DIR)
            .join(opencode_asset::RUNTIME_FILE)
    }

    pub(crate) fn opencode_preset_catalog_target(&self) -> PathBuf {
        self.opencode_target_dir()
            .join(opencode_asset::LIB_DIR)
            .join(opencode_asset::PRESET_CATALOG)
    }

    pub(crate) fn skill_tile_relative_path(skill_name: &str) -> String {
        format!(
            "{}/{}/{}",
            opencode_asset::SKILLS_DIR,
            skill_name,
            context_file::SKILL_DEFINITION
        )
    }

    pub(crate) fn pre_commit_hook_path(&self) -> PathBuf {
        self.repo_root
            .join(repo_dir::GIT)
            .join(hook_dir::HOOKS)
            .join(hook_dir::PRE_COMMIT)
    }

    pub(crate) fn commit_msg_hook_path(&self) -> PathBuf {
        self.repo_root
            .join(repo_dir::GIT)
            .join(hook_dir::HOOKS)
            .join(hook_dir::COMMIT_MSG)
    }

    pub(crate) fn post_commit_hook_path(&self) -> PathBuf {
        self.repo_root
            .join(repo_dir::GIT)
            .join(hook_dir::HOOKS)
            .join(hook_dir::POST_COMMIT)
    }
}
