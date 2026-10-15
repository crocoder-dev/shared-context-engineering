use std::ffi::OsStr;
use std::path::Path;
use std::process::{ExitStatus, Stdio};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

const GIT_PROGRAM: &str = "git";
const COMMIT_PATCH_ID_DEADLINE: Duration = Duration::from_secs(10);
const MAX_PATCH_INPUT_BYTES: u64 = 64 * 1024 * 1024;
const MAX_PATCH_ID_OUTPUT_BYTES: usize = 4 * 1024;
const MAX_REV_LIST_OUTPUT_BYTES: usize = 64 * 1024;
const MAX_RETAINED_STDERR_BYTES: u64 = 4 * 1024;
const STREAM_CHUNK_BYTES: usize = 64 * 1024;
const SHA1_HEX_LEN: usize = 40;
const SHA256_HEX_LEN: usize = 64;

const SHOW_CONFIG_OVERRIDES: &[&str] = &[
    "core.quotePath=true",
    "diff.noprefix=false",
    "diff.mnemonicPrefix=false",
    "diff.relative=false",
    "diff.context=3",
    "diff.interHunkContext=0",
    "diff.indentHeuristic=true",
    "diff.algorithm=myers",
    "diff.renames=false",
    "diff.suppressBlankEmpty=false",
    "diff.submodule=short",
    "diff.ignoreSubmodules=none",
    "color.ui=never",
    "color.diff=never",
    "log.showRoot=true",
    "log.showSignature=false",
];

const SHOW_ARGS: &[&str] = &[
    "show",
    "--format=",
    "--patch",
    "--binary",
    "--no-ext-diff",
    "--no-textconv",
    "--no-renames",
    "--no-color",
    "--diff-algorithm=myers",
    "--unified=3",
    "--inter-hunk-context=0",
    "--indent-heuristic",
    "--src-prefix=a/",
    "--dst-prefix=b/",
    "--no-relative",
    "-O/dev/null",
    "--root",
    "--no-show-signature",
    "--submodule=short",
    "--ignore-submodules=none",
];

const PATCH_ID_CONFIG_OVERRIDES: &[&str] = &["patchid.stable=true", "patchid.verbatim=false"];

const PATCH_ID_ARGS: &[&str] = &["patch-id", "--stable"];

const REMOVED_GIT_ENV: &[&str] = &["GIT_EXTERNAL_DIFF", "GIT_DIFF_OPTS"];

#[derive(Clone, Copy, Debug)]
struct CommitPatchIdRunner<'a> {
    program: &'a OsStr,
    leading_args: &'a [&'a OsStr],
    deadline: Duration,
    max_patch_input_bytes: u64,
    max_patch_id_output_bytes: usize,
}

impl Default for CommitPatchIdRunner<'_> {
    fn default() -> Self {
        CommitPatchIdRunner {
            program: OsStr::new(GIT_PROGRAM),
            leading_args: &[],
            deadline: COMMIT_PATCH_ID_DEADLINE,
            max_patch_input_bytes: MAX_PATCH_INPUT_BYTES,
            max_patch_id_output_bytes: MAX_PATCH_ID_OUTPUT_BYTES,
        }
    }
}

impl CommitPatchIdRunner<'_> {
    fn git_command(
        &self,
        repository_root: &Path,
        config_overrides: &[&str],
        args: &[&str],
    ) -> Command {
        let mut command = Command::new(self.program);
        command.args(self.leading_args);
        for config_override in config_overrides {
            command.arg("-c").arg(config_override);
        }
        command
            .args(args)
            .current_dir(repository_root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        for name in REMOVED_GIT_ENV {
            command.env_remove(name);
        }
        command
    }
}

pub(crate) async fn compute_commit_patch_id(
    repository_root: &Path,
    commit_oid: &str,
) -> Result<Option<String>> {
    compute_commit_patch_id_with(&CommitPatchIdRunner::default(), repository_root, commit_oid).await
}

