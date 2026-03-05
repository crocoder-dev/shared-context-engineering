use anyhow::{bail, Context, Result};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr;
use std::time::Instant;

use crate::services::agent_trace::{
    build_trace_payload, AgentTraceContributor, AgentTraceConversation, AgentTraceFile,
    AgentTraceRange, AgentTraceRecord, AgentTraceVcs, ContributorInput, ContributorType,
    ConversationInput, FileAttributionInput, QualityStatus, RangeInput, RewriteInfo,
    TraceAdapterInput, METADATA_IDEMPOTENCY_KEY, METADATA_QUALITY_STATUS, TRACE_CONTENT_TYPE,
    TRACE_VERSION, VCS_TYPE_GIT,
};
use crate::services::local_db::ensure_agent_trace_local_db_ready_blocking;

pub const NAME: &str = "hooks";
pub const CANONICAL_SCE_COAUTHOR_TRAILER: &str = "Co-authored-by: SCE <sce@crocoder.dev>";
pub const POST_COMMIT_PARENT_SHA_METADATA_KEY: &str = "dev.crocoder.sce.parent_revision";
const PRE_COMMIT_CHECKPOINT_GIT_PATH: &str = "sce/pre-commit-checkpoint.json";
const RETRY_QUEUE_MAX_ITEMS_PER_RUN: usize = 16;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HookSubcommand {
    PreCommit,
    CommitMsg { message_file: PathBuf },
    PostCommit,
    PostRewrite { rewrite_method: String },
}

pub fn hooks_usage_text() -> &'static str {
    "Usage:\n  sce hooks pre-commit\n  sce hooks commit-msg <message-file>\n  sce hooks post-commit\n  sce hooks post-rewrite <amend|rebase|other>\n\nGit executes hook scripts with these subcommands. `post-rewrite` reads rewrite pairs from STDIN."
}

pub fn parse_hooks_subcommand(args: Vec<String>) -> Result<HookSubcommand> {
    if args.is_empty() {
        bail!("Missing hook subcommand. Run 'sce hooks --help' to see valid usage.");
    }

    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        bail!("{}", hooks_usage_text());
    }

    match args[0].as_str() {
        "pre-commit" => {
            ensure_no_extra_hook_args("pre-commit", &args[1..])?;
            Ok(HookSubcommand::PreCommit)
        }
        "commit-msg" => {
            if args.len() < 2 {
                bail!(
                    "Missing required argument '<message-file>' for 'commit-msg'. Run 'sce hooks --help' to see valid usage."
                );
            }

            if args.len() > 2 {
                bail!(
                    "Unexpected extra argument '{}' for 'commit-msg'. Run 'sce hooks --help' to see valid usage.",
                    args[2]
                );
            }

            Ok(HookSubcommand::CommitMsg {
                message_file: PathBuf::from_str(&args[1])?,
            })
        }
        "post-commit" => {
            ensure_no_extra_hook_args("post-commit", &args[1..])?;
            Ok(HookSubcommand::PostCommit)
        }
        "post-rewrite" => {
            if args.len() < 2 {
                bail!(
                    "Missing required argument '<amend|rebase|other>' for 'post-rewrite'. Run 'sce hooks --help' to see valid usage."
                );
            }

            if args.len() > 2 {
                bail!(
                    "Unexpected extra argument '{}' for 'post-rewrite'. Run 'sce hooks --help' to see valid usage.",
                    args[2]
                );
            }

            Ok(HookSubcommand::PostRewrite {
                rewrite_method: args[1].clone(),
            })
        }
        unknown => bail!(
            "Unknown hook subcommand '{}'. Run 'sce hooks --help' to see valid usage.",
            unknown
        ),
    }
}

fn ensure_no_extra_hook_args(hook: &str, args: &[String]) -> Result<()> {
    if args.is_empty() {
        return Ok(());
    }

    bail!(
        "Unexpected extra argument '{}' for '{}'. Run 'sce hooks --help' to see valid usage.",
        args[0],
        hook
    )
}

pub fn run_hooks_subcommand(subcommand: HookSubcommand) -> Result<String> {
    match subcommand {
        HookSubcommand::PreCommit => run_pre_commit_subcommand(),
        HookSubcommand::CommitMsg { message_file } => run_commit_msg_subcommand(message_file),
        HookSubcommand::PostCommit => run_post_commit_subcommand(),
        HookSubcommand::PostRewrite { rewrite_method } => {
            run_post_rewrite_subcommand(&rewrite_method)
        }
    }
}

fn run_pre_commit_subcommand() -> Result<String> {
    let repository_root = std::env::current_dir()
        .context("Failed to determine current directory for pre-commit runtime invocation.")?;
    run_pre_commit_subcommand_in_repo(&repository_root)
}

fn run_pre_commit_subcommand_in_repo(repository_root: &Path) -> Result<String> {
    let runtime = resolve_pre_commit_runtime_state(repository_root);

    if runtime.sce_disabled || !runtime.cli_available || runtime.is_bare_repo {
        let reason = if runtime.sce_disabled {
            PreCommitNoOpReason::Disabled
        } else if !runtime.cli_available {
            PreCommitNoOpReason::CliUnavailable
        } else {
            PreCommitNoOpReason::BareRepository
        };

        return Ok(format!(
            "pre-commit hook executed with no-op runtime state: {reason:?}"
        ));
    }

    let anchors = match capture_pre_commit_tree_anchors(repository_root) {
        Ok(anchors) => anchors,
        Err(error) => {
            return Ok(format!(
                "pre-commit hook skipped checkpoint finalization: failed to capture git anchors ({error})"
            ));
        }
    };

    let pending = match collect_pending_checkpoint(repository_root) {
        Ok(pending) => pending,
        Err(error) => {
            return Ok(format!(
                "pre-commit hook skipped checkpoint finalization: failed to collect staged attribution ({error})"
            ));
        }
    };

    let outcome = finalize_pre_commit_checkpoint(&runtime, anchors, pending);

    let message = match outcome {
        PreCommitFinalization::NoOp(reason) => {
            format!("pre-commit hook executed with no-op runtime state: {reason:?}")
        }
        PreCommitFinalization::Finalized(checkpoint) => {
            if let Err(error) = write_finalized_checkpoint(repository_root, &checkpoint) {
                return Ok(format!(
                    "pre-commit hook finalized staged checkpoint for {} file(s) but failed to persist handoff artifact ({error})",
                    checkpoint.files.len()
                ));
            }
            format!(
                "pre-commit hook executed and finalized staged checkpoint for {} file(s).",
                checkpoint.files.len()
            )
        }
    };

    Ok(message)
}

fn resolve_pre_commit_runtime_state(repository_root: &Path) -> PreCommitRuntimeState {
    PreCommitRuntimeState {
        sce_disabled: env_flag_is_truthy("SCE_DISABLED"),
        cli_available: git_command_success(repository_root, &["--version"]),
        is_bare_repo: git_command_output(repository_root, &["rev-parse", "--is-bare-repository"])
            .is_some_and(|output| output == "true"),
    }
}

fn env_flag_is_truthy(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .is_some_and(|value| env_value_is_truthy(&value))
}

fn env_flag_is_enabled_by_default(name: &str) -> bool {
    match std::env::var(name) {
        Ok(value) => env_value_is_truthy(&value),
        Err(_) => true,
    }
}

fn env_value_is_truthy(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn git_command_success(repository_root: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .args(args)
        .current_dir(repository_root)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn git_command_output(repository_root: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repository_root)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8(output.stdout).ok()?;
    Some(stdout.trim().to_string())
}

fn run_git_command(repository_root: &Path, args: &[&str], context_message: &str) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repository_root)
        .output()
        .with_context(|| {
            format!(
                "{} (directory: '{}')",
                context_message,
                repository_root.display()
            )
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let diagnostic = if stderr.is_empty() {
            "git command exited with a non-zero status".to_string()
        } else {
            stderr
        };
        bail!("{} {}", context_message, diagnostic);
    }

    String::from_utf8(output.stdout)
        .context("git command output contained invalid UTF-8")
        .map(|stdout| stdout.trim().to_string())
}

fn run_git_command_allow_empty(
    repository_root: &Path,
    args: &[&str],
    context_message: &str,
) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repository_root)
        .output()
        .with_context(|| {
            format!(
                "{} (directory: '{}')",
                context_message,
                repository_root.display()
            )
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let diagnostic = if stderr.is_empty() {
            "git command exited with a non-zero status".to_string()
        } else {
            stderr
        };
        bail!("{} {}", context_message, diagnostic);
    }

    String::from_utf8(output.stdout).context("git command output contained invalid UTF-8")
}

fn capture_pre_commit_tree_anchors(repository_root: &Path) -> Result<PreCommitTreeAnchors> {
    let index_tree = run_git_command(
        repository_root,
        &["write-tree"],
        "Failed to capture index tree anchor for pre-commit checkpoint.",
    )?;
    let head_tree = git_command_output(repository_root, &["rev-parse", "--verify", "HEAD^{tree}"]);

    Ok(PreCommitTreeAnchors {
        index_tree,
        head_tree,
    })
}

fn collect_pending_checkpoint(repository_root: &Path) -> Result<PendingCheckpoint> {
    let staged_diff = run_git_command_allow_empty(
        repository_root,
        &[
            "diff",
            "--cached",
            "--unified=0",
            "--no-color",
            "--no-ext-diff",
        ],
        "Failed to collect staged diff for pre-commit attribution.",
    )?;
    let unstaged_diff = run_git_command_allow_empty(
        repository_root,
        &["diff", "--unified=0", "--no-color", "--no-ext-diff"],
        "Failed to collect unstaged diff for pre-commit attribution.",
    )?;

    let staged_ranges = parse_unified_zero_diff_ranges(&staged_diff)?;
    let unstaged_ranges = parse_unified_zero_diff_ranges(&unstaged_diff)?;

    let mut all_paths = BTreeMap::new();
    for path in staged_ranges.keys() {
        all_paths.insert(path.clone(), ());
    }
    for path in unstaged_ranges.keys() {
        all_paths.insert(path.clone(), ());
    }

    let files = all_paths
        .keys()
        .map(|path| PendingFileCheckpoint {
            path: path.clone(),
            staged_ranges: staged_ranges.get(path).cloned().unwrap_or_default(),
            unstaged_ranges: unstaged_ranges.get(path).cloned().unwrap_or_default(),
        })
        .collect();

    Ok(PendingCheckpoint { files })
}

