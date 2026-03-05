use anyhow::Result;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::services::agent_trace::{
    build_trace_payload, ContributorInput, ContributorType, ConversationInput,
    FileAttributionInput, QualityStatus, RangeInput, TraceAdapterInput, METADATA_QUALITY_STATUS,
    METADATA_REWRITE_CONFIDENCE, METADATA_REWRITE_FROM, METADATA_REWRITE_METHOD,
};
use crate::services::local_db::resolve_agent_trace_local_db_path;

use super::{
    apply_commit_msg_coauthor_policy, finalize_post_commit_trace, finalize_post_rewrite_remap,
    finalize_pre_commit_checkpoint, finalize_rewrite_trace, parse_hooks_subcommand,
    process_trace_retry_queue, resolve_pre_commit_checkpoint_path,
    run_commit_msg_subcommand_in_repo, run_hooks_subcommand, run_post_commit_subcommand_in_repo,
    run_post_rewrite_subcommand_in_repo, run_pre_commit_subcommand_in_repo, CommitMsgRuntimeState,
    HookSubcommand, PendingCheckpoint, PendingFileCheckpoint, PendingLineRange,
    PersistenceErrorClass, PersistenceFailure, PersistenceTarget, PersistenceWriteResult,
    PostCommitFinalization, PostCommitInput, PostCommitNoOpReason, PostCommitRuntimeState,
    PostRewriteFinalization, PostRewriteNoOpReason, PostRewriteRuntimeState, PreCommitFinalization,
    PreCommitNoOpReason, PreCommitRuntimeState, PreCommitTreeAnchors, RetryMetricsSink,
    RetryProcessingMetric, RewriteMethod, RewriteRemapIngestion, RewriteRemapRequest,
    RewriteTraceFinalization, RewriteTraceInput, RewriteTraceNoOpReason, TraceEmissionLedger,
    TraceNote, TraceNotesWriter, TraceRecordStore, TraceRetryQueue, TraceRetryQueueEntry,
    CANONICAL_SCE_COAUTHOR_TRAILER, POST_COMMIT_PARENT_SHA_METADATA_KEY,
};

fn run_git_in_repo(repo: &Path, args: &[&str]) -> Result<()> {
    let output = Command::new("git").args(args).current_dir(repo).output()?;
    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    anyhow::bail!(
        "git {:?} failed in '{}': {}",
        args,
        repo.display(),
        if stderr.is_empty() {
            "git command exited non-zero".to_string()
        } else {
            stderr
        }
    )
}

