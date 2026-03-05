use std::error::Error;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

type TestResult<T> = Result<T, Box<dyn Error>>;

#[derive(Debug)]
struct IntegrationTempDir {
    path: PathBuf,
}

impl IntegrationTempDir {
    fn new(prefix: &str) -> TestResult<Self> {
        let epoch_nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let path =
            std::env::temp_dir().join(format!("{prefix}-{}-{epoch_nanos}", std::process::id()));
        fs::create_dir_all(&path)?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for IntegrationTempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[derive(Debug)]
struct SetupIntegrationHarness {
    temp: IntegrationTempDir,
    repo_root: PathBuf,
    state_home: PathBuf,
    home_dir: PathBuf,
}

#[derive(Debug)]
struct CommandResult {
    status: std::process::ExitStatus,
    stdout: String,
    stderr: String,
}

const REQUIRED_HOOK_NAMES: [&str; 3] = ["pre-commit", "commit-msg", "post-commit"];

impl CommandResult {
    fn success(&self) -> bool {
        self.status.success()
    }
}

impl SetupIntegrationHarness {
    fn new(prefix: &str) -> TestResult<Self> {
        let temp = IntegrationTempDir::new(prefix)?;
        let repo_root = temp.path().join("repo");
        let state_home = temp.path().join("xdg-state");
        let home_dir = temp.path().join("home");

        fs::create_dir_all(&repo_root)?;
        fs::create_dir_all(&state_home)?;
        fs::create_dir_all(&home_dir)?;

        Ok(Self {
            temp,
            repo_root,
            state_home,
            home_dir,
        })
    }

    fn repo_root(&self) -> &Path {
        &self.repo_root
    }

    fn state_home(&self) -> &Path {
        &self.state_home
    }

    fn init_git_repo(&self) -> TestResult<()> {
        let result = self.run_git(["init", "-q"])?;
        if !result.success() {
            return Err(format!(
                "git init failed:\nstdout:\n{}\nstderr:\n{}",
                result.stdout, result.stderr
            )
            .into());
        }
        Ok(())
    }

    fn configure_local_hooks_path(&self, relative_hooks_path: &str) -> TestResult<()> {
        let result = self.run_git(["config", "core.hooksPath", relative_hooks_path])?;
        if !result.success() {
            return Err(format!(
                "git config core.hooksPath failed:\nstdout:\n{}\nstderr:\n{}",
                result.stdout, result.stderr
            )
            .into());
        }
        Ok(())
    }

    fn run_sce<I, S>(&self, args: I) -> TestResult<CommandResult>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let output = self.base_command(sce_binary_path()).args(args).output()?;
        Ok(render_command_result(output))
    }

    fn run_git<I, S>(&self, args: I) -> TestResult<CommandResult>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let output = self.base_command("git").args(args).output()?;
        Ok(render_command_result(output))
    }

    fn base_command<P: AsRef<OsStr>>(&self, program: P) -> Command {
        let mut command = Command::new(program);
        command
            .current_dir(&self.repo_root)
            .env("XDG_STATE_HOME", &self.state_home)
            .env("HOME", &self.home_dir)
            .env("GIT_CONFIG_GLOBAL", null_device_path())
            .env("GIT_CONFIG_NOSYSTEM", "1");
        command
    }
}

#[test]
fn setup_hooks_default_path_install_and_rerun_are_deterministic() -> TestResult<()> {
    let harness = SetupIntegrationHarness::new("sce-setup-integration")?;
    harness.init_git_repo()?;

    let expected_hooks_dir = harness.repo_root().join(".git/hooks");
    assert_setup_hooks_install_and_rerun(&harness, &expected_hooks_dir)?;

    Ok(())
}

#[test]
fn setup_hooks_custom_path_install_and_rerun_are_deterministic() -> TestResult<()> {
    let harness = SetupIntegrationHarness::new("sce-setup-integration")?;
    harness.init_git_repo()?;
    harness.configure_local_hooks_path(".githooks")?;

    let expected_hooks_dir = harness.repo_root().join(".githooks");
    assert_setup_hooks_install_and_rerun(&harness, &expected_hooks_dir)?;

    let result = harness.run_git(["config", "--get", "core.hooksPath"])?;
    assert!(
        result.success(),
        "git config --get core.hooksPath should succeed\nstdout:\n{}\nstderr:\n{}",
        result.stdout,
        result.stderr
    );
    assert_eq!(result.stdout.trim(), ".githooks");

    Ok(())
}