async fn compute_commit_patch_id_with(
    runner: &CommitPatchIdRunner<'_>,
    repository_root: &Path,
    commit_oid: &str,
) -> Result<Option<String>> {
    if !is_lowercase_object_id(commit_oid) {
        bail!("commit patch ID requires a 40- or 64-character lowercase hex commit object ID");
    }

    tokio::time::timeout(
        runner.deadline,
        compute_commit_patch_id_unbounded(runner, repository_root, commit_oid),
    )
    .await
    .map_err(|_| {
        anyhow!(
            "commit patch ID computation exceeded the {}ms deadline",
            runner.deadline.as_millis()
        )
    })?
}

async fn compute_commit_patch_id_unbounded(
    runner: &CommitPatchIdRunner<'_>,
    repository_root: &Path,
    commit_oid: &str,
) -> Result<Option<String>> {
    if commit_parent_count(runner, repository_root, commit_oid).await? > 1 {
        return Ok(None);
    }

    let output = run_patch_id_pipeline(runner, repository_root, commit_oid).await?;
    parse_patch_id_output(&output, commit_oid.len())
}

async fn commit_parent_count(
    runner: &CommitPatchIdRunner<'_>,
    repository_root: &Path,
    commit_oid: &str,
) -> Result<usize> {
    let mut rev_list = runner
        .git_command(
            repository_root,
            &[],
            &["rev-list", "--parents", "-n", "1", commit_oid, "--"],
        )
        .spawn()
        .context("Failed to spawn git rev-list for commit patch ID")?;
    let stdout = rev_list
        .stdout
        .take()
        .context("git rev-list stdout was not captured")?;
    let stderr = rev_list
        .stderr
        .take()
        .context("git rev-list stderr was not captured")?;

    let (stdout, (), status) = tokio::try_join!(
        read_bounded(stdout, MAX_REV_LIST_OUTPUT_BYTES, "git rev-list output"),
        drain_stderr(stderr),
        wait_for_exit(&mut rev_list, "git rev-list"),
    )?;
    ensure_success(status, "git rev-list")?;

    let stdout = String::from_utf8(stdout).context("git rev-list emitted invalid UTF-8")?;
    let mut lines = stdout.lines();
    let (Some(line), None) = (lines.next(), lines.next()) else {
        bail!("git rev-list emitted malformed parent output");
    };
    let mut tokens = line.split(' ');
    if tokens.next() != Some(commit_oid) {
        bail!("git rev-list emitted malformed parent output");
    }
    let parents: Vec<&str> = tokens.collect();
    if parents.iter().any(|parent| !is_lowercase_object_id(parent)) {
        bail!("git rev-list emitted malformed parent output");
    }
    Ok(parents.len())
}

async fn run_patch_id_pipeline(
    runner: &CommitPatchIdRunner<'_>,
    repository_root: &Path,
    commit_oid: &str,
) -> Result<Vec<u8>> {
    let mut show_args = SHOW_ARGS.to_vec();
    show_args.extend([commit_oid, "--"]);
    let mut show = runner
        .git_command(repository_root, SHOW_CONFIG_OVERRIDES, &show_args)
        .spawn()
        .context("Failed to spawn git show for commit patch ID")?;
    let mut patch_id = runner
        .git_command(repository_root, PATCH_ID_CONFIG_OVERRIDES, PATCH_ID_ARGS)
        .stdin(Stdio::piped())
        .spawn()
        .context("Failed to spawn git patch-id for commit patch ID")?;

    let show_stdout = show
        .stdout
        .take()
        .context("git show stdout was not captured")?;
    let show_stderr = show
        .stderr
        .take()
        .context("git show stderr was not captured")?;
    let patch_id_stdin = patch_id
        .stdin
        .take()
        .context("git patch-id stdin was not captured")?;
    let patch_id_stdout = patch_id
        .stdout
        .take()
        .context("git patch-id stdout was not captured")?;
    let patch_id_stderr = patch_id
        .stderr
        .take()
        .context("git patch-id stderr was not captured")?;

    let ((), output, (), (), show_status, patch_id_status) = tokio::try_join!(
        stream_patch(show_stdout, patch_id_stdin, runner.max_patch_input_bytes),
        read_bounded(
            patch_id_stdout,
            runner.max_patch_id_output_bytes,
            "git patch-id output"
        ),
        drain_stderr(show_stderr),
        drain_stderr(patch_id_stderr),
        wait_for_exit(&mut show, "git show"),
        wait_for_exit(&mut patch_id, "git patch-id"),
    )?;
    ensure_success(show_status, "git show")?;
    ensure_success(patch_id_status, "git patch-id")?;
    Ok(output)
}

