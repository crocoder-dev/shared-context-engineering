use anyhow::Result;

use crate::app::AppContext;
use crate::services::doctor::types::{DoctorFixResultRecord, DoctorProblem};
use crate::services::setup::{RequiredHooksInstallOutcome, SetupInstallOutcome};

pub type LifecycleProvider = Box<dyn ServiceLifecycle>;

#[allow(dead_code)]
pub type HealthProblem = DoctorProblem;

#[allow(dead_code)]
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SetupOutcome {
    pub setup_install: Option<SetupInstallOutcome>,
    pub required_hooks_install: Option<RequiredHooksInstallOutcome>,
}

#[allow(dead_code)]
pub trait ServiceLifecycle: Send + Sync {
    fn diagnose(&self, _ctx: &AppContext) -> Vec<HealthProblem> {
        Vec::new()
    }

    fn fix(&self, _ctx: &AppContext, _problems: &[HealthProblem]) -> Vec<DoctorFixResultRecord> {
        Vec::new()
    }

    fn setup(&self, _ctx: &AppContext) -> Result<SetupOutcome> {
        Ok(SetupOutcome::default())
    }
}

/// Returns lifecycle providers in deterministic orchestration order.
///
/// Provider order is config → `local_db` → hooks when hook lifecycle behavior is requested.
pub fn lifecycle_providers(include_hooks: bool) -> Vec<LifecycleProvider> {
    let mut providers: Vec<LifecycleProvider> = vec![
        Box::new(crate::services::config::lifecycle::ConfigLifecycle),
        Box::new(crate::services::local_db::lifecycle::LocalDbLifecycle),
    ];

    if include_hooks {
        providers.push(Box::new(crate::services::hooks::lifecycle::HooksLifecycle));
    }

    providers
}
