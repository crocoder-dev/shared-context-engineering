use anyhow::Result;

pub const NAME: &str = "setup";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SetupRequest {
    pub repository_root: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SetupPlan {
    pub tasks: Vec<&'static str>,
    pub ready_for_execution: bool,
}

pub trait SetupService {
    fn plan(&self, request: &SetupRequest) -> SetupPlan;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PlaceholderSetupService;

impl SetupService for PlaceholderSetupService {
    fn plan(&self, _request: &SetupRequest) -> SetupPlan {
        SetupPlan {
            tasks: vec![
                "Validate repository shape",
                "Initialize local development prerequisites",
                "Persist setup state for future runs",
            ],
            ready_for_execution: false,
        }
    }
}

pub fn run_placeholder_setup() -> Result<String> {
    let service = PlaceholderSetupService;
    let request = SetupRequest {
        repository_root: ".".to_string(),
    };
    let plan = service.plan(&request);

    Ok(format!(
        "TODO: '{NAME}' is planned and not implemented yet. Setup plan scaffolded with {} deferred step(s).",
        plan.tasks.len()
    ))
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::{run_placeholder_setup, PlaceholderSetupService, SetupRequest, SetupService};

    #[test]
    fn setup_placeholder_service_exposes_deferred_plan() {
        let service = PlaceholderSetupService;
        let plan = service.plan(&SetupRequest {
            repository_root: ".".to_string(),
        });

        assert_eq!(plan.tasks.len(), 3);
        assert!(!plan.ready_for_execution);
    }

    #[test]
    fn setup_placeholder_message_mentions_scaffolded_plan() -> Result<()> {
        let message = run_placeholder_setup()?;
        assert!(message.contains("Setup plan scaffolded"));
        Ok(())
    }
}