async fn stream_patch(
    mut source: ChildStdout,
    mut sink: ChildStdin,
    max_patch_input_bytes: u64,
) -> Result<()> {
    let mut buffer = vec![0_u8; STREAM_CHUNK_BYTES];
    let mut total_bytes: u64 = 0;
    loop {
        let read = source
            .read(&mut buffer)
            .await
            .context("Failed to read git show output")?;
        if read == 0 {
            break;
        }
        total_bytes += read as u64;
        if total_bytes > max_patch_input_bytes {
            bail!("git show patch exceeded the {max_patch_input_bytes}-byte limit");
        }
        sink.write_all(&buffer[..read])
            .await
            .context("Failed to stream patch into git patch-id")?;
    }
    sink.shutdown()
        .await
        .context("Failed to close git patch-id input")?;
    Ok(())
}

async fn read_bounded<R: AsyncRead + Unpin>(
    reader: R,
    max_bytes: usize,
    label: &str,
) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader
        .take(max_bytes as u64 + 1)
        .read_to_end(&mut bytes)
        .await
        .with_context(|| format!("Failed to read {label}"))?;
    if bytes.len() > max_bytes {
        bail!("{label} exceeded the {max_bytes}-byte limit");
    }
    Ok(bytes)
}

async fn drain_stderr<R: AsyncRead + Unpin>(mut reader: R) -> Result<()> {
    let mut retained = Vec::new();
    (&mut reader)
        .take(MAX_RETAINED_STDERR_BYTES)
        .read_to_end(&mut retained)
        .await
        .context("Failed to read git stderr")?;
    tokio::io::copy(&mut reader, &mut tokio::io::sink())
        .await
        .context("Failed to drain git stderr")?;
    Ok(())
}

async fn wait_for_exit(child: &mut Child, label: &str) -> Result<ExitStatus> {
    child
        .wait()
        .await
        .with_context(|| format!("Failed to wait for {label}"))
}

fn ensure_success(status: ExitStatus, label: &str) -> Result<()> {
    if status.success() {
        return Ok(());
    }
    match status.code() {
        Some(code) => bail!("{label} exited with status {code}"),
        None => bail!("{label} was terminated by a signal"),
    }
}

fn parse_patch_id_output(output: &[u8], object_id_len: usize) -> Result<Option<String>> {
    let output = std::str::from_utf8(output).context("git patch-id emitted invalid UTF-8")?;
    if output.is_empty() {
        return Ok(None);
    }
    let Some(line) = output.strip_suffix('\n') else {
        bail!("git patch-id emitted malformed output");
    };
    let mut tokens = line.split(' ');
    let (Some(patch_id), Some(commit_id), None) = (tokens.next(), tokens.next(), tokens.next())
    else {
        bail!("git patch-id emitted malformed output");
    };
    if patch_id.len() != object_id_len
        || commit_id.len() != object_id_len
        || !is_lowercase_object_id(patch_id)
        || !is_lowercase_object_id(commit_id)
    {
        bail!("git patch-id emitted malformed output");
    }
    Ok(Some(patch_id.to_string()))
}

