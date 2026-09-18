use crate::services::mutation_trace::types::ActorKind;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code)]
pub(crate) enum MutationScopeHealthStatus {
    Healthy,
    Recovering,
    Blocked,
    Invalid,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_status_variants_are_distinct() {
        assert_ne!(
            MutationScopeHealthStatus::Healthy,
            MutationScopeHealthStatus::Recovering
        );
        assert_ne!(
            MutationScopeHealthStatus::Recovering,
            MutationScopeHealthStatus::Blocked
        );
        assert_ne!(
            MutationScopeHealthStatus::Blocked,
            MutationScopeHealthStatus::Invalid
        );
        assert_eq!(
            MutationScopeHealthStatus::Healthy,
            MutationScopeHealthStatus::Healthy
        );
    }

    #[test]
    fn new_adapter_health_has_no_detail_by_default() {
        let health = MutationScopeAdapterHealth::new(
            ActorKind::ClaudeCode,
            MutationScopeHealthStatus::Healthy,
            "no persisted recovery problem",
        );

        assert_eq!(health.adapter, ActorKind::ClaudeCode);
        assert_eq!(health.status, MutationScopeHealthStatus::Healthy);
        assert_eq!(health.reason, "no persisted recovery problem");
        assert_eq!(health.detail, None);
    }

    #[test]
    fn with_detail_attaches_machine_detail_without_changing_status_or_reason() {
        let health = MutationScopeAdapterHealth::new(
            ActorKind::Codex,
            MutationScopeHealthStatus::Invalid,
            "state file failed to parse",
        )
        .with_detail("unexpected EOF at byte 12");

        assert_eq!(health.adapter, ActorKind::Codex);
        assert_eq!(health.status, MutationScopeHealthStatus::Invalid);
        assert_eq!(health.reason, "state file failed to parse");
        assert_eq!(health.detail.as_deref(), Some("unexpected EOF at byte 12"));
    }
}
