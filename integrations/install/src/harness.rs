use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

use fs_extra::dir;
use tempfile::TempDir;

use crate::channels::Channel;
use crate::error::HarnessError;
use crate::platform::ensure_executable;

const SCE_BINARY_ENV: &str = "SCE_INSTALL_CHANNEL_SCE_BIN";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HarnessMode {
    SharedHarnessSmoke,
}

impl HarnessMode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::SharedHarnessSmoke => "shared-harness-smoke",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct HarnessRequest {
    channel: Channel,
    mode: HarnessMode,
}

impl HarnessRequest {
    pub(crate) fn new(channel: Channel, mode: HarnessMode) -> Self {
        Self { channel, mode }
    }

    pub(crate) fn channel(&self) -> Channel {
        self.channel
    }

    pub(crate) fn mode(&self) -> HarnessMode {
        self.mode
    }
}

pub(crate) struct ChannelHarness {
    channel: Channel,
    temp_dir: TempDir,
    workdir: PathBuf,
    original_path: OsString,
}

impl ChannelHarness {
    pub(crate) fn new(channel: Channel) -> Result<Self, HarnessError> {
        let temp_dir = tempfile::Builder::new()
            .prefix(&format!("sce-install-channel-{}.", channel.as_str()))
            .tempdir()
            .map_err(|e| HarnessError::TempDirCreate(e.to_string()))?;
        let root = temp_dir.path().to_path_buf();
        let workdir = root.join("workdir");

        let harness = Self {
            channel,
            temp_dir,
            workdir,
            original_path: env::var_os("PATH").unwrap_or_default(),
        };

        harness.create_layout()?;
        Ok(harness)
    }

    #[cfg(test)]
    pub(crate) fn root(&self) -> &Path {
        self.temp_dir.path()
    }

    pub(crate) fn setup_message(&self) -> String {
        format!(
            "[PASS] channel={} isolated harness ready: {}",
            self.channel.as_str(),
            self.temp_dir.path().display()
        )
    }

    pub(crate) fn resolve_sce_binary(&self) -> Result<PathBuf, HarnessError> {
        if let Some(binary) = env::var_os(SCE_BINARY_ENV) {
            return self.resolve_executable_in_paths(
                binary.as_os_str(),
                &self.path_with_harness_bins(),
                true,
            );
        }

        self.resolve_executable_in_paths(OsStr::new("sce"), &self.path_with_harness_bins(), true)
    }

    pub(crate) fn assert_sce_version_success(
        &self,
        binary_path: &Path,
    ) -> Result<String, HarnessError> {
        ensure_executable(binary_path).map_err(|e| match e {
            HarnessError::UnixOnly => HarnessError::ExecutableNotFound {
                channel: self.channel.as_str().to_string(),
                path: binary_path.to_path_buf(),
                reason: "not executable or not found".to_string(),
            },
            _ => e,
        })?;

        let output = self.run_command(binary_path, ["version"])?;

        if !output.status.success() {
            return Err(HarnessError::SceVersionFailed {
                channel: self.channel.as_str().to_string(),
                path: binary_path.to_path_buf(),
                stderr: if output.stderr.is_empty() {
                    None
                } else {
                    Some(output.stderr)
                },
            });
        }

        if !output.stderr.is_empty() {
            return Err(HarnessError::SceVersionStderr {
                channel: self.channel.as_str().to_string(),
                stderr: output.stderr,
            });
        }

        Ok(output.stdout)
    }

    pub(crate) fn version_success_message(&self, version_output: &str) -> String {
        format!(
            "[PASS] channel={} deterministic sce version assertion succeeded: {}",
            self.channel.as_str(),
            version_output
        )
    }

    pub(crate) fn assert_sce_doctor_success(&self, binary_path: &Path) -> Result<(), HarnessError> {
        let output = self.run_command(binary_path, ["doctor", "--format", "json"])?;

        if !output.status.success() {
            return Err(HarnessError::CommandExitedNonZero {
                channel: self.channel.as_str().to_string(),
                program: binary_path.display().to_string(),
                status: output.status.code().unwrap_or(-1),
                stdout: output.stdout,
                stderr: output.stderr,
            });
        }

        Ok(())
    }

    pub(crate) fn create_temp_subdir(&self, name: &str) -> Result<PathBuf, HarnessError> {
        let path = self.temp_dir.path().join(name);
        fs::create_dir_all(&path).map_err(|e| HarnessError::DirectoryCreate {
            path: path.clone(),
            error: e.to_string(),
        })?;
        Ok(path)
    }

    pub(crate) fn resolve_program(&self, program: &str) -> Result<PathBuf, HarnessError> {
        self.resolve_executable(OsStr::new(program))
    }

    pub(crate) fn resolve_program_in_harness_bins(
        &self,
        program: &str,
    ) -> Result<PathBuf, HarnessError> {
        self.resolve_executable_in_harness_bins(OsStr::new(program))
    }