fn is_lowercase_object_id(value: &str) -> bool {
    matches!(value.len(), SHA1_HEX_LEN | SHA256_HEX_LEN)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::time::Duration;

    use super::{
        compute_commit_patch_id, compute_commit_patch_id_with, parse_patch_id_output,
        CommitPatchIdRunner,
    };

    const FAKE_GIT_DEADLINE: Duration = Duration::from_millis(500);
    const CHILD_EXIT_TIMEOUT: Duration = Duration::from_secs(10);
    const CHILD_EXIT_POLL_INTERVAL: Duration = Duration::from_millis(20);
    const FAKE_COMMIT_OID: &str = "1111111111111111111111111111111111111111";
    const FAKE_PARENT_OID: &str = "2222222222222222222222222222222222222222";

    fn isolated_git(cwd: &Path, args: &[&str]) -> std::process::Output {
        Command::new("git")
            .args(args)
            .current_dir(cwd)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .expect("git command should run")
    }

    fn git(cwd: &Path, args: &[&str]) -> String {
        let output = isolated_git(cwd, args);
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout)
            .expect("git output is UTF-8")
            .trim()
            .to_string()
    }

    fn init_repository(root: &Path, object_format: &str) {
        std::fs::create_dir_all(root).expect("repo dir");
        git(
            root,
            &[
                "init",
                "-q",
                "-b",
                "main",
                &format!("--object-format={object_format}"),
            ],
        );
        git(root, &["config", "user.email", "patch-id@example.invalid"]);
        git(root, &["config", "user.name", "Patch Id Test"]);
        git(root, &["config", "commit.gpgsign", "false"]);
    }

    fn write(root: &Path, path: &str, contents: &[u8]) {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("create parent");
        std::fs::write(path, contents).expect("write file");
    }

    fn commit_all(root: &Path, message: &str) -> String {
        git(root, &["add", "-A"]);
        git(root, &["commit", "-q", "--allow-empty", "-m", message]);
        git(root, &["rev-parse", "HEAD"])
    }

    fn reference_patch_id(root: &Path, oid: &str) -> String {
        let show = isolated_git(root, &["show", "--binary", "--no-renames", oid]);
        assert!(show.status.success(), "reference git show failed");
        let mut patch_id = Command::new("git")
            .args(["patch-id", "--stable"])
            .current_dir(root)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawn reference git patch-id");
        std::io::Write::write_all(
            &mut patch_id.stdin.take().expect("patch-id stdin"),
            &show.stdout,
        )
        .expect("write reference patch");
        let output = patch_id.wait_with_output().expect("reference patch-id");
        String::from_utf8(output.stdout)
            .expect("UTF-8")
            .split(' ')
            .next()
            .expect("patch id column")
            .to_string()
    }

    async fn patch_id(root: &Path, oid: &str) -> Option<String> {
        compute_commit_patch_id(root, oid)
            .await
            .expect("commit patch ID computation should succeed")
    }

    fn multi_hunk_contents(edited: bool) -> String {
        let mut lines: Vec<String> = (0..40).map(|index| format!("line {index}")).collect();
        lines.insert(5, String::new());
        if edited {
            lines[2] = "changed early".to_string();
            lines[9] = "changed nearby".to_string();
            lines[30] = "changed late".to_string();
            lines.insert(20, "{\n    inserted();\n}".to_string());
        }
        lines.join("\n") + "\n"
    }

    fn seed_history(root: &Path) -> (String, String) {
        write(root, "base.txt", b"base\n");
        write(
            root,
            "src/m\u{e4}in.txt",
            multi_hunk_contents(false).as_bytes(),
        );
        let root_oid = commit_all(root, "root");
        write(
            root,
            "src/m\u{e4}in.txt",
            multi_hunk_contents(true).as_bytes(),
        );
        write(root, "notes.txt", b"lowercase notes\n");
        let change_oid = commit_all(root, "change");
        (root_oid, change_oid)
    }

    fn apply_hostile_diff_config(root: &Path) {
        for (key, value) in [
            ("diff.noprefix", "true"),
            ("diff.mnemonicPrefix", "true"),
            ("core.quotePath", "false"),
            ("diff.context", "10"),
            ("diff.interHunkContext", "10"),
            ("diff.algorithm", "histogram"),
            ("diff.indentHeuristic", "false"),
            ("diff.relative", "true"),
            ("diff.orderFile", "/nonexistent/sce-order-file"),
            ("diff.renames", "copies"),
            ("diff.suppressBlankEmpty", "true"),
            ("diff.external", "/bin/false"),
            ("diff.upper.textconv", "tr a-z A-Z"),
            ("color.ui", "always"),
            ("color.diff", "always"),
            ("log.showRoot", "false"),
            ("patchid.verbatim", "true"),
        ] {
            git(root, &["config", key, value]);
        }
        write(root, ".git/info/attributes", b"*.txt diff=upper\n");
    }

    #[tokio::test]
    async fn commit_patch_id_matches_canonical_invocation_for_root_and_normal_commits() {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path();
        init_repository(root, "sha1");
        let (root_oid, change_oid) = seed_history(root);

        for oid in [&root_oid, &change_oid] {
            let computed = patch_id(root, oid).await.expect("non-empty commit");
            assert_eq!(computed, reference_patch_id(root, oid));
            assert_eq!(computed.len(), 40);
        }
    }

    #[tokio::test]
    async fn commit_patch_id_is_stable_across_message_amend_and_equivalent_cherry_pick() {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path();
        init_repository(root, "sha1");
        let (root_oid, change_oid) = seed_history(root);
        let original = patch_id(root, &change_oid).await.expect("patch id");

        git(root, &["commit", "-q", "--amend", "-m", "reworded message"]);
        let amended_oid = git(root, &["rev-parse", "HEAD"]);
        assert_ne!(amended_oid, change_oid);
        assert_eq!(patch_id(root, &amended_oid).await, Some(original.clone()));

        git(root, &["checkout", "-q", "-b", "other", &root_oid]);
        write(root, "unrelated.txt", b"other parent\n");
        commit_all(root, "different parent");
        git(root, &["cherry-pick", &change_oid]);
        let picked_oid = git(root, &["rev-parse", "HEAD"]);
        assert_ne!(picked_oid, change_oid);
        assert_eq!(patch_id(root, &picked_oid).await, Some(original));
    }

    #[tokio::test]
    async fn commit_patch_id_ignores_user_diff_configuration() {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path();
        init_repository(root, "sha1");
        let (root_oid, change_oid) = seed_history(root);
        let clean_root = patch_id(root, &root_oid).await;
        let clean_change = patch_id(root, &change_oid).await;

        apply_hostile_diff_config(root);

        assert_eq!(patch_id(root, &root_oid).await, clean_root);
        assert_eq!(patch_id(root, &change_oid).await, clean_change);
        assert!(clean_root.is_some() && clean_change.is_some());
    }

    #[tokio::test]
    async fn commit_patch_id_is_deterministic_for_binary_and_rename_only_commits() {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path();
        init_repository(root, "sha1");
        write(root, "image.bin", &[0, 159, 146, 150, 0, 1, 2]);
        write(root, "docs/old-name.md", b"same contents\n");
        commit_all(root, "seed");
        write(root, "image.bin", &[0, 255, 254, 253, 0, 9]);
        let binary_oid = commit_all(root, "binary only");
        git(root, &["mv", "docs/old-name.md", "docs/new-name.md"]);
        let rename_oid = commit_all(root, "rename only");

        for oid in [&binary_oid, &rename_oid] {
            let first = patch_id(root, oid).await.expect("patch id");
            assert_eq!(patch_id(root, oid).await, Some(first.clone()));
            assert_eq!(first, reference_patch_id(root, oid));
        }
        assert_ne!(
            patch_id(root, &binary_oid).await,
            patch_id(root, &rename_oid).await
        );
    }

    #[tokio::test]
    async fn commit_patch_id_is_none_for_empty_and_merge_commits() {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path();
        init_repository(root, "sha1");
        write(root, "base.txt", b"base\n");
        let base_oid = commit_all(root, "base");
        let empty_oid = commit_all(root, "empty");
        assert_eq!(patch_id(root, &empty_oid).await, None);

        git(root, &["checkout", "-q", "-b", "side", &base_oid]);
        write(root, "side.txt", b"side\n");
        commit_all(root, "side");
        git(root, &["checkout", "-q", "main"]);
        write(root, "main.txt", b"main\n");
        commit_all(root, "main");
        git(root, &["merge", "-q", "--no-ff", "-m", "merge", "side"]);
        let merge_oid = git(root, &["rev-parse", "HEAD"]);
        assert_eq!(patch_id(root, &merge_oid).await, None);
    }

    #[tokio::test]
    async fn commit_patch_id_supports_sha256_repositories() {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path();
        init_repository(root, "sha256");
        let (_, change_oid) = seed_history(root);
        assert_eq!(change_oid.len(), 64);

        let computed = patch_id(root, &change_oid).await.expect("patch id");
        assert_eq!(computed.len(), 64);
        assert_eq!(computed, reference_patch_id(root, &change_oid));
    }

    #[tokio::test]
    async fn commit_patch_id_rejects_invalid_object_ids_and_unknown_commits() {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path();
        init_repository(root, "sha1");
        let (_, change_oid) = seed_history(root);

        for invalid in [
            change_oid.to_uppercase(),
            change_oid[..39].to_string(),
            "--output=/tmp/x".to_string(),
            FAKE_COMMIT_OID.to_string(),
        ] {
            assert!(compute_commit_patch_id(root, &invalid).await.is_err());
        }
    }

    #[tokio::test]
    async fn commit_patch_id_enforces_patch_input_and_output_byte_limits() {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path();
        init_repository(root, "sha1");
        let (_, change_oid) = seed_history(root);

        let input_limited = CommitPatchIdRunner {
            max_patch_input_bytes: 16,
            ..CommitPatchIdRunner::default()
        };
        let error = compute_commit_patch_id_with(&input_limited, root, &change_oid)
            .await
            .expect_err("patch input limit");
        assert!(error.to_string().contains("16-byte limit"), "{error:#}");

        let output_limited = CommitPatchIdRunner {
            max_patch_id_output_bytes: 8,
            ..CommitPatchIdRunner::default()
        };
        let error = compute_commit_patch_id_with(&output_limited, root, &change_oid)
            .await
            .expect_err("patch-id output limit");
        assert!(error.to_string().contains("8-byte limit"), "{error:#}");
    }

    #[test]
    fn commit_patch_id_parser_rejects_malformed_output() {
        let valid = format!("{FAKE_PARENT_OID} {}\n", "0".repeat(40));
        assert_eq!(
            parse_patch_id_output(valid.as_bytes(), 40).expect("valid"),
            Some(FAKE_PARENT_OID.to_string())
        );
        assert_eq!(parse_patch_id_output(b"", 40).expect("empty"), None);
        for malformed in [
            format!("{FAKE_PARENT_OID} {}", "0".repeat(40)),
            format!("{} {}\n", "A".repeat(40), "0".repeat(40)),
            format!("{FAKE_PARENT_OID}\n"),
            format!("{FAKE_PARENT_OID} {}\n", "0".repeat(64)),
            format!(
                "{FAKE_PARENT_OID} {0}\n{FAKE_PARENT_OID} {0}\n",
                "0".repeat(40)
            ),
        ] {
            assert!(parse_patch_id_output(malformed.as_bytes(), 40).is_err());
        }
        assert!(parse_patch_id_output(&[0xff, b'\n'], 40).is_err());
    }

    struct FakeGit {
        _dir: tempfile::TempDir,
        script: PathBuf,
        show_pid: PathBuf,
        patch_id_pid: PathBuf,
    }

    fn fake_git(show_body: &str, patch_id_body: &str) -> FakeGit {
        let dir = tempfile::tempdir().expect("temp dir");
        let script = dir.path().join("fake-git.sh");
        let show_pid = dir.path().join("show.pid");
        let patch_id_pid = dir.path().join("patch-id.pid");
        let contents = format!(
            "while [ \"$1\" = \"-c\" ]; do shift 2; done\n\
             case \"$1\" in\n\
             rev-list) echo \"{FAKE_COMMIT_OID} {FAKE_PARENT_OID}\" ;;\n\
             show) echo $$ > '{show_pid}'; {show_body} ;;\n\
             patch-id) echo $$ > '{patch_id_pid}'; {patch_id_body} ;;\n\
             esac\n",
            show_pid = show_pid.display(),
            patch_id_pid = patch_id_pid.display(),
        );
        std::fs::write(&script, contents).expect("write fake git");
        FakeGit {
            _dir: dir,
            script,
            show_pid,
            patch_id_pid,
        }
    }

    async fn run_fake_git(fake: &FakeGit) -> anyhow::Result<Option<String>> {
        let leading_args = [fake.script.as_os_str()];
        let runner = CommitPatchIdRunner {
            program: OsStr::new("/bin/sh"),
            leading_args: &leading_args,
            deadline: FAKE_GIT_DEADLINE,
            ..CommitPatchIdRunner::default()
        };
        compute_commit_patch_id_with(&runner, Path::new("."), FAKE_COMMIT_OID).await
    }

    async fn assert_process_exited(pid_file: &Path) {
        let pid = std::fs::read_to_string(pid_file).expect("pid file");
        let stat = PathBuf::from(format!("/proc/{}/stat", pid.trim()));
        let deadline = tokio::time::Instant::now() + CHILD_EXIT_TIMEOUT;
        loop {
            let running = std::fs::read_to_string(&stat)
                .ok()
                .and_then(|stat| {
                    stat.rsplit_once(')')
                        .map(|(_, rest)| rest.trim_start().to_string())
                })
                .is_some_and(|state| !state.starts_with('Z') && !state.starts_with('X'));
            if !running {
                return;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "child {} is still running",
                pid.trim()
            );
            tokio::time::sleep(CHILD_EXIT_POLL_INTERVAL).await;
        }
    }

    #[tokio::test]
    async fn commit_patch_id_reports_non_zero_git_exit() {
        let fake = fake_git("exit 3", "cat >/dev/null");
        let error = run_fake_git(&fake).await.expect_err("non-zero show");
        assert!(
            error.to_string().contains("git show exited with status 3"),
            "{error:#}"
        );
    }

    #[tokio::test]
    async fn commit_patch_id_reports_malformed_patch_id_output() {
        let fake = fake_git("echo patch", "cat >/dev/null; echo not-a-patch-id");
        let error = run_fake_git(&fake).await.expect_err("malformed output");
        assert!(error.to_string().contains("malformed"), "{error:#}");
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn commit_patch_id_times_out_and_kills_both_git_processes() {
        let fake = fake_git("exec sleep 30", "exec cat");
        let started = std::time::Instant::now();
        let error = run_fake_git(&fake).await.expect_err("timeout");
        assert!(error.to_string().contains("deadline"), "{error:#}");
        assert!(started.elapsed() < CHILD_EXIT_TIMEOUT);

        assert_process_exited(&fake.show_pid).await;
        assert_process_exited(&fake.patch_id_pid).await;
    }
}
