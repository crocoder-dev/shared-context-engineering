use anyhow::Result;

use crate::services::agent_trace::{
    build_trace_payload, AgentTraceRecord, FileAttributionInput, QualityStatus, TraceAdapterInput,
    TRACE_CONTENT_TYPE,
};

pub const NAME: &str = "hooks";
pub const CANONICAL_SCE_COAUTHOR_TRAILER: &str = "Co-authored-by: SCE <sce@crocoder.dev>";
pub const POST_COMMIT_PARENT_SHA_METADATA_KEY: &str = "dev.crocoder.sce.parent_revision";

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
mod tests {
    use anyhow::Result;

    use crate::services::agent_trace::{
        ContributorInput, ContributorType, ConversationInput, FileAttributionInput, RangeInput,
    };

    use super::{
        apply_commit_msg_coauthor_policy, finalize_post_commit_trace, finalize_post_rewrite_remap,
        finalize_pre_commit_checkpoint, run_placeholder_hooks, CommitMsgRuntimeState,
        GeneratedRegionEvent, GeneratedRegionLifecycle, GitHookKind, HookEvent, HookService,
        PendingCheckpoint, PendingFileCheckpoint, PendingLineRange, PersistenceErrorClass,
        PersistenceFailure, PersistenceTarget, PersistenceWriteResult, PlaceholderHookService,
        PostCommitFinalization, PostCommitInput, PostCommitNoOpReason, PostCommitRuntimeState,
        PostRewriteFinalization, PostRewriteNoOpReason, PostRewriteRuntimeState,
        PreCommitFinalization, PreCommitNoOpReason, PreCommitRuntimeState, PreCommitTreeAnchors,
        RewriteMethod, RewriteRemapIngestion, RewriteRemapRequest, TraceEmissionLedger, TraceNote,
        TraceNotesWriter, TraceRecordStore, TraceRetryQueue, TraceRetryQueueEntry,
        CANONICAL_SCE_COAUTHOR_TRAILER, POST_COMMIT_PARENT_SHA_METADATA_KEY,
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
    fn post_rewrite_finalization_noops_when_sce_disabled() -> Result<()> {
        let mut runtime = sample_post_rewrite_runtime();
        runtime.sce_disabled = true;
        let mut ingestion = FakeRewriteRemapIngestion::default();

        let outcome =
            finalize_post_rewrite_remap(&runtime, "amend", "old1 new1\n", &mut ingestion)?;

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

        let error =
            finalize_post_rewrite_remap(&runtime, "amend", "missing_new_sha\n", &mut ingestion)
                .expect_err("invalid pair format should return error");

        assert!(error.to_string().contains("expected '<old_sha> <new_sha>'"));
        assert!(ingestion.seen_requests.is_empty());
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
}
