//! `LoadPersistedOptionalWorkflows` use case: reads the optional workflows
//! currently recorded in repo-local integration configuration, delegating to
//! an injected `IntegrationConfigRepository`.

use std::path::PathBuf;

use crate::application::ports::integration_config_repository::IntegrationConfigRepository;

/// The repository root to load persisted optional workflows from.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LoadPersistedOptionalWorkflowsRequest {
    pub(crate) repository_root: PathBuf,
}

/// Reads the optional workflows currently recorded in repo-local integration
/// configuration, via an injected `IntegrationConfigRepository`.
pub(crate) struct LoadPersistedOptionalWorkflows<R: IntegrationConfigRepository> {
    repository: R,
}

impl<R: IntegrationConfigRepository> LoadPersistedOptionalWorkflows<R> {
    pub(crate) fn new(repository: R) -> Self {
        Self { repository }
    }

    pub(crate) fn execute(
        &self,
        request: &LoadPersistedOptionalWorkflowsRequest,
    ) -> Result<Vec<String>, R::Error> {
        self.repository
            .load_optional_workflows(&request.repository_root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::path::Path;

    use crate::domain::integration::IntegrationTarget;

    struct FakeRepository {
        load_optional_workflows_calls: RefCell<Vec<PathBuf>>,
        load_optional_workflows_result: Result<Vec<String>, &'static str>,
    }

    impl IntegrationConfigRepository for FakeRepository {
        type Error = &'static str;

        fn ensure_exists(&self, _repository_root: &Path) -> Result<(), Self::Error> {
            unreachable!("not exercised by LoadPersistedOptionalWorkflows")
        }

        fn load_optional_workflows(
            &self,
            repository_root: &Path,
        ) -> Result<Vec<String>, Self::Error> {
            self.load_optional_workflows_calls
                .borrow_mut()
                .push(repository_root.to_path_buf());

            self.load_optional_workflows_result.clone()
        }

        fn record_installation(
            &self,
            _repository_root: &Path,
            _targets: &[IntegrationTarget],
            _optional_workflows: &[String],
        ) -> Result<(), Self::Error> {
            unreachable!("not exercised by LoadPersistedOptionalWorkflows")
        }
    }

    #[test]
    fn execute_returns_the_repositorys_workflows_unchanged() {
        let repository = FakeRepository {
            load_optional_workflows_calls: RefCell::new(Vec::new()),
            load_optional_workflows_result: Ok(vec!["research".to_string(), "docs".to_string()]),
        };
        let use_case = LoadPersistedOptionalWorkflows::new(repository);
        let repository_root = PathBuf::from("/repo");

        let workflows = use_case
            .execute(&LoadPersistedOptionalWorkflowsRequest {
                repository_root: repository_root.clone(),
            })
            .unwrap();

        assert_eq!(workflows, vec!["research".to_string(), "docs".to_string()]);
        assert_eq!(
            use_case
                .repository
                .load_optional_workflows_calls
                .borrow()
                .as_slice(),
            [repository_root]
        );
    }

    #[test]
    fn execute_propagates_repository_errors_unchanged() {
        let repository = FakeRepository {
            load_optional_workflows_calls: RefCell::new(Vec::new()),
            load_optional_workflows_result: Err("load failed"),
        };
        let use_case = LoadPersistedOptionalWorkflows::new(repository);

        let result = use_case.execute(&LoadPersistedOptionalWorkflowsRequest {
            repository_root: PathBuf::from("/repo"),
        });

        assert_eq!(result, Err("load failed"));
    }
}
