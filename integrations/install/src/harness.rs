use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::process::ExitStatus;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::channels::Channel;

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

    pub(crate) fn channel(self) -> Channel {
        self.channel
    }

    pub(crate) fn mode(self) -> HarnessMode {
        self.mode
    }
}

pub(crate) struct ChannelHarness {
    channel: Channel,
    root: PathBuf,
    workdir: PathBuf,
    original_path: OsString,
}

impl ChannelHarness {
    pub(crate) fn new(channel: Channel) -> Result<Self, String> {
        let root = create_harness_root(channel)?;
        let workdir = root.join("workdir");

        let harness = Self {
            channel,
            root,
            workdir,
            original_path: env::var_os("PATH").unwrap_or_default(),
        };

        harness.create_layout()?;
        Ok(harness)
    }

    #[cfg(test)]
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn setup_message(&self) -> String {
        format!(
            "[PASS] channel={} isolated harness ready: {}",
            self.channel.as_str(),
            self.root.display()
        )
    }

    pub(crate) fn resolve_sce_binary(&self) -> Result<PathBuf, String> {
        if let Some(binary) = env::var_os(SCE_BINARY_ENV) {
            return self.resolve_executable(binary.as_os_str());
        }

        self.resolve_executable(OsStr::new("sce"))
    }

    pub(crate) fn assert_sce_version_success(&self, binary_path: &Path) -> Result<String, String> {
        ensure_executable(binary_path).map_err(|reason| {
            format!(
                "[FAIL] channel={} expected executable not found: {} ({reason})",
                self.channel.as_str(),
                binary_path.display()
            )
        })?;

        let output = self.run_command(binary_path, ["version"])?;

        if !output.status.success() {
            let mut message = format!(
                "[FAIL] channel={} sce version failed via {}",
                self.channel.as_str(),
                binary_path.display()
            );
            if !output.stderr.is_empty() {
                message.push('\n');
                message.push_str(&output.stderr);
            }
            return Err(message);
        }

        if !is_valid_version_output(&output.stdout) {
            return Err(format!(
                "[FAIL] channel={} unexpected sce version output: {}",
                self.channel.as_str(),
                output.stdout
            ));
        }

        if !output.stderr.is_empty() {
            return Err(format!(
                "[FAIL] channel={} expected empty stderr for sce version.\n{}",
                self.channel.as_str(),
                output.stderr
            ));
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

    pub(crate) fn create_temp_subdir(&self, name: &str) -> Result<PathBuf, String> {
        let path = self.root.join(name);
        fs::create_dir_all(&path)
            .map_err(|error| format!("failed to create {}: {error}", path.display()))?;
        Ok(path)
    }

    pub(crate) fn resolve_program(&self, program: &str) -> Result<PathBuf, String> {
        self.resolve_executable(OsStr::new(program))
    }

    pub(crate) fn run_command<I, S>(&self, program: &Path, args: I) -> Result<CommandOutput, String>
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
    ) -> Result<CommandOutput, String>
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
            .env("SCE_CHANNEL_HARNESS_ROOT", &self.root)
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

        let output = command.output().map_err(|error| {
            format!(
                "[FAIL] channel={} failed to run {}: {error}",
                self.channel.as_str(),
                program.display()
            )
        })?;

        Ok(CommandOutput {
            status: output.status,
            stdout: normalize_output(&output.stdout),
            stderr: normalize_output(&output.stderr),
        })
    }

    fn create_layout(&self) -> Result<(), String> {
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
            fs::create_dir_all(&path)
                .map_err(|error| format!("failed to create {}: {error}", path.display()))?;
        }