fn run_git_output_in_repo(repo: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git").args(args).current_dir(repo).output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        anyhow::bail!(
            "git {:?} failed in '{}': {}",
            args,
            repo.display(),
            if stderr.is_empty() {
                "git command exited non-zero".to_string()
            } else {
                stderr
            }
        );
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn create_temp_repo() -> Result<PathBuf> {
    let unique = format!(
        "sce-hooks-tests-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    );
    let repo = std::env::temp_dir().join(unique);
    fs::create_dir_all(&repo)?;
    run_git_in_repo(&repo, &["init"])?;
    run_git_in_repo(&repo, &["config", "user.name", "SCE Test"])?;
    run_git_in_repo(&repo, &["config", "user.email", "sce@example.test"])?;
    Ok(repo)
}

fn agent_trace_db_test_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn sample_pending_checkpoint() -> PendingCheckpoint {
    PendingCheckpoint {
        files: vec![PendingFileCheckpoint {
            path: "src/lib.rs".to_string(),
            staged_ranges: vec![PendingLineRange {
                start_line: 1,
                end_line: 3,
            }],
            unstaged_ranges: vec![PendingLineRange {
                start_line: 4,
                end_line: 6,
            }],
        }],
    }
}

fn sample_runtime() -> PreCommitRuntimeState {
    PreCommitRuntimeState {
        sce_disabled: false,
        cli_available: true,
        is_bare_repo: false,
    }
}

fn sample_anchors() -> PreCommitTreeAnchors {
    PreCommitTreeAnchors {
        index_tree: "index-tree-sha".to_string(),
        head_tree: Some("head-tree-sha".to_string()),
    }
}

#[derive(Default)]
struct FakeEmissionLedger {
    emitted: Vec<String>,
}

impl TraceEmissionLedger for FakeEmissionLedger {
    fn has_emitted(&self, commit_sha: &str) -> bool {
        self.emitted.iter().any(|sha| sha == commit_sha)
    }

    fn mark_emitted(&mut self, commit_sha: &str) {
        self.emitted.push(commit_sha.to_string());
    }
}

struct FakeNotesWriter {
    result: PersistenceWriteResult,
    writes: Vec<TraceNote>,
}

impl FakeNotesWriter {
    fn new(result: PersistenceWriteResult) -> Self {
        Self {
            result,
            writes: Vec::new(),
        }
    }
}

impl TraceNotesWriter for FakeNotesWriter {
    fn write_note(&mut self, note: TraceNote) -> PersistenceWriteResult {
        self.writes.push(note);
        self.result.clone()
    }
}

struct FakeRecordStore {
    result: PersistenceWriteResult,
}

impl FakeRecordStore {
    fn new(result: PersistenceWriteResult) -> Self {
        Self { result }
    }
}

impl TraceRecordStore for FakeRecordStore {
    fn write_trace_record(
        &mut self,
        _record: super::PersistedTraceRecord,
    ) -> PersistenceWriteResult {
        self.result.clone()
    }
}

#[derive(Default)]
struct FakeRetryQueue {
    entries: Vec<TraceRetryQueueEntry>,
}

#[derive(Default)]
struct FakeRetryMetricsSink {
    events: Vec<RetryProcessingMetric>,
}

#[derive(Default)]
struct FakeRewriteRemapIngestion {
    seen_requests: Vec<RewriteRemapRequest>,
    duplicate_keys: Vec<String>,
    seen_keys: std::collections::BTreeSet<String>,
}

impl RewriteRemapIngestion for FakeRewriteRemapIngestion {
    fn ingest(&mut self, request: RewriteRemapRequest) -> Result<bool> {
        let accepted = self.seen_keys.insert(request.idempotency_key.clone());
        if !accepted {
            self.duplicate_keys.push(request.idempotency_key.clone());
        }
        self.seen_requests.push(request);
        Ok(accepted)
    }
}

impl TraceRetryQueue for FakeRetryQueue {
    fn enqueue(&mut self, entry: TraceRetryQueueEntry) -> Result<()> {
        self.entries.push(entry);
        Ok(())
    }

    fn dequeue_next(&mut self) -> Result<Option<TraceRetryQueueEntry>> {
        if self.entries.is_empty() {
            return Ok(None);
        }

        Ok(Some(self.entries.remove(0)))
    }
}

impl RetryMetricsSink for FakeRetryMetricsSink {
    fn record_retry_metric(&mut self, metric: RetryProcessingMetric) {
        self.events.push(metric);
    }
}

fn sample_retry_entry_with_target(target: PersistenceTarget) -> TraceRetryQueueEntry {
    let record = build_trace_payload(TraceAdapterInput {
        record_id: "990e8400-e29b-41d4-a716-446655440000".to_string(),
        timestamp_rfc3339: "2026-03-04T12:13:14Z".to_string(),
        commit_sha: "retrysha123".to_string(),
        files: vec![FileAttributionInput {
            path: "src/retry.rs".to_string(),
            conversations: vec![ConversationInput {
                url: "https://example.test/conversation/retry".to_string(),
                related: vec![],
                ranges: vec![RangeInput {
                    start_line: 4,
                    end_line: 6,
                    contributor: ContributorInput {
                        kind: ContributorType::Ai,
                        model_id: Some("openai/gpt-5.3-codex".to_string()),
                    },
                }],
            }],
        }],
        quality_status: QualityStatus::Final,
        rewrite: None,
        idempotency_key: Some("retry:key:retrysha123".to_string()),
    });

    TraceRetryQueueEntry {
        commit_sha: "retrysha123".to_string(),
        failed_targets: vec![target],
        content_type: "application/vnd.agent-trace.record+json".to_string(),
        notes_ref: "refs/notes/agent-trace".to_string(),
        record,
    }
}

fn sample_post_commit_runtime() -> PostCommitRuntimeState {
    PostCommitRuntimeState {
        sce_disabled: false,
        cli_available: true,
        is_bare_repo: false,
    }
}

fn sample_post_rewrite_runtime() -> PostRewriteRuntimeState {
    PostRewriteRuntimeState {
        sce_disabled: false,
        cli_available: true,
        is_bare_repo: false,
    }
}

fn sample_rewrite_trace_input() -> RewriteTraceInput {
    RewriteTraceInput {
        record_id: "660e8400-e29b-41d4-a716-446655440000".to_string(),
        timestamp_rfc3339: "2026-03-04T11:12:13Z".to_string(),
        rewritten_commit_sha: "newsha123".to_string(),
        rewrite_from_sha: "oldsha456".to_string(),
        rewrite_method: RewriteMethod::Rebase,
        rewrite_confidence: 0.91,
        idempotency_key: "post-rewrite:rebase:oldsha456:newsha123".to_string(),
        files: vec![FileAttributionInput {
            path: "src/lib.rs".to_string(),
            conversations: vec![ConversationInput {
                url: "https://example.test/conversation/rewritten".to_string(),
                related: vec![],
                ranges: vec![RangeInput {
                    start_line: 3,
                    end_line: 7,
                    contributor: ContributorInput {
                        kind: ContributorType::Ai,
                        model_id: Some("openai/gpt-5.3-codex".to_string()),
                    },
                }],
            }],
        }],
    }
}

fn sample_post_commit_input() -> PostCommitInput {
    PostCommitInput {
        record_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
        timestamp_rfc3339: "2026-03-04T10:11:12Z".to_string(),
        commit_sha: "abc123def456".to_string(),
        parent_sha: Some("def789ghi000".to_string()),
        idempotency_key: "repo:abc123def456".to_string(),
        files: vec![FileAttributionInput {
            path: "src/lib.rs".to_string(),
            conversations: vec![ConversationInput {
                url: "https://example.test/conversation/1".to_string(),
                related: vec![],
                ranges: vec![RangeInput {
                    start_line: 1,
                    end_line: 5,
                    contributor: ContributorInput {
                        kind: ContributorType::Ai,
                        model_id: Some("openai/gpt-5.3-codex".to_string()),
                    },
                }],
            }],
        }],
    }
}

#[test]
fn post_commit_finalization_noops_when_already_finalized() -> Result<()> {
    let runtime = sample_post_commit_runtime();
    let input = sample_post_commit_input();
    let mut notes = FakeNotesWriter::new(PersistenceWriteResult::Written);
    let mut store = FakeRecordStore::new(PersistenceWriteResult::Written);
    let mut queue = FakeRetryQueue::default();
    let mut ledger = FakeEmissionLedger {
        emitted: vec![input.commit_sha.clone()],
    };

    let outcome = finalize_post_commit_trace(
        &runtime,
        input,
        &mut notes,
        &mut store,
        &mut queue,
        &mut ledger,
    )?;

    assert_eq!(
        outcome,
        PostCommitFinalization::NoOp(PostCommitNoOpReason::AlreadyFinalized)
    );
    assert!(notes.writes.is_empty());
    assert!(queue.entries.is_empty());
    Ok(())
}

#[test]
fn post_commit_finalization_dual_writes_with_parent_metadata_and_mime() -> Result<()> {
    let runtime = sample_post_commit_runtime();
    let input = sample_post_commit_input();
    let mut notes = FakeNotesWriter::new(PersistenceWriteResult::Written);
    let mut store = FakeRecordStore::new(PersistenceWriteResult::Written);
    let mut queue = FakeRetryQueue::default();
    let mut ledger = FakeEmissionLedger::default();

    let outcome = finalize_post_commit_trace(
        &runtime,
        input.clone(),
        &mut notes,
        &mut store,
        &mut queue,
        &mut ledger,
    )?;

    let persisted = match outcome {
        PostCommitFinalization::Persisted(persisted) => persisted,
        _ => panic!("expected persisted post-commit outcome"),
    };
    assert_eq!(persisted.commit_sha, input.commit_sha);
    assert_eq!(persisted.trace_id, "550e8400-e29b-41d4-a716-446655440000");

    assert_eq!(notes.writes.len(), 1);
    assert_eq!(
        notes.writes[0].content_type,
        "application/vnd.agent-trace.record+json"
    );
    assert_eq!(notes.writes[0].notes_ref, "refs/notes/agent-trace");
    assert_eq!(
        notes.writes[0]
            .record
            .metadata
            .get(POST_COMMIT_PARENT_SHA_METADATA_KEY),
        Some(&"def789ghi000".to_string())
    );
    assert!(ledger.has_emitted("abc123def456"));
    Ok(())
}

#[test]
fn post_commit_finalization_queues_when_db_write_is_transient_failure() -> Result<()> {
    let runtime = sample_post_commit_runtime();
    let input = sample_post_commit_input();
    let mut notes = FakeNotesWriter::new(PersistenceWriteResult::Written);
    let mut store = FakeRecordStore::new(PersistenceWriteResult::Failed(PersistenceFailure {
        class: PersistenceErrorClass::Transient,
        message: "database unavailable".to_string(),
    }));
    let mut queue = FakeRetryQueue::default();
    let mut ledger = FakeEmissionLedger::default();

    let outcome = finalize_post_commit_trace(
        &runtime,
        input,
        &mut notes,
        &mut store,
        &mut queue,
        &mut ledger,
    )?;

    assert_eq!(
        outcome,
        PostCommitFinalization::QueuedFallback(super::PostCommitQueuedFallback {
            commit_sha: "abc123def456".to_string(),
            failed_targets: vec![PersistenceTarget::Database],
            trace_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
        })
    );
    assert_eq!(queue.entries.len(), 1);
    assert_eq!(
        queue.entries[0].failed_targets,
        vec![PersistenceTarget::Database]
    );
    assert!(!ledger.has_emitted("abc123def456"));
    Ok(())
}

#[test]
fn retry_processor_recovers_failed_notes_write_and_emits_success_metric() -> Result<()> {
    let mut queue = FakeRetryQueue {
        entries: vec![sample_retry_entry_with_target(PersistenceTarget::Notes)],
    };
    let mut notes = FakeNotesWriter::new(PersistenceWriteResult::Written);
    let mut store = FakeRecordStore::new(PersistenceWriteResult::Written);
    let mut metrics = FakeRetryMetricsSink::default();

    let summary = process_trace_retry_queue(&mut queue, &mut notes, &mut store, &mut metrics, 4)?;

    assert_eq!(summary.attempted, 1);
    assert_eq!(summary.recovered, 1);
    assert_eq!(summary.requeued, 0);
    assert!(queue.entries.is_empty());
    assert_eq!(metrics.events.len(), 1);
    assert_eq!(metrics.events[0].error_class, None);
    assert!(metrics.events[0].failed_targets.is_empty());
    Ok(())
}

#[test]
fn retry_processor_requeues_when_db_write_still_fails() -> Result<()> {
    let mut queue = FakeRetryQueue {
        entries: vec![sample_retry_entry_with_target(PersistenceTarget::Database)],
    };
    let mut notes = FakeNotesWriter::new(PersistenceWriteResult::Written);
    let mut store = FakeRecordStore::new(PersistenceWriteResult::Failed(PersistenceFailure {
        class: PersistenceErrorClass::Permanent,
        message: "database still unavailable".to_string(),
    }));
    let mut metrics = FakeRetryMetricsSink::default();

    let summary = process_trace_retry_queue(&mut queue, &mut notes, &mut store, &mut metrics, 4)?;

    assert_eq!(summary.attempted, 1);
    assert_eq!(summary.recovered, 0);
    assert_eq!(summary.requeued, 1);
    assert_eq!(queue.entries.len(), 1);
    assert_eq!(
        queue.entries[0].failed_targets,
        vec![PersistenceTarget::Database]
    );
    assert_eq!(metrics.events.len(), 1);
    assert_eq!(
        metrics.events[0].error_class,
        Some(PersistenceErrorClass::Permanent)
    );
    Ok(())
}

#[test]
fn post_rewrite_finalization_noops_when_sce_disabled() -> Result<()> {
    let mut runtime = sample_post_rewrite_runtime();
    runtime.sce_disabled = true;
    let mut ingestion = FakeRewriteRemapIngestion::default();

    let outcome = finalize_post_rewrite_remap(&runtime, "amend", "old1 new1\n", &mut ingestion)?;

    assert_eq!(
        outcome,
        PostRewriteFinalization::NoOp(PostRewriteNoOpReason::Disabled)
    );
    assert!(ingestion.seen_requests.is_empty());
    Ok(())
}

#[test]
fn post_rewrite_finalization_parses_amend_pairs_and_derives_idempotency() -> Result<()> {
    let runtime = sample_post_rewrite_runtime();
    let mut ingestion = FakeRewriteRemapIngestion::default();

    let outcome = finalize_post_rewrite_remap(
        &runtime,
        "amend",
        "oldsha1 newsha1\noldsha2 newsha2\n",
        &mut ingestion,
    )?;

    assert_eq!(
        outcome,
        PostRewriteFinalization::Ingested(super::PostRewriteIngested {
            rewrite_method: RewriteMethod::Amend,
            total_pairs: 2,
            ingested_pairs: 2,
            skipped_pairs: 0,
        })
    );
    assert_eq!(ingestion.seen_requests.len(), 2);
    assert_eq!(
        ingestion.seen_requests[0].idempotency_key,
        "post-rewrite:amend:oldsha1:newsha1"
    );
    assert_eq!(
        ingestion.seen_requests[1].idempotency_key,
        "post-rewrite:amend:oldsha2:newsha2"
    );
    Ok(())
}

#[test]
fn post_rewrite_finalization_skips_duplicate_pairs_with_rebase_method() -> Result<()> {
    let runtime = sample_post_rewrite_runtime();
    let mut ingestion = FakeRewriteRemapIngestion::default();

    let outcome = finalize_post_rewrite_remap(
        &runtime,
        "rebase",
        "oldsha1 newsha1\noldsha1 newsha1\n",
        &mut ingestion,
    )?;

    assert_eq!(
        outcome,
        PostRewriteFinalization::Ingested(super::PostRewriteIngested {
            rewrite_method: RewriteMethod::Rebase,
            total_pairs: 2,
            ingested_pairs: 1,
            skipped_pairs: 1,
        })
    );
    assert_eq!(ingestion.seen_requests.len(), 2);
    assert_eq!(ingestion.duplicate_keys.len(), 1);
    assert_eq!(
        ingestion.duplicate_keys[0],
        "post-rewrite:rebase:oldsha1:newsha1"
    );
    Ok(())
}

#[test]
fn post_rewrite_finalization_rejects_invalid_pair_line_format() {
    let runtime = sample_post_rewrite_runtime();
    let mut ingestion = FakeRewriteRemapIngestion::default();

    let error = finalize_post_rewrite_remap(&runtime, "amend", "missing_new_sha\n", &mut ingestion)
        .expect_err("invalid pair format should return error");

    assert!(error.to_string().contains("expected '<old_sha> <new_sha>'"));
    assert!(ingestion.seen_requests.is_empty());
}

#[test]
fn rewrite_trace_finalization_persists_metadata_and_notes_db_parity() -> Result<()> {
    let runtime = sample_post_rewrite_runtime();
    let input = sample_rewrite_trace_input();
    let mut notes = FakeNotesWriter::new(PersistenceWriteResult::Written);
    let mut store = FakeRecordStore::new(PersistenceWriteResult::Written);
    let mut queue = FakeRetryQueue::default();
    let mut ledger = FakeEmissionLedger::default();

    let outcome = finalize_rewrite_trace(
        &runtime,
        input,
        &mut notes,
        &mut store,
        &mut queue,
        &mut ledger,
    )?;

    let persisted = match outcome {
        RewriteTraceFinalization::Persisted(persisted) => persisted,
        _ => panic!("expected persisted rewrite trace outcome"),
    };

    assert_eq!(persisted.commit_sha, "newsha123");
    assert_eq!(persisted.trace_id, "660e8400-e29b-41d4-a716-446655440000");
    assert_eq!(persisted.quality_status, super::QualityStatus::Final);
    assert_eq!(notes.writes.len(), 1);
    assert_eq!(notes.writes[0].record.vcs.revision, "newsha123");
    assert_eq!(
        notes.writes[0].record.metadata.get(METADATA_REWRITE_FROM),
        Some(&"oldsha456".to_string())
    );
    assert_eq!(
        notes.writes[0].record.metadata.get(METADATA_REWRITE_METHOD),
        Some(&"rebase".to_string())
    );
    assert_eq!(
        notes.writes[0]
            .record
            .metadata
            .get(METADATA_REWRITE_CONFIDENCE),
        Some(&"0.91".to_string())
    );
    assert_eq!(
        notes.writes[0].record.metadata.get(METADATA_QUALITY_STATUS),
        Some(&"final".to_string())
    );
    assert!(queue.entries.is_empty());
    assert!(ledger.has_emitted("newsha123"));
    Ok(())
}

#[test]
fn rewrite_trace_finalization_applies_quality_thresholds() -> Result<()> {
    let runtime = sample_post_rewrite_runtime();
    let mut notes = FakeNotesWriter::new(PersistenceWriteResult::Written);
    let mut store = FakeRecordStore::new(PersistenceWriteResult::Written);
    let mut queue = FakeRetryQueue::default();
    let mut ledger = FakeEmissionLedger::default();

    let mut medium = sample_rewrite_trace_input();
    medium.record_id = "760e8400-e29b-41d4-a716-446655440000".to_string();
    medium.rewritten_commit_sha = "newsha-medium".to_string();
    medium.rewrite_confidence = 0.75;
    let medium_outcome = finalize_rewrite_trace(
        &runtime,
        medium,
        &mut notes,
        &mut store,
        &mut queue,
        &mut ledger,
    )?;
    assert!(matches!(
        medium_outcome,
        RewriteTraceFinalization::Persisted(super::RewriteTracePersisted {
            quality_status: super::QualityStatus::Partial,
            ..
        })
    ));

    let mut low = sample_rewrite_trace_input();
    low.record_id = "860e8400-e29b-41d4-a716-446655440000".to_string();
    low.rewritten_commit_sha = "newsha-low".to_string();
    low.rewrite_confidence = 0.40;
    let low_outcome = finalize_rewrite_trace(
        &runtime,
        low,
        &mut notes,
        &mut store,
        &mut queue,
        &mut ledger,
    )?;
    assert!(matches!(
        low_outcome,
        RewriteTraceFinalization::Persisted(super::RewriteTracePersisted {
            quality_status: super::QualityStatus::NeedsReview,
            ..
        })
    ));

    Ok(())
}

#[test]
fn rewrite_trace_finalization_rejects_confidence_outside_zero_to_one() {
    let runtime = sample_post_rewrite_runtime();
    let mut input = sample_rewrite_trace_input();
    input.rewrite_confidence = 1.2;

    let mut notes = FakeNotesWriter::new(PersistenceWriteResult::Written);
    let mut store = FakeRecordStore::new(PersistenceWriteResult::Written);
    let mut queue = FakeRetryQueue::default();
    let mut ledger = FakeEmissionLedger::default();

    let error = finalize_rewrite_trace(
        &runtime,
        input,
        &mut notes,
        &mut store,
        &mut queue,
        &mut ledger,
    )
    .expect_err("out-of-range confidence must fail");

    assert!(error
        .to_string()
        .contains("rewrite confidence must be within [0.0, 1.0]"));
    assert!(notes.writes.is_empty());
    assert!(queue.entries.is_empty());
}

#[test]
fn rewrite_trace_finalization_noops_when_commit_already_finalized() -> Result<()> {
    let runtime = sample_post_rewrite_runtime();
    let input = sample_rewrite_trace_input();
    let mut notes = FakeNotesWriter::new(PersistenceWriteResult::Written);
    let mut store = FakeRecordStore::new(PersistenceWriteResult::Written);
    let mut queue = FakeRetryQueue::default();
    let mut ledger = FakeEmissionLedger {
        emitted: vec!["newsha123".to_string()],
    };

    let outcome = finalize_rewrite_trace(
        &runtime,
        input,
        &mut notes,
        &mut store,
        &mut queue,
        &mut ledger,
    )?;

    assert_eq!(
        outcome,
        RewriteTraceFinalization::NoOp(RewriteTraceNoOpReason::AlreadyFinalized)
    );
    assert!(notes.writes.is_empty());
    assert!(queue.entries.is_empty());
    Ok(())
}

#[test]
fn pre_commit_finalization_noops_when_sce_disabled() {
    let mut runtime = sample_runtime();
    runtime.sce_disabled = true;

    let outcome =
        finalize_pre_commit_checkpoint(&runtime, sample_anchors(), sample_pending_checkpoint());
    assert_eq!(
        outcome,
        PreCommitFinalization::NoOp(PreCommitNoOpReason::Disabled)
    );
}

#[test]
fn pre_commit_finalization_noops_when_cli_unavailable() {
    let mut runtime = sample_runtime();
    runtime.cli_available = false;

    let outcome =
        finalize_pre_commit_checkpoint(&runtime, sample_anchors(), sample_pending_checkpoint());
    assert_eq!(
        outcome,
        PreCommitFinalization::NoOp(PreCommitNoOpReason::CliUnavailable)
    );
}

#[test]
fn pre_commit_finalization_noops_for_bare_repo() {
    let mut runtime = sample_runtime();
    runtime.is_bare_repo = true;

    let outcome =
        finalize_pre_commit_checkpoint(&runtime, sample_anchors(), sample_pending_checkpoint());
    assert_eq!(
        outcome,
        PreCommitFinalization::NoOp(PreCommitNoOpReason::BareRepository)
    );
}

#[test]
fn pre_commit_finalization_uses_only_staged_ranges_and_captures_anchors() {
    let pending = PendingCheckpoint {
        files: vec![
            PendingFileCheckpoint {
                path: "src/keep.rs".to_string(),
                staged_ranges: vec![PendingLineRange {
                    start_line: 10,
                    end_line: 20,
                }],
                unstaged_ranges: vec![PendingLineRange {
                    start_line: 21,
                    end_line: 30,
                }],
            },
            PendingFileCheckpoint {
                path: "src/drop.rs".to_string(),
                staged_ranges: vec![],
                unstaged_ranges: vec![PendingLineRange {
                    start_line: 1,
                    end_line: 2,
                }],
            },
        ],
    };
    let anchors = sample_anchors();

    let outcome = finalize_pre_commit_checkpoint(&sample_runtime(), anchors.clone(), pending);

    let finalized = match outcome {
        PreCommitFinalization::Finalized(finalized) => finalized,
        _ => panic!("expected finalized checkpoint"),
    };

    assert_eq!(finalized.anchors, anchors);
    assert_eq!(finalized.files.len(), 1);
    assert_eq!(finalized.files[0].path, "src/keep.rs");
    assert_eq!(finalized.files[0].ranges.len(), 1);
    assert_eq!(
        finalized.files[0].ranges[0],
        PendingLineRange {
            start_line: 10,
            end_line: 20
        }
    );
}

#[test]
fn pre_commit_runtime_persists_staged_only_checkpoint_artifact() -> Result<()> {
    let repo = create_temp_repo()?;
    let tracked_file = repo.join("src").join("lib.rs");
    fs::create_dir_all(
        tracked_file
            .parent()
            .expect("tracked file path should have parent"),
    )?;
    fs::write(&tracked_file, "one\ntwo\nthree\nfour\n")?;
    run_git_in_repo(&repo, &["add", "."])?;
    run_git_in_repo(&repo, &["commit", "-m", "initial"])?;

    fs::write(&tracked_file, "one\ntwo-staged\nthree\nfour\n")?;
    run_git_in_repo(&repo, &["add", "src/lib.rs"])?;
    fs::write(&tracked_file, "one\ntwo-staged\nthree\nfour-unstaged\n")?;

    let message = run_pre_commit_subcommand_in_repo(&repo)?;
    assert_eq!(
        message,
        "pre-commit hook executed and finalized staged checkpoint for 1 file(s)."
    );

    let checkpoint_path = resolve_pre_commit_checkpoint_path(&repo)?;
    let checkpoint = serde_json::from_slice::<serde_json::Value>(&fs::read(&checkpoint_path)?)?;

    assert_eq!(checkpoint["version"], 1);
    assert_eq!(checkpoint["files"].as_array().map(Vec::len), Some(1));
    assert_eq!(checkpoint["files"][0]["path"], "src/lib.rs");
    assert_eq!(
        checkpoint["files"][0]["ranges"].as_array().map(Vec::len),
        Some(1)
    );
    assert_eq!(checkpoint["files"][0]["ranges"][0]["start_line"], 2);
    assert_eq!(checkpoint["files"][0]["ranges"][0]["end_line"], 2);
    Ok(())
}

fn sample_commit_msg_runtime() -> CommitMsgRuntimeState {
    CommitMsgRuntimeState {
        sce_disabled: false,
        sce_coauthor_enabled: true,
        has_staged_sce_attribution: true,
    }
}

fn write_staged_checkpoint_artifact(repo: &Path) -> Result<()> {
    let checkpoint_path = resolve_pre_commit_checkpoint_path(repo)?;
    if let Some(parent) = checkpoint_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(
        checkpoint_path,
        r#"{
  "version": 1,
  "anchors": {
    "index_tree": "index-tree",
    "head_tree": "head-tree"
  },
  "files": [
    {
      "path": "src/lib.rs",
      "ranges": [
        {
          "start_line": 1,
          "end_line": 1
        }
      ]
    }
  ]
}
"#,
    )?;
    Ok(())
}

