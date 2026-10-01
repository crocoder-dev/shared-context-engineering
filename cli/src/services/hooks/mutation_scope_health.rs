use crate::services::mutation_trace::types::ActorKind;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code)]
pub(crate) enum MutationScopeHealthStatus {
    Healthy,
    Recovering,
    Blocked,
    Invalid,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code)]
pub(crate) enum Repairability {
    AutoFixable,
    ManualOnly,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(dead_code)]
pub(crate) struct MutationScopeAdapterHealth {
    pub(crate) adapter: ActorKind,
    pub(crate) status: MutationScopeHealthStatus,
    pub(crate) reason: String,
    pub(crate) detail: Option<String>,
}

#[allow(dead_code)]
impl MutationScopeAdapterHealth {
    pub(crate) fn new(
        adapter: ActorKind,
        status: MutationScopeHealthStatus,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            adapter,
            status,
            reason: reason.into(),
            detail: None,
        }
    }

    pub(crate) fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }
}