        Ok(())
    }

    fn resolve_executable(&self, program: &OsStr) -> Result<PathBuf, String> {
        let candidate = Path::new(program);
        if candidate.components().count() > 1 {
            return Ok(candidate.to_path_buf());
        }

        for path_entry in env::split_paths(&self.path_with_harness_bins()) {
            let resolved = path_entry.join(candidate);
            if ensure_executable(&resolved).is_ok() {
                return Ok(resolved);
            }
        }

        Err(format!(
            "Unable to resolve executable '{}' for channel={}. Set {} or ensure it is on PATH.",
            candidate.display(),
            self.channel.as_str(),
            SCE_BINARY_ENV
        ))
    }

    fn home_dir(&self) -> PathBuf {
        self.root.join("home")
    }

    fn xdg_config_home(&self) -> PathBuf {
        self.root.join("xdg/config")
    }

    fn xdg_state_home(&self) -> PathBuf {
        self.root.join("xdg/state")
    }

    fn xdg_cache_home(&self) -> PathBuf {
        self.root.join("xdg/cache")
    }

    fn npm_prefix_dir(&self) -> PathBuf {
        self.root.join("npm/prefix")
    }

    fn npm_prefix_bin(&self) -> PathBuf {
        self.npm_prefix_dir().join("bin")
    }

    fn npm_cache_dir(&self) -> PathBuf {
        self.root.join("npm/cache")
    }

    fn bun_install_dir(&self) -> PathBuf {
        self.root.join("bun")
    }

    fn bun_install_bin(&self) -> PathBuf {
        self.bun_install_dir().join("bin")
    }

    fn cargo_home_dir(&self) -> PathBuf {
        self.root.join("cargo/home")
    }

    fn cargo_home_bin(&self) -> PathBuf {
        self.cargo_home_dir().join("bin")
    }

    fn cargo_target_dir(&self) -> PathBuf {
        self.root.join("cargo/target")
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
}

impl Drop for ChannelHarness {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

pub(crate) struct CommandOutput {
    pub(crate) status: ExitStatus,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
}

fn create_harness_root(channel: Channel) -> Result<PathBuf, String> {
    let mut attempt = 0u32;
    let temp_dir = env::temp_dir();

    loop {
        let unique = unique_suffix(attempt);
        let root = temp_dir.join(format!(
            "sce-install-channel-{}.{}",
            channel.as_str(),
            unique
        ));

        match fs::create_dir(&root) {
            Ok(()) => return Ok(root),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists && attempt < 8 => {
                attempt += 1;
            }
            Err(error) => {
                return Err(format!(
                    "failed to create harness root {}: {error}",
                    root.display()
                ));
            }
        }
    }
}

fn unique_suffix(attempt: u32) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_nanos();
    format!("{nanos}-{}-{attempt}", std::process::id())
}

fn normalize_output(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .replace('\r', "")
        .trim_end_matches('\n')
        .to_string()
}

fn is_valid_version_output(output: &str) -> bool {
    let mut parts = output.splitn(3, ' ');
    let binary = parts.next().unwrap_or_default();
    let version = parts.next().unwrap_or_default();
    let profile = parts.next().unwrap_or_default();

    !binary.is_empty()
        && !binary.contains(char::is_whitespace)
        && !version.is_empty()
        && !version.contains(char::is_whitespace)
        && profile.starts_with('(')
        && profile.ends_with(')')
        && profile.len() > 2
}

fn ensure_executable(path: &Path) -> Result<(), &'static str> {
    let metadata = path.metadata().map_err(|_| "missing")?;
    if !metadata.is_file() {
        return Err("not a file");
    }

    #[cfg(unix)]
    {
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err("not executable");
        }
    }

    Ok(())
}

pub(crate) fn copy_directory_recursive(source: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination)
        .map_err(|error| format!("failed to create {}: {error}", destination.display()))?;

    for entry in fs::read_dir(source)
        .map_err(|error| format!("failed to read {}: {error}", source.display()))?
    {
        let entry = entry.map_err(|error| format!("failed to read directory entry: {error}"))?;
        let entry_type = entry
            .file_type()
            .map_err(|error| format!("failed to inspect {}: {error}", entry.path().display()))?;
        let destination_path = destination.join(entry.file_name());

        if entry_type.is_dir() {
            copy_directory_recursive(&entry.path(), &destination_path)?;
        } else if entry_type.is_file() {
            fs::copy(entry.path(), &destination_path).map_err(|error| {
                format!(
                    "failed to copy {} to {}: {error}",
                    entry.path().display(),
                    destination_path.display()
                )
            })?;
        }
    }

    Ok(())
}