#[test]
fn commit_msg_policy_noops_when_sce_disabled() {
    let mut runtime = sample_commit_msg_runtime();
    runtime.sce_disabled = true;

    let message = "feat: add attribution";
    let output = apply_commit_msg_coauthor_policy(&runtime, message);
    assert_eq!(output, message);
}

#[test]
fn commit_msg_policy_noops_when_coauthor_disabled() {
    let mut runtime = sample_commit_msg_runtime();
    runtime.sce_coauthor_enabled = false;

    let message = "feat: add attribution";
    let output = apply_commit_msg_coauthor_policy(&runtime, message);
    assert_eq!(output, message);
}

#[test]
fn commit_msg_policy_noops_without_staged_sce_attribution() {
    let mut runtime = sample_commit_msg_runtime();
    runtime.has_staged_sce_attribution = false;

    let message = "feat: add attribution";
    let output = apply_commit_msg_coauthor_policy(&runtime, message);
    assert_eq!(output, message);
}

#[test]
fn commit_msg_policy_appends_canonical_trailer_once_when_allowed() {
    let message = "feat: add attribution";
    let output = apply_commit_msg_coauthor_policy(&sample_commit_msg_runtime(), message);

    assert_eq!(
        output,
        format!(
            "feat: add attribution\n\n{}",
            CANONICAL_SCE_COAUTHOR_TRAILER
        )
    );
}

