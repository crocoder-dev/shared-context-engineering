//! `RecordIntegrationInstallation` use case: expands a target selection into
//! concrete targets and records their installation, delegating to an injected
//! `IntegrationConfigRepository`.

use std::path::PathBuf;

use crate::application::ports::integration_config_repository::IntegrationConfigRepository;
use crate::domain::integration::IntegrationTargetSelection;

/// The repository root, target selection, and optional-workflow selection to
/// record as installed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RecordIntegrationInstallationRequest {
    pub(crate) repository_root: PathBuf,
    pub(crate) selection: IntegrationTargetSelection,
    pub(crate) optional_workflows: Vec<String>,
}

/// Expands a target selection into concrete targets and records their
/// installation, via an injected `IntegrationConfigRepository`.
pub(crate) struct RecordIntegrationInstallation<R: IntegrationConfigRepository> {
    repository: R,
}

impl<R: IntegrationConfigRepository> RecordIntegrationInstallation<R> {
    pub(crate) fn new(repository: R) -> Self {
        Self { repository }
    }

    pub(crate) fn execute(
        &self,
        request: &RecordIntegrationInstallationRequest,
    ) -> Result<(), R::Error> {
        self.repository.record_installation(
            &request.repository_root,
            request.selection.targets(),
            &request.optional_workflows,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::path::Path;

    use crate::domain::integration::IntegrationTarget;

    type RecordInstallationCall = (PathBuf, Vec<IntegrationTarget>, Vec<String>);

    #[derive(Default)]
    struct FakeRepository {
        record_installation_calls: RefCell<Vec<RecordInstallationCall>>,
        record_installation_error: Option<&'static str>,
    }

    impl IntegrationConfigRepository for FakeRepository {
        type Error = &'static str;

        fn ensure_exists(&self, _repository_root: &Path) -> Result<(), Self::Error> {
            unreachable!("not exercised by RecordIntegrationInstallation")
        }

        fn load_optional_workflows(
            &self,
            _repository_root: &Path,
        ) -> Result<Vec<String>, Self::Error> {
            unreachable!("not exercised by RecordIntegrationInstallation")
        }

        fn record_installation(
            &self,
            repository_root: &Path,
            targets: &[IntegrationTarget],
            optional_workflows: &[String],
        ) -> Result<(), Self::Error> {
            self.record_installation_calls.borrow_mut().push((
                repository_root.to_path_buf(),
                targets.to_vec(),
                optional_workflows.to_vec(),
            ));

            self.record_installation_error.map_or(Ok(()), Err)
        }
    }

    #[test]
    fn one_selection_records_a_single_concrete_target() {
        let repository = FakeRepository::default();
        let use_case = RecordIntegrationInstallation::new(repository);
        let repository_root = PathBuf::from("/repo");
        let optional_workflows = vec!["research".to_string()];

        use_case
            .execute(&RecordIntegrationInstallationRequest {
                repository_root: repository_root.clone(),
                selection: IntegrationTargetSelection::One(IntegrationTarget::Claude),
                optional_workflows: optional_workflows.clone(),
            })
            .unwrap();

        let calls = use_case.repository.record_installation_calls.borrow();
        assert_eq!(
            calls.as_slice(),
            [(
                repository_root,
                vec![IntegrationTarget::Claude],
                optional_workflows
            )]
        );
    }

    #[test]
    fn all_selection_records_every_target_in_order() {
        let repository = FakeRepository::default();
        let use_case = RecordIntegrationInstallation::new(repository);
        let repository_root = PathBuf::from("/repo");

        use_case
            .execute(&RecordIntegrationInstallationRequest {
                repository_root: repository_root.clone(),
                selection: IntegrationTargetSelection::All,
                optional_workflows: Vec::new(),
            })
            .unwrap();

        let calls = use_case.repository.record_installation_calls.borrow();
        assert_eq!(
            calls.as_slice(),
            [(
                repository_root,
                vec![
                    IntegrationTarget::OpenCode,
                    IntegrationTarget::Claude,
                    IntegrationTarget::Pi,
                ],
                Vec::new()
            )]
        );
    }

    #[test]
    fn execute_forwards_the_optional_workflow_slice_unchanged() {
        let repository = FakeRepository::default();
        let use_case = RecordIntegrationInstallation::new(repository);
        let optional_workflows = vec!["research".to_string(), "docs".to_string()];

        use_case
            .execute(&RecordIntegrationInstallationRequest {
                repository_root: PathBuf::from("/repo"),
                selection: IntegrationTargetSelection::One(IntegrationTarget::OpenCode),
                optional_workflows: optional_workflows.clone(),
            })
            .unwrap();

        let calls = use_case.repository.record_installation_calls.borrow();
        assert_eq!(calls[0].2, optional_workflows);
    }

    #[test]
    fn execute_propagates_repository_errors() {
        let repository = FakeRepository {
            record_installation_calls: RefCell::new(Vec::new()),
            record_installation_error: Some("record_installation failed"),
        };
        let use_case = RecordIntegrationInstallation::new(repository);

        let result = use_case.execute(&RecordIntegrationInstallationRequest {
            repository_root: PathBuf::from("/repo"),
            selection: IntegrationTargetSelection::All,
            optional_workflows: Vec::new(),
        });

        assert_eq!(result, Err("record_installation failed"));
    }
}