    pub(crate) fn run_command<I, S>(
        &self,
        program: &Path,
        args: I,
    ) -> Result<CommandOutput, HarnessError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.run_command_in_dir_with_env(
            program,
            args,
            &self.workdir,
            std::iter::empty::<(&str, &str)>(),
        )
    }

    pub(crate) fn run_command_in_dir_with_env<I, S, K, V>(
        &self,
        program: &Path,
        args: I,
        current_dir: &Path,
        extra_env: impl IntoIterator<Item = (K, V)>,
    ) -> Result<CommandOutput, HarnessError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        let mut command = Command::new(program);
        command
            .args(args)
            .current_dir(current_dir)
            .env("SCE_INSTALL_CHANNEL", self.channel.as_str())
            .env("SCE_CHANNEL_HARNESS_ROOT", self.temp_dir.path())
            .env("SCE_CHANNEL_WORKDIR", &self.workdir)
            .env("HOME", self.home_dir())
            .env("XDG_CONFIG_HOME", self.xdg_config_home())
            .env("XDG_STATE_HOME", self.xdg_state_home())
            .env("XDG_CACHE_HOME", self.xdg_cache_home())
            .env("NPM_CONFIG_PREFIX", self.npm_prefix_dir())
            .env("NPM_CONFIG_CACHE", self.npm_cache_dir())
            .env("BUN_INSTALL", self.bun_install_dir())
            .env("CARGO_HOME", self.cargo_home_dir())
            .env("CARGO_TARGET_DIR", self.cargo_target_dir())
            .env("PATH", self.path_with_harness_bins());

        for (key, value) in extra_env {
            command.env(key, value);
        }

        let output = command.output().map_err(|e| HarnessError::CommandFailed {
            channel: self.channel.as_str().to_string(),
            program: program.display().to_string(),
            error: e.to_string(),
        })?;

        Ok(CommandOutput {
            status: output.status,
            stdout: normalize_output(&output.stdout),
            stderr: normalize_output(&output.stderr),
        })
    }

    fn create_layout(&self) -> Result<(), HarnessError> {
        for path in [
            self.workdir.clone(),
            self.home_dir(),
            self.xdg_config_home(),
            self.xdg_state_home(),
            self.xdg_cache_home(),
            self.npm_prefix_bin(),
            self.npm_cache_dir(),
            self.bun_install_bin(),
            self.cargo_home_bin(),
            self.cargo_target_dir(),
        ] {
            fs::create_dir_all(&path).map_err(|e| HarnessError::DirectoryCreate {
                path: path.clone(),
                error: e.to_string(),
            })?;
        }

        Ok(())
    }

    fn resolve_executable(&self, program: &OsStr) -> Result<PathBuf, HarnessError> {
        self.resolve_executable_in_paths(program, &self.path_with_harness_bins(), false)
    }

    fn resolve_executable_in_harness_bins(&self, program: &OsStr) -> Result<PathBuf, HarnessError> {
        self.resolve_executable_in_paths(program, &self.harness_bins_only_path(), false)
    }

    fn resolve_executable_in_paths(
        &self,
        program: &OsStr,
        paths: &OsStr,
        is_sce_binary: bool,
    ) -> Result<PathBuf, HarnessError> {
        let candidate = Path::new(program);
        if candidate.components().count() > 1 {
            return Ok(candidate.to_path_buf());
        }

        for path_entry in env::split_paths(paths) {
            let resolved = path_entry.join(candidate);
            if ensure_executable(&resolved).is_ok() {
                return Ok(resolved);
            }
        }

        if is_sce_binary {
            Err(HarnessError::SceBinaryResolve {
                program: candidate.display().to_string(),
                channel: self.channel.as_str().to_string(),
                env: SCE_BINARY_ENV.to_string(),
            })
        } else {
            Err(HarnessError::ExecutableResolve {
                program: candidate.display().to_string(),
                channel: self.channel.as_str().to_string(),
            })
        }
    }

    fn home_dir(&self) -> PathBuf {
        self.temp_dir.path().join("home")
    }

    fn xdg_config_home(&self) -> PathBuf {
        self.temp_dir.path().join("xdg/config")
    }

    fn xdg_state_home(&self) -> PathBuf {
        self.temp_dir.path().join("xdg/state")
    }

    fn xdg_cache_home(&self) -> PathBuf {
        self.temp_dir.path().join("xdg/cache")
    }

    fn npm_prefix_dir(&self) -> PathBuf {
        self.temp_dir.path().join("npm/prefix")
    }

    fn npm_prefix_bin(&self) -> PathBuf {
        self.npm_prefix_dir().join("bin")
    }

    fn npm_cache_dir(&self) -> PathBuf {
        self.temp_dir.path().join("npm/cache")
    }

    fn bun_install_dir(&self) -> PathBuf {
        self.temp_dir.path().join("bun")
    }

    fn bun_install_bin(&self) -> PathBuf {
        self.bun_install_dir().join("bin")
    }

    fn cargo_home_dir(&self) -> PathBuf {
        self.temp_dir.path().join("cargo/home")
    }

    fn cargo_home_bin(&self) -> PathBuf {
        self.cargo_home_dir().join("bin")
    }

    fn cargo_target_dir(&self) -> PathBuf {
        self.temp_dir.path().join("cargo/target")
    }

    fn path_with_harness_bins(&self) -> OsString {
        let mut paths = vec![
            self.npm_prefix_bin(),
            self.bun_install_bin(),
            self.cargo_home_bin(),
        ];
        paths.extend(env::split_paths(&self.original_path));
        env::join_paths(paths).expect("harness paths should be valid")
    }

    fn harness_bins_only_path(&self) -> OsString {
        let paths = vec![
            self.npm_prefix_bin(),
            self.bun_install_bin(),
            self.cargo_home_bin(),
        ];
        env::join_paths(paths).expect("harness paths should be valid")
    }
}

pub(crate) struct CommandOutput {
    pub(crate) status: ExitStatus,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
}

fn normalize_output(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .replace('\r', "")
        .trim_end_matches('\n')
        .to_string()
}

pub(crate) fn copy_directory_recursive(
    source: &Path,
    destination: &Path,
) -> Result<(), HarnessError> {
    let mut options = dir::CopyOptions::new();
    options.overwrite = true;
    options.copy_inside = true;
    options.content_only = true;

    dir::copy(source, destination, &options).map_err(|e| HarnessError::DirectoryCopy {
        src: source.to_path_buf(),
        dest: destination.to_path_buf(),
        error: e.to_string(),
    })?;

    Ok(())
}