#[test]
fn commit_msg_policy_dedupes_existing_canonical_trailers() {
    let message = format!(
        "feat: add attribution\n\n{}\n{}\n",
        CANONICAL_SCE_COAUTHOR_TRAILER, CANONICAL_SCE_COAUTHOR_TRAILER
    );

    let output = apply_commit_msg_coauthor_policy(&sample_commit_msg_runtime(), &message);

    assert_eq!(
        output,
        format!(
            "feat: add attribution\n\n{}\n",
            CANONICAL_SCE_COAUTHOR_TRAILER
        )
    );
}

#[test]
fn commit_msg_policy_is_idempotent() {
    let first =
        apply_commit_msg_coauthor_policy(&sample_commit_msg_runtime(), "feat: add attribution");
    let second = apply_commit_msg_coauthor_policy(&sample_commit_msg_runtime(), &first);

    assert_eq!(first, second);
}

#[test]
fn commit_msg_runtime_mutates_message_file_when_policy_gate_passes() -> Result<()> {
    let repo = create_temp_repo()?;
    write_staged_checkpoint_artifact(&repo)?;
    let message_file = repo.join("COMMIT_EDITMSG");
    fs::write(&message_file, "feat: add attribution\n")?;

    let message = run_commit_msg_subcommand_in_repo(&repo, &message_file)?;
    assert_eq!(
        message,
        format!(
            "commit-msg hook processed message file '{}' (policy_gate_passed=true, trailer_applied=true).",
            message_file.display()
        )
    );

    let mutated = fs::read_to_string(&message_file)?;
    assert_eq!(
        mutated,
        format!(
            "feat: add attribution\n\n{}\n",
            CANONICAL_SCE_COAUTHOR_TRAILER
        )
    );
    Ok(())
}

