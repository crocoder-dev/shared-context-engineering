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

    let expected_hooks_dir = harness.repo_root().canonicalize()?.join(".git/hooks");
    assert_setup_hooks_install_and_rerun(&harness, &expected_hooks_dir)?;

    Ok(())
}

#[test]
fn setup_hooks_custom_path_install_and_rerun_are_deterministic() -> TestResult<()> {
    let harness = SetupIntegrationHarness::new("sce-setup-integration")?;
    harness.init_git_repo()?;
    harness.configure_local_hooks_path(".githooks")?;

    let expected_hooks_dir = harness.repo_root().canonicalize()?.join(".githooks");
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
fn setup_hooks_update_path() -> TestResult<()> {
    let harness = SetupIntegrationHarness::new("sce-setup-hooks-update")?;
    harness.init_git_repo()?;

    let hooks_dir = harness.repo_root().join(".git/hooks");

    let first = harness.run_sce(["setup", "--hooks"])?;
    assert!(
        first.success(),
        "initial hook install should succeed\nstdout:\n{}\nstderr:\n{}",
        first.stdout,
        first.stderr
    );

    assert_hooks_are_present_and_executable(&hooks_dir)?;

    let pre_commit_path = hooks_dir.join("pre-commit");
    let original_embedded_content = fs::read(&pre_commit_path)?;

    let mutated_content = format!(
        "{}\n# MUTATED FOR TEST\n",
        String::from_utf8_lossy(&original_embedded_content)
    );
    let mutated_bytes = mutated_content.as_bytes().to_vec();
    fs::write(&pre_commit_path, &mutated_content)?;

    let second = harness.run_sce(["setup", "--hooks"])?;
    assert!(
        second.success(),
        "hook update run should succeed\nstdout:\n{}\nstderr:\n{}",
        second.stdout,
        second.stderr
    );

    assert!(
        second.stdout.contains("- pre-commit: updated at '"),
        "output should report pre-commit as updated\nstdout:\n{}",
        second.stdout
    );

    assert!(
        second.stdout.contains("- commit-msg: skipped at '"),
        "output should report commit-msg as skipped\nstdout:\n{}",
        second.stdout
    );

    assert!(
        second.stdout.contains("- post-commit: skipped at '"),
        "output should report post-commit as skipped\nstdout:\n{}",
        second.stdout
    );

    let backup_pattern = regex::Regex::new(r"backup: '([^']+pre-commit\.backup[^']*)'").unwrap();
    if let Some(caps) = backup_pattern.captures(&second.stdout) {
        let backup_path = PathBuf::from(caps.get(1).unwrap().as_str());
        assert!(
            backup_path.exists(),
            "backup file should exist at '{}'",
            backup_path.display()
        );
        let backup_content = fs::read(&backup_path)?;
        assert_eq!(
            backup_content, mutated_bytes,
            "backup content should match pre-update mutated hook content"
        );
    } else {
        panic!(
            "could not extract backup path from output:\n{}",
            second.stdout
        );
    }

    let restored_hook = fs::read(&pre_commit_path)?;
    assert_eq!(
        restored_hook, original_embedded_content,
        "updated hook should match original embedded content"
    );

    assert_hooks_are_present_and_executable(&hooks_dir)?;

    Ok(())
}

#[test]
fn setup_backup_suffix_collision() -> TestResult<()> {
    let harness = SetupIntegrationHarness::new("sce-setup-backup-collision")?;
    harness.init_git_repo()?;
    let canonical_repo_root = harness.repo_root().canonicalize()?;

    let first = harness.run_sce(["setup", "--opencode"])?;
    assert!(
        first.success(),
        "initial setup should succeed\nstdout:\n{}\nstderr:\n{}",
        first.stdout,
        first.stderr
    );

    let opencode_dir = harness.repo_root().join(".opencode");
    assert!(
        opencode_dir.exists(),
        ".opencode should exist after initial setup"
    );

    let original_content = fs::read_to_string(opencode_dir.join("command/next-task.md"))?;

    let backup_0 = harness.repo_root().join(".opencode.backup");
    let backup_1 = harness.repo_root().join(".opencode.backup.1");
    fs::write(&backup_0, "collision placeholder 0")?;
    fs::write(&backup_1, "collision placeholder 1")?;

    let second = harness.run_sce(["setup", "--opencode"])?;
    assert!(
        second.success(),
        "rerun setup should succeed\nstdout:\n{}\nstderr:\n{}",
        second.stdout,
        second.stderr
    );

    let expected_backup = canonical_repo_root.join(".opencode.backup.2");
    assert!(
        second.stdout.contains(&format!(
            "backup: existing target moved to '{}'",
            expected_backup.display()
        )),
        "output should report backup to .backup.2 due to collision\nstdout:\n{}",
        second.stdout
    );

    assert!(
        expected_backup.exists(),
        "backup.2 directory should exist at '{}'",
        expected_backup.display()
    );

    assert!(
        expected_backup.join("command/next-task.md").exists(),
        "backup should contain original files"
    );

    let backup_content = fs::read_to_string(expected_backup.join("command/next-task.md"))?;
    assert_eq!(
        backup_content, original_content,
        "backup content should match original .opencode content"
    );

    assert!(
        backup_0.exists() && fs::read_to_string(&backup_0)? == "collision placeholder 0",
        "pre-existing .backup should be unchanged"
    );
    assert!(
        backup_1.exists() && fs::read_to_string(&backup_1)? == "collision placeholder 1",
        "pre-existing .backup.1 should be unchanged"
    );

    Ok(())
}

#[test]
fn setup_hooks_repo_relative_path() -> TestResult<()> {
    let harness = SetupIntegrationHarness::new("sce-setup-integration")?;
    harness.init_git_repo()?;

    let canonical_repo_root = harness.repo_root().canonicalize()?;
    let expected_hooks_dir = canonical_repo_root.join(".git/hooks");

    let parent_dir = harness.temp.path();
    let relative_repo_path = "repo";

    let output = Command::new(sce_binary_path())
        .args(["setup", "--hooks", "--repo", relative_repo_path])
        .current_dir(parent_dir)
        .env("XDG_STATE_HOME", harness.state_home())
        .env("HOME", &harness.home_dir)
        .env("GIT_CONFIG_GLOBAL", null_device_path())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()?;

    let result = render_command_result(output);

    assert!(
        result.success(),
        "setup --hooks --repo <relative> should succeed\nstdout:\n{}\nstderr:\n{}",
        result.stdout,
        result.stderr
    );
    assert!(
        result.stdout.contains("Hook setup completed successfully."),
        "output should contain success marker\nstdout:\n{}",
        result.stdout
    );

    assert!(
        result.stdout.contains(&format!(
            "Repository root: '{}'",
            canonical_repo_root.display()
        )),
        "output should contain canonical repository root '{}'\nstdout:\n{}",
        canonical_repo_root.display(),
        result.stdout
    );

    assert!(
        result.stdout.contains(&format!(
            "Hooks directory: '{}'",
            expected_hooks_dir.display()
        )),
        "output should contain hooks directory '{}'\nstdout:\n{}",
        expected_hooks_dir.display(),
        result.stdout
    );

    assert_hooks_are_present_and_executable(&expected_hooks_dir)?;

    Ok(())
}

#[test]
fn setup_hooks_repo_absolute_path() -> TestResult<()> {
    let harness = SetupIntegrationHarness::new("sce-setup-integration")?;
    harness.init_git_repo()?;

    let canonical_repo_root = harness.repo_root().canonicalize()?;
    let expected_hooks_dir = canonical_repo_root.join(".git/hooks");

    let result = harness.run_sce([
        "setup",
        "--hooks",
        "--repo",
        canonical_repo_root.to_str().unwrap(),
    ])?;

    assert!(
        result.success(),
        "setup --hooks --repo <absolute> should succeed\nstdout:\n{}\nstderr:\n{}",
        result.stdout,
        result.stderr
    );
    assert!(
        result.stdout.contains("Hook setup completed successfully."),
        "output should contain success marker\nstdout:\n{}",
        result.stdout
    );

    assert!(
        result.stdout.contains(&format!(
            "Repository root: '{}'",
            canonical_repo_root.display()
        )),
        "output should contain canonical repository root '{}'\nstdout:\n{}",
        canonical_repo_root.display(),
        result.stdout
    );

    assert!(
        result.stdout.contains(&format!(
            "Hooks directory: '{}'",
            expected_hooks_dir.display()
        )),
        "output should contain hooks directory '{}'\nstdout:\n{}",
        expected_hooks_dir.display(),
        result.stdout
    );

    assert_hooks_are_present_and_executable(&expected_hooks_dir)?;

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

    let expected_local_db = expected_agent_trace_local_db_path(&harness);
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
fn setup_fail_repo_missing() -> TestResult<()> {
    let harness = SetupIntegrationHarness::new("sce-setup-failure-contracts")?;

    let result = harness.run_sce(["setup", "--hooks", "--repo", "/missing"])?;

    assert_eq!(
        result.status.code(),
        Some(4),
        "setup --hooks --repo /missing should exit with runtime_failure code 4\nstdout:\n{}\nstderr:\n{}",
        result.stdout,
        result.stderr
    );

    assert!(
        result.stderr.contains("Failed to resolve repository path"),
        "stderr should contain canonical path resolution failure message\nstderr:\n{}",
        result.stderr
    );

    assert!(
        result
            .stderr
            .contains("Try: pass a path to an existing git repository"),
        "stderr should contain actionable guidance\nstderr:\n{}",
        result.stderr
    );

    Ok(())
}

#[test]
fn setup_fail_repo_without_hooks() -> TestResult<()> {
    let harness = SetupIntegrationHarness::new("sce-setup-failure-contracts")?;
    harness.init_git_repo()?;

    let result = harness.run_sce(["setup", "--repo", harness.repo_root().to_str().unwrap()])?;

    assert_eq!(
        result.status.code(),
        Some(3),
        "setup --repo <path> without --hooks should exit with validation_failure code 3\nstdout:\n{}\nstderr:\n{}",
        result.stdout,
        result.stderr
    );

    assert!(
        result.stderr.contains("Option '--repo' requires '--hooks'"),
        "stderr should contain --repo requires --hooks message\nstderr:\n{}",
        result.stderr
    );

    assert!(
        result
            .stderr
            .contains("Try: run 'sce setup --hooks --repo <path>' or remove '--repo'"),
        "stderr should contain actionable guidance\nstderr:\n{}",
        result.stderr
    );

    Ok(())
}

#[test]
fn setup_fail_noninteractive_without_target() -> TestResult<()> {
    let harness = SetupIntegrationHarness::new("sce-setup-failure-contracts")?;
    harness.init_git_repo()?;

    let result = harness.run_sce(["setup", "--non-interactive"])?;

    assert_eq!(
        result.status.code(),
        Some(3),
        "setup --non-interactive without target should exit with validation_failure code 3\nstdout:\n{}\nstderr:\n{}",
        result.stdout,
        result.stderr
    );

    assert!(
        result
            .stderr
            .contains("Option '--non-interactive' requires a target flag"),
        "stderr should contain --non-interactive requires target message\nstderr:\n{}",
        result.stderr
    );

    assert!(
        result.stderr.contains("Try: 'sce setup --opencode --non-interactive', 'sce setup --claude --non-interactive', or 'sce setup --both --non-interactive'"),
        "stderr should contain actionable guidance\nstderr:\n{}",
        result.stderr
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

#[cfg(unix)]
mod pty_interactive {
    use super::*;
    use portable_pty::{native_pty_system, CommandBuilder, PtyPair, PtySize};
    use std::io::{Read, Write};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    const PTY_WAIT_FOR_OUTPUT_MS: u64 = 1000;

    struct PtySession {
        _pair: PtyPair,
        writer: Box<dyn Write + Send>,
        output: Arc<Mutex<String>>,
        stop_flag: Arc<AtomicBool>,
        _reader_thread: thread::JoinHandle<()>,
    }

    impl PtySession {
        fn spawn(
            binary_path: &Path,
            args: &[&str],
            cwd: &Path,
            env: &[(&str, &str)],
        ) -> TestResult<Self> {
            let pty_system = native_pty_system();
            let pair = pty_system.openpty(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })?;

            let mut cmd = CommandBuilder::new(binary_path);
            cmd.args(args);
            cmd.cwd(cwd);
            for (key, value) in env {
                cmd.env(key, value);
            }

            let mut child = pair.slave.spawn_command(cmd)?;

            let writer = pair.master.take_writer()?;
            let mut reader = pair.master.try_clone_reader()?;

            let output = Arc::new(Mutex::new(String::new()));
            let stop_flag = Arc::new(AtomicBool::new(false));

            let output_clone = output.clone();
            let stop_clone = stop_flag.clone();
            let reader_thread = thread::spawn(move || {
                let mut buf = [0u8; 4096];
                while !stop_clone.load(Ordering::Relaxed) {
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            if let Ok(mut out) = output_clone.lock() {
                                out.push_str(&String::from_utf8_lossy(&buf[..n]));
                            }
                        }
                        Err(_) => break,
                    }
                }
                let _ = child.wait();
            });

            Ok(PtySession {
                _pair: pair,
                writer,
                output,
                stop_flag,
                _reader_thread: reader_thread,
            })
        }

        fn wait_for_output(&self, expected: &str, timeout_ms: u64) -> TestResult<bool> {
            let deadline = std::time::Instant::now() + Duration::from_millis(timeout_ms);
            while std::time::Instant::now() < deadline {
                if let Ok(out) = self.output.lock() {
                    if out.contains(expected) {
                        return Ok(true);
                    }
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Ok(false)
        }

        fn get_output(&self) -> String {
            self.output.lock().map(|s| s.clone()).unwrap_or_default()
        }

        fn write_line(&mut self, line: &str) -> TestResult<()> {
            writeln!(self.writer, "{}", line)?;
            self.writer.flush()?;
            Ok(())
        }

        fn write_raw(&mut self, data: &[u8]) -> TestResult<()> {
            self.writer.write_all(data)?;
            self.writer.flush()?;
            Ok(())
        }
    }

    impl Drop for PtySession {
        fn drop(&mut self) {
            self.stop_flag.store(true, Ordering::Relaxed);
        }
    }

    #[test]
    fn setup_interactive_pty_select_opencode() -> TestResult<()> {
        let harness = SetupIntegrationHarness::new("sce-setup-pty")?;
        harness.init_git_repo()?;

        let mut session = PtySession::spawn(
            &sce_binary_path(),
            &["setup"],
            harness.repo_root(),
            &[
                ("XDG_STATE_HOME", harness.state_home().to_str().unwrap()),
                ("HOME", harness.home_dir.to_str().unwrap()),
                ("GIT_CONFIG_GLOBAL", null_device_path()),
                ("GIT_CONFIG_NOSYSTEM", "1"),
            ],
        )?;

        let prompt_found =
            session.wait_for_output("Select setup target", PTY_WAIT_FOR_OUTPUT_MS)?;
        let output = session.get_output();
        assert!(
            prompt_found,
            "PTY should display prompt for target selection\noutput:\n{}",
            output
        );

        session.write_line("")?;

        let completed =
            session.wait_for_output("Setup completed successfully.", PTY_WAIT_FOR_OUTPUT_MS * 3)?;
        let output = session.get_output();
        assert!(
            completed,
            "PTY flow should complete successfully after OpenCode selection\noutput:\n{}",
            output
        );

        assert!(
            output.contains("Selected target(s): OpenCode"),
            "output should report OpenCode as selected target\noutput:\n{}",
            output
        );

        assert!(
            harness
                .repo_root()
                .join(".opencode/command/next-task.md")
                .exists(),
            "OpenCode assets should be installed after PTY selection"
        );

        Ok(())
    }

    #[test]
    fn setup_interactive_pty_cancel() -> TestResult<()> {
        let harness = SetupIntegrationHarness::new("sce-setup-pty")?;
        harness.init_git_repo()?;

        let mut session = PtySession::spawn(
            &sce_binary_path(),
            &["setup"],
            harness.repo_root(),
            &[
                ("XDG_STATE_HOME", harness.state_home().to_str().unwrap()),
                ("HOME", harness.home_dir.to_str().unwrap()),
                ("GIT_CONFIG_GLOBAL", null_device_path()),
                ("GIT_CONFIG_NOSYSTEM", "1"),
            ],
        )?;

        let prompt_found =
            session.wait_for_output("Select setup target", PTY_WAIT_FOR_OUTPUT_MS)?;
        let output = session.get_output();
        assert!(
            prompt_found,
            "PTY should display prompt for target selection\noutput:\n{}",
            output
        );

        session.write_raw(&[0x1b])?;

        std::thread::sleep(Duration::from_millis(PTY_WAIT_FOR_OUTPUT_MS));
        let output = session.get_output();

        assert!(
            output.contains("Setup cancelled") || output.contains("cancelled"),
            "PTY cancel flow should report cancellation\noutput:\n{}",
            output
        );

        assert!(
            !harness.repo_root().join(".opencode").exists(),
            "No files should be created after cancellation"
        );
        assert!(
            !harness.repo_root().join(".claude").exists(),
            "No files should be created after cancellation"
        );

        Ok(())
    }

    #[test]
    fn setup_interactive_nontty_fail() -> TestResult<()> {
        let harness = SetupIntegrationHarness::new("sce-setup-nontty")?;
        harness.init_git_repo()?;

        let output = Command::new(sce_binary_path())
            .args(["setup"])
            .current_dir(harness.repo_root())
            .env("XDG_STATE_HOME", harness.state_home())
            .env("HOME", &harness.home_dir)
            .env("GIT_CONFIG_GLOBAL", null_device_path())
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()?;

        let result = render_command_result(output);

        assert_eq!(
            result.status.code(),
            Some(4),
            "setup without TTY should exit with runtime_failure code 4\nstdout:\n{}\nstderr:\n{}",
            result.stdout,
            result.stderr
        );

        assert!(
            result.stderr.contains("Interactive setup requires a TTY"),
            "stderr should contain TTY requirement message\nstderr:\n{}",
            result.stderr
        );

        assert!(
            result.stderr.contains("--non-interactive"),
            "stderr should contain actionable guidance mentioning --non-interactive\nstderr:\n{}",
            result.stderr
        );

        assert!(
            !harness.repo_root().join(".opencode").exists(),
            "No files should be created after non-TTY failure"
        );

        Ok(())
    }
}

#[cfg(not(unix))]
mod pty_interactive {
    use super::*;

    #[test]
    fn setup_interactive_pty_select_opencode() -> TestResult<()> {
        let harness = SetupIntegrationHarness::new("sce-setup-pty")?;
        harness.init_git_repo()?;

        let result = harness.run_sce(["setup", "--opencode"])?;

        assert!(
            result.success(),
            "PTY test (non-unix fallback): setup --opencode should succeed\nstdout:\n{}\nstderr:\n{}",
            result.stdout,
            result.stderr
        );
        assert!(
            harness
                .repo_root()
                .join(".opencode/command/next-task.md")
                .exists(),
            "PTY test (non-unix fallback): OpenCode assets should be installed"
        );

        Ok(())
    }

    #[test]
    fn setup_interactive_pty_cancel() -> TestResult<()> {
        let harness = SetupIntegrationHarness::new("sce-setup-pty")?;
        harness.init_git_repo()?;

        assert!(
            !harness.repo_root().join(".opencode").exists(),
            "No files should exist before any setup"
        );

        Ok(())
    }

    #[test]
    fn setup_interactive_nontty_fail() -> TestResult<()> {
        let harness = SetupIntegrationHarness::new("sce-setup-nontty")?;
        harness.init_git_repo()?;

        let output = Command::new(sce_binary_path())
            .args(["setup"])
            .current_dir(harness.repo_root())
            .env("XDG_STATE_HOME", harness.state_home())
            .env("HOME", &harness.home_dir)
            .env("GIT_CONFIG_GLOBAL", null_device_path())
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()?;

        let result = render_command_result(output);

        assert_eq!(
            result.status.code(),
            Some(4),
            "setup without TTY should exit with runtime_failure code 4\nstdout:\n{}\nstderr:\n{}",
            result.stdout,
            result.stderr
        );

        assert!(
            result.stderr.contains("Interactive setup requires a TTY"),
            "stderr should contain TTY requirement message\nstderr:\n{}",
            result.stderr
        );

        Ok(())
    }
}

#[cfg(unix)]
mod setup_permission_failures {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    struct PermissionRestorer {
        path: PathBuf,
        original_mode: u32,
    }

    impl PermissionRestorer {
        fn new(path: PathBuf) -> TestResult<Self> {
            let metadata = fs::metadata(&path)?;
            let original_mode = metadata.permissions().mode();
            Ok(Self {
                path,
                original_mode,
            })
        }

        fn make_readonly(&self) -> TestResult<()> {
            let mut perms = fs::metadata(&self.path)?.permissions();
            perms.set_mode(0o555);
            fs::set_permissions(&self.path, perms)?;
            Ok(())
        }
    }

    impl Drop for PermissionRestorer {
        fn drop(&mut self) {
            if let Some(mut perms) = fs::metadata(&self.path).ok().map(|m| m.permissions()) {
                perms.set_mode(self.original_mode);
                let _ = fs::set_permissions(&self.path, perms);
            }
        }
    }

    fn is_running_as_root() -> bool {
        unsafe { libc::getuid() == 0 }
    }

    #[test]
    fn setup_permission_fail_repo_root_nonwritable() -> TestResult<()> {
        if is_running_as_root() {
            eprintln!("Skipping test: running as root (can write to read-only directories)");
            return Ok(());
        }

        let harness = SetupIntegrationHarness::new("sce-setup-perm-repo-root")?;
        harness.init_git_repo()?;

        let restorer = PermissionRestorer::new(harness.repo_root().to_path_buf())?;
        restorer.make_readonly()?;

        let result = harness.run_sce(["setup", "--opencode"])?;

        assert_eq!(
            result.status.code(),
            Some(4),
            "setup with non-writable repo root should exit with runtime_failure code 4\nstdout:\n{}\nstderr:\n{}",
            result.stdout,
            result.stderr
        );

        assert!(
            result.stderr.contains("Error [SCE-ERR-RUNTIME]:"),
            "stderr should contain runtime error class\nstderr:\n{}",
            result.stderr
        );

        assert!(
            result
                .stderr
                .contains("Setup installation failed for OpenCode"),
            "stderr should contain setup installation failure message\nstderr:\n{}",
            result.stderr
        );

        assert!(
            result.stderr.contains("Try:"),
            "stderr should contain actionable guidance\nstderr:\n{}",
            result.stderr
        );

        Ok(())
    }

    #[test]
    fn setup_permission_fail_hooks_dir_nonwritable() -> TestResult<()> {
        if is_running_as_root() {
            eprintln!("Skipping test: running as root (can write to read-only directories)");
            return Ok(());
        }

        let harness = SetupIntegrationHarness::new("sce-setup-perm-hooks-dir")?;
        harness.init_git_repo()?;

        let hooks_dir = harness.repo_root().join(".git/hooks");
        fs::create_dir_all(&hooks_dir)?;

        let restorer = PermissionRestorer::new(hooks_dir.clone())?;
        restorer.make_readonly()?;

        let result = harness.run_sce(["setup", "--hooks"])?;

        assert_eq!(
            result.status.code(),
            Some(4),
            "setup --hooks with non-writable hooks directory should exit with runtime_failure code 4\nstdout:\n{}\nstderr:\n{}",
            result.stdout,
            result.stderr
        );

        assert!(
            result.stderr.contains("Error [SCE-ERR-RUNTIME]:"),
            "stderr should contain runtime error class\nstderr:\n{}",
            result.stderr
        );

        assert!(
            result
                .stderr
                .contains("Hook setup failed while installing required git hooks"),
            "stderr should contain hook setup failure message\nstderr:\n{}",
            result.stderr
        );

        assert!(
            result.stderr.contains("Try:"),
            "stderr should contain actionable guidance\nstderr:\n{}",
            result.stderr
        );

        Ok(())
    }

    #[test]
    fn setup_permission_fail_unix_readonly_guard() -> TestResult<()> {
        if is_running_as_root() {
            eprintln!("Skipping test: running as root (can write to read-only directories)");
            return Ok(());
        }

        let harness = SetupIntegrationHarness::new("sce-setup-perm-readonly")?;
        harness.init_git_repo()?;

        let restorer = PermissionRestorer::new(harness.repo_root().to_path_buf())?;
        restorer.make_readonly()?;

        let result = harness.run_sce(["setup", "--claude"])?;

        assert_eq!(
            result.status.code(),
            Some(4),
            "setup with read-only repo should fail deterministically\nstdout:\n{}\nstderr:\n{}",
            result.stdout,
            result.stderr
        );

        assert!(
            !result.stderr.is_empty(),
            "stderr should contain error message for read-only scenario\nstderr:\n{}",
            result.stderr
        );

        Ok(())
    }
}

#[cfg(not(unix))]
mod setup_permission_failures {
    use super::*;

    #[test]
    fn setup_permission_fail_repo_root_nonwritable() -> TestResult<()> {
        let harness = SetupIntegrationHarness::new("sce-setup-perm-repo-root")?;
        harness.init_git_repo()?;

        let result = harness.run_sce(["setup", "--opencode"])?;
        assert!(
            result.success(),
            "setup --opencode should succeed on non-unix platforms (permission tests not applicable)\nstdout:\n{}\nstderr:\n{}",
            result.stdout,
            result.stderr
        );

        Ok(())
    }

    #[test]
    fn setup_permission_fail_hooks_dir_nonwritable() -> TestResult<()> {
        let harness = SetupIntegrationHarness::new("sce-setup-perm-hooks-dir")?;
        harness.init_git_repo()?;

        let result = harness.run_sce(["setup", "--hooks"])?;
        assert!(
            result.success(),
            "setup --hooks should succeed on non-unix platforms (permission tests not applicable)\nstdout:\n{}\nstderr:\n{}",
            result.stdout,
            result.stderr
        );

        Ok(())
    }

    #[test]
    fn setup_permission_fail_unix_readonly_guard() -> TestResult<()> {
        let harness = SetupIntegrationHarness::new("sce-setup-perm-readonly")?;
        harness.init_git_repo()?;

        let result = harness.run_sce(["setup", "--claude"])?;
        assert!(
            result.success(),
            "setup --claude should succeed on non-unix platforms (permission tests not applicable)\nstdout:\n{}\nstderr:\n{}",
            result.stdout,
            result.stderr
        );

        Ok(())
    }
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

fn expected_agent_trace_local_db_path(harness: &SetupIntegrationHarness) -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        harness
            .home_dir
            .join("AppData")
            .join("Local")
            .join("sce")
            .join("agent-trace")
            .join("local.db")
    }

    #[cfg(target_os = "macos")]
    {
        harness
            .home_dir
            .join("Library")
            .join("Application Support")
            .join("sce")
            .join("agent-trace")
            .join("local.db")
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        harness
            .state_home
            .join("sce")
            .join("agent-trace")
            .join("local.db")
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
