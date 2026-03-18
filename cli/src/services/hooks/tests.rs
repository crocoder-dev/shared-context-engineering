use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde_json::Value;

use crate::services::agent_trace::{
    build_trace_payload, ContributorInput, ContributorType, ConversationInput,
    FileAttributionInput, QualityStatus, RangeInput, TraceAdapterInput, METADATA_QUALITY_STATUS,
    METADATA_REWRITE_CONFIDENCE, METADATA_REWRITE_FROM, METADATA_REWRITE_METHOD,
};
use crate::services::local_db::{apply_core_schema_migrations, LocalDatabaseTarget};
use crate::services::trace::{render_prompt_trace_for_test, TraceFormat};

use super::{
    apply_commit_msg_coauthor_policy, build_post_commit_input,
    checkpoint_has_explicit_sce_attribution, collect_pending_checkpoint,
    finalize_post_commit_trace, finalize_post_rewrite_remap, finalize_pre_commit_checkpoint,
    finalize_rewrite_trace, load_pending_prompts, load_post_commit_prompt_records,
    process_trace_retry_queue, resolve_pre_commit_git_branch, run_hooks_subcommand,
    write_finalized_checkpoint, CommitMsgRuntimeState, HookSubcommand, LocalDbTraceRecordStore,
    PendingCheckpoint, PendingFileCheckpoint, PendingLineRange, PendingPromptCheckpoint,
    PersistenceErrorClass, PersistenceFailure, PersistenceTarget, PersistenceWriteResult,
    PostCommitFinalization, PostCommitInput, PostCommitNoOpReason, PostCommitRuntimeState,
    PostRewriteFinalization, PostRewriteNoOpReason, PostRewriteRuntimeState, PreCommitFinalization,
    PreCommitNoOpReason, PreCommitRuntimeState, PreCommitTreeAnchors, RetryMetricsSink,
    RetryProcessingMetric, RewriteMethod, RewriteRemapIngestion, RewriteRemapRequest,
    RewriteTraceFinalization, RewriteTraceInput, RewriteTraceNoOpReason, TraceEmissionLedger,
    TraceNote, TraceNotesWriter, TraceRecordStore, TraceRetryQueue, TraceRetryQueueEntry,
    CANONICAL_SCE_COAUTHOR_TRAILER, POST_COMMIT_PARENT_SHA_METADATA_KEY,
};