#[test]
fn commit_msg_runtime_noops_when_staged_attribution_checkpoint_missing() -> Result<()> {
    let repo = create_temp_repo()?;
    let message_file = repo.join("COMMIT_EDITMSG");
    let original = "feat: add attribution\n";
    fs::write(&message_file, original)?;

    let message = run_commit_msg_subcommand_in_repo(&repo, &message_file)?;
    assert_eq!(
        message,
        format!(
            "commit-msg hook processed message file '{}' (policy_gate_passed=false, trailer_applied=false).",
            message_file.display()
        )
    );

    let persisted = fs::read_to_string(&message_file)?;
    assert_eq!(persisted, original);
    Ok(())
}

#[test]
fn post_commit_runtime_persists_notes_and_local_record_store() -> Result<()> {
    let _db_guard = agent_trace_db_test_lock()
        .lock()
        .expect("agent trace DB test lock poisoned");

    let repo = create_temp_repo()?;
    let tracked_file = repo.join("src").join("lib.rs");
    fs::create_dir_all(
        tracked_file
            .parent()
            .expect("tracked file path should have parent"),
    )?;
    fs::write(&tracked_file, "one\ntwo\n")?;
    run_git_in_repo(&repo, &["add", "."])?;
    run_git_in_repo(&repo, &["commit", "-m", "initial"])?;

    fs::write(&tracked_file, "one\ntwo\nthree\n")?;
    run_git_in_repo(&repo, &["add", "src/lib.rs"])?;
    run_git_in_repo(&repo, &["commit", "-m", "feat: update file"])?;
    write_staged_checkpoint_artifact(&repo)?;

    let message = run_post_commit_subcommand_in_repo(&repo)?;
    assert!(message.contains("post-commit hook finalized trace"));

    let head_sha = run_git_output_in_repo(&repo, &["rev-parse", "--verify", "HEAD"])?;
    let note = run_git_output_in_repo(
        &repo,
        &[
            "notes",
            "--ref",
            "refs/notes/agent-trace",
            "show",
            &head_sha,
        ],
    )?;
    let note_json = serde_json::from_str::<serde_json::Value>(&note)?;
    assert_eq!(
        note_json
            .get("content_type")
            .and_then(serde_json::Value::as_str),
        Some("application/vnd.agent-trace.record+json")
    );
    assert_eq!(
        note_json
            .get("record")
            .and_then(|record| record.get("metadata"))
            .and_then(|metadata| metadata.get("dev.crocoder.sce.notes_ref"))
            .and_then(serde_json::Value::as_str),
        Some("refs/notes/agent-trace")
    );

    let db_path = resolve_agent_trace_local_db_path()?;
    let runtime = tokio::runtime::Builder::new_current_thread().build()?;
    let persisted_count = runtime.block_on(async {
        let location = db_path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("test DB path must be UTF-8"))?;
        let db = turso::Builder::new_local(location).build().await?;
        let conn = db.connect()?;
        let mut rows = conn
            .query(
                "SELECT COUNT(*) FROM trace_records tr JOIN commits c ON c.id = tr.commit_id WHERE c.commit_sha = ?1",
                [head_sha.as_str()],
            )
            .await?;
        let row = rows
            .next()
            .await?
            .ok_or_else(|| anyhow::anyhow!("trace record count query returned no rows"))?;
        let value = row.get_value(0)?;
        value
            .as_integer()
            .copied()
            .ok_or_else(|| anyhow::anyhow!("trace record count query returned non-integer"))
    })?;

    assert_eq!(persisted_count, 1);

    Ok(())
}

