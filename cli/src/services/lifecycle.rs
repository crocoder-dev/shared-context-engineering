use anyhow::Result;

use crate::app::AppContext;
use crate::services::doctor::types::{DoctorFixResultRecord, DoctorProblem};
use crate::services::setup::{RequiredHooksInstallOutcome, SetupInstallOutcome};

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

    fn fix(
        &self,
        _ctx: &AppContext,
        _problems: &[HealthProblem],
    ) -> Vec<DoctorFixResultRecord> {
        Vec::new()
    }

    fn setup(&self, _ctx: &AppContext) -> Result<SetupOutcome> {
        Ok(SetupOutcome::default())
    }
}
