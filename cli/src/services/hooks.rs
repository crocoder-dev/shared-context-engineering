use anyhow::Result;

pub const NAME: &str = "hooks";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GitHookKind {
    PreCommit,
    PrePush,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code)]
pub enum GeneratedRegionLifecycle {
    Discovered,
    Updated,
    Removed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(dead_code)]
pub struct GeneratedRegionEvent {
    pub file_path: String,
    pub marker_id: String,
    pub lifecycle: GeneratedRegionLifecycle,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(dead_code)]
pub struct HookEvent {
    pub hook: GitHookKind,
    pub region_event: Option<GeneratedRegionEvent>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HookEventModel {
    pub supported_hooks: Vec<GitHookKind>,
    pub generated_region_tracking: bool,
}

#[allow(dead_code)]
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

    fn record(&self, _event: HookEvent) -> Result<()> {
        Ok(())
    }
}

pub fn run_placeholder_hooks() -> Result<String> {
    let service = PlaceholderHookService;
    let model = service.event_model();
    Ok(format!(
        "TODO: '{NAME}' is planned and not implemented yet. Hook event model reserves {} git hook(s) with generated-region tracking placeholders.",
        model.supported_hooks.len()
    ))
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::{run_placeholder_hooks, HookService, PlaceholderHookService};

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
}
