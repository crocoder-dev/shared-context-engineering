//! Top-level `sce sync` command and Agent Trace synchronization service.

#[allow(dead_code)]
pub mod auto_sync;
pub mod command;
pub mod progress;
pub mod render_sync;
#[allow(clippy::module_inception)]
pub mod sync;

pub const NAME: &str = "sync";

/// Internal process-boundary marker used only by the post-commit detached
/// launcher. It is deliberately separate from the user-facing auto-sync
/// configuration setting.
pub(crate) const AUTOMATIC_SYNC_INVOCATION_ENV: &str = "SCE_INTERNAL_AUTO_SYNC";
pub(crate) const AUTOMATIC_SYNC_INVOCATION_VALUE: &str = "1";

use crate::services::output_format::OutputFormat;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncInvocation {
    Manual,
    Automatic,
}

impl SyncInvocation {
    pub(crate) fn from_environment() -> Self {
        Self::from_marker(std::env::var(AUTOMATIC_SYNC_INVOCATION_ENV).ok().as_deref())
    }

    fn from_marker(value: Option<&str>) -> Self {
        match value {
            Some(AUTOMATIC_SYNC_INVOCATION_VALUE) => Self::Automatic,
            _ => Self::Manual,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyncRequest {
    pub format: OutputFormat,
    pub invocation: SyncInvocation,
}

#[cfg(test)]
mod tests {
    use super::SyncInvocation;

    #[test]
    fn automatic_invocation_requires_the_internal_marker_value() {
        assert_eq!(
            SyncInvocation::from_marker(Some("1")),
            SyncInvocation::Automatic
        );
        assert_eq!(
            SyncInvocation::from_marker(Some("true")),
            SyncInvocation::Manual
        );
        assert_eq!(SyncInvocation::from_marker(None), SyncInvocation::Manual);
    }
}