#[test]
fn harness_scopes_turso_state_home_to_test_temp_root() -> TestResult<()> {
    let harness = SetupIntegrationHarness::new("sce-setup-integration")?;
    harness.init_git_repo()?;

    let post_commit = harness.run_sce(["hooks", "post-commit"])?;
    assert!(
        post_commit.success(),
        "hooks post-commit should run successfully\nstdout:\n{}\nstderr:\n{}",
        post_commit.stdout,
        post_commit.stderr
    );

    let expected_local_db = harness.state_home().join("sce/agent-trace/local.db");
    assert!(
        expected_local_db.exists(),
        "expected Turso local DB path '{}' to exist",
        expected_local_db.display()
    );
    assert!(
        expected_local_db.starts_with(harness.temp.path()),
        "expected Turso local DB path '{}' to stay within test temp root '{}')",
        expected_local_db.display(),
        harness.temp.path().display()
    );

    Ok(())
}

#[test]
fn setup_targets_opencode_install_and_rerun_are_deterministic() -> TestResult<()> {
    let harness = SetupIntegrationHarness::new("sce-setup-targets")?;
    harness.init_git_repo()?;

    assert_setup_target_install_and_rerun(
        &harness,
        &["setup", "--opencode"],
        "Selected target(s): OpenCode",
        &["OpenCode: installed"],
        &[".opencode/command/next-task.md"],
        &[".claude"],
    )?;

    Ok(())
}

#[test]
fn setup_targets_claude_install_and_rerun_are_deterministic() -> TestResult<()> {
    let harness = SetupIntegrationHarness::new("sce-setup-targets")?;
    harness.init_git_repo()?;

    assert_setup_target_install_and_rerun(
        &harness,
        &["setup", "--claude"],
        "Selected target(s): Claude",
        &["Claude: installed"],
        &[".claude/commands/next-task.md"],
        &[".opencode"],
    )?;

    Ok(())
}

#[test]
fn setup_targets_both_install_and_rerun_are_deterministic() -> TestResult<()> {
    let harness = SetupIntegrationHarness::new("sce-setup-targets")?;
    harness.init_git_repo()?;

    assert_setup_target_install_and_rerun(
        &harness,
        &["setup", "--both"],
        "Selected target(s): OpenCode, Claude",
        &["OpenCode: installed", "Claude: installed"],
        &[
            ".opencode/command/next-task.md",
            ".claude/commands/next-task.md",
        ],
        &[],
    )?;

    Ok(())
}

fn assert_setup_target_install_and_rerun(
    harness: &SetupIntegrationHarness,
    args: &[&str],
    expected_selected_targets_line: &str,
    expected_target_status_markers: &[&str],
    required_paths: &[&str],
    forbidden_paths: &[&str],
) -> TestResult<()> {
    let first = harness.run_sce(args.iter().copied())?;
    assert!(
        first.success(),
        "initial setup run should succeed\nstdout:\n{}\nstderr:\n{}",
        first.stdout,
        first.stderr
    );
    assert!(first.stdout.contains("Setup completed successfully."));
    assert!(
        first.stdout.contains(expected_selected_targets_line),
        "initial setup output should include expected selected-target line '{}'\nstdout:\n{}",
        expected_selected_targets_line,
        first.stdout
    );
    for marker in expected_target_status_markers {
        assert!(
            first.stdout.contains(marker),
            "initial setup output should include marker '{}'\nstdout:\n{}",
            marker,
            first.stdout
        );
    }
    assert!(
        first
            .stdout
            .contains("backup: not needed (no existing target)"),
        "initial setup run should report no backup requirement\nstdout:\n{}",
        first.stdout
    );

    for relative_path in required_paths {
        assert!(
            harness.repo_root().join(relative_path).exists(),
            "expected installed path '{}' to exist after first run",
            relative_path
        );
    }

    for relative_path in forbidden_paths {
        assert!(
            !harness.repo_root().join(relative_path).exists(),
            "expected path '{}' to remain absent for this setup target",
            relative_path
        );
    }

    let second = harness.run_sce(args.iter().copied())?;
    assert!(
        second.success(),
        "second setup run should succeed\nstdout:\n{}\nstderr:\n{}",
        second.stdout,
        second.stderr
    );
    assert!(second.stdout.contains("Setup completed successfully."));
    assert!(
        second.stdout.contains(expected_selected_targets_line),
        "second setup output should include expected selected-target line '{}'\nstdout:\n{}",
        expected_selected_targets_line,
        second.stdout
    );
    for marker in expected_target_status_markers {
        assert!(
            second.stdout.contains(marker),
            "second setup output should include marker '{}'\nstdout:\n{}",
            marker,
            second.stdout
        );
    }
    assert!(
        second.stdout.contains("backup: existing target moved to"),
        "second setup run should report backup-and-replace behavior\nstdout:\n{}",
        second.stdout
    );

    for relative_path in required_paths {
        assert!(
            harness.repo_root().join(relative_path).exists(),
            "expected installed path '{}' to exist after second run",
            relative_path
        );
    }

    for relative_path in forbidden_paths {
        assert!(
            !harness.repo_root().join(relative_path).exists(),
            "expected path '{}' to remain absent after second run",
            relative_path
        );
    }

    Ok(())
}