fn parse_unified_zero_diff_ranges(
    contents: &str,
) -> Result<BTreeMap<String, Vec<PendingLineRange>>> {
    let mut ranges_by_path: BTreeMap<String, Vec<PendingLineRange>> = BTreeMap::new();
    let mut current_path: Option<String> = None;

    for line in contents.lines() {
        if let Some(path) = line.strip_prefix("+++ b/") {
            current_path = Some(path.to_string());
            continue;
        }

        if line.starts_with("+++") {
            current_path = None;
            continue;
        }

        if !line.starts_with("@@") {
            continue;
        }

        let Some(path) = current_path.clone() else {
            continue;
        };

        if let Some(range) = parse_hunk_new_range(line)? {
            ranges_by_path.entry(path).or_default().push(range);
        }
    }

    Ok(ranges_by_path)
}

fn parse_hunk_new_range(header_line: &str) -> Result<Option<PendingLineRange>> {
    let mut fields = header_line.split_whitespace();
    let _ = fields.next();
    let _ = fields.next();
    let Some(new_range_field) = fields.next() else {
        bail!(
            "Invalid unified diff hunk header '{}': missing new-range field",
            header_line
        );
    };

    let Some(range_body) = new_range_field.strip_prefix('+') else {
        bail!(
            "Invalid unified diff hunk header '{}': malformed new-range field",
            header_line
        );
    };

    let mut parts = range_body.split(',');
    let start_line: u32 = parts
        .next()
        .context("Unified diff hunk is missing start line")?
        .parse()
        .with_context(|| {
            format!(
                "Invalid hunk start line in '{}': expected integer",
                header_line
            )
        })?;
    let line_count: u32 = parts
        .next()
        .map(str::parse)
        .transpose()
        .with_context(|| {
            format!(
                "Invalid hunk line count in '{}': expected integer",
                header_line
            )
        })?
        .unwrap_or(1);

    if line_count == 0 {
        return Ok(None);
    }

    Ok(Some(PendingLineRange {
        start_line,
        end_line: start_line + line_count - 1,
    }))
}

fn resolve_pre_commit_checkpoint_path(repository_root: &Path) -> Result<PathBuf> {
    let resolved = run_git_command(
        repository_root,
        &["rev-parse", "--git-path", PRE_COMMIT_CHECKPOINT_GIT_PATH],
        "Failed to resolve pre-commit checkpoint handoff path.",
    )?;
    let path = PathBuf::from(resolved);

    if path.is_absolute() {
        return Ok(path);
    }

    Ok(repository_root.join(path))
}

fn write_finalized_checkpoint(
    repository_root: &Path,
    checkpoint: &FinalizedCheckpoint,
) -> Result<()> {
    let checkpoint_path = resolve_pre_commit_checkpoint_path(repository_root)?;
    let parent = checkpoint_path
        .parent()
        .context("Resolved pre-commit checkpoint path has no parent directory")?;
    fs::create_dir_all(parent).with_context(|| {
        format!(
            "Failed to create pre-commit checkpoint directory '{}'.",
            parent.display()
        )
    })?;

    let mut files = Vec::new();
    for file in &checkpoint.files {
        let mut ranges = Vec::new();
        for range in &file.ranges {
            ranges.push(serde_json::json!({
                "start_line": range.start_line,
                "end_line": range.end_line,
            }));
        }
        files.push(serde_json::json!({
            "path": file.path,
            "ranges": ranges,
        }));
    }

    let payload = serde_json::json!({
        "version": 1,
        "anchors": {
            "index_tree": checkpoint.anchors.index_tree.clone(),
            "head_tree": checkpoint.anchors.head_tree.clone(),
        },
        "files": files,
    });

    let serialized = serde_json::to_vec_pretty(&payload)
        .context("Failed to serialize pre-commit checkpoint artifact")?;
    fs::write(&checkpoint_path, serialized).with_context(|| {
        format!(
            "Failed to persist pre-commit checkpoint artifact '{}'.",
            checkpoint_path.display()
        )
    })
}

fn run_commit_msg_subcommand(message_file: PathBuf) -> Result<String> {
    let repository_root = std::env::current_dir()
        .context("Failed to determine current directory for commit-msg runtime invocation.")?;
    run_commit_msg_subcommand_in_repo(&repository_root, &message_file)
}

fn run_commit_msg_subcommand_in_repo(
    repository_root: &Path,
    message_file: &Path,
) -> Result<String> {
    let metadata = fs::metadata(message_file).with_context(|| {
        format!(
            "Invalid commit message file '{}': file does not exist or is not readable.",
            message_file.display()
        )
    })?;

    if !metadata.is_file() {
        bail!(
            "Invalid commit message file '{}': expected a regular file path.",
            message_file.display()
        );
    }

    let runtime = resolve_commit_msg_runtime_state(repository_root);
    let original = fs::read_to_string(message_file).with_context(|| {
        format!(
            "Invalid commit message file '{}': failed to read UTF-8 content.",
            message_file.display()
        )
    })?;

    let transformed = apply_commit_msg_coauthor_policy(&runtime, &original);
    let gate_passed =
        !runtime.sce_disabled && runtime.sce_coauthor_enabled && runtime.has_staged_sce_attribution;
    let trailer_applied = gate_passed && transformed != original;

    if trailer_applied {
        fs::write(message_file, transformed.as_bytes()).with_context(|| {
            format!(
                "Failed to update commit message file '{}' with canonical co-author trailer.",
                message_file.display()
            )
        })?;
    }

    Ok(format!(
        "commit-msg hook processed message file '{}' (policy_gate_passed={}, trailer_applied={}).",
        message_file.display(),
        gate_passed,
        trailer_applied
    ))
}

fn resolve_commit_msg_runtime_state(repository_root: &Path) -> CommitMsgRuntimeState {
    CommitMsgRuntimeState {
        sce_disabled: env_flag_is_truthy("SCE_DISABLED"),
        sce_coauthor_enabled: env_flag_is_enabled_by_default("SCE_COAUTHOR_ENABLED"),
        has_staged_sce_attribution: staged_sce_attribution_present(repository_root),
    }
}

fn staged_sce_attribution_present(repository_root: &Path) -> bool {
    let Ok(checkpoint_path) = resolve_pre_commit_checkpoint_path(repository_root) else {
        return false;
    };

    let Ok(payload) = fs::read_to_string(&checkpoint_path) else {
        return false;
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&payload) else {
        return false;
    };

    json.get("files")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|files| {
            files.iter().any(|file| {
                file.get("ranges")
                    .and_then(serde_json::Value::as_array)
                    .is_some_and(|ranges| !ranges.is_empty())
            })
        })
}

fn run_post_commit_subcommand() -> Result<String> {
    let repository_root = std::env::current_dir()
        .context("Failed to determine current directory for post-commit runtime invocation.")?;
    run_post_commit_subcommand_in_repo(&repository_root)
}

fn run_post_commit_subcommand_in_repo(repository_root: &Path) -> Result<String> {
    let runtime = resolve_post_commit_runtime_state(repository_root);

    if runtime.sce_disabled || !runtime.cli_available || runtime.is_bare_repo {
        let reason = if runtime.sce_disabled {
            PostCommitNoOpReason::Disabled
        } else if !runtime.cli_available {
            PostCommitNoOpReason::CliUnavailable
        } else {
            PostCommitNoOpReason::BareRepository
        };

        return Ok(format!(
            "post-commit hook executed with no-op runtime state: {reason:?}"
        ));
    }

    let runtime_paths = match resolve_post_commit_runtime_paths(repository_root) {
        Ok(paths) => paths,
        Err(error) => {
            return Ok(format!(
                "post-commit hook skipped trace finalization: failed to resolve persistence targets ({error})"
            ));
        }
    };

    let input = match build_post_commit_input(repository_root) {
        Ok(input) => input,
        Err(error) => {
            return Ok(format!(
                "post-commit hook skipped trace finalization: failed to collect commit attribution input ({error})"
            ));
        }
    };

    let mut notes_writer = GitNotesTraceWriter {
        repository_root: repository_root.to_path_buf(),
    };
    let mut record_store = LocalDbTraceRecordStore {
        repository_root: repository_root.to_path_buf(),
        db_path: runtime_paths.local_db_path,
    };
    let mut retry_queue = JsonFileTraceRetryQueue {
        path: runtime_paths.retry_queue_path,
    };
    let mut emission_ledger = FileTraceEmissionLedger {
        path: runtime_paths.emission_ledger_path,
    };

    let outcome = match finalize_post_commit_trace(
        &runtime,
        input,
        &mut notes_writer,
        &mut record_store,
        &mut retry_queue,
        &mut emission_ledger,
    ) {
        Ok(outcome) => outcome,
        Err(error) => {
            return Ok(format!(
                "post-commit hook skipped trace finalization: finalizer execution failed ({error})"
            ));
        }
    };

    let retry_report =
        match process_runtime_retry_queue(&mut retry_queue, &mut notes_writer, &mut record_store) {
            Ok(report) => report,
            Err(error) => {
                return Ok(format!(
                "post-commit hook completed trace finalization but retry replay failed ({error})"
            ));
            }
        };

    let message = match outcome {
        PostCommitFinalization::NoOp(reason) => {
            format!("post-commit hook executed with no-op runtime state: {reason:?}")
        }
        PostCommitFinalization::Persisted(persisted) => format!(
            "post-commit hook finalized trace for commit '{}' (trace_id='{}', notes={:?}, database={:?}) {}.",
            persisted.commit_sha, persisted.trace_id, persisted.notes, persisted.database
            , retry_report.summary_text()
        ),
        PostCommitFinalization::QueuedFallback(queued) => format!(
            "post-commit hook enqueued fallback for commit '{}' (trace_id='{}', failed_targets={:?}) {}.",
            queued.commit_sha,
            queued.trace_id,
            queued.failed_targets,
            retry_report.summary_text()
        ),
    };

    Ok(message)
}