#[test]
fn post_rewrite_runtime_ingests_remap_and_persists_rewrite_trace() -> Result<()> {
    let _db_guard = agent_trace_db_test_lock()
        .lock()
        .expect("agent trace DB test lock poisoned");

    let repo = create_temp_repo()?;
    let tracked_file = repo.join("src").join("lib.rs");
    fs::create_dir_all(
        tracked_file
            .parent()
            .expect("tracked file path should have parent"),
    )?;
    fs::write(&tracked_file, "one\ntwo\n")?;
    run_git_in_repo(&repo, &["add", "."])?;
    run_git_in_repo(&repo, &["commit", "-m", "initial"])?;

    fs::write(&tracked_file, "one\ntwo\nthree\n")?;
    run_git_in_repo(&repo, &["add", "src/lib.rs"])?;
    run_git_in_repo(&repo, &["commit", "-m", "feat: rewrite target"])?;

    let old_sha = run_git_output_in_repo(&repo, &["rev-parse", "--verify", "HEAD"])?;
    run_git_in_repo(&repo, &["commit", "--amend", "-m", "feat: rewrite amended"])?;
    let new_sha = run_git_output_in_repo(&repo, &["rev-parse", "--verify", "HEAD"])?;

    let message =
        run_post_rewrite_subcommand_in_repo(&repo, "amend", &format!("{} {}\n", old_sha, new_sha))?;
    assert!(
        message.contains("post-rewrite hook ingested 1 pair(s), skipped 0 duplicate pair(s)"),
        "unexpected message: {message}"
    );
    assert!(
        message.contains("rewrite_traces=(persisted=1, queued=0, no_op=0, failed=0)"),
        "unexpected message: {message}"
    );

    let note = run_git_output_in_repo(
        &repo,
        &["notes", "--ref", "refs/notes/agent-trace", "show", &new_sha],
    )?;
    let note_json = serde_json::from_str::<serde_json::Value>(&note)?;
    assert_eq!(
        note_json
            .get("record")
            .and_then(|record| record.get("metadata"))
            .and_then(|metadata| metadata.get("dev.crocoder.sce.rewrite_from"))
            .and_then(serde_json::Value::as_str),
        Some(old_sha.as_str())
    );
    assert_eq!(
        note_json
            .get("record")
            .and_then(|record| record.get("metadata"))
            .and_then(|metadata| metadata.get("dev.crocoder.sce.rewrite_method"))
            .and_then(serde_json::Value::as_str),
        Some("amend")
    );

    let db_path = resolve_agent_trace_local_db_path()?;
    let runtime = tokio::runtime::Builder::new_current_thread().build()?;
    let (rewrite_mapping_count, rewrite_trace_count) = runtime.block_on(async {
        let location = db_path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("test DB path must be UTF-8"))?;
        let db = turso::Builder::new_local(location).build().await?;
        let conn = db.connect()?;

        let mut mapping_rows = conn
            .query(
                "SELECT COUNT(*) FROM rewrite_mappings WHERE old_commit_sha = ?1 AND new_commit_sha = ?2",
                (old_sha.as_str(), new_sha.as_str()),
            )
            .await?;
        let mapping_row = mapping_rows
            .next()
            .await?
            .ok_or_else(|| anyhow::anyhow!("rewrite mapping count query returned no rows"))?;
        let mapping_value = mapping_row.get_value(0)?;
        let mapping_count = mapping_value
            .as_integer()
            .copied()
            .ok_or_else(|| anyhow::anyhow!("rewrite mapping count query returned non-integer"))?;

        let mut trace_rows = conn
            .query(
                "SELECT COUNT(*) FROM trace_records tr JOIN commits c ON c.id = tr.commit_id WHERE c.commit_sha = ?1",
                [new_sha.as_str()],
            )
            .await?;
        let trace_row = trace_rows
            .next()
            .await?
            .ok_or_else(|| anyhow::anyhow!("rewrite trace count query returned no rows"))?;
        let trace_value = trace_row.get_value(0)?;
        let trace_count = trace_value
            .as_integer()
            .copied()
            .ok_or_else(|| anyhow::anyhow!("rewrite trace count query returned non-integer"))?;

        Ok::<(i64, i64), anyhow::Error>((mapping_count, trace_count))
    })?;

    assert_eq!(rewrite_mapping_count, 1);
    assert_eq!(rewrite_trace_count, 1);

    Ok(())
}