fn assert_setup_hooks_install_and_rerun(
    harness: &SetupIntegrationHarness,
    expected_hooks_directory: &Path,
) -> TestResult<()> {
    let first = harness.run_sce(["setup", "--hooks"])?;
    assert!(
        first.success(),
        "first setup --hooks run should succeed\nstdout:\n{}\nstderr:\n{}",
        first.stdout,
        first.stderr
    );
    assert!(first.stdout.contains("Hook setup completed successfully."));
    assert!(first.stdout.contains(&format!(
        "Hooks directory: '{}'",
        expected_hooks_directory.display()
    )));

    for hook in REQUIRED_HOOK_NAMES {
        let expected_hook_path = expected_hooks_directory.join(hook);
        assert!(
            first.stdout.contains(&format!(
                "- {hook}: installed at '{}'",
                expected_hook_path.display()
            )),
            "first setup run should report '{}' as installed\nstdout:\n{}",
            hook,
            first.stdout
        );
    }
    assert_eq!(
        first.stdout.matches("backup: not needed").count(),
        REQUIRED_HOOK_NAMES.len(),
        "first setup run should report backup: not needed for each required hook\nstdout:\n{}",
        first.stdout
    );

    assert_hooks_are_present_and_executable(expected_hooks_directory)?;

    let second = harness.run_sce(["setup", "--hooks"])?;
    assert!(
        second.success(),
        "second setup --hooks run should succeed\nstdout:\n{}\nstderr:\n{}",
        second.stdout,
        second.stderr
    );
    assert!(second.stdout.contains("Hook setup completed successfully."));
    assert!(second.stdout.contains(&format!(
        "Hooks directory: '{}'",
        expected_hooks_directory.display()
    )));

    for hook in REQUIRED_HOOK_NAMES {
        let expected_hook_path = expected_hooks_directory.join(hook);
        assert!(
            second.stdout.contains(&format!(
                "- {hook}: skipped at '{}'",
                expected_hook_path.display()
            )),
            "second setup run should report '{}' as skipped\nstdout:\n{}",
            hook,
            second.stdout
        );
    }
    assert_eq!(
        second.stdout.matches("backup: not needed").count(),
        REQUIRED_HOOK_NAMES.len(),
        "second setup run should report backup: not needed for each required hook\nstdout:\n{}",
        second.stdout
    );

    assert_hooks_are_present_and_executable(expected_hooks_directory)?;

    Ok(())
}

fn assert_hooks_are_present_and_executable(hooks_directory: &Path) -> TestResult<()> {
    for hook in REQUIRED_HOOK_NAMES {
        let hook_path = hooks_directory.join(hook);
        assert!(
            hook_path.exists(),
            "expected required hook '{}' to exist at '{}'",
            hook,
            hook_path.display()
        );
        assert!(
            hook_path.is_file(),
            "expected required hook '{}' at '{}' to be a file",
            hook,
            hook_path.display()
        );
        assert_executable_file(&hook_path)?;
    }

    Ok(())
}

#[cfg(unix)]
fn assert_executable_file(path: &Path) -> TestResult<()> {
    use std::os::unix::fs::PermissionsExt;

    let metadata = fs::metadata(path)?;
    let mode = metadata.permissions().mode();
    assert!(
        mode & 0o111 != 0,
        "expected hook '{}' to have at least one executable bit set (mode {:o})",
        path.display(),
        mode
    );
    Ok(())
}

#[cfg(not(unix))]
fn assert_executable_file(path: &Path) -> TestResult<()> {
    let metadata = fs::metadata(path)?;
    assert!(
        metadata.is_file(),
        "expected hook '{}' to resolve to a regular file",
        path.display()
    );
    Ok(())
}

fn sce_binary_path() -> PathBuf {
    if let Some(path) = std::env::var_os("CARGO_BIN_EXE_sce") {
        return PathBuf::from(path);
    }

    let test_executable = std::env::current_exe()
        .expect("integration test should resolve current executable path for binary fallback");
    let debug_root = test_executable
        .parent()
        .and_then(Path::parent)
        .expect("integration test executable should run from target/{profile}/deps");

    let candidate = debug_root.join(binary_filename("sce"));
    assert!(
        candidate.exists(),
        "integration test could not resolve compiled sce binary at '{}'",
        candidate.display()
    );

    candidate
}

fn binary_filename(base: &str) -> String {
    #[cfg(windows)]
    {
        format!("{base}.exe")
    }

    #[cfg(not(windows))]
    {
        base.to_string()
    }
}

fn render_command_result(output: Output) -> CommandResult {
    CommandResult {
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

fn null_device_path() -> &'static str {
    #[cfg(windows)]
    {
        "NUL"
    }

    #[cfg(not(windows))]
    {
        "/dev/null"
    }
}