fn resolve_post_commit_runtime_state(repository_root: &Path) -> PostCommitRuntimeState {
    PostCommitRuntimeState {
        sce_disabled: env_flag_is_truthy("SCE_DISABLED"),
        cli_available: git_command_success(repository_root, &["--version"]),
        is_bare_repo: git_command_output(repository_root, &["rev-parse", "--is-bare-repository"])
            .is_some_and(|output| output == "true"),
    }
}

struct PostCommitRuntimePaths {
    local_db_path: PathBuf,
    retry_queue_path: PathBuf,
    emission_ledger_path: PathBuf,
}

fn resolve_post_commit_runtime_paths(repository_root: &Path) -> Result<PostCommitRuntimePaths> {
    let local_db_path = ensure_agent_trace_local_db_ready_blocking()?;
    let retry_queue_path = resolve_git_path(repository_root, "sce/trace-retry-queue.jsonl")?;
    let emission_ledger_path = resolve_git_path(repository_root, "sce/trace-emission-ledger.txt")?;

    Ok(PostCommitRuntimePaths {
        local_db_path,
        retry_queue_path,
        emission_ledger_path,
    })
}

fn resolve_git_path(repository_root: &Path, git_path: &str) -> Result<PathBuf> {
    let resolved = run_git_command(
        repository_root,
        &["rev-parse", "--git-path", git_path],
        "Failed to resolve git persistence path.",
    )?;
    let path = PathBuf::from(resolved);
    if path.is_absolute() {
        return Ok(path);
    }

    Ok(repository_root.join(path))
}

fn build_post_commit_input(repository_root: &Path) -> Result<PostCommitInput> {
    let commit_sha = run_git_command(
        repository_root,
        &["rev-parse", "--verify", "HEAD"],
        "Failed to resolve post-commit HEAD SHA.",
    )?;
    let parent_sha = git_command_output(repository_root, &["rev-parse", "--verify", "HEAD^"]);
    let timestamp_rfc3339 = run_git_command(
        repository_root,
        &["show", "-s", "--format=%cI", "HEAD"],
        "Failed to resolve post-commit timestamp.",
    )?;
    let files = collect_post_commit_file_attribution(repository_root)?;
    let idempotency_key = format!("post-commit:{commit_sha}");
    let record_id = deterministic_uuid_v4_from_seed(&format!("{commit_sha}:{timestamp_rfc3339}"));

    Ok(PostCommitInput {
        record_id,
        timestamp_rfc3339,
        commit_sha,
        parent_sha,
        idempotency_key,
        files,
    })
}

fn collect_post_commit_file_attribution(
    repository_root: &Path,
) -> Result<Vec<FileAttributionInput>> {
    let checkpoint_files = load_post_commit_checkpoint_files(repository_root)?;
    if !checkpoint_files.is_empty() {
        return Ok(checkpoint_files);
    }

    collect_commit_file_attribution(
        repository_root,
        "HEAD",
        "https://crocoder.dev/sce/local-hooks/post-commit",
    )
}

fn collect_commit_file_attribution(
    repository_root: &Path,
    revision: &str,
    conversation_url: &str,
) -> Result<Vec<FileAttributionInput>> {
    let changed_paths = run_git_command_allow_empty(
        repository_root,
        &["show", "--pretty=format:", "--name-only", revision],
        "Failed to resolve changed files for commit attribution.",
    )?;

    let mut files = Vec::new();
    for line in changed_paths.lines() {
        let path = line.trim();
        if path.is_empty() {
            continue;
        }

        files.push(FileAttributionInput {
            path: path.to_string(),
            conversations: vec![ConversationInput {
                url: conversation_url.to_string(),
                related: Vec::new(),
                ranges: vec![RangeInput {
                    start_line: 1,
                    end_line: 1,
                    contributor: ContributorInput {
                        kind: ContributorType::Unknown,
                        model_id: None,
                    },
                }],
            }],
        });
    }

    Ok(files)
}

