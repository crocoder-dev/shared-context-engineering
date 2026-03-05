use anyhow::{bail, Context, Result};
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Instant;

use crate::services::agent_trace::{
    build_trace_payload, AgentTraceRecord, FileAttributionInput, QualityStatus, RewriteInfo,
    TraceAdapterInput, METADATA_IDEMPOTENCY_KEY, TRACE_CONTENT_TYPE,
};

pub const NAME: &str = "hooks";
pub const CANONICAL_SCE_COAUTHOR_TRAILER: &str = "Co-authored-by: SCE <sce@crocoder.dev>";
pub const POST_COMMIT_PARENT_SHA_METADATA_KEY: &str = "dev.crocoder.sce.parent_revision";

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
    let outcome = finalize_pre_commit_checkpoint(
        &PreCommitRuntimeState {
            sce_disabled: false,
            cli_available: true,
            is_bare_repo: false,
        },
        PreCommitTreeAnchors {
            index_tree: "pending-index-tree".to_string(),
            head_tree: None,
        },
        PendingCheckpoint { files: Vec::new() },
    );

    let message = match outcome {
        PreCommitFinalization::NoOp(reason) => {
            format!("pre-commit hook executed with no-op runtime state: {reason:?}")
        }
        PreCommitFinalization::Finalized(checkpoint) => format!(
            "pre-commit hook executed and finalized staged checkpoint for {} file(s).",
            checkpoint.files.len()
        ),
    };

    Ok(message)
}

fn run_commit_msg_subcommand(message_file: PathBuf) -> Result<String> {
    let metadata = fs::metadata(&message_file).with_context(|| {
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

    Ok(format!(
        "commit-msg hook accepted message file '{}'.",
        message_file.display()
    ))
}

fn run_post_commit_subcommand() -> Result<String> {
    Ok("post-commit hook accepted runtime invocation.".to_string())
}

fn run_post_rewrite_subcommand(rewrite_method: &str) -> Result<String> {
    let stdin = std::io::read_to_string(std::io::stdin())
        .context("Failed to read post-rewrite pair input from STDIN")?;
    let mut ingestion = AcceptAllRewriteRemapIngestion;
    let outcome = finalize_post_rewrite_remap(
        &PostRewriteRuntimeState {
            sce_disabled: false,
            cli_available: true,
            is_bare_repo: false,
        },
        rewrite_method,
        &stdin,
        &mut ingestion,
    )?;

    match outcome {
        PostRewriteFinalization::NoOp(reason) => Ok(format!(
            "post-rewrite hook executed with no-op runtime state: {reason:?}"
        )),
        PostRewriteFinalization::Ingested(ingested) => Ok(format!(
            "post-rewrite hook ingested {} pair(s), skipped {} duplicate pair(s), method='{}'.",
            ingested.ingested_pairs,
            ingested.skipped_pairs,
            ingested.rewrite_method.canonical_label()
        )),
    }
}

struct AcceptAllRewriteRemapIngestion;

impl RewriteRemapIngestion for AcceptAllRewriteRemapIngestion {
    fn ingest(&mut self, _request: RewriteRemapRequest) -> Result<bool> {
        Ok(true)
    }
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
