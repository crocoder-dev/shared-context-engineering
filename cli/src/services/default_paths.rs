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

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) mod repo_dir {
    pub const SCE: &str = ".sce";
    pub const OPENCODE: &str = ".opencode";
    pub const CLAUDE: &str = ".claude";
    pub const GIT: &str = ".git";
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) mod repo_file {
    pub const SCE_CONFIG: &str = "config.json";
    pub const SCE_LOG: &str = "sce.log";
    pub const OPENCODE_MANIFEST: &str = "opencode.json";
    pub const GIT_COMMIT_EDITMSG: &str = "COMMIT_EDITMSG";
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) mod hook_dir {
    pub const HOOKS: &str = "hooks";
    pub const PRE_COMMIT: &str = "pre-commit";
    pub const COMMIT_MSG: &str = "commit-msg";
    pub const POST_COMMIT: &str = "post-commit";
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) mod git_relative_path {
    pub const SCE_DIR: &str = "sce";
    pub const PRE_COMMIT_CHECKPOINT: &str = "sce/pre-commit-checkpoint.json";
    pub const PROMPTS: &str = "sce/prompts.jsonl";
    pub const TRACE_RETRY_QUEUE: &str = "sce/trace-retry-queue.jsonl";
    pub const TRACE_EMISSION_LEDGER: &str = "sce/trace-emission-ledger.txt";
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) mod embedded_asset_root {
    pub const GENERATED_CONFIG: &str = "assets/generated/config";
    pub const HOOKS: &str = "assets/hooks";
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) mod opencode_asset {
    pub const OPENCODE_DIR: &str = "opencode";
    pub const PLUGINS_DIR: &str = "plugins";
    pub const PLUGIN_FILE: &str = "sce-bash-policy.ts";
    pub const PLUGIN_MANIFEST_ENTRY: &str = "./plugins/sce-bash-policy.ts";
    pub const RUNTIME_DIR: &str = "plugins/bash-policy";
    pub const RUNTIME_FILE: &str = "runtime.ts";
    pub const LIB_DIR: &str = "lib";
    pub const PRESET_CATALOG: &str = "bash-policy-presets.json";
    pub const SKILLS_DIR: &str = "skills";
    pub const AGENTS_DIR: &str = "agents";
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) mod claude_asset {
    pub const CLAUDE_DIR: &str = "claude";
    pub const SKILLS_DIR: &str = "skills";
    pub const AGENTS_DIR: &str = "agents";
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) mod context_dir {
    pub const CONTEXT_ROOT: &str = "context";
    pub const PLANS: &str = "plans";
    pub const DECISIONS: &str = "decisions";
    pub const HANDOVERS: &str = "handovers";
    pub const TMP: &str = "tmp";
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) mod context_file {
    pub const OVERVIEW: &str = "overview.md";
    pub const ARCHITECTURE: &str = "architecture.md";
    pub const GLOSSARY: &str = "glossary.md";
    pub const PATTERNS: &str = "patterns.md";
    pub const CONTEXT_MAP: &str = "context-map.md";
    pub const SKILL_DEFINITION: &str = "SKILL.md";
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) mod schema {
    pub const SCHEMA_DIR: &str = "config/schema";
    pub const SCE_CONFIG_SCHEMA: &str = "sce-config.schema.json";
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RepoPaths {
    root: PathBuf,
}

#[cfg_attr(not(test), allow(dead_code))]
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

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EmbeddedAssetPaths {
    cli_root: PathBuf,
}

#[cfg_attr(not(test), allow(dead_code))]
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

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct InstallTargetPaths {
    repo_root: PathBuf,
}

#[cfg_attr(not(test), allow(dead_code))]
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

    pub(crate) fn hook_backup_path(&self, hook_name: &str) -> PathBuf {
        self.repo_root
            .join(repo_dir::GIT)
            .join(hook_dir::HOOKS)
            .join(format!("{hook_name}.backup"))
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
    use std::fs;
    use std::path::{Path, PathBuf};

    use anyhow::Result;

    use super::{
        context_dir, context_file, git_relative_path, hook_dir, opencode_asset, repo_dir,
        repo_file, resolve_sce_default_locations_for, EmbeddedAssetPaths, InstallTargetPaths,
        PersistedArtifactId, PersistedArtifactRootKind, PlatformFamily, RepoPaths,
        SystemDirectories,
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

    #[test]
    fn repo_paths_sce_directory_structure() {
        let repo = RepoPaths::new("/home/user/myrepo");

        assert_eq!(repo.sce_dir(), PathBuf::from("/home/user/myrepo/.sce"));
        assert_eq!(
            repo.sce_config_file(),
            PathBuf::from("/home/user/myrepo/.sce/config.json")
        );
        assert_eq!(
            repo.sce_log_file(),
            PathBuf::from("/home/user/myrepo/.sce/sce.log")
        );
    }

    #[test]
    fn repo_paths_opencode_directory_structure() {
        let repo = RepoPaths::new("/home/user/myrepo");

        assert_eq!(
            repo.opencode_dir(),
            PathBuf::from("/home/user/myrepo/.opencode")
        );
        assert_eq!(
            repo.opencode_manifest_file(),
            PathBuf::from("/home/user/myrepo/.opencode/opencode.json")
        );
    }

    #[test]
    fn repo_paths_claude_directory_structure() {
        let repo = RepoPaths::new("/home/user/myrepo");

        assert_eq!(
            repo.claude_dir(),
            PathBuf::from("/home/user/myrepo/.claude")
        );
    }

    #[test]
    fn repo_paths_git_directory_structure() {
        let repo = RepoPaths::new("/home/user/myrepo");

        assert_eq!(repo.git_dir(), PathBuf::from("/home/user/myrepo/.git"));
        assert_eq!(
            repo.git_hooks_dir(),
            PathBuf::from("/home/user/myrepo/.git/hooks")
        );
        assert_eq!(
            repo.git_hook_file("pre-commit"),
            PathBuf::from("/home/user/myrepo/.git/hooks/pre-commit")
        );
        assert_eq!(
            repo.git_commit_editmsg(),
            PathBuf::from("/home/user/myrepo/.git/COMMIT_EDITMSG")
        );
    }

    #[test]
    fn repo_paths_context_directory_structure() {
        let repo = RepoPaths::new("/home/user/myrepo");

        assert_eq!(
            repo.context_dir(),
            PathBuf::from("/home/user/myrepo/context")
        );
        assert_eq!(
            repo.context_plans_dir(),
            PathBuf::from("/home/user/myrepo/context/plans")
        );
        assert_eq!(
            repo.context_decisions_dir(),
            PathBuf::from("/home/user/myrepo/context/decisions")
        );
        assert_eq!(
            repo.context_handovers_dir(),
            PathBuf::from("/home/user/myrepo/context/handovers")
        );
        assert_eq!(
            repo.context_tmp_dir(),
            PathBuf::from("/home/user/myrepo/context/tmp")
        );
        assert_eq!(
            repo.context_overview_file(),
            PathBuf::from("/home/user/myrepo/context/overview.md")
        );
        assert_eq!(
            repo.context_architecture_file(),
            PathBuf::from("/home/user/myrepo/context/architecture.md")
        );
        assert_eq!(
            repo.context_glossary_file(),
            PathBuf::from("/home/user/myrepo/context/glossary.md")
        );
        assert_eq!(
            repo.context_patterns_file(),
            PathBuf::from("/home/user/myrepo/context/patterns.md")
        );
        assert_eq!(
            repo.context_map_file(),
            PathBuf::from("/home/user/myrepo/context/context-map.md")
        );
    }

    #[test]
    fn embedded_asset_paths_structure() {
        let assets = EmbeddedAssetPaths::new("/workspace/sce/cli");

        assert_eq!(
            assets.generated_config_root(),
            PathBuf::from("/workspace/sce/cli/assets/generated/config")
        );
        assert_eq!(
            assets.hooks_root(),
            PathBuf::from("/workspace/sce/cli/assets/hooks")
        );
        assert_eq!(
            assets.opencode_assets_dir(),
            PathBuf::from("/workspace/sce/cli/assets/generated/config/opencode")
        );
        assert_eq!(
            assets.opencode_plugins_dir(),
            PathBuf::from("/workspace/sce/cli/assets/generated/config/opencode/plugins")
        );
        assert_eq!(
            assets.opencode_plugin_file(),
            PathBuf::from(
                "/workspace/sce/cli/assets/generated/config/opencode/plugins/sce-bash-policy.ts"
            )
        );
        assert_eq!(
            assets.opencode_runtime_dir(),
            PathBuf::from(
                "/workspace/sce/cli/assets/generated/config/opencode/plugins/bash-policy"
            )
        );
        assert_eq!(
            assets.opencode_runtime_file(),
            PathBuf::from("/workspace/sce/cli/assets/generated/config/opencode/plugins/bash-policy/runtime.ts")
        );
        assert_eq!(
            assets.opencode_lib_dir(),
            PathBuf::from("/workspace/sce/cli/assets/generated/config/opencode/lib")
        );
        assert_eq!(
            assets.opencode_preset_catalog(),
            PathBuf::from(
                "/workspace/sce/cli/assets/generated/config/opencode/lib/bash-policy-presets.json"
            )
        );
        assert_eq!(
            assets.opencode_skills_dir(),
            PathBuf::from("/workspace/sce/cli/assets/generated/config/opencode/skills")
        );
        assert_eq!(
            assets.opencode_agents_dir(),
            PathBuf::from("/workspace/sce/cli/assets/generated/config/opencode/agents")
        );
        assert_eq!(
            assets.claude_assets_dir(),
            PathBuf::from("/workspace/sce/cli/assets/generated/config/claude")
        );
        assert_eq!(
            assets.claude_skills_dir(),
            PathBuf::from("/workspace/sce/cli/assets/generated/config/claude/skills")
        );
        assert_eq!(
            assets.claude_agents_dir(),
            PathBuf::from("/workspace/sce/cli/assets/generated/config/claude/agents")
        );
        assert_eq!(
            assets.config_schema_dir(),
            PathBuf::from("/workspace/sce/cli/config/schema")
        );
        assert_eq!(
            assets.sce_config_schema_file(),
            PathBuf::from("/workspace/sce/cli/config/schema/sce-config.schema.json")
        );
    }

    #[test]
    fn install_target_paths_structure() {
        let targets = InstallTargetPaths::new("/home/user/myrepo");

        assert_eq!(
            targets.opencode_target_dir(),
            PathBuf::from("/home/user/myrepo/.opencode")
        );
        assert_eq!(
            targets.claude_target_dir(),
            PathBuf::from("/home/user/myrepo/.claude")
        );
        assert_eq!(
            targets.opencode_plugin_target(),
            PathBuf::from("/home/user/myrepo/.opencode/plugins/sce-bash-policy.ts")
        );
        assert_eq!(
            targets.opencode_runtime_target(),
            PathBuf::from("/home/user/myrepo/.opencode/plugins/bash-policy/runtime.ts")
        );
        assert_eq!(
            targets.opencode_preset_catalog_target(),
            PathBuf::from("/home/user/myrepo/.opencode/lib/bash-policy-presets.json")
        );
        assert_eq!(
            targets.pre_commit_hook_path(),
            PathBuf::from("/home/user/myrepo/.git/hooks/pre-commit")
        );
        assert_eq!(
            targets.commit_msg_hook_path(),
            PathBuf::from("/home/user/myrepo/.git/hooks/commit-msg")
        );
        assert_eq!(
            targets.post_commit_hook_path(),
            PathBuf::from("/home/user/myrepo/.git/hooks/post-commit")
        );
        assert_eq!(
            targets.hook_backup_path("pre-commit"),
            PathBuf::from("/home/user/myrepo/.git/hooks/pre-commit.backup")
        );
    }

    #[test]
    fn skill_tile_relative_path_format() {
        assert_eq!(
            InstallTargetPaths::skill_tile_relative_path("sce-plan-review"),
            "skills/sce-plan-review/SKILL.md"
        );
        assert_eq!(
            InstallTargetPaths::skill_tile_relative_path("my-skill"),
            "skills/my-skill/SKILL.md"
        );
    }

    #[test]
    fn path_constants_are_stable() {
        // These constants must not change without explicit versioning
        assert_eq!(repo_dir::SCE, ".sce");
        assert_eq!(repo_dir::OPENCODE, ".opencode");
        assert_eq!(repo_dir::CLAUDE, ".claude");
        assert_eq!(repo_dir::GIT, ".git");

        assert_eq!(repo_file::SCE_CONFIG, "config.json");
        assert_eq!(repo_file::SCE_LOG, "sce.log");
        assert_eq!(repo_file::OPENCODE_MANIFEST, "opencode.json");
        assert_eq!(repo_file::GIT_COMMIT_EDITMSG, "COMMIT_EDITMSG");
        assert_eq!(
            opencode_asset::PLUGIN_MANIFEST_ENTRY,
            "./plugins/sce-bash-policy.ts"
        );

        assert_eq!(hook_dir::HOOKS, "hooks");
        assert_eq!(hook_dir::PRE_COMMIT, "pre-commit");
        assert_eq!(hook_dir::COMMIT_MSG, "commit-msg");
        assert_eq!(hook_dir::POST_COMMIT, "post-commit");

        assert_eq!(git_relative_path::SCE_DIR, "sce");
        assert_eq!(
            git_relative_path::PRE_COMMIT_CHECKPOINT,
            "sce/pre-commit-checkpoint.json"
        );
        assert_eq!(git_relative_path::PROMPTS, "sce/prompts.jsonl");
        assert_eq!(
            git_relative_path::TRACE_RETRY_QUEUE,
            "sce/trace-retry-queue.jsonl"
        );
        assert_eq!(
            git_relative_path::TRACE_EMISSION_LEDGER,
            "sce/trace-emission-ledger.txt"
        );

        assert_eq!(context_dir::CONTEXT_ROOT, "context");
        assert_eq!(context_dir::PLANS, "plans");
        assert_eq!(context_dir::DECISIONS, "decisions");
        assert_eq!(context_dir::HANDOVERS, "handovers");

        assert_eq!(context_file::OVERVIEW, "overview.md");
        assert_eq!(context_file::ARCHITECTURE, "architecture.md");
        assert_eq!(context_file::GLOSSARY, "glossary.md");
        assert_eq!(context_file::PATTERNS, "patterns.md");
        assert_eq!(context_file::CONTEXT_MAP, "context-map.md");
        assert_eq!(context_file::SKILL_DEFINITION, "SKILL.md");
    }

    #[test]
    fn production_cli_paths_are_centralized_in_default_paths() {
        let src_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut source_files = Vec::new();
        collect_rust_source_files(&src_root, &mut source_files);
        source_files.sort();

        let forbidden_literals = [
            ".sce/",
            ".opencode",
            ".claude",
            ".git/",
            "config/schema/",
            "plugins/sce-bash-policy.ts",
            "plugins/bash-policy/runtime.ts",
            "lib/bash-policy-presets.json",
            "./plugins/sce-bash-policy.ts",
            "context/",
            "sce/pre-commit-checkpoint.json",
            "sce/prompts.jsonl",
            "sce/trace-retry-queue.jsonl",
            "sce/trace-emission-ledger.txt",
        ];

        let mut violations = Vec::new();

        for file in source_files {
            if file.ends_with(Path::new("services/default_paths.rs")) {
                continue;
            }

            let Ok(contents) = fs::read_to_string(&file) else {
                violations.push(format!("{}: unreadable source file", file.display()));
                continue;
            };

            for (line_index, line) in production_source_prefix(&contents).lines().enumerate() {
                if line.contains("include_str!(") {
                    continue;
                }

                if let Some(literal) = forbidden_literals
                    .iter()
                    .find(|literal| line.contains(**literal))
                {
                    violations.push(format!(
                        "{}:{} contains hardcoded production path literal '{}': {}",
                        file.display(),
                        line_index + 1,
                        literal,
                        line.trim()
                    ));
                }
            }
        }

        assert!(
            violations.is_empty(),
            "production path literal regression(s):\n{}",
            violations.join("\n")
        );
    }

    fn collect_rust_source_files(directory: &Path, files: &mut Vec<PathBuf>) {
        let entries = fs::read_dir(directory).expect("source directory should be readable");

        for entry in entries {
            let entry = entry.expect("directory entry should be readable");
            let path = entry.path();

            if path.is_dir() {
                collect_rust_source_files(&path, files);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                files.push(path);
            }
        }
    }

    fn production_source_prefix(contents: &str) -> &str {
        contents
            .split("#[cfg(test)]\nmod tests {")
            .next()
            .unwrap_or(contents)
    }
}
