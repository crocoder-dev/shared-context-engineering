use anyhow::Result;

pub const NAME: &str = "hooks";

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
        "TODO: '{NAME}' is planned and not implemented yet. Hook event model reserves {} git hook(s) with generated-region tracking placeholders.",
        model.supported_hooks.len()
    ))
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::{
        run_placeholder_hooks, GeneratedRegionEvent, GeneratedRegionLifecycle, GitHookKind,
        HookEvent, HookService, PlaceholderHookService,
    };

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
