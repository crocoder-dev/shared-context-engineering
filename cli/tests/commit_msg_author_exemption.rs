use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Output};

const SCE_AUTHOR_EMAIL: &str = "sce@crocoder.dev";

struct Sandbox {
    _dir: tempfile::TempDir,
    root: std::path::PathBuf,
    home: std::path::PathBuf,
    work: std::path::PathBuf,
}

impl Sandbox {
    fn new(user_email: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let home = root.join("home");
        let work = root.join("work");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(work.join("hooks")).unwrap();

        let hook = work.join("hooks").join("commit-msg");
        std::fs::write(
            &hook,
            format!(
                "#!/bin/sh\nexec '{}' hooks commit-msg \"$1\"\n",
                env!("CARGO_BIN_EXE_sce")
            ),
        )
        .unwrap();
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();

        let sandbox = Self {
            _dir: dir,
            root,
            home,
            work,
        };
        sandbox.git_ok(&["init", "-q", "-b", "main"]);
        sandbox.git_ok(&["config", "user.name", "Developer"]);
        sandbox.git_ok(&["config", "user.email", user_email]);
        sandbox.git_ok(&["config", "commit.gpgsign", "false"]);
        sandbox.git_ok(&["config", "core.hooksPath", "hooks"]);
        std::fs::write(sandbox.work.join("a.txt"), "one\n").unwrap();
        sandbox.git_ok(&["add", "a.txt"]);
        sandbox
    }

    fn git(&self, args: &[&str], envs: &[(&str, &str)]) -> Output {
        let mut command = Command::new("git");
        command
            .args(args)
            .current_dir(&self.work)
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap())
            .env("HOME", &self.home)
            .env("XDG_CONFIG_HOME", self.home.join("config"))
            .env("XDG_STATE_HOME", self.home.join("state"))
            .env("XDG_DATA_HOME", self.home.join("data"))
            .env("XDG_CACHE_HOME", self.home.join("cache"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CEILING_DIRECTORIES", &self.root)
            .env("NO_COLOR", "1");
        for (key, value) in envs {
            command.env(key, value);
        }
        command.output().unwrap()
    }

    fn git_ok(&self, args: &[&str]) -> String {
        let output = self.git(args, &[]);
        assert_success(&output, args);
        String::from_utf8(output.stdout).unwrap()
    }
}

fn assert_success(output: &Output, args: &[&str]) {
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn home_is_empty(home: &Path) -> bool {
    std::fs::read_dir(home).unwrap().next().is_none()
}

fn sce_state_root(home: &Path) -> std::path::PathBuf {
    if cfg!(target_os = "macos") {
        home.join("Library").join("Application Support")
    } else {
        home.join("state")
    }
}

fn sce_logs(home: &Path) -> String {
    let Ok(entries) = std::fs::read_dir(sce_state_root(home).join("sce").join("logs")) else {
        return String::new();
    };
    entries
        .map(|entry| std::fs::read_to_string(entry.unwrap().path()).unwrap())
        .collect()
}

#[test]
fn commit_msg_hook_skips_coauthor_and_preflight_for_sce_author() {
    struct AuthorCase {
        user_email: &'static str,
        extra_args: &'static [&'static str],
        envs: &'static [(&'static str, &'static str)],
    }

    let cases = [
        AuthorCase {
            user_email: "dev@example.invalid",
            extra_args: &[],
            envs: &[("GIT_AUTHOR_EMAIL", SCE_AUTHOR_EMAIL)],
        },
        AuthorCase {
            user_email: "dev@example.invalid",
            extra_args: &["--author", "SCE <SCE@CroCoder.dev>"],
            envs: &[],
        },
        AuthorCase {
            user_email: SCE_AUTHOR_EMAIL,
            extra_args: &[],
            envs: &[],
        },
    ];

    for case in cases {
        let sandbox = Sandbox::new(case.user_email);
        let mut args = vec!["commit", "-q", "-m", "subject"];
        args.extend_from_slice(case.extra_args);

        let output = sandbox.git(&args, case.envs);
        assert_success(&output, &args);
        let hook_report = String::from_utf8_lossy(&output.stderr);
        assert!(
            hook_report.contains("policy_gate_passed=true, trailer_applied=false"),
            "{hook_report}"
        );

        assert_eq!(sandbox.git_ok(&["log", "-1", "--format=%B"]), "subject\n\n");
        assert!(sandbox
            .git_ok(&["log", "-1", "--format=%ae"])
            .trim()
            .eq_ignore_ascii_case(SCE_AUTHOR_EMAIL));
        assert!(
            home_is_empty(&sandbox.home),
            "AI-overlap preflight must not open Agent Trace storage for the SCE author"
        );
    }
}

#[test]
fn commit_msg_hook_runs_preflight_for_other_authors() {
    let sandbox = Sandbox::new("dev@example.invalid");
    let args = ["commit", "-q", "-m", "subject"];

    let output = sandbox.git(&args, &[]);
    assert_success(&output, &args);

    assert_eq!(sandbox.git_ok(&["log", "-1", "--format=%B"]), "subject\n\n");
    let logs = sce_logs(&sandbox.home);
    assert!(
        logs.contains("event_id=sce.hooks.commit_msg.ai_overlap_error"),
        "{logs}"
    );
}
