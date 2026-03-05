use anyhow::Result;

use crate::services::agent_trace::{
    build_trace_payload, ContributorInput, ContributorType, ConversationInput,
    FileAttributionInput, QualityStatus, RangeInput, TraceAdapterInput, METADATA_QUALITY_STATUS,
    METADATA_REWRITE_CONFIDENCE, METADATA_REWRITE_FROM, METADATA_REWRITE_METHOD,
};

use super::{
    apply_commit_msg_coauthor_policy, finalize_post_commit_trace, finalize_post_rewrite_remap,
    finalize_pre_commit_checkpoint, finalize_rewrite_trace, parse_hooks_subcommand,
    process_trace_retry_queue, run_hooks_subcommand, run_placeholder_hooks, CommitMsgRuntimeState,
    GeneratedRegionEvent, GeneratedRegionLifecycle, GitHookKind, HookEvent, HookService,
    HookSubcommand, PendingCheckpoint, PendingFileCheckpoint, PendingLineRange,
    PersistenceErrorClass, PersistenceFailure, PersistenceTarget, PersistenceWriteResult,
    PlaceholderHookService, PostCommitFinalization, PostCommitInput, PostCommitNoOpReason,
    PostCommitRuntimeState, PostRewriteFinalization, PostRewriteNoOpReason,
    PostRewriteRuntimeState, PreCommitFinalization, PreCommitNoOpReason, PreCommitRuntimeState,
    PreCommitTreeAnchors, RetryMetricsSink, RetryProcessingMetric, RewriteMethod,
    RewriteRemapIngestion, RewriteRemapRequest, RewriteTraceFinalization, RewriteTraceInput,
    RewriteTraceNoOpReason, TraceEmissionLedger, TraceNote, TraceNotesWriter, TraceRecordStore,
    TraceRetryQueue, TraceRetryQueueEntry, CANONICAL_SCE_COAUTHOR_TRAILER,
    POST_COMMIT_PARENT_SHA_METADATA_KEY,
};

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

fn sample_commit_msg_runtime() -> CommitMsgRuntimeState {
    CommitMsgRuntimeState {
        sce_disabled: false,
        sce_coauthor_enabled: true,
        has_staged_sce_attribution: true,
    }
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
fn hooks_placeholder_event_model_reserves_generated_region_tracking() {
    let service = PlaceholderHookService;
    let model = service.event_model();
    assert!(model.generated_region_tracking);
    assert_eq!(model.supported_hooks.len(), 3);
}

#[test]
fn hooks_placeholder_message_mentions_event_model() -> Result<()> {
    let message = run_placeholder_hooks()?;
    assert!(message.contains("Hook event model reserves"));
    Ok(())
}

#[test]
fn hooks_placeholder_accepts_generated_region_events() -> Result<()> {
    let service = PlaceholderHookService;
    let event = HookEvent {
        hook: GitHookKind::PreCommit,
        region_event: Some(GeneratedRegionEvent {
            file_path: "context/plans/example.md".to_string(),
            marker_id: "generated:example".to_string(),
            lifecycle: GeneratedRegionLifecycle::Updated,
        }),
    };

    service.record(event)
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
        "Missing hook subcommand. Run 'sce hooks --help' to see valid usage."
    );
}

#[test]
fn parse_hooks_subcommand_requires_commit_msg_path() {
    let error = parse_hooks_subcommand(vec!["commit-msg".to_string()])
        .expect_err("commit-msg requires <message-file>");
    assert_eq!(
        error.to_string(),
        "Missing required argument '<message-file>' for 'commit-msg'. Run 'sce hooks --help' to see valid usage."
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