fn sample_pending_checkpoint() -> PendingCheckpoint {
    PendingCheckpoint {
        files: vec![PendingFileCheckpoint {
            path: "src/lib.rs".to_string(),
            has_sce_attribution: false,
            staged_ranges: vec![PendingLineRange {
                start_line: 1,
                end_line: 3,
            }],
            unstaged_ranges: vec![PendingLineRange {
                start_line: 4,
                end_line: 6,
            }],
        }],
        harness_type: "claude_code".to_string(),
        git_branch: Some("feature/prompt-capture".to_string()),
        model_id: Some("claude-sonnet-4-20250514".to_string()),
        prompts: vec![PendingPromptCheckpoint {
            turn_number: 1,
            prompt_text: "add attribution".to_string(),
            prompt_length: 15,
            is_truncated: false,
            cwd: Some("/repo".to_string()),
            transcript_path: Some("/tmp/claude-session.jsonl".to_string()),
            captured_at: "2026-03-18T10:00:00Z".to_string(),
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
    writes: Vec<super::PersistedTraceRecord>,
}

impl FakeRecordStore {
    fn new(result: PersistenceWriteResult) -> Self {
        Self {
            result,
            writes: Vec::new(),
        }
    }
}

impl TraceRecordStore for FakeRecordStore {
    fn write_trace_record(
        &mut self,
        record: super::PersistedTraceRecord,
    ) -> PersistenceWriteResult {
        self.writes.push(record);
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
        prompts: Vec::new(),
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
        committed_at_unix_ms: 1_772_586_672_000,
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
        prompts: vec![super::PersistedPromptRecord {
            turn_number: 1,
            prompt_text: "capture prompt metrics".to_string(),
            prompt_length: 22,
            is_truncated: false,
            harness_type: "claude_code".to_string(),
            model_id: Some("claude-sonnet-4-20250514".to_string()),
            cwd: Some("/repo".to_string()),
            git_branch: Some("feature/prompt-capture".to_string()),
            tool_call_count: 3,
            duration_ms: 45_000,
            captured_at: "2026-03-04T10:10:27Z".to_string(),
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

    let PostCommitFinalization::Persisted(persisted) = outcome else {
        panic!("expected persisted post-commit outcome");
    };
    assert_eq!(persisted.commit_sha, input.commit_sha);
    assert_eq!(persisted.trace_id, "550e8400-e29b-41d4-a716-446655440000");

    assert_eq!(notes.writes.len(), 1);
    assert_eq!(store.writes.len(), 1);
    assert_eq!(store.writes[0].prompts.len(), 1);
    assert_eq!(store.writes[0].prompts[0].tool_call_count, 3);
    assert_eq!(store.writes[0].prompts[0].duration_ms, 45_000);
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
    assert_eq!(queue.entries[0].prompts.len(), 1);
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

    let RewriteTraceFinalization::Persisted(persisted) = outcome else {
        panic!("expected persisted rewrite trace outcome");
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
                has_sce_attribution: false,
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
                has_sce_attribution: true,
                staged_ranges: vec![],
                unstaged_ranges: vec![PendingLineRange {
                    start_line: 1,
                    end_line: 2,
                }],
            },
        ],
        harness_type: "claude_code".to_string(),
        git_branch: Some("feature/prompt-capture".to_string()),
        model_id: Some("claude-sonnet-4-20250514".to_string()),
        prompts: vec![PendingPromptCheckpoint {
            turn_number: 1,
            prompt_text: "fix edge case".to_string(),
            prompt_length: 13,
            is_truncated: false,
            cwd: Some("/repo/src".to_string()),
            transcript_path: Some("/tmp/claude-session.jsonl".to_string()),
            captured_at: "2026-03-18T10:00:00Z".to_string(),
        }],
    };
    let anchors = sample_anchors();

    let outcome = finalize_pre_commit_checkpoint(&sample_runtime(), anchors.clone(), pending);

    let PreCommitFinalization::Finalized(finalized) = outcome else {
        panic!("expected finalized checkpoint");
    };

    assert_eq!(finalized.anchors, anchors);
    assert_eq!(finalized.harness_type, "claude_code");
    assert_eq!(
        finalized.git_branch.as_deref(),
        Some("feature/prompt-capture")
    );
    assert_eq!(
        finalized.model_id.as_deref(),
        Some("claude-sonnet-4-20250514")
    );
    assert_eq!(finalized.files.len(), 1);
    assert_eq!(finalized.files[0].path, "src/keep.rs");
    assert!(!finalized.files[0].has_sce_attribution);
    assert_eq!(finalized.files[0].ranges.len(), 1);
    assert_eq!(
        finalized.files[0].ranges[0],
        PendingLineRange {
            start_line: 10,
            end_line: 20
        }
    );
    assert_eq!(finalized.prompts.len(), 1);
    assert_eq!(finalized.prompts[0].turn_number, 1);
    assert_eq!(finalized.prompts[0].prompt_text, "fix edge case");
    assert_eq!(
        finalized.prompts[0].transcript_path.as_deref(),
        Some("/tmp/claude-session.jsonl")
    );
    assert_eq!(finalized.prompts[0].captured_at, "2026-03-18T10:00:00Z");
}

#[test]
fn load_pending_prompts_dedupes_entries_and_skips_invalid_rows() -> Result<()> {
    let repository_root = std::env::temp_dir().join(format!(
        "sce-hooks-prompts-{}-{}",
        std::process::id(),
        "dedupe"
    ));
    if repository_root.exists() {
        std::fs::remove_dir_all(&repository_root)?;
    }
    std::fs::create_dir_all(&repository_root)?;

    let init = std::process::Command::new("git")
        .args(["init"])
        .current_dir(&repository_root)
        .output()?;
    assert!(init.status.success());

    let prompt_dir = repository_root.join(".git").join("sce");
    std::fs::create_dir_all(&prompt_dir)?;
    std::fs::write(
        prompt_dir.join("prompts.jsonl"),
        concat!(
            "{\"prompt\":\"first prompt\",\"cwd\":\"/repo\",\"transcript_path\":\"/tmp/claude-a.jsonl\",\"timestamp\":\"2026-03-18T10:00:00Z\"}\n",
            "not-json\n",
            "{\"prompt\":\"first prompt\",\"cwd\":\"/repo\",\"transcript_path\":\"/tmp/claude-a.jsonl\",\"timestamp\":\"2026-03-18T10:00:00Z\"}\n",
            "{\"prompt\":\"second prompt\",\"cwd\":\"/repo/tests\",\"transcript_path\":\"/tmp/claude-b.jsonl\",\"timestamp\":\"2026-03-18T10:01:00Z\"}\n"
        ),
    )?;

    let prompts = load_pending_prompts(&repository_root)?;

    assert_eq!(prompts.len(), 2);
    assert_eq!(prompts[0].turn_number, 1);
    assert_eq!(prompts[0].prompt_text, "first prompt");
    assert_eq!(
        prompts[0].transcript_path.as_deref(),
        Some("/tmp/claude-a.jsonl")
    );
    assert_eq!(prompts[1].turn_number, 2);
    assert_eq!(prompts[1].prompt_text, "second prompt");
    assert_eq!(prompts[1].cwd.as_deref(), Some("/repo/tests"));

    std::fs::remove_dir_all(&repository_root)?;
    Ok(())
}

#[test]
fn load_pending_prompts_inherits_last_known_cwd() -> Result<()> {
    let repository_root = std::env::temp_dir().join(format!(
        "sce-hooks-prompts-{}-{}",
        std::process::id(),
        "cwd-inherit"
    ));
    if repository_root.exists() {
        std::fs::remove_dir_all(&repository_root)?;
    }
    std::fs::create_dir_all(&repository_root)?;

    let init = std::process::Command::new("git")
        .args(["init"])
        .current_dir(&repository_root)
        .output()?;
    assert!(init.status.success());

    let prompt_dir = repository_root.join(".git").join("sce");
    std::fs::create_dir_all(&prompt_dir)?;
    std::fs::write(
        prompt_dir.join("prompts.jsonl"),
        concat!(
            "{\"prompt\":\"first prompt\",\"cwd\":\"/repo/src\",\"timestamp\":\"2026-03-18T10:00:00Z\"}\n",
            "{\"prompt\":\"second prompt\",\"timestamp\":\"2026-03-18T10:01:00Z\"}\n"
        ),
    )?;

    let prompts = load_pending_prompts(&repository_root)?;

    assert_eq!(prompts.len(), 2);
    assert_eq!(prompts[1].cwd.as_deref(), Some("/repo/src"));

    std::fs::remove_dir_all(&repository_root)?;
    Ok(())
}

#[test]
fn load_post_commit_prompt_records_computes_tool_counts_durations_and_truncation() -> Result<()> {
    let repository_root = std::env::temp_dir().join(format!(
        "sce-hooks-post-commit-prompts-{}-{}",
        std::process::id(),
        "metrics"
    ));
    if repository_root.exists() {
        std::fs::remove_dir_all(&repository_root)?;
    }
    std::fs::create_dir_all(&repository_root)?;

    let init = std::process::Command::new("git")
        .args(["init"])
        .current_dir(&repository_root)
        .output()?;
    assert!(init.status.success());

    let transcript_path = repository_root.join("claude-session.jsonl");
    std::fs::write(
        &transcript_path,
        concat!(
            "{\"type\":\"assistant\",\"timestamp\":\"2026-03-18T10:00:05.000Z\",\"message\":{\"content\":[{\"type\":\"tool_use\",\"name\":\"Read\"},{\"type\":\"tool_use\",\"name\":\"Edit\"}]}}\n",
            "{\"type\":\"assistant\",\"timestamp\":\"2026-03-18T10:01:05.000Z\",\"message\":{\"content\":[{\"type\":\"tool_use\",\"name\":\"Bash\"}]}}\n"
        ),
    )?;

    let checkpoint_dir = repository_root.join(".git").join("sce");
    std::fs::create_dir_all(&checkpoint_dir)?;
    let long_prompt = "x".repeat(10_500);
    let checkpoint_json = serde_json::json!({
        "harness_type": "claude_code",
        "git_branch": "feature/prompt-capture",
        "model_id": "claude-sonnet-4-20250514",
        "prompts": [
            {
                "turn_number": 1,
                "prompt_text": "first prompt",
                "prompt_length": 12,
                "is_truncated": false,
                "cwd": "/repo",
                "transcript_path": transcript_path,
                "captured_at": "2026-03-18T10:00:00.000Z"
            },
            {
                "turn_number": 2,
                "prompt_text": long_prompt,
                "prompt_length": 10500,
                "is_truncated": false,
                "cwd": "/repo/tests",
                "transcript_path": transcript_path,
                "captured_at": "2026-03-18T10:01:00.000Z"
            }
        ]
    });
    std::fs::write(
        checkpoint_dir.join("pre-commit-checkpoint.json"),
        serde_json::to_vec_pretty(&checkpoint_json)?,
    )?;

    let prompts = load_post_commit_prompt_records(
        &repository_root,
        1_773_828_090_000,
        "2026-03-18T10:01:30+00:00",
    )?;

    assert_eq!(prompts.len(), 2);
    assert_eq!(prompts[0].tool_call_count, 2);
    assert_eq!(prompts[0].duration_ms, 60_000);
    assert_eq!(prompts[1].tool_call_count, 1);
    assert_eq!(prompts[1].duration_ms, 30_000);
    assert!(prompts[1].is_truncated);
    assert_eq!(prompts[1].prompt_length, 10_500);
    assert_eq!(prompts[1].prompt_text.len(), 10_240);

    std::fs::remove_dir_all(&repository_root)?;
    Ok(())
}

#[test]
fn resolve_pre_commit_git_branch_returns_current_branch() -> Result<()> {
    let repository_root = std::env::temp_dir().join(format!(
        "sce-hooks-branch-{}-{}",
        std::process::id(),
        "current"
    ));
    if repository_root.exists() {
        std::fs::remove_dir_all(&repository_root)?;
    }
    std::fs::create_dir_all(&repository_root)?;

    let init = std::process::Command::new("git")
        .args(["init"])
        .current_dir(&repository_root)
        .output()?;
    assert!(init.status.success());

    let checkout = std::process::Command::new("git")
        .args(["checkout", "-b", "feature/prompt-capture"])
        .current_dir(&repository_root)
        .output()?;
    assert!(checkout.status.success());

    let branch = resolve_pre_commit_git_branch(&repository_root)?;

    assert_eq!(branch.as_deref(), Some("feature/prompt-capture"));

    std::fs::remove_dir_all(&repository_root)?;
    Ok(())
}

#[test]
fn prompt_capture_flow_persists_and_queries_end_to_end() -> Result<()> {
    let suffix = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let temp_root = std::env::temp_dir().join(format!("sce-prompt-flow-{suffix}"));
    let repository_root = temp_root.join("repo");
    std::fs::create_dir_all(repository_root.join("src"))?;

    git_ok(&repository_root, &["init"])?;
    git_ok(&repository_root, &["config", "user.name", "SCE Test"])?;
    git_ok(
        &repository_root,
        &["config", "user.email", "sce-test@example.com"],
    )?;
    git_ok(
        &repository_root,
        &["checkout", "-b", "feature/prompt-capture"],
    )?;

    std::fs::write(repository_root.join("src/lib.rs"), "fn main() {}\n")?;
    git_ok(&repository_root, &["add", "src/lib.rs"])?;
    git_ok(&repository_root, &["commit", "-m", "initial"])?;

    std::fs::write(
        repository_root.join("src/lib.rs"),
        "fn main() {\n    println!(\"prompt capture\");\n}\n",
    )?;
    git_ok(&repository_root, &["add", "src/lib.rs"])?;

    let prompt_dir = repository_root.join(".git").join("sce");
    std::fs::create_dir_all(&prompt_dir)?;
    let transcript_path = repository_root.join("claude-session.jsonl");
    std::fs::write(
        &transcript_path,
        concat!(
            "{\"type\":\"assistant\",\"timestamp\":\"2026-03-18T10:00:05.000Z\",\"message\":{\"content\":[{\"type\":\"tool_use\",\"name\":\"Read\"}]}}\n",
            "{\"type\":\"assistant\",\"timestamp\":\"2026-03-18T10:00:45.000Z\",\"message\":{\"content\":[{\"type\":\"tool_use\",\"name\":\"Edit\"},{\"type\":\"tool_use\",\"name\":\"Bash\"}]}}\n"
        ),
    )?;
    std::fs::write(
        prompt_dir.join("prompts.jsonl"),
        format!(
            concat!(
                "{{\"prompt\":\"add prompt persistence\",\"cwd\":\"/repo/src\",\"transcript_path\":\"{}\",\"timestamp\":\"2026-03-18T10:00:00.000Z\"}}\n",
                "{{\"prompt\":\"verify trace query output\",\"cwd\":\"/repo/src\",\"transcript_path\":\"{}\",\"timestamp\":\"2026-03-18T10:00:30.000Z\"}}\n"
            ),
            transcript_path.display(),
            transcript_path.display(),
        ),
    )?;

    let pending = collect_pending_checkpoint(&repository_root)?;
    assert_eq!(pending.prompts.len(), 2);
    let PreCommitFinalization::Finalized(checkpoint) =
        finalize_pre_commit_checkpoint(&sample_runtime(), sample_anchors(), pending)
    else {
        panic!("expected finalized checkpoint");
    };
    write_finalized_checkpoint(&repository_root, &checkpoint)?;

    git_ok(
        &repository_root,
        &["commit", "-m", "persist prompt-capture trace"],
    )?;

    let input = build_post_commit_input(&repository_root)?;
    let commit_sha = input.commit_sha.clone();
    let db_path = temp_root.join("agent-trace.db");
    let runtime = tokio::runtime::Builder::new_current_thread().build()?;
    runtime.block_on(apply_core_schema_migrations(LocalDatabaseTarget::Path(
        &db_path,
    )))?;

    let mut notes_writer = FakeNotesWriter::new(PersistenceWriteResult::Written);
    let mut record_store = LocalDbTraceRecordStore {
        repository_root: repository_root.clone(),
        db_path: db_path.clone(),
    };
    let mut retry_queue = FakeRetryQueue::default();
    let mut emission_ledger = FakeEmissionLedger::default();

    let finalization = finalize_post_commit_trace(
        &sample_post_commit_runtime(),
        input,
        &mut notes_writer,
        &mut record_store,
        &mut retry_queue,
        &mut emission_ledger,
    )?;
    assert!(matches!(finalization, PostCommitFinalization::Persisted(_)));
    assert!(retry_queue.entries.is_empty());

    let short_sha = &commit_sha[..7];
    let text_output =
        render_prompt_trace_for_test(&db_path, &repository_root, short_sha, TraceFormat::Text)?;
    assert!(text_output.contains("Commit:"));
    assert!(text_output.contains("Harness: claude_code"));
    assert!(text_output.contains("Branch: feature/prompt-capture"));
    assert!(text_output.contains("Total prompts: 2"));
    assert!(text_output.contains("add prompt persistence"));
    assert!(text_output.contains("verify trace query output"));

    let json_output =
        render_prompt_trace_for_test(&db_path, &repository_root, &commit_sha, TraceFormat::Json)?;
    let parsed: Value = serde_json::from_str(&json_output)?;
    assert_eq!(parsed["commit"], commit_sha);
    assert_eq!(parsed["prompt_count"], 2);
    assert_eq!(parsed["prompts"][0]["tool_call_count"], 1);
    assert_eq!(parsed["prompts"][0]["duration_ms"], 30_000);
    assert_eq!(parsed["prompts"][1]["tool_call_count"], 2);

    std::fs::remove_dir_all(&temp_root)?;
    Ok(())
}

fn git_ok(repository_root: &Path, args: &[&str]) -> Result<()> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repository_root)
        .output()
        .with_context(|| format!("failed to run git {}", args.join(" ")))?;
    if output.status.success() {
        return Ok(());
    }

    anyhow::bail!(
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr).trim()
    )
}

#[test]
fn checkpoint_has_explicit_sce_attribution_requires_marker() {
    let payload = serde_json::json!({
        "version": 1,
        "anchors": {
            "index_tree": "index-tree-sha",
            "head_tree": "head-tree-sha",
        },
        "files": [
            {
                "path": "src/lib.rs",
                "has_sce_attribution": false,
                "ranges": [
                    {
                        "start_line": 10,
                        "end_line": 20,
                    }
                ],
            }
        ],
    });

    assert!(!checkpoint_has_explicit_sce_attribution(&payload));
}

#[test]
fn checkpoint_has_explicit_sce_attribution_accepts_marked_staged_ranges() {
    let payload = serde_json::json!({
        "version": 1,
        "anchors": {
            "index_tree": "index-tree-sha",
            "head_tree": "head-tree-sha",
        },
        "files": [
            {
                "path": "src/lib.rs",
                "has_sce_attribution": true,
                "ranges": [
                    {
                        "start_line": 10,
                        "end_line": 20,
                    }
                ],
            }
        ],
    });

    assert!(checkpoint_has_explicit_sce_attribution(&payload));
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
        format!("feat: add attribution\n\n{CANONICAL_SCE_COAUTHOR_TRAILER}")
    );
}

#[test]
fn commit_msg_policy_dedupes_existing_canonical_trailers() {
    let message = format!(
        "feat: add attribution\n\n{CANONICAL_SCE_COAUTHOR_TRAILER}\n{CANONICAL_SCE_COAUTHOR_TRAILER}\n"
    );

    let output = apply_commit_msg_coauthor_policy(&sample_commit_msg_runtime(), &message);

    assert_eq!(
        output,
        format!("feat: add attribution\n\n{CANONICAL_SCE_COAUTHOR_TRAILER}\n")
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