#[test]
fn post_rewrite_runtime_skips_duplicate_pair_replay() -> Result<()> {
    let _db_guard = agent_trace_db_test_lock()
        .lock()
        .expect("agent trace DB test lock poisoned");

    let repo = create_temp_repo()?;
    let tracked_file = repo.join("src").join("lib.rs");
    fs::create_dir_all(
        tracked_file
            .parent()
            .expect("tracked file path should have parent"),
    )?;
    fs::write(&tracked_file, "one\n")?;
    run_git_in_repo(&repo, &["add", "."])?;
    run_git_in_repo(&repo, &["commit", "-m", "initial"])?;

    fs::write(&tracked_file, "one\ntwo\n")?;
    run_git_in_repo(&repo, &["add", "src/lib.rs"])?;
    run_git_in_repo(&repo, &["commit", "-m", "feat: rewrite target"])?;

    let old_sha = run_git_output_in_repo(&repo, &["rev-parse", "--verify", "HEAD"])?;
    run_git_in_repo(&repo, &["commit", "--amend", "-m", "feat: rewrite amended"])?;
    let new_sha = run_git_output_in_repo(&repo, &["rev-parse", "--verify", "HEAD"])?;
    let pair_input = format!("{} {}\n", old_sha, new_sha);

    let _first = run_post_rewrite_subcommand_in_repo(&repo, "amend", &pair_input)?;
    let second = run_post_rewrite_subcommand_in_repo(&repo, "amend", &pair_input)?;

    assert!(
        second.contains("post-rewrite hook ingested 0 pair(s), skipped 1 duplicate pair(s)"),
        "unexpected message: {second}"
    );
    assert!(
        second.contains("rewrite_traces=(persisted=0, queued=0, no_op=0, failed=0)"),
        "unexpected message: {second}"
    );

    Ok(())
}

