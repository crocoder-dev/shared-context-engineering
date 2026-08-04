//! Integration targets and target selection for embedded-asset installation.

/// A concrete integration target an outbound adapter may install assets for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum IntegrationTarget {
    OpenCode,
    Claude,
    Pi,
}

const ALL_TARGETS: [IntegrationTarget; 3] = [
    IntegrationTarget::OpenCode,
    IntegrationTarget::Claude,
    IntegrationTarget::Pi,
];

impl IntegrationTarget {
    /// The canonical `integrations.target` identifier persisted for this
    /// target in repo-local `.sce/config.json`.
    pub(crate) fn config_id(self) -> &'static str {
        match self {
            Self::OpenCode => "opencode",
            Self::Claude => "claude",
            Self::Pi => "pi",
        }
    }
}

/// A caller's target selection: a single target, or all of them.
///
/// `All` is expanded into concrete `IntegrationTarget` values via
/// [`IntegrationTargetSelection::targets`] before any application port is
/// invoked, so no outbound adapter is ever called with a meta value
/// representing "all targets".
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum IntegrationTargetSelection {
    One(IntegrationTarget),
    All,
}

impl IntegrationTargetSelection {
    /// The concrete targets this selection expands to.
    ///
    /// `One` yields the single wrapped target; `All` yields
    /// `[OpenCode, Claude, Pi]` in that order.
    pub(crate) fn targets(&self) -> &[IntegrationTarget] {
        match self {
            Self::One(target) => std::slice::from_ref(target),
            Self::All => &ALL_TARGETS,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_selection_targets_wraps_the_single_target() {
        let selection = IntegrationTargetSelection::One(IntegrationTarget::Claude);

        assert_eq!(selection.targets(), &[IntegrationTarget::Claude]);
    }

    #[test]
    fn all_selection_targets_returns_every_target_in_order() {
        let selection = IntegrationTargetSelection::All;

        assert_eq!(
            selection.targets(),
            &[
                IntegrationTarget::OpenCode,
                IntegrationTarget::Claude,
                IntegrationTarget::Pi,
            ]
        );
    }

    #[test]
    fn config_id_maps_each_concrete_target() {
        assert_eq!(IntegrationTarget::OpenCode.config_id(), "opencode");
        assert_eq!(IntegrationTarget::Claude.config_id(), "claude");
        assert_eq!(IntegrationTarget::Pi.config_id(), "pi");
    }
}
