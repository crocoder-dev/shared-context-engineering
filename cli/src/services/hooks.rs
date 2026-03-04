use anyhow::Result;

pub const NAME: &str = "hooks";

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
            supported_hooks: vec![GitHookKind::PreCommit, GitHookKind::PrePush],
            generated_region_tracking: true,
        }
    }

    fn record(&self, event: HookEvent) -> Result<()> {
        match event.hook {
            GitHookKind::PreCommit | GitHookKind::PrePush => {}
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
        "TODO: '{NAME}' is planned and not implemented yet. Hook event model reserves {} git hook(s) with generated-region tracking placeholders and staged-only pre-commit checkpoint preview over {} file(s).",
        model.supported_hooks.len(),
        staged_file_count
    ))
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::{
        finalize_pre_commit_checkpoint, run_placeholder_hooks, GeneratedRegionEvent,
        GeneratedRegionLifecycle, GitHookKind, HookEvent, HookService, PendingCheckpoint,
        PendingFileCheckpoint, PendingLineRange, PlaceholderHookService, PreCommitFinalization,
        PreCommitNoOpReason, PreCommitRuntimeState, PreCommitTreeAnchors,
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
    fn hooks_placeholder_event_model_reserves_generated_region_tracking() {
        let service = PlaceholderHookService;
        let model = service.event_model();
        assert!(model.generated_region_tracking);
        assert_eq!(model.supported_hooks.len(), 2);
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
