use crate::services::hooks_lifecycle::{HooksLifecycleService, HOOKS_SERVICE_ID};
use crate::services::lifecycle::{DiagnosticLifecycle, FixLifecycle, ServiceId, SetupLifecycle};
use crate::services::local_db_lifecycle::{LocalDbLifecycleService, LOCAL_DB_SERVICE_ID};

static HOOKS_LIFECYCLE_SERVICE: HooksLifecycleService = HooksLifecycleService;
static LOCAL_DB_LIFECYCLE_SERVICE: LocalDbLifecycleService = LocalDbLifecycleService;

#[derive(Clone, Copy, Debug, Default)]
pub struct LifecycleRegistry;

impl LifecycleRegistry {
    pub fn setup_lifecycle(service_id: ServiceId) -> Option<&'static dyn SetupLifecycle> {
        match service_id {
            HOOKS_SERVICE_ID => Some(&HOOKS_LIFECYCLE_SERVICE),
            _ => None,
        }
    }

    pub fn diagnostic_lifecycle(service_id: ServiceId) -> Option<&'static dyn DiagnosticLifecycle> {
        match service_id {
            HOOKS_SERVICE_ID => Some(&HOOKS_LIFECYCLE_SERVICE),
            LOCAL_DB_SERVICE_ID => Some(&LOCAL_DB_LIFECYCLE_SERVICE),
            _ => None,
        }
    }

    pub fn fix_lifecycle(service_id: ServiceId) -> Option<&'static dyn FixLifecycle> {
        match service_id {
            HOOKS_SERVICE_ID => Some(&HOOKS_LIFECYCLE_SERVICE),
            LOCAL_DB_SERVICE_ID => Some(&LOCAL_DB_LIFECYCLE_SERVICE),
            _ => None,
        }
    }
}