fn load_post_commit_checkpoint_files(repository_root: &Path) -> Result<Vec<FileAttributionInput>> {
    let checkpoint_path = resolve_pre_commit_checkpoint_path(repository_root)?;
    let payload = match fs::read_to_string(&checkpoint_path) {
        Ok(payload) => payload,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            bail!(
                "Failed to read pre-commit checkpoint '{}' for post-commit finalization: {}",
                checkpoint_path.display(),
                error
            )
        }
    };

    let checkpoint = serde_json::from_str::<serde_json::Value>(&payload).with_context(|| {
        format!(
            "Failed to parse pre-commit checkpoint '{}' as JSON.",
            checkpoint_path.display()
        )
    })?;

    let Some(files_json) = checkpoint
        .get("files")
        .and_then(serde_json::Value::as_array)
    else {
        return Ok(Vec::new());
    };

    let mut files = Vec::new();
    for file_json in files_json {
        let Some(path) = file_json.get("path").and_then(serde_json::Value::as_str) else {
            continue;
        };

        let ranges = file_json
            .get("ranges")
            .and_then(serde_json::Value::as_array)
            .map(|ranges| {
                ranges
                    .iter()
                    .filter_map(|range_json| {
                        let start_line = range_json
                            .get("start_line")
                            .and_then(serde_json::Value::as_u64)
                            .map(|value| value as u32)?;
                        let end_line = range_json
                            .get("end_line")
                            .and_then(serde_json::Value::as_u64)
                            .map(|value| value as u32)?;

                        Some(RangeInput {
                            start_line,
                            end_line,
                            contributor: ContributorInput {
                                kind: ContributorType::Unknown,
                                model_id: None,
                            },
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        if ranges.is_empty() {
            continue;
        }

        files.push(FileAttributionInput {
            path: path.to_string(),
            conversations: vec![ConversationInput {
                url: "https://crocoder.dev/sce/local-hooks/pre-commit-checkpoint".to_string(),
                related: Vec::new(),
                ranges,
            }],
        });
    }

    Ok(files)
}

fn deterministic_uuid_v4_from_seed(seed: &str) -> String {
    use sha2::{Digest, Sha256};

    let digest = Sha256::digest(seed.as_bytes());
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);

    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;

    format!(
        "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
        u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        u16::from_be_bytes([bytes[4], bytes[5]]),
        u16::from_be_bytes([bytes[6], bytes[7]]),
        u16::from_be_bytes([bytes[8], bytes[9]]),
        u64::from_be_bytes([
            0, 0, bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]
        ])
    )
}

fn run_post_rewrite_subcommand(rewrite_method: &str) -> Result<String> {
    let repository_root = std::env::current_dir()
        .context("Failed to determine current directory for post-rewrite runtime invocation.")?;
    let stdin = std::io::read_to_string(std::io::stdin())
        .context("Failed to read post-rewrite pair input from STDIN")?;

    run_post_rewrite_subcommand_in_repo(&repository_root, rewrite_method, &stdin)
}

fn run_post_rewrite_subcommand_in_repo(
    repository_root: &Path,
    rewrite_method: &str,
    pairs_file_contents: &str,
) -> Result<String> {
    let runtime = resolve_post_rewrite_runtime_state(repository_root);

    if runtime.sce_disabled || !runtime.cli_available || runtime.is_bare_repo {
        let reason = if runtime.sce_disabled {
            PostRewriteNoOpReason::Disabled
        } else if !runtime.cli_available {
            PostRewriteNoOpReason::CliUnavailable
        } else {
            PostRewriteNoOpReason::BareRepository
        };

        return Ok(format!(
            "post-rewrite hook executed with no-op runtime state: {reason:?}"
        ));
    }

    let runtime_paths = match resolve_post_commit_runtime_paths(repository_root) {
        Ok(paths) => paths,
        Err(error) => {
            return Ok(format!(
                "post-rewrite hook skipped rewrite finalization: failed to resolve persistence targets ({error})"
            ));
        }
    };

    let mut ingestion = LocalDbRewriteRemapIngestion {
        repository_root: repository_root.to_path_buf(),
        db_path: runtime_paths.local_db_path.clone(),
        accepted_requests: Vec::new(),
    };

    let outcome = match finalize_post_rewrite_remap(
        &runtime,
        rewrite_method,
        pairs_file_contents,
        &mut ingestion,
    ) {
        Ok(outcome) => outcome,
        Err(error) => {
            return Ok(format!(
                "post-rewrite hook skipped rewrite finalization: remap ingestion failed ({error})"
            ));
        }
    };

    let mut notes_writer = GitNotesTraceWriter {
        repository_root: repository_root.to_path_buf(),
    };
    let mut record_store = LocalDbTraceRecordStore {
        repository_root: repository_root.to_path_buf(),
        db_path: runtime_paths.local_db_path,
    };
    let mut retry_queue = JsonFileTraceRetryQueue {
        path: runtime_paths.retry_queue_path,
    };
    let mut emission_ledger = FileTraceEmissionLedger {
        path: runtime_paths.emission_ledger_path,
    };

    let mut rewrite_persisted = 0_usize;
    let mut rewrite_queued = 0_usize;
    let mut rewrite_noops = 0_usize;
    let mut rewrite_failures = 0_usize;

    for request in &ingestion.accepted_requests {
        let input = match build_rewrite_trace_input(repository_root, request) {
            Ok(input) => input,
            Err(_) => {
                rewrite_failures += 1;
                continue;
            }
        };

        match finalize_rewrite_trace(
            &runtime,
            input,
            &mut notes_writer,
            &mut record_store,
            &mut retry_queue,
            &mut emission_ledger,
        ) {
            Ok(RewriteTraceFinalization::Persisted(_)) => rewrite_persisted += 1,
            Ok(RewriteTraceFinalization::QueuedFallback(_)) => rewrite_queued += 1,
            Ok(RewriteTraceFinalization::NoOp(_)) => rewrite_noops += 1,
            Err(_) => rewrite_failures += 1,
        }
    }

    let retry_report =
        match process_runtime_retry_queue(&mut retry_queue, &mut notes_writer, &mut record_store) {
            Ok(report) => report,
            Err(error) => {
                return Ok(format!(
                "post-rewrite hook completed rewrite finalization but retry replay failed ({error})"
            ));
            }
        };

    match outcome {
        PostRewriteFinalization::NoOp(reason) => Ok(format!(
            "post-rewrite hook executed with no-op runtime state: {reason:?}"
        )),
        PostRewriteFinalization::Ingested(ingested) => Ok(format!(
            "post-rewrite hook ingested {} pair(s), skipped {} duplicate pair(s), method='{}', rewrite_traces=(persisted={}, queued={}, no_op={}, failed={}) {}.",
            ingested.ingested_pairs,
            ingested.skipped_pairs,
            ingested.rewrite_method.canonical_label(),
            rewrite_persisted,
            rewrite_queued,
            rewrite_noops,
            rewrite_failures,
            retry_report.summary_text()
        )),
    }
}

#[derive(Default)]
struct InMemoryRetryMetricsSink {
    events: Vec<RetryProcessingMetric>,
}

impl RetryMetricsSink for InMemoryRetryMetricsSink {
    fn record_retry_metric(&mut self, metric: RetryProcessingMetric) {
        self.events.push(metric);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RuntimeRetryReport {
    summary: RetryQueueProcessSummary,
    transient_failures: usize,
    permanent_failures: usize,
}

impl RuntimeRetryReport {
    fn summary_text(&self) -> String {
        format!(
            "retry_queue=(attempted={}, recovered={}, requeued={}, transient_failures={}, permanent_failures={})",
            self.summary.attempted,
            self.summary.recovered,
            self.summary.requeued,
            self.transient_failures,
            self.permanent_failures
        )
    }
}

fn process_runtime_retry_queue(
    retry_queue: &mut impl TraceRetryQueue,
    notes_writer: &mut impl TraceNotesWriter,
    record_store: &mut impl TraceRecordStore,
) -> Result<RuntimeRetryReport> {
    let mut metrics_sink = InMemoryRetryMetricsSink::default();
    let summary = process_trace_retry_queue(
        retry_queue,
        notes_writer,
        record_store,
        &mut metrics_sink,
        RETRY_QUEUE_MAX_ITEMS_PER_RUN,
    )?;

    let mut transient_failures = 0_usize;
    let mut permanent_failures = 0_usize;

    for metric in metrics_sink.events {
        match metric.error_class {
            Some(PersistenceErrorClass::Transient) => transient_failures += 1,
            Some(PersistenceErrorClass::Permanent) => permanent_failures += 1,
            None => {}
        }
    }

    Ok(RuntimeRetryReport {
        summary,
        transient_failures,
        permanent_failures,
    })
}

fn resolve_post_rewrite_runtime_state(repository_root: &Path) -> PostRewriteRuntimeState {
    PostRewriteRuntimeState {
        sce_disabled: env_flag_is_truthy("SCE_DISABLED"),
        cli_available: git_command_success(repository_root, &["--version"]),
        is_bare_repo: git_command_output(repository_root, &["rev-parse", "--is-bare-repository"])
            .is_some_and(|output| output == "true"),
    }
}

fn build_rewrite_trace_input(
    repository_root: &Path,
    request: &RewriteRemapRequest,
) -> Result<RewriteTraceInput> {
    let timestamp_rfc3339 = run_git_command(
        repository_root,
        &["show", "-s", "--format=%cI", request.new_sha.as_str()],
        "Failed to resolve rewritten commit timestamp.",
    )?;
    let files = collect_commit_file_attribution(
        repository_root,
        request.new_sha.as_str(),
        "https://crocoder.dev/sce/local-hooks/post-rewrite",
    )?;

    Ok(RewriteTraceInput {
        record_id: deterministic_uuid_v4_from_seed(&format!(
            "{}:{}",
            request.idempotency_key, timestamp_rfc3339
        )),
        timestamp_rfc3339,
        rewritten_commit_sha: request.new_sha.clone(),
        rewrite_from_sha: request.old_sha.clone(),
        rewrite_method: request.rewrite_method.clone(),
        rewrite_confidence: 1.0,
        idempotency_key: request.idempotency_key.clone(),
        files,
    })
}

struct LocalDbRewriteRemapIngestion {
    repository_root: PathBuf,
    db_path: PathBuf,
    accepted_requests: Vec<RewriteRemapRequest>,
}

impl RewriteRemapIngestion for LocalDbRewriteRemapIngestion {
    fn ingest(&mut self, request: RewriteRemapRequest) -> Result<bool> {
        let runtime = tokio::runtime::Builder::new_current_thread().build()?;
        let accepted = runtime.block_on(ingest_rewrite_mapping_to_local_db(
            &self.repository_root,
            &self.db_path,
            &request,
        ))?;
        if accepted {
            self.accepted_requests.push(request);
        }
        Ok(accepted)
    }
}

async fn ingest_rewrite_mapping_to_local_db(
    repository_root: &Path,
    db_path: &Path,
    request: &RewriteRemapRequest,
) -> Result<bool> {
    let location = db_path.to_str().ok_or_else(|| {
        anyhow::anyhow!("Local DB path must be valid UTF-8: {}", db_path.display())
    })?;
    let db = turso::Builder::new_local(location).build().await?;
    let conn = db.connect()?;
    conn.execute("PRAGMA foreign_keys = ON", ()).await?;

    let canonical_root = repository_root
        .canonicalize()
        .unwrap_or_else(|_| repository_root.to_path_buf())
        .to_string_lossy()
        .to_string();

    conn.execute(
        "INSERT OR IGNORE INTO repositories (canonical_root) VALUES (?1)",
        [canonical_root.as_str()],
    )
    .await?;

    let repository_id = {
        let mut rows = conn
            .query(
                "SELECT id FROM repositories WHERE canonical_root = ?1 LIMIT 1",
                [canonical_root.as_str()],
            )
            .await?;
        let row = rows
            .next()
            .await?
            .ok_or_else(|| anyhow::anyhow!("repository id query returned no rows"))?;
        let value = row.get_value(0)?;
        value
            .as_integer()
            .copied()
            .ok_or_else(|| anyhow::anyhow!("repository id query returned non-integer"))?
    };

    let existing = {
        let mut rows = conn
            .query(
                "SELECT COUNT(*) FROM rewrite_mappings WHERE repository_id = ?1 AND idempotency_key = ?2",
                (repository_id, request.idempotency_key.as_str()),
            )
            .await?;
        let row = rows
            .next()
            .await?
            .ok_or_else(|| anyhow::anyhow!("rewrite mapping count query returned no rows"))?;
        let value = row.get_value(0)?;
        value
            .as_integer()
            .copied()
            .ok_or_else(|| anyhow::anyhow!("rewrite mapping count query returned non-integer"))?
    };
    if existing > 0 {
        return Ok(false);
    }

    let reconciliation_key = format!(
        "local-post-rewrite:{}:{}",
        request.rewrite_method.canonical_label(),
        request.new_sha
    );
    conn.execute(
        "INSERT OR IGNORE INTO reconciliation_runs (repository_id, provider, idempotency_key, status) VALUES (?1, ?2, ?3, ?4)",
        (repository_id, "local-hook", reconciliation_key.as_str(), "completed"),
    )
    .await?;

    let run_id = {
        let mut rows = conn
            .query(
                "SELECT id FROM reconciliation_runs WHERE repository_id = ?1 AND idempotency_key = ?2 LIMIT 1",
                (repository_id, reconciliation_key.as_str()),
            )
            .await?;
        let row = rows
            .next()
            .await?
            .ok_or_else(|| anyhow::anyhow!("reconciliation run id query returned no rows"))?;
        let value = row.get_value(0)?;
        value
            .as_integer()
            .copied()
            .ok_or_else(|| anyhow::anyhow!("reconciliation run id query returned non-integer"))?
    };

    conn.execute(
        "INSERT INTO rewrite_mappings (reconciliation_run_id, repository_id, old_commit_sha, new_commit_sha, mapping_status, confidence, idempotency_key) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        (
            run_id,
            repository_id,
            request.old_sha.as_str(),
            request.new_sha.as_str(),
            "mapped",
            1.0_f64,
            request.idempotency_key.as_str(),
        ),
    )
    .await?;

    Ok(true)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreCommitRuntimeState {
    pub sce_disabled: bool,
    pub cli_available: bool,
    pub is_bare_repo: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreCommitTreeAnchors {
    pub index_tree: String,
    pub head_tree: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingLineRange {
    pub start_line: u32,
    pub end_line: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingFileCheckpoint {
    pub path: String,
    pub staged_ranges: Vec<PendingLineRange>,
    pub unstaged_ranges: Vec<PendingLineRange>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingCheckpoint {
    pub files: Vec<PendingFileCheckpoint>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinalizedFileCheckpoint {
    pub path: String,
    pub ranges: Vec<PendingLineRange>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinalizedCheckpoint {
    pub anchors: PreCommitTreeAnchors,
    pub files: Vec<FinalizedFileCheckpoint>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PreCommitNoOpReason {
    Disabled,
    CliUnavailable,
    BareRepository,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PreCommitFinalization {
    NoOp(PreCommitNoOpReason),
    Finalized(FinalizedCheckpoint),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitMsgRuntimeState {
    pub sce_disabled: bool,
    pub sce_coauthor_enabled: bool,
    pub has_staged_sce_attribution: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PostCommitRuntimeState {
    pub sce_disabled: bool,
    pub cli_available: bool,
    pub is_bare_repo: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PostCommitInput {
    pub record_id: String,
    pub timestamp_rfc3339: String,
    pub commit_sha: String,
    pub parent_sha: Option<String>,
    pub idempotency_key: String,
    pub files: Vec<FileAttributionInput>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceNote {
    pub notes_ref: String,
    pub commit_sha: String,
    pub content_type: String,
    pub record: AgentTraceRecord,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PersistedTraceRecord {
    pub commit_sha: String,
    pub idempotency_key: String,
    pub content_type: String,
    pub notes_ref: String,
    pub record: AgentTraceRecord,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PersistenceErrorClass {
    Transient,
    Permanent,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PersistenceFailure {
    pub class: PersistenceErrorClass,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PersistenceWriteResult {
    Written,
    AlreadyExists,
    Failed(PersistenceFailure),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PersistenceTarget {
    Notes,
    Database,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceRetryQueueEntry {
    pub commit_sha: String,
    pub failed_targets: Vec<PersistenceTarget>,
    pub content_type: String,
    pub notes_ref: String,
    pub record: AgentTraceRecord,
}

pub trait TraceNotesWriter {
    fn write_note(&mut self, note: TraceNote) -> PersistenceWriteResult;
}

pub trait TraceRecordStore {
    fn write_trace_record(&mut self, record: PersistedTraceRecord) -> PersistenceWriteResult;
}

pub trait TraceRetryQueue {
    fn enqueue(&mut self, entry: TraceRetryQueueEntry) -> Result<()>;
    fn dequeue_next(&mut self) -> Result<Option<TraceRetryQueueEntry>>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RetryProcessingMetric {
    pub commit_sha: String,
    pub trace_id: String,
    pub runtime_ms: u128,
    pub error_class: Option<PersistenceErrorClass>,
    pub failed_targets: Vec<PersistenceTarget>,
}

pub trait RetryMetricsSink {
    fn record_retry_metric(&mut self, metric: RetryProcessingMetric);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RetryQueueProcessSummary {
    pub attempted: usize,
    pub recovered: usize,
    pub requeued: usize,
}

pub trait TraceEmissionLedger {
    fn has_emitted(&self, commit_sha: &str) -> bool;
    fn mark_emitted(&mut self, commit_sha: &str);
}

struct GitNotesTraceWriter {
    repository_root: PathBuf,
}

impl TraceNotesWriter for GitNotesTraceWriter {
    fn write_note(&mut self, note: TraceNote) -> PersistenceWriteResult {
        let payload = match serialize_note_payload(&note) {
            Ok(payload) => payload,
            Err(error) => {
                return PersistenceWriteResult::Failed(PersistenceFailure {
                    class: PersistenceErrorClass::Permanent,
                    message: format!("failed to serialize trace note payload: {error}"),
                });
            }
        };

        let existing = Command::new("git")
            .args([
                "notes",
                "--ref",
                note.notes_ref.as_str(),
                "show",
                note.commit_sha.as_str(),
            ])
            .current_dir(&self.repository_root)
            .output();
        if let Ok(output) = &existing {
            if output.status.success() {
                let existing_payload = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if existing_payload == payload {
                    return PersistenceWriteResult::AlreadyExists;
                }
            }
        }

        match Command::new("git")
            .args([
                "notes",
                "--ref",
                note.notes_ref.as_str(),
                "add",
                "-f",
                "-m",
                payload.as_str(),
                note.commit_sha.as_str(),
            ])
            .current_dir(&self.repository_root)
            .output()
        {
            Ok(output) if output.status.success() => PersistenceWriteResult::Written,
            Ok(output) => PersistenceWriteResult::Failed(PersistenceFailure {
                class: classify_persistence_error_class_from_stderr(&String::from_utf8_lossy(
                    &output.stderr,
                )),
                message: format!(
                    "failed to write git note for commit '{}': {}",
                    note.commit_sha,
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
            }),
            Err(error) => PersistenceWriteResult::Failed(PersistenceFailure {
                class: classify_persistence_error_class_from_io(&error),
                message: format!(
                    "failed to execute git notes command for commit '{}': {}",
                    note.commit_sha, error
                ),
            }),
        }
    }
}

struct LocalDbTraceRecordStore {
    repository_root: PathBuf,
    db_path: PathBuf,
}

impl TraceRecordStore for LocalDbTraceRecordStore {
    fn write_trace_record(&mut self, record: PersistedTraceRecord) -> PersistenceWriteResult {
        let runtime = match tokio::runtime::Builder::new_current_thread().build() {
            Ok(runtime) => runtime,
            Err(error) => {
                return PersistenceWriteResult::Failed(PersistenceFailure {
                    class: PersistenceErrorClass::Permanent,
                    message: format!("failed to initialize local DB runtime: {error}"),
                })
            }
        };

        match runtime.block_on(write_trace_record_to_local_db(
            &self.db_path,
            &self.repository_root,
            &record,
        )) {
            Ok(written) => {
                if written {
                    PersistenceWriteResult::Written
                } else {
                    PersistenceWriteResult::AlreadyExists
                }
            }
            Err(error) => PersistenceWriteResult::Failed(PersistenceFailure {
                class: classify_persistence_error_class_from_message(&error.to_string()),
                message: format!(
                    "failed to persist trace record in local DB '{}': {error}",
                    self.db_path.display()
                ),
            }),
        }
    }
}

async fn write_trace_record_to_local_db(
    db_path: &Path,
    repository_root: &Path,
    record: &PersistedTraceRecord,
) -> Result<bool> {
    let location = db_path.to_str().ok_or_else(|| {
        anyhow::anyhow!("Local DB path must be valid UTF-8: {}", db_path.display())
    })?;
    let db = turso::Builder::new_local(location).build().await?;
    let conn = db.connect()?;
    conn.execute("PRAGMA foreign_keys = ON", ()).await?;

    let canonical_root = repository_root
        .canonicalize()
        .unwrap_or_else(|_| repository_root.to_path_buf())
        .to_string_lossy()
        .to_string();

    conn.execute(
        "INSERT OR IGNORE INTO repositories (canonical_root) VALUES (?1)",
        [canonical_root.as_str()],
    )
    .await?;

    let repository_id = {
        let mut rows = conn
            .query(
                "SELECT id FROM repositories WHERE canonical_root = ?1 LIMIT 1",
                [canonical_root.as_str()],
            )
            .await?;
        let row = rows
            .next()
            .await?
            .ok_or_else(|| anyhow::anyhow!("repository id query returned no rows"))?;
        let value = row.get_value(0)?;
        value
            .as_integer()
            .copied()
            .ok_or_else(|| anyhow::anyhow!("repository id query returned non-integer"))?
    };

    conn.execute(
        "INSERT OR IGNORE INTO commits (repository_id, commit_sha, idempotency_key) VALUES (?1, ?2, ?3)",
        (
            repository_id,
            record.commit_sha.as_str(),
            record.idempotency_key.as_str(),
        ),
    )
    .await?;

    let commit_id = {
        let mut rows = conn
            .query(
                "SELECT id FROM commits WHERE repository_id = ?1 AND commit_sha = ?2 LIMIT 1",
                (repository_id, record.commit_sha.as_str()),
            )
            .await?;
        let row = rows
            .next()
            .await?
            .ok_or_else(|| anyhow::anyhow!("commit id query returned no rows"))?;
        let value = row.get_value(0)?;
        value
            .as_integer()
            .copied()
            .ok_or_else(|| anyhow::anyhow!("commit id query returned non-integer"))?
    };

    let existing = {
        let mut rows = conn
            .query(
                "SELECT COUNT(*) FROM trace_records WHERE repository_id = ?1 AND (commit_id = ?2 OR idempotency_key = ?3)",
                (repository_id, commit_id, record.idempotency_key.as_str()),
            )
            .await?;
        let row = rows
            .next()
            .await?
            .ok_or_else(|| anyhow::anyhow!("existing trace count query returned no rows"))?;
        let value = row.get_value(0)?;
        value
            .as_integer()
            .copied()
            .ok_or_else(|| anyhow::anyhow!("existing trace count query returned non-integer"))?
    };
    if existing > 0 {
        return Ok(false);
    }

    let payload_json = serde_json::to_string(&trace_record_to_json(&record.record))
        .context("failed to serialize trace record JSON payload")?;
    let quality_status = record
        .record
        .metadata
        .get(METADATA_QUALITY_STATUS)
        .cloned()
        .unwrap_or_else(|| "final".to_string());

    conn.execute(
        "INSERT INTO trace_records (repository_id, commit_id, trace_id, version, content_type, notes_ref, payload_json, quality_status, idempotency_key, recorded_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        (
            repository_id,
            commit_id,
            record.record.id.as_str(),
            record.record.version.as_str(),
            record.content_type.as_str(),
            record.notes_ref.as_str(),
            payload_json.as_str(),
            quality_status.as_str(),
            record.idempotency_key.as_str(),
            record.record.timestamp.as_str(),
        ),
    )
    .await?;

    let trace_record_id = {
        let mut rows = conn
            .query(
                "SELECT id FROM trace_records WHERE trace_id = ?1 LIMIT 1",
                [record.record.id.as_str()],
            )
            .await?;
        let row = rows
            .next()
            .await?
            .ok_or_else(|| anyhow::anyhow!("trace record id query returned no rows"))?;
        let value = row.get_value(0)?;
        value
            .as_integer()
            .copied()
            .ok_or_else(|| anyhow::anyhow!("trace record id query returned non-integer"))?
    };

    for file in &record.record.files {
        for conversation in &file.conversations {
            for range in &conversation.ranges {
                conn.execute(
                    "INSERT INTO trace_ranges (trace_record_id, file_path, conversation_url, start_line, end_line, contributor_type, contributor_model_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    (
                        trace_record_id,
                        file.path.as_str(),
                        conversation.url.as_str(),
                        i64::from(range.start_line),
                        i64::from(range.end_line),
                        range.contributor.r#type.as_str(),
                        range.contributor.model_id.as_deref(),
                    ),
                )
                .await?;
            }
        }
    }

    Ok(true)
}

struct JsonFileTraceRetryQueue {
    path: PathBuf,
}

impl TraceRetryQueue for JsonFileTraceRetryQueue {
    fn enqueue(&mut self, entry: TraceRetryQueueEntry) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "Failed to create retry queue directory '{}'",
                    parent.display()
                )
            })?;
        }

        let line = serde_json::json!({
            "commit_sha": entry.commit_sha,
            "failed_targets": entry
                .failed_targets
                .iter()
                .map(persistence_target_label)
                .collect::<Vec<_>>(),
            "content_type": entry.content_type,
            "notes_ref": entry.notes_ref,
            "record": trace_record_to_json(&entry.record),
        })
        .to_string();
        append_jsonl_line(&self.path, &line)?;

        Ok(())
    }

    fn dequeue_next(&mut self) -> Result<Option<TraceRetryQueueEntry>> {
        if !self.path.exists() {
            return Ok(None);
        }

        let payload = fs::read_to_string(&self.path).with_context(|| {
            format!(
                "Failed to read retry queue file '{}' for dequeue.",
                self.path.display()
            )
        })?;

        let mut lines = payload.lines();
        let Some(first_line) = lines.next() else {
            return Ok(None);
        };

        let mut remaining = String::new();
        for line in lines {
            remaining.push_str(line);
            remaining.push('\n');
        }
        fs::write(&self.path, remaining).with_context(|| {
            format!(
                "Failed to rewrite retry queue file '{}' after dequeue.",
                self.path.display()
            )
        })?;

        let parsed = serde_json::from_str::<serde_json::Value>(first_line)
            .context("Failed to parse retry queue entry JSON during dequeue")?;
        let commit_sha = parsed
            .get("commit_sha")
            .and_then(serde_json::Value::as_str)
            .context("Retry queue entry missing 'commit_sha' string")?
            .to_string();
        let content_type = parsed
            .get("content_type")
            .and_then(serde_json::Value::as_str)
            .context("Retry queue entry missing 'content_type' string")?
            .to_string();
        let notes_ref = parsed
            .get("notes_ref")
            .and_then(serde_json::Value::as_str)
            .context("Retry queue entry missing 'notes_ref' string")?
            .to_string();
        let record = trace_record_from_json(
            parsed
                .get("record")
                .context("Retry queue entry missing 'record' object")?,
        )?;

        let failed_targets = parsed
            .get("failed_targets")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|value| value.as_str())
            .filter_map(persistence_target_from_label)
            .collect::<Vec<_>>();

        Ok(Some(TraceRetryQueueEntry {
            commit_sha,
            failed_targets,
            content_type,
            notes_ref,
            record,
        }))
    }
}

struct FileTraceEmissionLedger {
    path: PathBuf,
}

impl TraceEmissionLedger for FileTraceEmissionLedger {
    fn has_emitted(&self, commit_sha: &str) -> bool {
        fs::read_to_string(&self.path)
            .ok()
            .is_some_and(|contents| contents.lines().any(|line| line.trim() == commit_sha))
    }

    fn mark_emitted(&mut self, commit_sha: &str) {
        if self.has_emitted(commit_sha) {
            return;
        }

        if let Some(parent) = self.path.parent() {
            if fs::create_dir_all(parent).is_err() {
                return;
            }
        }

        if let Ok(mut file) = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            let _ = writeln!(file, "{commit_sha}");
        }
    }
}

fn append_jsonl_line(path: &Path, line: &str) -> std::io::Result<()> {
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(file, "{line}")?;
    Ok(())
}

fn classify_persistence_error_class_from_io(error: &std::io::Error) -> PersistenceErrorClass {
    match error.kind() {
        std::io::ErrorKind::Interrupted
        | std::io::ErrorKind::WouldBlock
        | std::io::ErrorKind::TimedOut
        | std::io::ErrorKind::ConnectionRefused
        | std::io::ErrorKind::ConnectionReset
        | std::io::ErrorKind::ConnectionAborted
        | std::io::ErrorKind::NotConnected => PersistenceErrorClass::Transient,
        _ => PersistenceErrorClass::Permanent,
    }
}

fn classify_persistence_error_class_from_stderr(stderr: &str) -> PersistenceErrorClass {
    let lowered = stderr.to_ascii_lowercase();
    if lowered.contains("timed out")
        || lowered.contains("temporar")
        || lowered.contains("try again")
        || lowered.contains("index.lock")
    {
        return PersistenceErrorClass::Transient;
    }

    PersistenceErrorClass::Permanent
}

fn classify_persistence_error_class_from_message(message: &str) -> PersistenceErrorClass {
    let lowered = message.to_ascii_lowercase();
    if lowered.contains("locked")
        || lowered.contains("timed out")
        || lowered.contains("temporar")
        || lowered.contains("try again")
    {
        return PersistenceErrorClass::Transient;
    }

    PersistenceErrorClass::Permanent
}

fn persistence_target_label(target: &PersistenceTarget) -> &'static str {
    match target {
        PersistenceTarget::Notes => "notes",
        PersistenceTarget::Database => "database",
    }
}

fn persistence_target_from_label(label: &str) -> Option<PersistenceTarget> {
    match label {
        "notes" => Some(PersistenceTarget::Notes),
        "database" => Some(PersistenceTarget::Database),
        _ => None,
    }
}

fn serialize_note_payload(note: &TraceNote) -> Result<String> {
    serde_json::to_string_pretty(&serde_json::json!({
        "content_type": note.content_type,
        "record": trace_record_to_json(&note.record),
    }))
    .context("Failed to serialize trace note payload")
}

fn trace_record_to_json(record: &AgentTraceRecord) -> serde_json::Value {
    serde_json::json!({
        "version": record.version,
        "id": record.id,
        "timestamp": record.timestamp,
        "vcs": {
            "type": record.vcs.r#type,
            "revision": record.vcs.revision,
        },
        "files": record.files.iter().map(|file| {
            serde_json::json!({
                "path": file.path,
                "conversations": file.conversations.iter().map(|conversation| {
                    serde_json::json!({
                        "url": conversation.url,
                        "related": conversation.related,
                        "ranges": conversation.ranges.iter().map(|range| {
                            serde_json::json!({
                                "start_line": range.start_line,
                                "end_line": range.end_line,
                                "contributor": {
                                    "type": range.contributor.r#type,
                                    "model_id": range.contributor.model_id,
                                },
                            })
                        }).collect::<Vec<_>>(),
                    })
                }).collect::<Vec<_>>(),
            })
        }).collect::<Vec<_>>(),
        "metadata": record.metadata,
    })
}

fn trace_record_from_json(value: &serde_json::Value) -> Result<AgentTraceRecord> {
    let version = value
        .get("version")
        .and_then(serde_json::Value::as_str)
        .unwrap_or(TRACE_VERSION)
        .to_string();
    let id = value
        .get("id")
        .and_then(serde_json::Value::as_str)
        .context("trace record JSON missing id")?
        .to_string();
    let timestamp = value
        .get("timestamp")
        .and_then(serde_json::Value::as_str)
        .context("trace record JSON missing timestamp")?
        .to_string();

    let vcs = value
        .get("vcs")
        .and_then(serde_json::Value::as_object)
        .context("trace record JSON missing vcs object")?;
    let vcs_type = vcs
        .get("type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or(VCS_TYPE_GIT)
        .to_string();
    let vcs_revision = vcs
        .get("revision")
        .and_then(serde_json::Value::as_str)
        .context("trace record JSON missing vcs.revision")?
        .to_string();

    let files_json = value
        .get("files")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut files = Vec::new();
    for file in files_json {
        let Some(path) = file.get("path").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let mut conversations = Vec::new();
        for conversation in file
            .get("conversations")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(url) = conversation.get("url").and_then(serde_json::Value::as_str) else {
                continue;
            };
            let related = conversation
                .get("related")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|value| value.as_str().map(ToString::to_string))
                .collect::<Vec<_>>();
            let mut ranges = Vec::new();
            for range in conversation
                .get("ranges")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
            {
                let Some(start_line) = range
                    .get("start_line")
                    .and_then(serde_json::Value::as_u64)
                    .map(|value| value as u32)
                else {
                    continue;
                };
                let Some(end_line) = range
                    .get("end_line")
                    .and_then(serde_json::Value::as_u64)
                    .map(|value| value as u32)
                else {
                    continue;
                };
                let contributor = range
                    .get("contributor")
                    .and_then(serde_json::Value::as_object)
                    .cloned()
                    .unwrap_or_default();
                ranges.push(AgentTraceRange {
                    start_line,
                    end_line,
                    contributor: AgentTraceContributor {
                        r#type: contributor
                            .get("type")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("unknown")
                            .to_string(),
                        model_id: contributor
                            .get("model_id")
                            .and_then(serde_json::Value::as_str)
                            .map(ToString::to_string),
                    },
                });
            }

            conversations.push(AgentTraceConversation {
                url: url.to_string(),
                related,
                ranges,
            });
        }

        files.push(AgentTraceFile {
            path: path.to_string(),
            conversations,
        });
    }

    let metadata = value
        .get("metadata")
        .and_then(serde_json::Value::as_object)
        .map(|map| {
            map.iter()
                .filter_map(|(key, value)| {
                    value.as_str().map(|value| (key.clone(), value.to_string()))
                })
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();

    Ok(AgentTraceRecord {
        version,
        id,
        timestamp,
        vcs: AgentTraceVcs {
            r#type: vcs_type,
            revision: vcs_revision,
        },
        files,
        metadata,
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PostCommitNoOpReason {
    Disabled,
    CliUnavailable,
    BareRepository,
    AlreadyFinalized,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PostCommitPersisted {
    pub commit_sha: String,
    pub notes: PersistenceWriteResult,
    pub database: PersistenceWriteResult,
    pub trace_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PostCommitQueuedFallback {
    pub commit_sha: String,
    pub failed_targets: Vec<PersistenceTarget>,
    pub trace_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PostCommitFinalization {
    NoOp(PostCommitNoOpReason),
    Persisted(PostCommitPersisted),
    QueuedFallback(PostCommitQueuedFallback),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PostRewriteRuntimeState {
    pub sce_disabled: bool,
    pub cli_available: bool,
    pub is_bare_repo: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RewriteTraceInput {
    pub record_id: String,
    pub timestamp_rfc3339: String,
    pub rewritten_commit_sha: String,
    pub rewrite_from_sha: String,
    pub rewrite_method: RewriteMethod,
    pub rewrite_confidence: f32,
    pub idempotency_key: String,
    pub files: Vec<FileAttributionInput>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RewriteTraceNoOpReason {
    Disabled,
    CliUnavailable,
    BareRepository,
    AlreadyFinalized,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RewriteTracePersisted {
    pub commit_sha: String,
    pub trace_id: String,
    pub quality_status: QualityStatus,
    pub notes: PersistenceWriteResult,
    pub database: PersistenceWriteResult,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RewriteTraceQueuedFallback {
    pub commit_sha: String,
    pub trace_id: String,
    pub quality_status: QualityStatus,
    pub failed_targets: Vec<PersistenceTarget>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RewriteTraceFinalization {
    NoOp(RewriteTraceNoOpReason),
    Persisted(RewriteTracePersisted),
    QueuedFallback(RewriteTraceQueuedFallback),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PostRewriteNoOpReason {
    Disabled,
    CliUnavailable,
    BareRepository,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RewriteMethod {
    Amend,
    Rebase,
    Other(String),
}

impl RewriteMethod {
    fn canonical_label(&self) -> &str {
        match self {
            RewriteMethod::Amend => "amend",
            RewriteMethod::Rebase => "rebase",
            RewriteMethod::Other(method) => method.as_str(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RewritePair {
    pub old_sha: String,
    pub new_sha: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RewriteRemapRequest {
    pub rewrite_method: RewriteMethod,
    pub old_sha: String,
    pub new_sha: String,
    pub idempotency_key: String,
}

pub trait RewriteRemapIngestion {
    fn ingest(&mut self, request: RewriteRemapRequest) -> Result<bool>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PostRewriteIngested {
    pub rewrite_method: RewriteMethod,
    pub total_pairs: usize,
    pub ingested_pairs: usize,
    pub skipped_pairs: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PostRewriteFinalization {
    NoOp(PostRewriteNoOpReason),
    Ingested(PostRewriteIngested),
}

pub fn finalize_post_rewrite_remap(
    runtime: &PostRewriteRuntimeState,
    rewrite_method: &str,
    pairs_file_contents: &str,
    remap_ingestion: &mut impl RewriteRemapIngestion,
) -> Result<PostRewriteFinalization> {
    if runtime.sce_disabled {
        return Ok(PostRewriteFinalization::NoOp(
            PostRewriteNoOpReason::Disabled,
        ));
    }

    if !runtime.cli_available {
        return Ok(PostRewriteFinalization::NoOp(
            PostRewriteNoOpReason::CliUnavailable,
        ));
    }

    if runtime.is_bare_repo {
        return Ok(PostRewriteFinalization::NoOp(
            PostRewriteNoOpReason::BareRepository,
        ));
    }

    let method = normalize_rewrite_method(rewrite_method);
    let pairs = parse_post_rewrite_pairs(pairs_file_contents)?;

    let mut ingested_pairs = 0_usize;
    for pair in &pairs {
        let idempotency_key = format!(
            "post-rewrite:{}:{}:{}",
            method.canonical_label(),
            pair.old_sha,
            pair.new_sha
        );
        let accepted = remap_ingestion.ingest(RewriteRemapRequest {
            rewrite_method: method.clone(),
            old_sha: pair.old_sha.clone(),
            new_sha: pair.new_sha.clone(),
            idempotency_key,
        })?;
        if accepted {
            ingested_pairs += 1;
        }
    }

    let total_pairs = pairs.len();
    Ok(PostRewriteFinalization::Ingested(PostRewriteIngested {
        rewrite_method: method,
        total_pairs,
        ingested_pairs,
        skipped_pairs: total_pairs.saturating_sub(ingested_pairs),
    }))
}

pub fn finalize_rewrite_trace(
    runtime: &PostRewriteRuntimeState,
    input: RewriteTraceInput,
    notes_writer: &mut impl TraceNotesWriter,
    record_store: &mut impl TraceRecordStore,
    retry_queue: &mut impl TraceRetryQueue,
    emission_ledger: &mut impl TraceEmissionLedger,
) -> Result<RewriteTraceFinalization> {
    if runtime.sce_disabled {
        return Ok(RewriteTraceFinalization::NoOp(
            RewriteTraceNoOpReason::Disabled,
        ));
    }

    if !runtime.cli_available {
        return Ok(RewriteTraceFinalization::NoOp(
            RewriteTraceNoOpReason::CliUnavailable,
        ));
    }

    if runtime.is_bare_repo {
        return Ok(RewriteTraceFinalization::NoOp(
            RewriteTraceNoOpReason::BareRepository,
        ));
    }

    if emission_ledger.has_emitted(&input.rewritten_commit_sha) {
        return Ok(RewriteTraceFinalization::NoOp(
            RewriteTraceNoOpReason::AlreadyFinalized,
        ));
    }

    let confidence = normalize_rewrite_confidence(input.rewrite_confidence)?;
    let quality_status = quality_status_for_confidence(input.rewrite_confidence);
    let record = build_trace_payload(TraceAdapterInput {
        record_id: input.record_id,
        timestamp_rfc3339: input.timestamp_rfc3339,
        commit_sha: input.rewritten_commit_sha.clone(),
        files: input.files,
        quality_status,
        rewrite: Some(RewriteInfo {
            from_sha: input.rewrite_from_sha,
            method: input.rewrite_method.canonical_label().to_string(),
            confidence,
        }),
        idempotency_key: Some(input.idempotency_key.clone()),
    });

    let note = TraceNote {
        notes_ref: crate::services::agent_trace::NOTES_REF.to_string(),
        commit_sha: input.rewritten_commit_sha.clone(),
        content_type: TRACE_CONTENT_TYPE.to_string(),
        record: record.clone(),
    };
    let persisted = PersistedTraceRecord {
        commit_sha: input.rewritten_commit_sha.clone(),
        idempotency_key: input.idempotency_key,
        content_type: TRACE_CONTENT_TYPE.to_string(),
        notes_ref: crate::services::agent_trace::NOTES_REF.to_string(),
        record: record.clone(),
    };

    let notes_result = notes_writer.write_note(note);
    let database_result = record_store.write_trace_record(persisted);

    let failed_targets = collect_failed_targets(&notes_result, &database_result);
    if failed_targets.is_empty() {
        emission_ledger.mark_emitted(&input.rewritten_commit_sha);
        return Ok(RewriteTraceFinalization::Persisted(RewriteTracePersisted {
            commit_sha: input.rewritten_commit_sha,
            trace_id: record.id,
            quality_status,
            notes: notes_result,
            database: database_result,
        }));
    }

    retry_queue.enqueue(TraceRetryQueueEntry {
        commit_sha: input.rewritten_commit_sha.clone(),
        failed_targets: failed_targets.clone(),
        content_type: TRACE_CONTENT_TYPE.to_string(),
        notes_ref: crate::services::agent_trace::NOTES_REF.to_string(),
        record: record.clone(),
    })?;

    Ok(RewriteTraceFinalization::QueuedFallback(
        RewriteTraceQueuedFallback {
            commit_sha: input.rewritten_commit_sha,
            trace_id: record.id,
            quality_status,
            failed_targets,
        },
    ))
}

fn normalize_rewrite_confidence(confidence: f32) -> Result<String> {
    if !confidence.is_finite() {
        anyhow::bail!("rewrite confidence must be finite")
    }

    if !(0.0..=1.0).contains(&confidence) {
        anyhow::bail!("rewrite confidence must be within [0.0, 1.0]")
    }

    Ok(format!("{confidence:.2}"))
}

fn quality_status_for_confidence(confidence: f32) -> QualityStatus {
    if confidence >= 0.90 {
        return QualityStatus::Final;
    }

    if confidence >= 0.60 {
        return QualityStatus::Partial;
    }

    QualityStatus::NeedsReview
}

fn parse_post_rewrite_pairs(contents: &str) -> Result<Vec<RewritePair>> {
    let mut pairs = Vec::new();

    for (line_index, line) in contents.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let mut fields = trimmed.split_whitespace();
        let Some(old_sha) = fields.next() else {
            continue;
        };
        let Some(new_sha) = fields.next() else {
            anyhow::bail!(
                "Invalid post-rewrite pair format on line {}: expected '<old_sha> <new_sha>'",
                line_index + 1
            );
        };

        if fields.next().is_some() {
            anyhow::bail!(
                "Invalid post-rewrite pair format on line {}: expected exactly two fields",
                line_index + 1
            );
        }

        if old_sha == new_sha {
            continue;
        }

        pairs.push(RewritePair {
            old_sha: old_sha.to_string(),
            new_sha: new_sha.to_string(),
        });
    }

    Ok(pairs)
}

fn normalize_rewrite_method(method: &str) -> RewriteMethod {
    match method.trim().to_ascii_lowercase().as_str() {
        "amend" => RewriteMethod::Amend,
        "rebase" => RewriteMethod::Rebase,
        other => RewriteMethod::Other(other.to_string()),
    }
}

pub fn finalize_post_commit_trace(
    runtime: &PostCommitRuntimeState,
    input: PostCommitInput,
    notes_writer: &mut impl TraceNotesWriter,
    record_store: &mut impl TraceRecordStore,
    retry_queue: &mut impl TraceRetryQueue,
    emission_ledger: &mut impl TraceEmissionLedger,
) -> Result<PostCommitFinalization> {
    if runtime.sce_disabled {
        return Ok(PostCommitFinalization::NoOp(PostCommitNoOpReason::Disabled));
    }

    if !runtime.cli_available {
        return Ok(PostCommitFinalization::NoOp(
            PostCommitNoOpReason::CliUnavailable,
        ));
    }

    if runtime.is_bare_repo {
        return Ok(PostCommitFinalization::NoOp(
            PostCommitNoOpReason::BareRepository,
        ));
    }

    if emission_ledger.has_emitted(&input.commit_sha) {
        return Ok(PostCommitFinalization::NoOp(
            PostCommitNoOpReason::AlreadyFinalized,
        ));
    }

    let mut record = build_trace_payload(TraceAdapterInput {
        record_id: input.record_id,
        timestamp_rfc3339: input.timestamp_rfc3339,
        commit_sha: input.commit_sha.clone(),
        files: input.files,
        quality_status: QualityStatus::Final,
        rewrite: None,
        idempotency_key: Some(input.idempotency_key.clone()),
    });

    if let Some(parent_sha) = input.parent_sha {
        record
            .metadata
            .insert(POST_COMMIT_PARENT_SHA_METADATA_KEY.to_string(), parent_sha);
    }

    let note = TraceNote {
        notes_ref: crate::services::agent_trace::NOTES_REF.to_string(),
        commit_sha: input.commit_sha.clone(),
        content_type: TRACE_CONTENT_TYPE.to_string(),
        record: record.clone(),
    };
    let persisted = PersistedTraceRecord {
        commit_sha: input.commit_sha.clone(),
        idempotency_key: input.idempotency_key,
        content_type: TRACE_CONTENT_TYPE.to_string(),
        notes_ref: crate::services::agent_trace::NOTES_REF.to_string(),
        record: record.clone(),
    };

    let notes_result = notes_writer.write_note(note);
    let database_result = record_store.write_trace_record(persisted);

    let failed_targets = collect_failed_targets(&notes_result, &database_result);
    if failed_targets.is_empty() {
        emission_ledger.mark_emitted(&input.commit_sha);
        return Ok(PostCommitFinalization::Persisted(PostCommitPersisted {
            commit_sha: input.commit_sha,
            notes: notes_result,
            database: database_result,
            trace_id: record.id,
        }));
    }

    retry_queue.enqueue(TraceRetryQueueEntry {
        commit_sha: input.commit_sha.clone(),
        failed_targets: failed_targets.clone(),
        content_type: TRACE_CONTENT_TYPE.to_string(),
        notes_ref: crate::services::agent_trace::NOTES_REF.to_string(),
        record: record.clone(),
    })?;

    Ok(PostCommitFinalization::QueuedFallback(
        PostCommitQueuedFallback {
            commit_sha: input.commit_sha,
            failed_targets,
            trace_id: record.id,
        },
    ))
}

fn collect_failed_targets(
    notes_result: &PersistenceWriteResult,
    database_result: &PersistenceWriteResult,
) -> Vec<PersistenceTarget> {
    let mut failed_targets = Vec::new();
    if matches!(notes_result, PersistenceWriteResult::Failed(_)) {
        failed_targets.push(PersistenceTarget::Notes);
    }
    if matches!(database_result, PersistenceWriteResult::Failed(_)) {
        failed_targets.push(PersistenceTarget::Database);
    }
    failed_targets
}

pub fn process_trace_retry_queue(
    retry_queue: &mut impl TraceRetryQueue,
    notes_writer: &mut impl TraceNotesWriter,
    record_store: &mut impl TraceRecordStore,
    metrics_sink: &mut impl RetryMetricsSink,
    max_items: usize,
) -> Result<RetryQueueProcessSummary> {
    let mut processed_trace_ids = HashSet::new();
    let mut summary = RetryQueueProcessSummary {
        attempted: 0,
        recovered: 0,
        requeued: 0,
    };

    for _ in 0..max_items {
        let Some(entry) = retry_queue.dequeue_next()? else {
            break;
        };

        if !processed_trace_ids.insert(entry.record.id.clone()) {
            retry_queue.enqueue(entry)?;
            break;
        }

        summary.attempted += 1;
        let started = Instant::now();

        let notes_result = if entry.failed_targets.contains(&PersistenceTarget::Notes) {
            notes_writer.write_note(TraceNote {
                notes_ref: entry.notes_ref.clone(),
                commit_sha: entry.commit_sha.clone(),
                content_type: entry.content_type.clone(),
                record: entry.record.clone(),
            })
        } else {
            PersistenceWriteResult::AlreadyExists
        };

        let database_result = if entry.failed_targets.contains(&PersistenceTarget::Database) {
            let idempotency_key = entry
                .record
                .metadata
                .get(METADATA_IDEMPOTENCY_KEY)
                .cloned()
                .unwrap_or_else(|| format!("retry:{}:{}", entry.commit_sha, entry.record.id));
            record_store.write_trace_record(PersistedTraceRecord {
                commit_sha: entry.commit_sha.clone(),
                idempotency_key,
                content_type: entry.content_type.clone(),
                notes_ref: entry.notes_ref.clone(),
                record: entry.record.clone(),
            })
        } else {
            PersistenceWriteResult::AlreadyExists
        };

        let failed_targets = collect_failed_targets(&notes_result, &database_result);
        let error_class = first_failure_class(&notes_result, &database_result);

        metrics_sink.record_retry_metric(RetryProcessingMetric {
            commit_sha: entry.commit_sha.clone(),
            trace_id: entry.record.id.clone(),
            runtime_ms: started.elapsed().as_millis(),
            error_class,
            failed_targets: failed_targets.clone(),
        });

        if failed_targets.is_empty() {
            summary.recovered += 1;
            continue;
        }

        summary.requeued += 1;
        retry_queue.enqueue(TraceRetryQueueEntry {
            commit_sha: entry.commit_sha,
            failed_targets,
            content_type: entry.content_type,
            notes_ref: entry.notes_ref,
            record: entry.record,
        })?;
    }

    Ok(summary)
}

fn first_failure_class(
    notes_result: &PersistenceWriteResult,
    database_result: &PersistenceWriteResult,
) -> Option<PersistenceErrorClass> {
    match notes_result {
        PersistenceWriteResult::Failed(failure) => return Some(failure.class.clone()),
        PersistenceWriteResult::Written | PersistenceWriteResult::AlreadyExists => {}
    }

    match database_result {
        PersistenceWriteResult::Failed(failure) => Some(failure.class.clone()),
        PersistenceWriteResult::Written | PersistenceWriteResult::AlreadyExists => None,
    }
}

pub fn apply_commit_msg_coauthor_policy(
    runtime: &CommitMsgRuntimeState,
    commit_message: &str,
) -> String {
    if runtime.sce_disabled || !runtime.sce_coauthor_enabled || !runtime.has_staged_sce_attribution
    {
        return commit_message.to_string();
    }

    let mut lines: Vec<&str> = commit_message.lines().collect();
    lines.retain(|line| *line != CANONICAL_SCE_COAUTHOR_TRAILER);

    if !lines.is_empty() && !lines.last().is_some_and(|line| line.is_empty()) {
        lines.push("");
    }
    lines.push(CANONICAL_SCE_COAUTHOR_TRAILER);

    let mut normalized = lines.join("\n");
    if commit_message.ends_with('\n') {
        normalized.push('\n');
    }

    normalized
}

pub fn finalize_pre_commit_checkpoint(
    runtime: &PreCommitRuntimeState,
    anchors: PreCommitTreeAnchors,
    pending: PendingCheckpoint,
) -> PreCommitFinalization {
    if runtime.sce_disabled {
        return PreCommitFinalization::NoOp(PreCommitNoOpReason::Disabled);
    }

    if !runtime.cli_available {
        return PreCommitFinalization::NoOp(PreCommitNoOpReason::CliUnavailable);
    }

    if runtime.is_bare_repo {
        return PreCommitFinalization::NoOp(PreCommitNoOpReason::BareRepository);
    }

    let files = pending
        .files
        .into_iter()
        .filter_map(|file| {
            if file.staged_ranges.is_empty() {
                return None;
            }

            Some(FinalizedFileCheckpoint {
                path: file.path,
                ranges: file.staged_ranges,
            })
        })
        .collect();

    PreCommitFinalization::Finalized(FinalizedCheckpoint { anchors, files })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GitHookKind {
    PreCommit,
    PostCommit,
    PrePush,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GeneratedRegionLifecycle {
    Discovered,
    Updated,
    Removed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GeneratedRegionEvent {
    pub file_path: String,
    pub marker_id: String,
    pub lifecycle: GeneratedRegionLifecycle,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HookEvent {
    pub hook: GitHookKind,
    pub region_event: Option<GeneratedRegionEvent>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HookEventModel {
    pub supported_hooks: Vec<GitHookKind>,
    pub generated_region_tracking: bool,
}

pub trait HookService {
    fn event_model(&self) -> HookEventModel;
    fn record(&self, event: HookEvent) -> Result<()>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PlaceholderHookService;

impl HookService for PlaceholderHookService {
    fn event_model(&self) -> HookEventModel {
        HookEventModel {
            supported_hooks: vec![
                GitHookKind::PreCommit,
                GitHookKind::PostCommit,
                GitHookKind::PrePush,
            ],
            generated_region_tracking: true,
        }
    }

    fn record(&self, event: HookEvent) -> Result<()> {
        match event.hook {
            GitHookKind::PreCommit | GitHookKind::PostCommit | GitHookKind::PrePush => {}
        }

        if let Some(region_event) = event.region_event {
            match region_event.lifecycle {
                GeneratedRegionLifecycle::Discovered
                | GeneratedRegionLifecycle::Updated
                | GeneratedRegionLifecycle::Removed => {}
            }

            let _ = (region_event.file_path, region_event.marker_id);
        }

        Ok(())
    }
}

pub fn run_placeholder_hooks() -> Result<String> {
    let service = PlaceholderHookService;
    let model = service.event_model();

    let staged_only_preview = finalize_pre_commit_checkpoint(
        &PreCommitRuntimeState {
            sce_disabled: false,
            cli_available: true,
            is_bare_repo: false,
        },
        PreCommitTreeAnchors {
            index_tree: "placeholder-index-tree".to_string(),
            head_tree: Some("placeholder-head-tree".to_string()),
        },
        PendingCheckpoint {
            files: vec![PendingFileCheckpoint {
                path: "context/generated/hooks.md".to_string(),
                staged_ranges: vec![PendingLineRange {
                    start_line: 1,
                    end_line: 1,
                }],
                unstaged_ranges: vec![PendingLineRange {
                    start_line: 2,
                    end_line: 2,
                }],
            }],
        },
    );

    let staged_file_count = match staged_only_preview {
        PreCommitFinalization::Finalized(checkpoint) => checkpoint.files.len(),
        PreCommitFinalization::NoOp(_) => 0,
    };

    let commit_message_preview = apply_commit_msg_coauthor_policy(
        &CommitMsgRuntimeState {
            sce_disabled: false,
            sce_coauthor_enabled: true,
            has_staged_sce_attribution: true,
        },
        "chore: hooks placeholder preview",
    );
    let trailer_applied = commit_message_preview.contains(CANONICAL_SCE_COAUTHOR_TRAILER);

    for lifecycle in [
        GeneratedRegionLifecycle::Discovered,
        GeneratedRegionLifecycle::Updated,
        GeneratedRegionLifecycle::Removed,
    ] {
        service.record(HookEvent {
            hook: GitHookKind::PreCommit,
            region_event: Some(GeneratedRegionEvent {
                file_path: "context/generated/hooks.md".to_string(),
                marker_id: "placeholder-generated-region".to_string(),
                lifecycle,
            }),
        })?;
    }

    Ok(format!(
        "TODO: '{NAME}' is planned and not implemented yet. Hook event model reserves {} git hook(s) with generated-region tracking placeholders, staged-only pre-commit checkpoint preview over {} file(s), and commit-msg canonical trailer preview applied={}.",
        model.supported_hooks.len(),
        staged_file_count,
        trailer_applied
    ))
}

#[cfg(test)]
mod tests;