#[test]
fn parse_hooks_subcommand_routes_pre_commit() -> Result<()> {
    let parsed = parse_hooks_subcommand(vec!["pre-commit".to_string()])?;
    assert_eq!(parsed, HookSubcommand::PreCommit);
    Ok(())
}

#[test]
fn parse_hooks_subcommand_rejects_missing_hook_name() {
    let error = parse_hooks_subcommand(Vec::new())
        .expect_err("missing hook subcommand should return usage error");
    assert_eq!(
        error.to_string(),
        "Missing hook subcommand. Try: run 'sce hooks --help' and use one of 'pre-commit', 'commit-msg', 'post-commit', or 'post-rewrite'."
    );
}

#[test]
fn parse_hooks_subcommand_requires_commit_msg_path() {
    let error = parse_hooks_subcommand(vec!["commit-msg".to_string()])
        .expect_err("commit-msg requires <message-file>");
    assert_eq!(
        error.to_string(),
        "Missing required argument '<message-file>' for 'commit-msg'. Try: run 'sce hooks commit-msg .git/COMMIT_EDITMSG'."
    );
}

#[test]
fn run_hooks_subcommand_commit_msg_rejects_missing_file() {
    let missing = std::env::temp_dir().join(format!(
        "sce-hooks-missing-{}-{}.msg",
        std::process::id(),
        "nope"
    ));
    let error = run_hooks_subcommand(HookSubcommand::CommitMsg {
        message_file: missing.clone(),
    })
    .expect_err("missing commit message file should fail deterministically");

    assert_eq!(
        error.to_string(),
        format!(
            "Invalid commit message file '{}': file does not exist or is not readable.",
            missing.display()
        )
    );
}
